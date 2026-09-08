//! Representation-only lowering from concrete canonical HIR to the existing
//! machine-independent systems IR.
//!
//! This layer must not resolve source names, infer types, validate ownership,
//! decide generic arguments, or recompute aggregate layouts.

use crate::diagnostics::{Diagnostic, DiagnosticCategory, NativeResult};
use crate::ir::{CompareOp, LabelId, LocalId as NativeLocalId, StringId};
use crate::system_types::SystemType;
use crate::systems_ir::{
    ExternalFunction, ExternalGlobal, GlobalSection, SystemIntrinsic, SystemsFunction,
    SystemsGlobal, SystemsInst, SystemsModuleIr, SystemsParam, SystemsSlot, Visibility,
};
use dbyte_ast::{BinOp, CaptureMode, FunctionKind, Span, UnaryOp};
use dbyte_hir::{
    collect_called_instances_block, layout_of, CanonicalType, ConcreteHirFunction,
    ConcreteHirProgram, HirBlock, HirExpr, HirExprKind, HirIntrinsic, HirMatchArm, HirPlace,
    HirPlaceBase, HirProjection, HirStatement, IntegerType, TypeId,
};
use std::collections::{BTreeMap, BTreeSet};

type ModuleSelection<'a> = (
    dbyte_hir::ModuleId,
    &'a BTreeMap<dbyte_hir::FunctionInstanceKey, dbyte_hir::ModuleId>,
    &'a BTreeMap<dbyte_hir::SymbolId, dbyte_hir::ModuleId>,
);

pub fn lower_hir_module_to_native_ir(
    module_name: &str,
    program: &ConcreteHirProgram,
) -> NativeResult<SystemsModuleIr> {
    lower_selected_hir_module_to_native_ir(module_name, program, None)
}

pub fn lower_hir_project_to_native_ir(
    project: &dbyte_hir::CanonicalHirProject,
    concrete: &ConcreteHirProgram,
    module_names: &BTreeMap<dbyte_hir::ModuleId, String>,
) -> NativeResult<Vec<(dbyte_hir::ModuleId, SystemsModuleIr)>> {
    let mut concrete = concrete.clone();
    let instance_owners = concrete
        .functions
        .iter()
        .map(|function| {
            let owner = if function.key.arguments.is_empty() {
                project.symbol_owners[&function.key.symbol]
            } else {
                project.root
            };
            (function.key.clone(), owner)
        })
        .collect::<BTreeMap<_, _>>();
    for function in &mut concrete.functions {
        let source = project
            .link_names
            .get(&function.key.symbol)
            .ok_or_else(|| {
                Diagnostic::without_span(
                    DiagnosticCategory::InvalidModuleImport,
                    "concrete function has no canonical module ownership",
                )
            })?;
        let owner = project.symbol_owners[&function.key.symbol];
        let module_name = module_names.get(&owner).ok_or_else(|| {
            Diagnostic::without_span(
                DiagnosticCategory::InvalidModuleImport,
                "function owner has no module name",
            )
        })?;
        let base = if function.public
            || (function.kind == dbyte_ast::FunctionKind::Drop
                && function.drop_target.is_some_and(|target| {
                    concrete
                        .semantic
                        .definitions
                        .iter()
                        .any(|definition| definition.symbol == target && definition.public)
                }))
            || function.convention == dbyte_ast::CallConvention::C
            || (owner == project.root && (source == "kernel_main" || source == "user_main"))
        {
            source.clone()
        } else {
            format!("__dbyte_{module_name}_{source}")
        };
        function.symbol_name = if function.key.arguments.is_empty() {
            base
        } else {
            let mut name = format!("__dbyte_mono_{module_name}_{source}");
            for argument in &function.key.arguments {
                name.push_str(&format!("_t{}", argument.0));
            }
            name
        };
    }
    for global in &mut concrete.globals {
        let source = project
            .link_names
            .get(&global.symbol)
            .cloned()
            .ok_or_else(|| {
                Diagnostic::without_span(
                    DiagnosticCategory::InvalidModuleImport,
                    "concrete global has no canonical module ownership",
                )
            })?;
        let owner = project.symbol_owners[&global.symbol];
        let module_name = &module_names[&owner];
        global.name = if global.public {
            source
        } else {
            format!("__dbyte_{module_name}_{source}")
        };
    }
    project
        .module_order
        .iter()
        .map(|module| {
            let name = module_names.get(module).ok_or_else(|| {
                Diagnostic::without_span(
                    DiagnosticCategory::InvalidModuleImport,
                    format!("module m{} has no canonical name", module.0),
                )
            })?;
            let ir = lower_selected_hir_module_to_native_ir(
                name,
                &concrete,
                Some((*module, &instance_owners, &project.symbol_owners)),
            )?;
            Ok((*module, ir))
        })
        .collect()
}

fn lower_selected_hir_module_to_native_ir(
    module_name: &str,
    program: &ConcreteHirProgram,
    selection: Option<ModuleSelection<'_>>,
) -> NativeResult<SystemsModuleIr> {
    dbyte_hir::validate_profile(program, program.semantic.profile).map_err(hir_diagnostic)?;
    let symbols: BTreeMap<_, _> = program
        .functions
        .iter()
        .map(|function| (function.key.clone(), function.symbol_name.clone()))
        .collect();
    // A consumer-owned generic specialization can legitimately call a
    // source-private helper in the generic's defining module. Such a helper
    // remains source-private, but its deterministic module-qualified symbol
    // must have global object binding so the specialization can resolve it
    // across ET_REL objects.
    let mut cross_object_helpers = BTreeSet::new();
    if let Some((_, owners, _)) = selection {
        for caller in program
            .functions
            .iter()
            .filter(|function| !function.declaration_only)
        {
            let Some(caller_owner) = owners.get(&caller.key) else {
                continue;
            };
            let mut callees = BTreeSet::new();
            collect_called_instances_block(&caller.body, &mut callees);
            for callee in callees {
                if owners
                    .get(&callee)
                    .is_some_and(|owner| owner != caller_owner)
                {
                    if let Some(symbol) = symbols.get(&callee) {
                        cross_object_helpers.insert(symbol.clone());
                    }
                }
            }
        }
    }
    // Preserve the frozen native string-pool contract independently of the
    // monomorphization worklist order. Concrete functions are intentionally
    // keyed by canonical identities, while legacy objects interned literals
    // in source order. Pre-interning by canonical source span keeps old object
    // bytes stable and remains deterministic for specializations.
    let mut string_literals = Vec::new();
    for function in program.functions.iter().filter(|function| {
        !function.declaration_only
            && selection
                .is_none_or(|(module, owners, _)| owners.get(&function.key) == Some(&module))
    }) {
        collect_string_literals_block(&function.body, &mut string_literals);
    }
    string_literals.sort_by(|left, right| {
        (left.0.line, left.0.col, &left.1).cmp(&(right.0.line, right.0.col, &right.1))
    });
    let mut strings = Vec::<Vec<u8>>::new();
    for (_, bytes) in string_literals {
        if !strings.contains(&bytes) {
            strings.push(bytes);
        }
    }
    let selected_functions = program
        .functions
        .iter()
        .filter(|function| {
            !function.declaration_only
                && selection
                    .is_none_or(|(module, owners, _)| owners.get(&function.key) == Some(&module))
        })
        .collect::<Vec<_>>();
    let runtime_drop_enabled = program
        .semantic
        .functions
        .iter()
        .any(|function| function.kind == FunctionKind::Drop);
    let mut closure_infos = Vec::new();
    for function in &selected_functions {
        collect_closures_block(&function.body, function, &mut closure_infos);
    }
    closure_infos.sort_by_key(|closure| closure.id);
    let closure_symbols = closure_infos
        .iter()
        .map(|closure| {
            (
                closure.id,
                format!("__dbyte_closure_{module_name}_c{}", closure.id.0),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut functions = Vec::with_capacity(program.functions.len() + closure_infos.len());
    for function in selected_functions {
        functions.push(
            FunctionLowerer::new(
                program,
                function,
                &symbols,
                &closure_symbols,
                &mut strings,
                runtime_drop_enabled,
            )?
            .lower()?,
        );
    }
    for closure in &closure_infos {
        functions.push(
            FunctionLowerer::new_closure(
                program,
                closure,
                &symbols,
                &closure_symbols,
                &mut strings,
                runtime_drop_enabled,
            )?
            .lower()?,
        );
    }
    for function in &mut functions {
        if cross_object_helpers.contains(&function.symbol) {
            function.visibility = Visibility::Public;
        }
    }
    let mut referenced_functions = BTreeSet::new();
    let mut referenced_globals = BTreeSet::new();
    for instruction in functions
        .iter()
        .flat_map(|function| function.instructions.iter())
    {
        match instruction {
            SystemsInst::Call { symbol, .. }
            | SystemsInst::CallPacked { symbol, .. }
            | SystemsInst::FunctionAddress { symbol }
            | SystemsInst::InterruptAddress { symbol, .. }
            | SystemsInst::TaskAddress { symbol } => {
                referenced_functions.insert(symbol.clone());
            }
            SystemsInst::GlobalAddress { symbol, .. } => {
                referenced_globals.insert(symbol.clone());
            }
            _ => {}
        }
    }
    let external_functions = program
        .functions
        .iter()
        .filter(|function| {
            referenced_functions.contains(&function.symbol_name)
                && (function.declaration_only
                    || selection.is_some_and(|(module, owners, _)| {
                        owners.get(&function.key) != Some(&module)
                    }))
        })
        .map(|function| {
            Ok(ExternalFunction {
                symbol: function.symbol_name.clone(),
                parameter_types: function
                    .parameters
                    .iter()
                    .map(|parameter| lower_type(program, parameter.ty, parameter.span))
                    .collect::<NativeResult<_>>()?,
                return_type: lower_type(program, function.result, function.span)?,
                call_convention: function.convention,
            })
        })
        .collect::<NativeResult<Vec<_>>>()?;
    let globals = program
        .globals
        .iter()
        .filter(|global| {
            !global.constant
                && selection
                    .is_none_or(|(module, _, owners)| owners.get(&global.symbol) == Some(&module))
        })
        .map(|global| lower_global(program, global))
        .collect::<NativeResult<Vec<_>>>()?;
    let external_globals = program
        .globals
        .iter()
        .filter(|global| {
            referenced_globals.contains(&global.name)
                && !global.constant
                && selection
                    .is_some_and(|(module, _, owners)| owners.get(&global.symbol) != Some(&module))
        })
        .map(|global| {
            Ok(ExternalGlobal {
                symbol: global.name.clone(),
                ty: lower_type(program, global.ty, global.span)?,
                mutable: global.mutable,
            })
        })
        .collect::<NativeResult<Vec<_>>>()?;
    Ok(SystemsModuleIr {
        module_name: module_name.to_owned(),
        user_profile: program.semantic.profile == dbyte_hir::Profile::I686User,
        functions,
        globals,
        strings,
        external_functions,
        external_globals,
    })
}

fn collect_string_literals_block(block: &HirBlock, output: &mut Vec<(Span, Vec<u8>)>) {
    for statement in &block.statements {
        match statement {
            HirStatement::Let { value, .. } | HirStatement::Expression(value) => {
                collect_string_literals_expr(value, output)
            }
            HirStatement::Assign {
                value,
                cleanup_before_commit,
                ..
            } => {
                collect_string_literals_expr(value, output);
                if let Some(cleanup) = cleanup_before_commit {
                    collect_string_literals_block(cleanup, output);
                }
            }
            HirStatement::Return { value, cleanup, .. } => {
                if let Some(value) = value {
                    collect_string_literals_expr(value, output);
                }
                for cleanup in cleanup {
                    collect_string_literals_block(cleanup, output);
                }
            }
            HirStatement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                collect_string_literals_expr(condition, output);
                collect_string_literals_block(then_block, output);
                if let Some(else_block) = else_block {
                    collect_string_literals_block(else_block, output);
                }
            }
            HirStatement::While {
                condition, body, ..
            } => {
                collect_string_literals_expr(condition, output);
                collect_string_literals_block(body, output);
            }
            HirStatement::For { iterable, body, .. } => {
                collect_string_literals_expr(iterable, output);
                collect_string_literals_block(body, output);
            }
            HirStatement::Match { value, arms, .. } => {
                collect_string_literals_expr(value, output);
                for arm in arms {
                    collect_string_literals_block(&arm.body, output);
                }
            }
            HirStatement::Defer { body, .. } => collect_string_literals_block(body, output),
            HirStatement::Break { cleanup, .. } | HirStatement::Continue { cleanup, .. } => {
                for cleanup in cleanup {
                    collect_string_literals_block(cleanup, output);
                }
            }
        }
    }
}

fn collect_string_literals_expr(expression: &HirExpr, output: &mut Vec<(Span, Vec<u8>)>) {
    if let HirExprKind::String(bytes) = &expression.kind {
        output.push((expression.span, bytes.clone()));
    }
    match &expression.kind {
        HirExprKind::List(values)
        | HirExprKind::DirectCall {
            arguments: values, ..
        }
        | HirExprKind::IntrinsicCall {
            arguments: values, ..
        }
        | HirExprKind::HostCall {
            arguments: values, ..
        }
        | HirExprKind::Enum {
            payload: values, ..
        } => {
            for value in values {
                collect_string_literals_expr(value, output);
            }
        }
        HirExprKind::Binary { left, right, .. } => {
            collect_string_literals_expr(left, output);
            collect_string_literals_expr(right, output);
        }
        HirExprKind::Unary { value, .. }
        | HirExprKind::Cast { value, .. }
        | HirExprKind::Bitcast { value, .. }
        | HirExprKind::Length { value } => collect_string_literals_expr(value, output),
        HirExprKind::Drop { place, cleanup } => {
            collect_string_literals_place(place, output);
            collect_string_literals_block(cleanup, output);
        }
        HirExprKind::Constant { value, .. } => collect_string_literals_expr(value, output),
        HirExprKind::IndirectCall {
            callee, arguments, ..
        } => {
            collect_string_literals_expr(callee, output);
            for argument in arguments {
                collect_string_literals_expr(argument, output);
            }
        }
        HirExprKind::Struct { fields, .. } => {
            for (_, value) in fields {
                collect_string_literals_expr(value, output);
            }
        }
        HirExprKind::Place(place) | HirExprKind::Borrow { place, .. } => {
            collect_string_literals_place(place, output)
        }
        HirExprKind::Try { value, cleanup, .. } => {
            collect_string_literals_expr(value, output);
            for cleanup in cleanup {
                collect_string_literals_block(cleanup, output);
            }
        }
        HirExprKind::Closure { body, .. } => collect_string_literals_block(body, output),
        HirExprKind::FormatString(parts) => {
            for part in parts {
                match part {
                    dbyte_hir::HirFormatPart::Literal(bytes) => {
                        output.push((expression.span, bytes.clone()));
                    }
                    dbyte_hir::HirFormatPart::Value(value) => {
                        collect_string_literals_expr(value, output);
                    }
                }
            }
        }
        HirExprKind::Unit
        | HirExprKind::Integer(..)
        | HirExprKind::LayoutOf { .. }
        | HirExprKind::Float(..)
        | HirExprKind::Bool(..)
        | HirExprKind::String(..)
        | HirExprKind::Bytes(..)
        | HirExprKind::Null
        | HirExprKind::Local(..)
        | HirExprKind::Global(..)
        | HirExprKind::Function(..)
        | HirExprKind::InterruptAddress { .. }
        | HirExprKind::TaskAddress { .. } => {}
    }
}

fn collect_string_literals_place(place: &HirPlace, output: &mut Vec<(Span, Vec<u8>)>) {
    if let HirPlaceBase::Pointer(pointer) = &place.base {
        collect_string_literals_expr(pointer, output);
    }
    for projection in &place.projections {
        if let HirProjection::Index { index, .. } = projection {
            collect_string_literals_expr(index, output);
        }
    }
}

fn collect_closures_block(
    block: &HirBlock,
    parent: &ConcreteHirFunction,
    output: &mut Vec<ClosureInfo>,
) {
    for statement in &block.statements {
        match statement {
            HirStatement::Let { value, .. } | HirStatement::Expression(value) => {
                collect_closures_expr(value, parent, output)
            }
            HirStatement::Assign {
                value,
                cleanup_before_commit,
                ..
            } => {
                collect_closures_expr(value, parent, output);
                if let Some(cleanup) = cleanup_before_commit {
                    collect_closures_block(cleanup, parent, output);
                }
            }
            HirStatement::Return { value, cleanup, .. } => {
                if let Some(value) = value {
                    collect_closures_expr(value, parent, output);
                }
                for cleanup in cleanup {
                    collect_closures_block(cleanup, parent, output);
                }
            }
            HirStatement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                collect_closures_expr(condition, parent, output);
                collect_closures_block(then_block, parent, output);
                if let Some(else_block) = else_block {
                    collect_closures_block(else_block, parent, output);
                }
            }
            HirStatement::While {
                condition, body, ..
            } => {
                collect_closures_expr(condition, parent, output);
                collect_closures_block(body, parent, output);
            }
            HirStatement::For { iterable, body, .. } => {
                collect_closures_expr(iterable, parent, output);
                collect_closures_block(body, parent, output);
            }
            HirStatement::Match { value, arms, .. } => {
                collect_closures_expr(value, parent, output);
                for arm in arms {
                    collect_closures_block(&arm.body, parent, output);
                }
            }
            HirStatement::Defer { body, .. } => collect_closures_block(body, parent, output),
            HirStatement::Break { cleanup, .. } | HirStatement::Continue { cleanup, .. } => {
                for cleanup in cleanup {
                    collect_closures_block(cleanup, parent, output);
                }
            }
        }
    }
}

fn collect_closures_expr(
    expression: &HirExpr,
    parent: &ConcreteHirFunction,
    output: &mut Vec<ClosureInfo>,
) {
    match &expression.kind {
        HirExprKind::Closure {
            closure,
            captures,
            parameters,
            result,
            body,
        } => {
            output.push(ClosureInfo {
                id: *closure,
                captures: captures.clone(),
                parameters: parameters.clone(),
                result: *result,
                body: body.clone(),
                parent: parent.clone(),
                span: expression.span,
            });
            collect_closures_block(body, parent, output);
        }
        HirExprKind::Binary { left, right, .. } => {
            collect_closures_expr(left, parent, output);
            collect_closures_expr(right, parent, output);
        }
        HirExprKind::Unary { value, .. }
        | HirExprKind::Cast { value, .. }
        | HirExprKind::Bitcast { value, .. }
        | HirExprKind::Length { value }
        | HirExprKind::Try { value, .. }
        | HirExprKind::Constant { value, .. } => collect_closures_expr(value, parent, output),
        HirExprKind::Drop { cleanup, .. } => collect_closures_block(cleanup, parent, output),
        HirExprKind::DirectCall { arguments, .. }
        | HirExprKind::IntrinsicCall { arguments, .. }
        | HirExprKind::HostCall { arguments, .. }
        | HirExprKind::List(arguments)
        | HirExprKind::Enum {
            payload: arguments, ..
        } => {
            for argument in arguments {
                collect_closures_expr(argument, parent, output);
            }
        }
        HirExprKind::IndirectCall {
            callee, arguments, ..
        } => {
            collect_closures_expr(callee, parent, output);
            for argument in arguments {
                collect_closures_expr(argument, parent, output);
            }
        }
        HirExprKind::Struct { fields, .. } => {
            for (_, value) in fields {
                collect_closures_expr(value, parent, output);
            }
        }
        HirExprKind::FormatString(parts) => {
            for part in parts {
                if let dbyte_hir::HirFormatPart::Value(value) = part {
                    collect_closures_expr(value, parent, output);
                }
            }
        }
        HirExprKind::Place(place) | HirExprKind::Borrow { place, .. } => {
            if let HirPlaceBase::Pointer(pointer) = &place.base {
                collect_closures_expr(pointer, parent, output);
            }
            for projection in &place.projections {
                if let HirProjection::Index { index, .. } = projection {
                    collect_closures_expr(index, parent, output);
                }
            }
        }
        HirExprKind::Unit
        | HirExprKind::Integer(..)
        | HirExprKind::LayoutOf { .. }
        | HirExprKind::Float(..)
        | HirExprKind::Bool(..)
        | HirExprKind::String(..)
        | HirExprKind::Bytes(..)
        | HirExprKind::Null
        | HirExprKind::Local(..)
        | HirExprKind::Global(..)
        | HirExprKind::Function(..)
        | HirExprKind::InterruptAddress { .. }
        | HirExprKind::TaskAddress { .. } => {}
    }
}

struct FunctionLowerer<'a, 's> {
    program: &'a ConcreteHirProgram,
    function: ConcreteHirFunction,
    symbols: &'a BTreeMap<dbyte_hir::FunctionInstanceKey, String>,
    closure_symbols: &'a BTreeMap<dbyte_hir::ClosureId, String>,
    strings: &'s mut Vec<Vec<u8>>,
    slots: Vec<SystemsSlot>,
    params: Vec<SystemsParam>,
    instructions: Vec<SystemsInst>,
    labels: u32,
    loops: Vec<(LabelId, LabelId)>,
    sret: Option<NativeLocalId>,
    capture_env: Option<CaptureEnvironment>,
    runtime_drop_enabled: bool,
}

#[derive(Clone)]
struct ClosureInfo {
    id: dbyte_hir::ClosureId,
    captures: Vec<dbyte_hir::HirCapture>,
    parameters: Vec<dbyte_hir::HirLocal>,
    result: TypeId,
    body: HirBlock,
    parent: ConcreteHirFunction,
    span: Span,
}

#[derive(Clone)]
struct CaptureAccess {
    offset: u32,
    mode: CaptureMode,
    ty: TypeId,
}

#[derive(Clone)]
struct CaptureEnvironment {
    parameter: NativeLocalId,
    captures: BTreeMap<dbyte_hir::LocalId, CaptureAccess>,
}

enum PackedArgument<'a> {
    Address(NativeLocalId),
    Value(&'a HirExpr),
}

impl<'a, 's> FunctionLowerer<'a, 's> {
    fn runtime_drop_enabled(&self) -> bool {
        self.runtime_drop_enabled
    }

    fn new(
        program: &'a ConcreteHirProgram,
        function: &ConcreteHirFunction,
        symbols: &'a BTreeMap<dbyte_hir::FunctionInstanceKey, String>,
        closure_symbols: &'a BTreeMap<dbyte_hir::ClosureId, String>,
        strings: &'s mut Vec<Vec<u8>>,
        runtime_drop_enabled: bool,
    ) -> NativeResult<Self> {
        Self::from_function(
            program,
            function.clone(),
            symbols,
            closure_symbols,
            strings,
            None,
            runtime_drop_enabled,
        )
    }

    fn new_closure(
        program: &'a ConcreteHirProgram,
        closure: &ClosureInfo,
        symbols: &'a BTreeMap<dbyte_hir::FunctionInstanceKey, String>,
        closure_symbols: &'a BTreeMap<dbyte_hir::ClosureId, String>,
        strings: &'s mut Vec<Vec<u8>>,
        runtime_drop_enabled: bool,
    ) -> NativeResult<Self> {
        let parameter_ids = closure
            .parameters
            .iter()
            .map(|parameter| parameter.id)
            .collect::<BTreeSet<_>>();
        let mut locals = closure.parent.locals.clone();
        for local in &mut locals {
            local.parameter = parameter_ids.contains(&local.id);
        }
        let function = ConcreteHirFunction {
            instance: closure.parent.instance,
            key: closure.parent.key.clone(),
            symbol_name: closure_symbols[&closure.id].clone(),
            public: false,
            kind: dbyte_ast::FunctionKind::Normal,
            drop_target: None,
            convention: dbyte_ast::CallConvention::DByte,
            interrupt: None,
            declaration_only: false,
            parameters: closure.parameters.clone(),
            result: closure.result,
            locals,
            body: closure.body.clone(),
            span: closure.span,
        };
        let mut lowerer = Self::from_function(
            program,
            function,
            symbols,
            closure_symbols,
            strings,
            None,
            runtime_drop_enabled,
        )?;
        let env_id = NativeLocalId(lowerer.slots.len() as u32);
        let env_ty = SystemType::Pointer(Box::new(SystemType::U8));
        lowerer.slots.push(SystemsSlot {
            id: env_id,
            name: "__dbyte_closure_env".into(),
            ty: env_ty.clone(),
            storage_size: 4,
            alignment: 4,
            is_parameter: true,
        });
        lowerer.params.insert(
            0,
            SystemsParam {
                name: "__dbyte_closure_env".into(),
                ty: env_ty,
                slot: env_id,
            },
        );
        let (_, captures) = closure_environment_layout(program, &closure.captures, closure.span)?;
        lowerer.capture_env = Some(CaptureEnvironment {
            parameter: env_id,
            captures,
        });
        Ok(lowerer)
    }

    fn from_function(
        program: &'a ConcreteHirProgram,
        function: ConcreteHirFunction,
        symbols: &'a BTreeMap<dbyte_hir::FunctionInstanceKey, String>,
        closure_symbols: &'a BTreeMap<dbyte_hir::ClosureId, String>,
        strings: &'s mut Vec<Vec<u8>>,
        capture_env: Option<CaptureEnvironment>,
        runtime_drop_enabled: bool,
    ) -> NativeResult<Self> {
        let mut slots = Vec::new();
        let mut params = Vec::new();
        let parameter_ids = function
            .parameters
            .iter()
            .map(|parameter| parameter.id)
            .collect::<BTreeSet<_>>();
        for local in &function.locals {
            let ty = lower_type(program, local.ty, local.span)?;
            let layout = layout_of(&program.semantic, local.ty).map_err(hir_diagnostic)?;
            let (storage_size, alignment) =
                if matches!(program.semantic.type_of(local.ty), CanonicalType::Unit) {
                    (4, 4)
                } else {
                    (layout.size, layout.align)
                };
            let id = NativeLocalId(local.id.0);
            let is_parameter = parameter_ids.contains(&local.id);
            slots.push(SystemsSlot {
                id,
                name: local.name.clone(),
                ty: ty.clone(),
                storage_size,
                alignment,
                is_parameter,
            });
            if is_parameter {
                params.push(SystemsParam {
                    name: local.name.clone(),
                    ty,
                    slot: id,
                });
            }
        }
        let result_ty = lower_type(program, function.result, function.span)?;
        let mut sret = None;
        if !result_ty.is_scalar() {
            let id = NativeLocalId(slots.len() as u32);
            let pointer = SystemType::Pointer(Box::new(SystemType::U8));
            slots.push(SystemsSlot {
                id,
                name: "__dbyte_sret".into(),
                ty: pointer.clone(),
                storage_size: 4,
                alignment: 4,
                is_parameter: true,
            });
            params.insert(
                0,
                SystemsParam {
                    name: "__dbyte_sret".into(),
                    ty: pointer,
                    slot: id,
                },
            );
            sret = Some(id);
        }
        Ok(Self {
            program,
            function,
            symbols,
            closure_symbols,
            strings,
            slots,
            params,
            instructions: Vec::new(),
            labels: 0,
            loops: Vec::new(),
            sret,
            capture_env,
            runtime_drop_enabled,
        })
    }

    fn lower(mut self) -> NativeResult<SystemsFunction> {
        let body = self.function.body.clone();
        self.lower_block(&body)?;
        let terminal = matches!(
            self.instructions.last(),
            Some(SystemsInst::Return)
                | Some(SystemsInst::Intrinsic {
                    intrinsic: SystemIntrinsic::InterruptResume,
                    ..
                })
        );
        if !terminal
            && !self
                .instructions
                .iter()
                .any(|instruction| matches!(instruction, SystemsInst::Return))
        {
            if matches!(
                self.program.semantic.type_of(self.function.result),
                CanonicalType::Unit
            ) {
                self.instructions.push(SystemsInst::Const {
                    bits: 0,
                    ty: SystemType::I32,
                });
                self.instructions.push(SystemsInst::Return);
            } else {
                return Err(self.error(
                    DiagnosticCategory::InvalidReturn,
                    "canonical HIR function can fall through without a value",
                    self.function.span,
                ));
            }
        }
        let exported_drop_glue = self.function.kind == dbyte_ast::FunctionKind::Drop
            && self.function.drop_target.is_some_and(|target| {
                self.program
                    .semantic
                    .definitions
                    .iter()
                    .any(|definition| definition.symbol == target && definition.public)
            });
        Ok(SystemsFunction {
            source_name: self.function.symbol_name.clone(),
            symbol: self.function.symbol_name.clone(),
            visibility: if self.function.public || exported_drop_glue {
                Visibility::Public
            } else {
                Visibility::Private
            },
            interrupt: self.function.interrupt,
            call_convention: self.function.convention,
            parameter_types: self.params.iter().map(|param| param.ty.clone()).collect(),
            params: self.params,
            return_type: if self.sret.is_some() {
                SystemType::I32
            } else {
                lower_type(self.program, self.function.result, self.function.span)?
            },
            slots: self.slots,
            instructions: self.instructions,
            span: self.function.span,
        })
    }

    fn lower_block(&mut self, block: &HirBlock) -> NativeResult<()> {
        for statement in &block.statements {
            self.lower_statement(statement)?;
        }
        Ok(())
    }

    fn lower_statement(&mut self, statement: &HirStatement) -> NativeResult<()> {
        match statement {
            HirStatement::Let { local, value, .. } => {
                let ty = lower_type(self.program, value.ty, value.span)?;
                if ty.is_scalar() {
                    self.lower_expr(value)?;
                    self.instructions.push(SystemsInst::LocalStore {
                        local: NativeLocalId(local.0),
                        ty,
                    });
                } else {
                    self.lower_aggregate_to_local(NativeLocalId(local.0), value)?;
                }
            }
            HirStatement::Assign {
                place,
                value,
                cleanup_before_commit,
                ..
            } => {
                if let Some(cleanup) = cleanup_before_commit {
                    let layout = self.layout(value.ty, value.span)?;
                    let replacement = self.allocate_temp(value.ty, "reassign", value.span)?;
                    self.lower_aggregate_to_local(replacement, value)?;
                    self.lower_block(cleanup)?;
                    self.lower_place_address(place)?;
                    self.instructions
                        .push(SystemsInst::LocalAddress(replacement));
                    self.instructions
                        .push(SystemsInst::AggregateCopy { size: layout.size });
                    return Ok(());
                }
                if let HirPlace {
                    base: HirPlaceBase::Local(local),
                    projections,
                    ..
                } = place
                {
                    if projections.is_empty() {
                        let ty = lower_type(self.program, value.ty, value.span)?;
                        if ty.is_scalar() {
                            self.lower_expr(value)?;
                            self.instructions.push(SystemsInst::LocalStore {
                                local: NativeLocalId(local.0),
                                ty,
                            });
                        } else {
                            self.lower_aggregate_to_local(NativeLocalId(local.0), value)?;
                        }
                        return Ok(());
                    }
                }
                self.lower_place_address(place)?;
                self.lower_expr(value)?;
                let ty = lower_type(self.program, value.ty, value.span)?;
                if ty.is_scalar() {
                    self.instructions.push(SystemsInst::Store(ty));
                } else {
                    self.instructions.push(SystemsInst::AggregateCopy {
                        size: self.layout(value.ty, value.span)?.size,
                    });
                }
            }
            HirStatement::Expression(value) => {
                self.lower_expr(value)?;
                if matches!(value.kind, HirExprKind::Unit) {
                    self.instructions.push(SystemsInst::Pop);
                } else if !matches!(self.program.semantic.type_of(value.ty), CanonicalType::Unit) {
                    let lowered = lower_type(self.program, value.ty, value.span)?;
                    if lowered.is_wide_integer() {
                        self.instructions.push(SystemsInst::PopWords(2));
                    } else {
                        self.instructions.push(SystemsInst::Pop);
                    }
                }
            }
            HirStatement::Return { value, cleanup, .. } => {
                if !self.runtime_drop_enabled() {
                    for block in cleanup {
                        self.lower_block(block)?;
                    }
                    if let Some(sret) = self.sret {
                        let value = value.as_ref().ok_or_else(|| {
                            self.error(
                                DiagnosticCategory::InvalidReturn,
                                "aggregate function requires a return value",
                                self.function.span,
                            )
                        })?;
                        self.instructions.push(SystemsInst::LocalLoad {
                            local: sret,
                            ty: SystemType::Pointer(Box::new(SystemType::U8)),
                        });
                        self.lower_expr(value)?;
                        self.instructions.push(SystemsInst::AggregateCopy {
                            size: self.layout(value.ty, value.span)?.size,
                        });
                        self.instructions.push(SystemsInst::Const {
                            bits: 0,
                            ty: SystemType::I32,
                        });
                    } else if let Some(value) = value {
                        if matches!(self.program.semantic.type_of(value.ty), CanonicalType::Unit) {
                            self.instructions.push(SystemsInst::Const {
                                bits: 0,
                                ty: SystemType::I32,
                            });
                        } else {
                            self.lower_expr(value)?;
                        }
                    } else {
                        self.instructions.push(SystemsInst::Const {
                            bits: 0,
                            ty: SystemType::I32,
                        });
                    }
                    self.instructions.push(SystemsInst::Return);
                    return Ok(());
                }
                if let Some(sret) = self.sret {
                    let value = value.as_ref().ok_or_else(|| {
                        self.error(
                            DiagnosticCategory::InvalidReturn,
                            "aggregate function requires a return value",
                            self.function.span,
                        )
                    })?;
                    self.instructions.push(SystemsInst::LocalLoad {
                        local: sret,
                        ty: SystemType::Pointer(Box::new(SystemType::U8)),
                    });
                    self.lower_expr(value)?;
                    self.instructions.push(SystemsInst::AggregateCopy {
                        size: self.layout(value.ty, value.span)?.size,
                    });
                    for block in cleanup {
                        self.lower_block(block)?;
                    }
                    self.instructions.push(SystemsInst::Const {
                        bits: 0,
                        ty: SystemType::I32,
                    });
                } else {
                    if let Some(value) = value {
                        if !matches!(self.program.semantic.type_of(value.ty), CanonicalType::Unit) {
                            self.lower_expr(value)?;
                            if cleanup.is_empty() {
                                self.instructions.push(SystemsInst::Return);
                                return Ok(());
                            }
                            let result = self.allocate_system_temp(
                                lower_type(self.program, value.ty, value.span)?,
                                self.layout(value.ty, value.span)?.size.max(1),
                                self.layout(value.ty, value.span)?.align.max(1),
                                "return_value",
                            )?;
                            let result_ty = lower_type(self.program, value.ty, value.span)?;
                            self.instructions.push(SystemsInst::LocalStore {
                                local: result,
                                ty: result_ty.clone(),
                            });
                            for block in cleanup {
                                self.lower_block(block)?;
                            }
                            self.instructions.push(SystemsInst::LocalLoad {
                                local: result,
                                ty: result_ty,
                            });
                            self.instructions.push(SystemsInst::Return);
                            return Ok(());
                        }
                    }
                    for block in cleanup {
                        self.lower_block(block)?;
                    }
                    self.instructions.push(SystemsInst::Const {
                        bits: 0,
                        ty: SystemType::I32,
                    });
                }
                self.instructions.push(SystemsInst::Return);
            }
            HirStatement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                let otherwise = self.label();
                self.lower_expr(condition)?;
                self.instructions.push(SystemsInst::JumpIfFalse(otherwise));
                self.lower_block(then_block)?;
                if let Some(block) = else_block {
                    let done = self.label();
                    self.instructions.push(SystemsInst::Jump(done));
                    self.instructions.push(SystemsInst::Label(otherwise));
                    self.lower_block(block)?;
                    self.instructions.push(SystemsInst::Label(done));
                } else {
                    self.instructions.push(SystemsInst::Label(otherwise));
                }
            }
            HirStatement::While {
                condition, body, ..
            } => {
                let head = self.label();
                let done = self.label();
                self.instructions.push(SystemsInst::Label(head));
                self.lower_expr(condition)?;
                self.instructions.push(SystemsInst::JumpIfFalse(done));
                self.loops.push((head, done));
                self.lower_block(body)?;
                self.loops.pop();
                self.instructions.push(SystemsInst::Jump(head));
                self.instructions.push(SystemsInst::Label(done));
            }
            HirStatement::For { span, .. } => {
                return Err(self.unsupported("hosted for iteration", *span));
            }
            HirStatement::Break { cleanup, span } => {
                for block in cleanup {
                    self.lower_block(block)?;
                }
                let (_, done) = self.loops.last().copied().ok_or_else(|| {
                    self.error(
                        DiagnosticCategory::InvalidType,
                        "break reached native lowering outside a loop",
                        *span,
                    )
                })?;
                self.instructions.push(SystemsInst::Jump(done));
            }
            HirStatement::Continue { cleanup, span } => {
                for block in cleanup {
                    self.lower_block(block)?;
                }
                let (head, _) = self.loops.last().copied().ok_or_else(|| {
                    self.error(
                        DiagnosticCategory::InvalidType,
                        "continue reached native lowering outside a loop",
                        *span,
                    )
                })?;
                self.instructions.push(SystemsInst::Jump(head));
            }
            HirStatement::Defer { .. } => {
                // Registration emits no runtime code. Canonical cleanup blocks
                // are attached to each exit edge and fallthrough is appended by
                // the executable-HIR builder.
            }
            HirStatement::Match {
                value, arms, span, ..
            } => self.lower_match(value, arms, *span)?,
        }
        Ok(())
    }

    fn lower_expr(&mut self, expression: &HirExpr) -> NativeResult<()> {
        let ty = lower_type(self.program, expression.ty, expression.span)?;
        match &expression.kind {
            HirExprKind::LayoutOf { .. } => {
                return Err(
                    self.unsupported("unmaterialized canonical layout query", expression.span)
                )
            }
            HirExprKind::Length { value } => match self.program.semantic.type_of(value.ty) {
                CanonicalType::Array { length, .. } => {
                    self.instructions.push(SystemsInst::Const {
                        bits: *length,
                        ty: SystemType::Usize,
                    });
                }
                CanonicalType::Slice { .. } | CanonicalType::StringView => {
                    self.lower_expr(value)?;
                    self.instructions
                        .push(SystemsInst::FieldAddress { offset: 4 });
                    self.instructions.push(SystemsInst::Load(SystemType::Usize));
                }
                _ => {
                    return Err(self.unsupported(
                        "len operand without a canonical bounded representation",
                        expression.span,
                    ));
                }
            },
            // DBYTE32 gives source-level unit a deterministic physical word
            // when it participates in an aggregate or packed call ABI.
            HirExprKind::Unit => self.instructions.push(SystemsInst::Const {
                bits: 0,
                ty: SystemType::I32,
            }),
            HirExprKind::Integer(value) => {
                if matches!(ty, SystemType::I64 | SystemType::U64) {
                    let bits = integer_bits64(*value, &ty, expression.span)?;
                    self.instructions.push(SystemsInst::Const64 { bits, ty });
                } else {
                    let bits = integer_bits(*value, &ty, expression.span)?;
                    self.instructions.push(SystemsInst::Const { bits, ty });
                }
            }
            HirExprKind::Bool(value) => self.instructions.push(SystemsInst::ConstBool(*value)),
            HirExprKind::Null => self.instructions.push(SystemsInst::Const { bits: 0, ty }),
            HirExprKind::String(bytes) => {
                let id = if let Some(index) = self.strings.iter().position(|value| value == bytes) {
                    index
                } else {
                    self.strings.push(bytes.clone());
                    self.strings.len() - 1
                };
                if self.runtime_drop_enabled()
                    && matches!(
                        self.program.semantic.type_of(expression.ty),
                        CanonicalType::StringView
                    )
                {
                    let temp = self.allocate_temp(expression.ty, "string_view", expression.span)?;
                    self.instructions.push(SystemsInst::LocalAddress(temp));
                    self.instructions
                        .push(SystemsInst::StringAddress(StringId(id as u32)));
                    self.instructions
                        .push(SystemsInst::Store(SystemType::Usize));
                    self.instructions.push(SystemsInst::LocalAddress(temp));
                    self.instructions
                        .push(SystemsInst::FieldAddress { offset: 4 });
                    self.instructions.push(SystemsInst::Const {
                        bits: bytes.len() as u32,
                        ty: SystemType::Usize,
                    });
                    self.instructions
                        .push(SystemsInst::Store(SystemType::Usize));
                    self.instructions.push(SystemsInst::LocalAddress(temp));
                } else {
                    self.instructions
                        .push(SystemsInst::StringAddress(StringId(id as u32)));
                }
            }
            HirExprKind::Local(local) => {
                self.lower_local_value(*local, expression.ty, expression.span)?;
            }
            HirExprKind::Place(place) => {
                let place_ty = lower_type(self.program, place.ty, place.span)?;
                self.lower_place_address(place)?;
                // A canonical borrow place records its pointer provenance in
                // `place.ty`, so dereferencing it uses the expression's
                // pointee width. Other legacy places retain their established
                // load-then-normalize sequence byte-for-byte.
                if matches!(
                    self.program.semantic.type_of(place.ty),
                    CanonicalType::Borrow { .. }
                ) {
                    if ty.is_scalar() {
                        self.instructions.push(SystemsInst::Load(ty.clone()));
                    }
                } else if place_ty.is_scalar() {
                    self.instructions.push(SystemsInst::Load(place_ty.clone()));
                    if place_ty != ty {
                        self.instructions.push(SystemsInst::Cast {
                            from: place_ty,
                            to: ty.clone(),
                        });
                    }
                }
            }
            HirExprKind::Binary { left, op, right } => {
                self.lower_expr(left)?;
                self.lower_expr(right)?;
                let operand = lower_type(self.program, left.ty, left.span)?;
                match op {
                    BinOp::Add => self.instructions.push(SystemsInst::Add(operand)),
                    BinOp::Sub => self.instructions.push(SystemsInst::Sub(operand)),
                    BinOp::Mul => self.instructions.push(SystemsInst::Mul(operand)),
                    BinOp::Div => self.instructions.push(SystemsInst::Div(operand)),
                    BinOp::Rem => self.instructions.push(SystemsInst::Rem(operand)),
                    BinOp::BitAnd => self.instructions.push(SystemsInst::BitAnd(operand)),
                    BinOp::BitOr => self.instructions.push(SystemsInst::BitOr(operand)),
                    BinOp::BitXor => self.instructions.push(SystemsInst::BitXor(operand)),
                    BinOp::Shl => self.instructions.push(SystemsInst::ShiftLeft(operand)),
                    BinOp::Shr => self.instructions.push(SystemsInst::ShiftRight(operand)),
                    BinOp::EqEq
                    | BinOp::NotEq
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::LtEq
                    | BinOp::GtEq => self.instructions.push(SystemsInst::Compare {
                        op: compare(op.clone()),
                        operand_type: operand,
                    }),
                }
            }
            HirExprKind::Unary { op, value } => {
                self.lower_expr(value)?;
                match op {
                    UnaryOp::Neg => self.instructions.push(SystemsInst::Neg(ty)),
                    UnaryOp::Not => self.instructions.push(SystemsInst::BoolNot),
                    UnaryOp::BitNot => self.instructions.push(SystemsInst::BitNot(ty)),
                    UnaryOp::Deref => self.instructions.push(SystemsInst::Load(ty)),
                    UnaryOp::AddressOf => {
                        return Err(
                            self.unsupported("unresolved address-of expression", expression.span)
                        );
                    }
                }
            }
            HirExprKind::Borrow { place, .. } => self.lower_place_address(place)?,
            HirExprKind::Cast { value, .. } => {
                self.require_scalar(&ty, expression.span)?;
                let from = lower_type(self.program, value.ty, value.span)?;
                self.require_scalar(&from, value.span)?;
                if let HirExprKind::Integer(bits) = value.kind {
                    if matches!(ty, SystemType::I64 | SystemType::U64) {
                        if let Ok(bits) = integer_bits64(bits, &ty, value.span) {
                            self.instructions.push(SystemsInst::Const64 {
                                bits,
                                ty: ty.clone(),
                            });
                            return Ok(());
                        }
                    }
                    if let Ok(bits) = integer_bits(bits, &ty, value.span) {
                        self.instructions.push(SystemsInst::Const {
                            bits,
                            ty: ty.clone(),
                        });
                        return Ok(());
                    }
                }
                self.lower_expr(value)?;
                // An explicit cast is itself an executable normalization
                // operation, including same-width/same-type narrow casts.
                // Retaining it preserves legacy object bytes and prevents the
                // adapter from silently optimizing a semantic HIR node away.
                self.instructions.push(SystemsInst::Cast { from, to: ty });
            }
            HirExprKind::Bitcast { .. } => {
                return Err(self.unsupported(
                    "exact bitcast requires Native MIR lowering",
                    expression.span,
                ));
            }
            HirExprKind::Drop { cleanup, .. } => self.lower_block(cleanup)?,
            HirExprKind::DirectCall {
                callee,
                convention,
                arguments,
            } => {
                let symbol = self.symbols.get(callee).ok_or_else(|| {
                    self.error(
                        DiagnosticCategory::UnresolvedFunction,
                        "concrete call target was not emitted",
                        expression.span,
                    )
                })?;
                let has_packed_argument = arguments.iter().any(|argument| {
                    !self.is_scalar(argument.ty)
                        || self
                            .layout(argument.ty, argument.span)
                            .is_ok_and(|layout| layout.size > 4)
                });
                let aggregate_result = !self.is_scalar(expression.ty);
                if !has_packed_argument && !aggregate_result {
                    let ordered: Box<dyn Iterator<Item = &HirExpr>> =
                        if *convention == dbyte_ast::CallConvention::C {
                            Box::new(arguments.iter().rev())
                        } else {
                            Box::new(arguments.iter())
                        };
                    for argument in ordered {
                        self.lower_expr(argument)?;
                    }
                    self.instructions.push(SystemsInst::Call {
                        symbol: symbol.clone(),
                        argument_count: arguments.len(),
                        result: ty,
                        call_convention: *convention,
                    });
                    if matches!(
                        self.program.semantic.type_of(expression.ty),
                        CanonicalType::Unit
                    ) {
                        // The proven DBYTE32 physical ABI always materializes
                        // EAX after a call. Canonical unit has no semantic
                        // value, so consume that physical result at the
                        // representation boundary.
                        self.instructions.push(SystemsInst::Pop);
                    }
                } else {
                    if *convention == dbyte_ast::CallConvention::C
                        && (arguments
                            .iter()
                            .any(|argument| !self.is_scalar(argument.ty))
                            || aggregate_result)
                    {
                        return Err(self.unsupported("aggregate C ABI call", expression.span));
                    }
                    let result_temp = if aggregate_result {
                        Some(self.allocate_temp(expression.ty, "call_result", expression.span)?)
                    } else {
                        None
                    };
                    let mut physical = Vec::new();
                    if *convention == dbyte_ast::CallConvention::C {
                        for argument in arguments.iter().rev() {
                            physical.push(PackedArgument::Value(argument));
                        }
                    } else {
                        // DBYTE32 evaluates and pushes the hidden sret first,
                        // followed by source arguments in declaration order.
                        // Consequently the last source argument occupies the
                        // lowest stack address (EBP+8), while sret is highest.
                        if let Some(result) = result_temp {
                            physical.push(PackedArgument::Address(result));
                        }
                        for argument in arguments {
                            physical.push(PackedArgument::Value(argument));
                        }
                    }
                    if *convention == dbyte_ast::CallConvention::C {
                        if let Some(result) = result_temp {
                            physical.insert(0, PackedArgument::Address(result));
                        }
                    }
                    let pack = self.materialize_call_pack(&physical, expression.span)?;
                    let bytes = packed_size(&physical, self.program)?;
                    self.instructions.push(SystemsInst::LocalAddress(pack));
                    self.instructions.push(SystemsInst::CallPacked {
                        // Use the concrete instance's deterministic link name,
                        // including module ownership and specialization. The
                        // source-level name is not a linkable identity.
                        symbol: symbol.clone(),
                        argument_bytes: bytes,
                        result: if aggregate_result {
                            SystemType::I32
                        } else {
                            ty
                        },
                        call_convention: *convention,
                    });
                    if let Some(result) = result_temp {
                        self.instructions.push(SystemsInst::Pop);
                        self.instructions.push(SystemsInst::LocalAddress(result));
                    } else if matches!(
                        self.program.semantic.type_of(expression.ty),
                        CanonicalType::Unit
                    ) {
                        self.instructions.push(SystemsInst::Pop);
                    }
                }
            }
            HirExprKind::Global(symbol) => {
                let global = self.global(*symbol, expression.span)?;
                let ty = lower_type(self.program, global.ty, expression.span)?;
                self.instructions.push(SystemsInst::GlobalAddress {
                    symbol: global.name.clone(),
                    pointee: ty.clone(),
                });
                if ty.is_scalar() {
                    self.instructions.push(SystemsInst::Load(ty));
                }
            }
            HirExprKind::Constant { value, .. } => self.lower_expr(value)?,
            HirExprKind::IntrinsicCall {
                intrinsic,
                arguments,
            } => self.lower_intrinsic(*intrinsic, arguments, expression)?,
            HirExprKind::InterruptAddress {
                vector,
                handler,
                class,
            } => {
                let symbol = self.symbols.get(handler).ok_or_else(|| {
                    self.error(
                        DiagnosticCategory::UnresolvedFunction,
                        "resolved interrupt handler was not emitted",
                        expression.span,
                    )
                })?;
                self.instructions.push(SystemsInst::InterruptAddress {
                    vector: *vector,
                    symbol: symbol.clone(),
                    class: *class,
                });
            }
            HirExprKind::TaskAddress { function } => {
                let symbol = self.symbols.get(function).ok_or_else(|| {
                    self.error(
                        DiagnosticCategory::UnresolvedFunction,
                        "resolved task function was not emitted",
                        expression.span,
                    )
                })?;
                self.instructions.push(SystemsInst::TaskAddress {
                    symbol: symbol.clone(),
                });
            }
            HirExprKind::Float(..) => return Err(self.unsupported("float value", expression.span)),
            HirExprKind::Bytes(..) => return Err(self.unsupported("byte string", expression.span)),
            HirExprKind::Function(key) => {
                let symbol = self.symbols.get(key).ok_or_else(|| {
                    self.error(
                        DiagnosticCategory::UnresolvedFunction,
                        "function value target was not emitted",
                        expression.span,
                    )
                })?;
                self.instructions.push(SystemsInst::FunctionAddress {
                    symbol: symbol.clone(),
                });
            }
            HirExprKind::List(..) => {
                return Err(self.unsupported("aggregate initializer", expression.span))
            }
            HirExprKind::IndirectCall {
                callee,
                convention,
                arguments,
            } => {
                if matches!(
                    self.program.semantic.type_of(callee.ty),
                    CanonicalType::ClosureShape { .. }
                ) {
                    if arguments
                        .iter()
                        .any(|argument| !self.is_scalar(argument.ty))
                        || !self.is_scalar(expression.ty)
                    {
                        return Err(self.unsupported("aggregate closure call", expression.span));
                    }
                    let closure_pointer = self.allocate_system_temp(
                        SystemType::Pointer(Box::new(SystemType::U8)),
                        4,
                        4,
                        "closure_value",
                    )?;
                    self.lower_expr(callee)?;
                    self.instructions.push(SystemsInst::LocalStore {
                        local: closure_pointer,
                        ty: SystemType::Pointer(Box::new(SystemType::U8)),
                    });
                    self.instructions.push(SystemsInst::LocalLoad {
                        local: closure_pointer,
                        ty: SystemType::Pointer(Box::new(SystemType::U8)),
                    });
                    self.instructions.push(SystemsInst::Load(SystemType::Usize));
                    self.instructions.push(SystemsInst::LocalLoad {
                        local: closure_pointer,
                        ty: SystemType::Pointer(Box::new(SystemType::U8)),
                    });
                    self.instructions
                        .push(SystemsInst::FieldAddress { offset: 4 });
                    self.instructions.push(SystemsInst::Load(SystemType::Usize));
                    for argument in arguments {
                        self.lower_expr(argument)?;
                    }
                    self.instructions.push(SystemsInst::CallIndirect {
                        argument_count: arguments.len() + 1,
                        result: ty,
                        call_convention: *convention,
                    });
                    if matches!(
                        self.program.semantic.type_of(expression.ty),
                        CanonicalType::Unit
                    ) {
                        self.instructions.push(SystemsInst::Pop);
                    }
                    return Ok(());
                }
                let has_packed_argument = arguments.iter().any(|argument| {
                    !self.is_scalar(argument.ty)
                        || self
                            .layout(argument.ty, argument.span)
                            .is_ok_and(|layout| layout.size > 4)
                });
                if has_packed_argument && self.is_scalar(expression.ty) {
                    if arguments
                        .iter()
                        .any(|argument| !self.is_scalar(argument.ty))
                    {
                        return Err(
                            self.unsupported("aggregate indirect-call argument", expression.span)
                        );
                    }
                    let callee_temp = self.allocate_system_temp(
                        SystemType::Pointer(Box::new(SystemType::U8)),
                        4,
                        4,
                        "indirect_callee",
                    )?;
                    self.lower_expr(callee)?;
                    self.instructions.push(SystemsInst::LocalStore {
                        local: callee_temp,
                        ty: SystemType::Pointer(Box::new(SystemType::U8)),
                    });
                    let physical = if *convention == dbyte_ast::CallConvention::C {
                        arguments
                            .iter()
                            .rev()
                            .map(PackedArgument::Value)
                            .collect::<Vec<_>>()
                    } else {
                        arguments
                            .iter()
                            .map(PackedArgument::Value)
                            .collect::<Vec<_>>()
                    };
                    let pack = self.materialize_call_pack(&physical, expression.span)?;
                    let bytes = packed_size(&physical, self.program)?;
                    self.instructions.push(SystemsInst::LocalAddress(pack));
                    self.instructions.push(SystemsInst::LocalLoad {
                        local: callee_temp,
                        ty: SystemType::Pointer(Box::new(SystemType::U8)),
                    });
                    self.instructions.push(SystemsInst::CallIndirectPacked {
                        argument_bytes: bytes,
                        result: ty,
                        call_convention: *convention,
                    });
                    return Ok(());
                }
                if arguments
                    .iter()
                    .any(|argument| !self.is_scalar(argument.ty))
                {
                    return Err(
                        self.unsupported("aggregate indirect-call argument", expression.span)
                    );
                }
                if !self.is_scalar(expression.ty) {
                    if *convention == dbyte_ast::CallConvention::C {
                        return Err(
                            self.unsupported("aggregate C indirect-call result", expression.span)
                        );
                    }
                    let result =
                        self.allocate_temp(expression.ty, "indirect_call_result", expression.span)?;
                    self.lower_expr(callee)?;
                    self.instructions.push(SystemsInst::LocalAddress(result));
                    for argument in arguments {
                        self.lower_expr(argument)?;
                    }
                    self.instructions.push(SystemsInst::CallIndirect {
                        argument_count: arguments.len() + 1,
                        result: SystemType::I32,
                        call_convention: *convention,
                    });
                    self.instructions.push(SystemsInst::Pop);
                    self.instructions.push(SystemsInst::LocalAddress(result));
                    return Ok(());
                }
                self.lower_expr(callee)?;
                let ordered: Box<dyn Iterator<Item = &HirExpr>> =
                    if *convention == dbyte_ast::CallConvention::C {
                        Box::new(arguments.iter().rev())
                    } else {
                        Box::new(arguments.iter())
                    };
                for argument in ordered {
                    self.lower_expr(argument)?;
                }
                self.instructions.push(SystemsInst::CallIndirect {
                    argument_count: arguments.len(),
                    result: ty,
                    call_convention: *convention,
                });
                if matches!(
                    self.program.semantic.type_of(expression.ty),
                    CanonicalType::Unit
                ) {
                    self.instructions.push(SystemsInst::Pop);
                }
            }
            HirExprKind::Struct { symbol: _, fields } => {
                let temp = self.allocate_temp(expression.ty, "struct", expression.span)?;
                let size = self.layout(expression.ty, expression.span)?.size;
                self.instructions.push(SystemsInst::LocalAddress(temp));
                self.instructions.push(SystemsInst::AggregateZero { size });
                for (field, value) in fields {
                    let offset = hir_field_offset(
                        &self.program.semantic,
                        expression.ty,
                        *field,
                        expression.span,
                    )?;
                    self.instructions.push(SystemsInst::LocalAddress(temp));
                    if offset != 0 {
                        self.instructions.push(SystemsInst::FieldAddress { offset });
                    }
                    self.lower_expr(value)?;
                    self.store_value(value.ty, value.span)?;
                }
                self.instructions.push(SystemsInst::LocalAddress(temp));
            }
            HirExprKind::Enum {
                variant,
                discriminant,
                payload,
                ..
            } => {
                let temp = self.allocate_temp(expression.ty, "enum", expression.span)?;
                let size = self.layout(expression.ty, expression.span)?.size;
                self.instructions.push(SystemsInst::LocalAddress(temp));
                self.instructions.push(SystemsInst::AggregateZero { size });
                self.instructions.push(SystemsInst::LocalAddress(temp));
                self.instructions.push(SystemsInst::Const {
                    bits: *discriminant,
                    ty: SystemType::U32,
                });
                self.instructions.push(SystemsInst::Store(SystemType::U32));
                let (_, _, layouts) =
                    dbyte_hir::enum_variant_layout(&self.program.semantic, expression.ty, *variant)
                        .map_err(hir_diagnostic)?;
                for (value, (_, offset)) in payload.iter().zip(layouts) {
                    if matches!(self.program.semantic.type_of(value.ty), CanonicalType::Unit) {
                        continue;
                    }
                    self.instructions.push(SystemsInst::LocalAddress(temp));
                    self.instructions.push(SystemsInst::FieldAddress { offset });
                    self.lower_expr(value)?;
                    self.store_value(value.ty, value.span)?;
                }
                self.instructions.push(SystemsInst::LocalAddress(temp));
            }
            HirExprKind::Try {
                value,
                ok_variant,
                cleanup,
                ..
            } => {
                let result_pointer = self.allocate_system_temp(
                    SystemType::Pointer(Box::new(SystemType::U8)),
                    4,
                    4,
                    "try_value",
                )?;
                self.lower_expr(value)?;
                self.instructions.push(SystemsInst::LocalStore {
                    local: result_pointer,
                    ty: SystemType::Pointer(Box::new(SystemType::U8)),
                });
                let (ok_discriminant, _, payload) =
                    dbyte_hir::enum_variant_layout(&self.program.semantic, value.ty, *ok_variant)
                        .map_err(hir_diagnostic)?;
                let error = self.label();
                let success = self.label();
                self.instructions.push(SystemsInst::LocalLoad {
                    local: result_pointer,
                    ty: SystemType::Pointer(Box::new(SystemType::U8)),
                });
                self.instructions.push(SystemsInst::Load(SystemType::U32));
                self.instructions.push(SystemsInst::Const {
                    bits: ok_discriminant,
                    ty: SystemType::U32,
                });
                self.instructions.push(SystemsInst::Compare {
                    op: CompareOp::Eq,
                    operand_type: SystemType::U32,
                });
                self.instructions.push(SystemsInst::JumpIfFalse(error));
                self.instructions.push(SystemsInst::Jump(success));
                self.instructions.push(SystemsInst::Label(error));
                for block in cleanup {
                    self.lower_block(block)?;
                }
                let sret = self.sret.ok_or_else(|| {
                    self.error(
                        DiagnosticCategory::InvalidReturn,
                        "Result propagation requires an aggregate Result return",
                        expression.span,
                    )
                })?;
                self.instructions.push(SystemsInst::LocalLoad {
                    local: sret,
                    ty: SystemType::Pointer(Box::new(SystemType::U8)),
                });
                self.instructions.push(SystemsInst::LocalLoad {
                    local: result_pointer,
                    ty: SystemType::Pointer(Box::new(SystemType::U8)),
                });
                self.instructions.push(SystemsInst::AggregateCopy {
                    size: self.layout(value.ty, value.span)?.size,
                });
                self.instructions.push(SystemsInst::Const {
                    bits: 0,
                    ty: SystemType::I32,
                });
                self.instructions.push(SystemsInst::Return);
                self.instructions.push(SystemsInst::Label(success));
                let (payload_ty, offset) = payload.first().copied().ok_or_else(|| {
                    self.error(
                        DiagnosticCategory::InvalidType,
                        "Result.Ok must contain one payload",
                        expression.span,
                    )
                })?;
                if !matches!(
                    self.program.semantic.type_of(payload_ty),
                    CanonicalType::Unit
                ) {
                    self.instructions.push(SystemsInst::LocalLoad {
                        local: result_pointer,
                        ty: SystemType::Pointer(Box::new(SystemType::U8)),
                    });
                    if offset != 0 {
                        self.instructions.push(SystemsInst::FieldAddress { offset });
                    }
                    if self.is_scalar(payload_ty) {
                        self.instructions.push(SystemsInst::Load(lower_type(
                            self.program,
                            payload_ty,
                            expression.span,
                        )?));
                    }
                }
            }
            HirExprKind::Closure {
                closure, captures, ..
            } => {
                let temp = self.allocate_temp(expression.ty, "closure", expression.span)?;
                self.materialize_closure(temp, *closure, captures, expression.span)?;
                self.instructions.push(SystemsInst::LocalAddress(temp));
            }
            HirExprKind::HostCall { .. } => {
                return Err(self.unsupported("host capability call", expression.span));
            }
            HirExprKind::FormatString(..) => {
                return Err(self.unsupported("host formatted string", expression.span));
            }
        }
        Ok(())
    }

    fn lower_intrinsic(
        &mut self,
        intrinsic: HirIntrinsic,
        arguments: &[HirExpr],
        expression: &HirExpr,
    ) -> NativeResult<()> {
        if intrinsic.is_runtime_view() {
            let [pointer, length] = arguments else {
                return Err(self.internal_hir(
                    "runtime view constructor lost pointer or length",
                    expression.span,
                ));
            };
            let view = self.allocate_temp(expression.ty, "runtime_view", expression.span)?;
            self.instructions.push(SystemsInst::LocalAddress(view));
            self.instructions
                .push(SystemsInst::AggregateZero { size: 8 });
            self.instructions.push(SystemsInst::LocalAddress(view));
            self.lower_expr(pointer)?;
            self.instructions
                .push(SystemsInst::Store(SystemType::Usize));
            self.instructions.push(SystemsInst::LocalAddress(view));
            self.instructions
                .push(SystemsInst::FieldAddress { offset: 4 });
            self.lower_expr(length)?;
            self.instructions
                .push(SystemsInst::Store(SystemType::Usize));
            self.instructions.push(SystemsInst::LocalAddress(view));
            return Ok(());
        }
        let linker_symbol = match intrinsic {
            HirIntrinsic::KernelImageStart => Some("__dbyte_image_start"),
            HirIntrinsic::KernelImageEnd => Some("__dbyte_image_end"),
            HirIntrinsic::BootstrapStackStart => Some("__dbyte_stack_bottom"),
            HirIntrinsic::BootstrapStackEnd => Some("__dbyte_stack_top"),
            HirIntrinsic::TaskTrampolineAddress => Some("__dbyte_task_trampoline"),
            _ => None,
        };
        if let Some(symbol) = linker_symbol {
            if !arguments.is_empty() {
                return Err(self.internal_hir(
                    "linker address intrinsic retained arguments",
                    expression.span,
                ));
            }
            self.instructions.push(SystemsInst::LinkerAddress {
                symbol: symbol.to_owned(),
            });
            return Ok(());
        }
        if intrinsic == HirIntrinsic::CpuReloadSegments {
            let [code, data] = arguments else {
                return Err(
                    self.internal_hir("segment reload lost its two constants", expression.span)
                );
            };
            let code_selector = hir_constant_u16(code).ok_or_else(|| {
                self.internal_hir("segment code selector is not concrete", code.span)
            })?;
            let data_selector = hir_constant_u16(data).ok_or_else(|| {
                self.internal_hir("segment data selector is not concrete", data.span)
            })?;
            self.instructions.push(SystemsInst::ReloadSegments {
                code_selector,
                data_selector,
            });
            return Ok(());
        }
        if intrinsic.is_linux() {
            return Err(self.unsupported("Linux runtime intrinsic", expression.span));
        }
        for argument in arguments {
            self.lower_expr(argument)?;
        }
        let intrinsic = match intrinsic {
            HirIntrinsic::VolatileLoad8 => SystemIntrinsic::VolatileLoad8,
            HirIntrinsic::VolatileStore8 => SystemIntrinsic::VolatileStore8,
            HirIntrinsic::VolatileLoad16 => SystemIntrinsic::VolatileLoad16,
            HirIntrinsic::VolatileStore16 => SystemIntrinsic::VolatileStore16,
            HirIntrinsic::VolatileLoad32 => SystemIntrinsic::VolatileLoad32,
            HirIntrinsic::VolatileStore32 => SystemIntrinsic::VolatileStore32,
            HirIntrinsic::VolatileLoad64 => SystemIntrinsic::VolatileLoad64,
            HirIntrinsic::VolatileStore64 => SystemIntrinsic::VolatileStore64,
            HirIntrinsic::IoOut8 => SystemIntrinsic::IoOut8,
            HirIntrinsic::IoIn8 => SystemIntrinsic::IoIn8,
            HirIntrinsic::IoOut16 => SystemIntrinsic::IoOut16,
            HirIntrinsic::IoIn16 => SystemIntrinsic::IoIn16,
            HirIntrinsic::IoOut32 => SystemIntrinsic::IoOut32,
            HirIntrinsic::IoIn32 => SystemIntrinsic::IoIn32,
            HirIntrinsic::CpuHlt => SystemIntrinsic::CpuHlt,
            HirIntrinsic::CpuCli => SystemIntrinsic::CpuCli,
            HirIntrinsic::CpuSti => SystemIntrinsic::CpuSti,
            HirIntrinsic::CpuInt3 => SystemIntrinsic::CpuInt3,
            HirIntrinsic::CpuReadEflags => SystemIntrinsic::CpuReadEflags,
            HirIntrinsic::CpuCpuidEax => SystemIntrinsic::CpuCpuidEax,
            HirIntrinsic::CpuCpuidEbx => SystemIntrinsic::CpuCpuidEbx,
            HirIntrinsic::CpuCpuidEcx => SystemIntrinsic::CpuCpuidEcx,
            HirIntrinsic::CpuCpuidEdx => SystemIntrinsic::CpuCpuidEdx,
            HirIntrinsic::CpuRdmsr => SystemIntrinsic::CpuRdmsr,
            HirIntrinsic::CpuWrmsr => SystemIntrinsic::CpuWrmsr,
            HirIntrinsic::CpuReadCr0 => SystemIntrinsic::CpuReadCr0,
            HirIntrinsic::CpuWriteCr0 => SystemIntrinsic::CpuWriteCr0,
            HirIntrinsic::CpuReadCr2 => SystemIntrinsic::CpuReadCr2,
            HirIntrinsic::CpuReadCr3 => SystemIntrinsic::CpuReadCr3,
            HirIntrinsic::CpuWriteCr3 => SystemIntrinsic::CpuWriteCr3,
            HirIntrinsic::CpuReadCr4 => SystemIntrinsic::CpuReadCr4,
            HirIntrinsic::CpuWriteCr4 => SystemIntrinsic::CpuWriteCr4,
            HirIntrinsic::CpuInvlpg => SystemIntrinsic::CpuInvlpg,
            HirIntrinsic::CpuLgdt => SystemIntrinsic::CpuLgdt,
            HirIntrinsic::CpuLidt => SystemIntrinsic::CpuLidt,
            HirIntrinsic::CpuLtr => SystemIntrinsic::CpuLtr,
            HirIntrinsic::TaskBlockTrap => SystemIntrinsic::TaskBlockTrap,
            HirIntrinsic::InterruptResume => SystemIntrinsic::InterruptResume,
            HirIntrinsic::UserSyscall0 => SystemIntrinsic::UserSyscall0,
            HirIntrinsic::UserSyscall1 => SystemIntrinsic::UserSyscall1,
            HirIntrinsic::UserSyscall2 => SystemIntrinsic::UserSyscall2,
            HirIntrinsic::UserSyscall3 => SystemIntrinsic::UserSyscall3,
            HirIntrinsic::CpuReloadSegments
            | HirIntrinsic::KernelImageStart
            | HirIntrinsic::KernelImageEnd
            | HirIntrinsic::BootstrapStackStart
            | HirIntrinsic::BootstrapStackEnd
            | HirIntrinsic::TaskTrampolineAddress
            | HirIntrinsic::RuntimeSliceFromRaw
            | HirIntrinsic::RuntimeMutSliceFromRaw
            | HirIntrinsic::RuntimeStringViewFromRaw
            | HirIntrinsic::RuntimeMemoryAlloc
            | HirIntrinsic::RuntimeMemoryResize
            | HirIntrinsic::RuntimeMemoryFree
            | HirIntrinsic::RuntimeMemoryRead8
            | HirIntrinsic::RuntimeMemoryWrite8
            | HirIntrinsic::LinuxExit
            | HirIntrinsic::LinuxRead
            | HirIntrinsic::LinuxWrite
            | HirIntrinsic::LinuxMmap
            | HirIntrinsic::LinuxMunmap
            | HirIntrinsic::LinuxOpenAt
            | HirIntrinsic::LinuxClose
            | HirIntrinsic::LinuxClockGettime
            | HirIntrinsic::WindowsGetStdHandle
            | HirIntrinsic::WindowsReadFile
            | HirIntrinsic::WindowsWriteFile
            | HirIntrinsic::WindowsCloseHandle
            | HirIntrinsic::WindowsVirtualAlloc
            | HirIntrinsic::WindowsVirtualFree
            | HirIntrinsic::WindowsQueryPerformanceCounter => unreachable!(),
        };
        let result = if matches!(
            self.program.semantic.type_of(expression.ty),
            CanonicalType::Unit
        ) {
            None
        } else {
            Some(lower_type(self.program, expression.ty, expression.span)?)
        };
        self.instructions.push(SystemsInst::Intrinsic {
            intrinsic,
            argument_count: arguments.len(),
            result,
        });
        Ok(())
    }

    fn lower_aggregate_to_local(
        &mut self,
        destination: NativeLocalId,
        value: &HirExpr,
    ) -> NativeResult<()> {
        match &value.kind {
            HirExprKind::Closure {
                closure, captures, ..
            } => self.materialize_closure(destination, *closure, captures, value.span),
            HirExprKind::String(bytes)
                if matches!(
                    self.program.semantic.type_of(value.ty),
                    CanonicalType::StringView
                ) =>
            {
                self.instructions
                    .push(SystemsInst::LocalAddress(destination));
                if self.runtime_drop_enabled() {
                    let id = if let Some(id) =
                        self.strings.iter().position(|candidate| candidate == bytes)
                    {
                        id
                    } else {
                        self.strings.push(bytes.clone());
                        self.strings.len() - 1
                    };
                    self.instructions
                        .push(SystemsInst::StringAddress(StringId(id as u32)));
                } else {
                    self.lower_expr(value)?;
                }
                self.instructions
                    .push(SystemsInst::Store(SystemType::Usize));
                self.instructions
                    .push(SystemsInst::LocalAddress(destination));
                self.instructions
                    .push(SystemsInst::FieldAddress { offset: 4 });
                self.instructions.push(SystemsInst::Const {
                    bits: u32::try_from(bytes.len()).map_err(|_| {
                        self.error(
                            DiagnosticCategory::LayoutOverflow,
                            "StringView literal length exceeds usize",
                            value.span,
                        )
                    })?,
                    ty: SystemType::Usize,
                });
                self.instructions
                    .push(SystemsInst::Store(SystemType::Usize));
                Ok(())
            }
            HirExprKind::Borrow { place, .. }
                if matches!(
                    self.program.semantic.type_of(value.ty),
                    CanonicalType::Slice { .. }
                ) =>
            {
                let CanonicalType::Array { length, .. } = self.program.semantic.type_of(place.ty)
                else {
                    return Err(self.unsupported("slice source view", value.span));
                };
                self.instructions
                    .push(SystemsInst::LocalAddress(destination));
                self.lower_place_address(place)?;
                self.instructions
                    .push(SystemsInst::Store(SystemType::Usize));
                self.instructions
                    .push(SystemsInst::LocalAddress(destination));
                self.instructions
                    .push(SystemsInst::FieldAddress { offset: 4 });
                self.instructions.push(SystemsInst::Const {
                    bits: *length,
                    ty: SystemType::Usize,
                });
                self.instructions
                    .push(SystemsInst::Store(SystemType::Usize));
                Ok(())
            }
            HirExprKind::Struct { fields, .. } => {
                for (field, field_value) in fields {
                    let offset =
                        hir_field_offset(&self.program.semantic, value.ty, *field, value.span)?;
                    self.instructions
                        .push(SystemsInst::LocalAddress(destination));
                    if offset != 0 {
                        self.instructions.push(SystemsInst::FieldAddress { offset });
                    }
                    self.lower_expr(field_value)?;
                    self.store_value(field_value.ty, field_value.span)?;
                }
                Ok(())
            }
            HirExprKind::List(values) => {
                let CanonicalType::Array { element, .. } = self.program.semantic.type_of(value.ty)
                else {
                    return Err(self.unsupported("hosted list initializer", value.span));
                };
                let element_size = self.layout(*element, value.span)?.size;
                for (index, element) in values.iter().enumerate() {
                    self.instructions
                        .push(SystemsInst::LocalAddress(destination));
                    let offset = u32::try_from(index)
                        .ok()
                        .and_then(|index| index.checked_mul(element_size))
                        .ok_or_else(|| {
                            self.error(
                                DiagnosticCategory::LayoutOverflow,
                                "array initializer offset overflow",
                                element.span,
                            )
                        })?;
                    if offset != 0 {
                        self.instructions.push(SystemsInst::FieldAddress { offset });
                    }
                    self.lower_expr(element)?;
                    self.store_value(element.ty, element.span)?;
                }
                Ok(())
            }
            _ => {
                let size = self.layout(value.ty, value.span)?.size;
                self.instructions
                    .push(SystemsInst::LocalAddress(destination));
                self.lower_expr(value)?;
                self.instructions.push(SystemsInst::AggregateCopy { size });
                Ok(())
            }
        }
    }

    fn lower_local_value(
        &mut self,
        local: dbyte_hir::LocalId,
        ty: TypeId,
        span: Span,
    ) -> NativeResult<()> {
        let native_ty = lower_type(self.program, ty, span)?;
        if self
            .capture_env
            .as_ref()
            .is_some_and(|environment| environment.captures.contains_key(&local))
        {
            self.lower_local_address(local, span)?;
            if native_ty.is_scalar() {
                self.instructions.push(SystemsInst::Load(native_ty));
            }
        } else if native_ty.is_scalar() {
            self.instructions.push(SystemsInst::LocalLoad {
                local: NativeLocalId(local.0),
                ty: native_ty,
            });
        } else {
            self.instructions
                .push(SystemsInst::LocalAddress(NativeLocalId(local.0)));
        }
        Ok(())
    }

    fn lower_local_address(&mut self, local: dbyte_hir::LocalId, span: Span) -> NativeResult<()> {
        let capture = self
            .capture_env
            .as_ref()
            .and_then(|environment| environment.captures.get(&local).cloned());
        if let Some(capture) = capture {
            let environment = self.capture_env.as_ref().expect("capture environment");
            self.instructions.push(SystemsInst::LocalLoad {
                local: environment.parameter,
                ty: SystemType::Pointer(Box::new(SystemType::U8)),
            });
            if capture.offset != 0 {
                self.instructions.push(SystemsInst::FieldAddress {
                    offset: capture.offset,
                });
            }
            if capture.mode != CaptureMode::Move {
                self.instructions
                    .push(SystemsInst::Load(SystemType::Pointer(Box::new(
                        lower_type(self.program, capture.ty, span)?,
                    ))));
            }
        } else {
            self.instructions
                .push(SystemsInst::LocalAddress(NativeLocalId(local.0)));
        }
        Ok(())
    }

    fn materialize_closure(
        &mut self,
        destination: NativeLocalId,
        closure: dbyte_hir::ClosureId,
        captures: &[dbyte_hir::HirCapture],
        span: Span,
    ) -> NativeResult<()> {
        let symbol = self
            .closure_symbols
            .get(&closure)
            .cloned()
            .ok_or_else(|| self.internal_hir("closure helper symbol is missing", span))?;
        let (layout, accesses) = closure_environment_layout(self.program, captures, span)?;
        let environment = if captures.is_empty() {
            None
        } else {
            let local = self.allocate_system_temp(
                SystemType::Array(Box::new(SystemType::U8), layout.size),
                layout.size,
                layout.align,
                "closure_env",
            )?;
            self.instructions.push(SystemsInst::LocalAddress(local));
            self.instructions
                .push(SystemsInst::AggregateZero { size: layout.size });
            for capture in captures {
                let access = &accesses[&capture.local];
                self.instructions.push(SystemsInst::LocalAddress(local));
                if access.offset != 0 {
                    self.instructions.push(SystemsInst::FieldAddress {
                        offset: access.offset,
                    });
                }
                if capture.mode == CaptureMode::Move {
                    self.lower_local_value(capture.local, capture.ty, capture.span)?;
                    self.store_value(capture.ty, capture.span)?;
                } else {
                    self.lower_local_address(capture.local, capture.span)?;
                    self.instructions
                        .push(SystemsInst::Store(SystemType::Pointer(Box::new(
                            lower_type(self.program, capture.ty, capture.span)?,
                        ))));
                }
            }
            Some(local)
        };
        self.instructions
            .push(SystemsInst::LocalAddress(destination));
        self.instructions
            .push(SystemsInst::FunctionAddress { symbol });
        self.instructions
            .push(SystemsInst::Store(SystemType::Usize));
        self.instructions
            .push(SystemsInst::LocalAddress(destination));
        self.instructions
            .push(SystemsInst::FieldAddress { offset: 4 });
        if let Some(environment) = environment {
            self.instructions
                .push(SystemsInst::LocalAddress(environment));
        } else {
            self.instructions.push(SystemsInst::Const {
                bits: 0,
                ty: SystemType::Usize,
            });
        }
        self.instructions
            .push(SystemsInst::Store(SystemType::Usize));
        Ok(())
    }

    fn lower_place_address(&mut self, place: &HirPlace) -> NativeResult<()> {
        let mut current_ty = match &place.base {
            HirPlaceBase::Local(local) => self
                .function
                .locals
                .iter()
                .find(|candidate| candidate.id == *local)
                .map(|local| local.ty)
                .ok_or_else(|| self.internal_hir("place local type is missing", place.span))?,
            HirPlaceBase::Pointer(pointer)
                if matches!(place.projections.first(), Some(HirProjection::Index { .. })) =>
            {
                // Raw-pointer indexing operates on the pointer's pointee, but
                // an explicit dereference of a borrowed aggregate (for
                // example `(*view)[index]`) must first expose that aggregate's
                // canonical representation before applying the index.
                match self.program.semantic.type_of(pointer.ty) {
                    CanonicalType::Borrow { pointee, .. } => *pointee,
                    _ => pointer.ty,
                }
            }
            HirPlaceBase::Pointer(pointer) => match self.program.semantic.type_of(pointer.ty) {
                CanonicalType::Pointer { pointee, .. } | CanonicalType::Borrow { pointee, .. } => {
                    *pointee
                }
                _ => pointer.ty,
            },
            HirPlaceBase::Global(symbol) => self.global(*symbol, place.span)?.ty,
        };
        match &place.base {
            HirPlaceBase::Local(local) => self.lower_local_address(*local, place.span)?,
            HirPlaceBase::Pointer(pointer) => self.lower_expr(pointer)?,
            HirPlaceBase::Global(symbol) => {
                let global = self.global(*symbol, place.span)?;
                self.instructions.push(SystemsInst::GlobalAddress {
                    symbol: global.name.clone(),
                    pointee: lower_type(self.program, global.ty, place.span)?,
                });
            }
        }
        for projection in &place.projections {
            match projection {
                HirProjection::Field { ty, offset, .. }
                | HirProjection::Payload { ty, offset, .. } => {
                    self.instructions
                        .push(SystemsInst::FieldAddress { offset: *offset });
                    current_ty = *ty;
                }
                HirProjection::Index {
                    index,
                    element_size,
                    ..
                } => match self.program.semantic.type_of(current_ty).clone() {
                    CanonicalType::Slice { element, .. } => {
                        let view = self.allocate_system_temp(
                            SystemType::Pointer(Box::new(SystemType::U8)),
                            4,
                            4,
                            "slice_view",
                        )?;
                        let index_local =
                            self.allocate_system_temp(SystemType::Usize, 4, 4, "slice_index")?;
                        self.instructions.push(SystemsInst::LocalStore {
                            local: view,
                            ty: SystemType::Pointer(Box::new(SystemType::U8)),
                        });
                        self.lower_expr(index)?;
                        let index_ty = lower_type(self.program, index.ty, index.span)?;
                        if index_ty != SystemType::Usize {
                            self.instructions.push(SystemsInst::Cast {
                                from: index_ty,
                                to: SystemType::Usize,
                            });
                        }
                        self.instructions.push(SystemsInst::LocalStore {
                            local: index_local,
                            ty: SystemType::Usize,
                        });
                        self.instructions.push(SystemsInst::LocalLoad {
                            local: index_local,
                            ty: SystemType::Usize,
                        });
                        self.instructions.push(SystemsInst::LocalLoad {
                            local: view,
                            ty: SystemType::Pointer(Box::new(SystemType::U8)),
                        });
                        self.instructions
                            .push(SystemsInst::FieldAddress { offset: 4 });
                        self.instructions.push(SystemsInst::Load(SystemType::Usize));
                        self.instructions.push(SystemsInst::Compare {
                            op: CompareOp::Lt,
                            operand_type: SystemType::Usize,
                        });
                        let invalid = self.label();
                        let valid = self.label();
                        self.instructions.push(SystemsInst::JumpIfFalse(invalid));
                        self.instructions.push(SystemsInst::Jump(valid));
                        self.instructions.push(SystemsInst::Label(invalid));
                        self.instructions.push(SystemsInst::Trap);
                        self.instructions.push(SystemsInst::Label(valid));
                        self.instructions.push(SystemsInst::LocalLoad {
                            local: view,
                            ty: SystemType::Pointer(Box::new(SystemType::U8)),
                        });
                        self.instructions.push(SystemsInst::Load(SystemType::Usize));
                        self.instructions.push(SystemsInst::LocalLoad {
                            local: index_local,
                            ty: SystemType::Usize,
                        });
                        self.instructions.push(SystemsInst::IndexAddress {
                            element_size: *element_size,
                        });
                        current_ty = element;
                    }
                    CanonicalType::StringView => {
                        let element = self
                            .program
                            .semantic
                            .types
                            .iter()
                            .position(|ty| matches!(ty, CanonicalType::Integer(IntegerType::U8)))
                            .map(|index| TypeId(index as u32))
                            .ok_or_else(|| {
                                self.internal_hir("StringView byte type is missing", index.span)
                            })?;
                        let view = self.allocate_system_temp(
                            SystemType::Pointer(Box::new(SystemType::U8)),
                            4,
                            4,
                            "string_view",
                        )?;
                        let index_local =
                            self.allocate_system_temp(SystemType::Usize, 4, 4, "string_index")?;
                        self.instructions.push(SystemsInst::LocalStore {
                            local: view,
                            ty: SystemType::Pointer(Box::new(SystemType::U8)),
                        });
                        self.lower_expr(index)?;
                        let index_ty = lower_type(self.program, index.ty, index.span)?;
                        if index_ty != SystemType::Usize {
                            self.instructions.push(SystemsInst::Cast {
                                from: index_ty,
                                to: SystemType::Usize,
                            });
                        }
                        self.instructions.push(SystemsInst::LocalStore {
                            local: index_local,
                            ty: SystemType::Usize,
                        });
                        self.instructions.push(SystemsInst::LocalLoad {
                            local: index_local,
                            ty: SystemType::Usize,
                        });
                        self.instructions.push(SystemsInst::LocalLoad {
                            local: view,
                            ty: SystemType::Pointer(Box::new(SystemType::U8)),
                        });
                        self.instructions
                            .push(SystemsInst::FieldAddress { offset: 4 });
                        self.instructions.push(SystemsInst::Load(SystemType::Usize));
                        self.instructions.push(SystemsInst::Compare {
                            op: CompareOp::Lt,
                            operand_type: SystemType::Usize,
                        });
                        let invalid = self.label();
                        let valid = self.label();
                        self.instructions.push(SystemsInst::JumpIfFalse(invalid));
                        self.instructions.push(SystemsInst::Jump(valid));
                        self.instructions.push(SystemsInst::Label(invalid));
                        self.instructions.push(SystemsInst::Trap);
                        self.instructions.push(SystemsInst::Label(valid));
                        self.instructions.push(SystemsInst::LocalLoad {
                            local: view,
                            ty: SystemType::Pointer(Box::new(SystemType::U8)),
                        });
                        self.instructions.push(SystemsInst::Load(SystemType::Usize));
                        self.instructions.push(SystemsInst::LocalLoad {
                            local: index_local,
                            ty: SystemType::Usize,
                        });
                        self.instructions.push(SystemsInst::IndexAddress {
                            element_size: *element_size,
                        });
                        current_ty = element;
                    }
                    CanonicalType::Array { element, .. } => {
                        self.lower_expr(index)?;
                        self.instructions.push(SystemsInst::IndexAddress {
                            element_size: *element_size,
                        });
                        current_ty = element;
                    }
                    CanonicalType::Pointer { pointee, .. } => {
                        self.lower_expr(index)?;
                        self.instructions.push(SystemsInst::IndexAddress {
                            element_size: *element_size,
                        });
                        current_ty = pointee;
                    }
                    CanonicalType::LegacyStr => {
                        self.lower_expr(index)?;
                        self.instructions.push(SystemsInst::IndexAddress {
                            element_size: *element_size,
                        });
                        current_ty = place.ty;
                    }
                    _ => {
                        return Err(self.internal_hir(
                            "index projection has no aggregate representation",
                            index.span,
                        ))
                    }
                },
            }
        }
        Ok(())
    }

    fn lower_match(
        &mut self,
        value: &HirExpr,
        arms: &[HirMatchArm],
        span: Span,
    ) -> NativeResult<()> {
        let enum_pointer = self.allocate_system_temp(
            SystemType::Pointer(Box::new(SystemType::U8)),
            4,
            4,
            "match_value",
        )?;
        self.lower_expr(value)?;
        self.instructions.push(SystemsInst::LocalStore {
            local: enum_pointer,
            ty: SystemType::Pointer(Box::new(SystemType::U8)),
        });
        let done = self.label();
        for arm in arms {
            let next = self.label();
            if let Some(variant) = arm.variant {
                self.instructions.push(SystemsInst::LocalLoad {
                    local: enum_pointer,
                    ty: SystemType::Pointer(Box::new(SystemType::U8)),
                });
                self.instructions.push(SystemsInst::Load(SystemType::U32));
                self.instructions.push(SystemsInst::Const {
                    bits: variant.discriminant,
                    ty: SystemType::U32,
                });
                self.instructions.push(SystemsInst::Compare {
                    op: CompareOp::Eq,
                    operand_type: SystemType::U32,
                });
                self.instructions.push(SystemsInst::JumpIfFalse(next));
                let (_, _, payload) = dbyte_hir::enum_variant_layout(
                    &self.program.semantic,
                    value.ty,
                    variant.variant,
                )
                .map_err(hir_diagnostic)?;
                for (binding, (payload_ty, offset)) in arm.bindings.iter().zip(payload) {
                    if matches!(
                        self.program.semantic.type_of(payload_ty),
                        CanonicalType::Unit
                    ) {
                        continue;
                    }
                    let binding_ty = lower_type(self.program, binding.ty, binding.span)?;
                    if matches!(
                        self.program.semantic.type_of(binding.ty),
                        CanonicalType::Borrow { .. }
                    ) && !matches!(
                        self.program.semantic.type_of(payload_ty),
                        CanonicalType::Borrow { .. }
                    ) {
                        // `match borrow` creates a view into an owned payload.
                        // A payload which is already a borrow, however, stores
                        // the pointer value itself and must be loaded below;
                        // taking the payload slot address adds a bogus level
                        // of indirection.
                        self.instructions.push(SystemsInst::LocalLoad {
                            local: enum_pointer,
                            ty: SystemType::Pointer(Box::new(SystemType::U8)),
                        });
                        if offset != 0 {
                            self.instructions.push(SystemsInst::FieldAddress { offset });
                        }
                        self.instructions.push(SystemsInst::LocalStore {
                            local: NativeLocalId(binding.id.0),
                            ty: binding_ty,
                        });
                    } else if self.is_scalar(payload_ty) {
                        self.instructions.push(SystemsInst::LocalLoad {
                            local: enum_pointer,
                            ty: SystemType::Pointer(Box::new(SystemType::U8)),
                        });
                        if offset != 0 {
                            self.instructions.push(SystemsInst::FieldAddress { offset });
                        }
                        self.instructions.push(SystemsInst::Load(lower_type(
                            self.program,
                            payload_ty,
                            span,
                        )?));
                        self.instructions.push(SystemsInst::LocalStore {
                            local: NativeLocalId(binding.id.0),
                            ty: binding_ty,
                        });
                    } else {
                        self.instructions
                            .push(SystemsInst::LocalAddress(NativeLocalId(binding.id.0)));
                        self.instructions.push(SystemsInst::LocalLoad {
                            local: enum_pointer,
                            ty: SystemType::Pointer(Box::new(SystemType::U8)),
                        });
                        if offset != 0 {
                            self.instructions.push(SystemsInst::FieldAddress { offset });
                        }
                        self.instructions.push(SystemsInst::AggregateCopy {
                            size: self.layout(payload_ty, span)?.size,
                        });
                    }
                }
            }
            self.lower_block(&arm.body)?;
            self.instructions.push(SystemsInst::Jump(done));
            self.instructions.push(SystemsInst::Label(next));
        }
        self.instructions.push(SystemsInst::Trap);
        self.instructions.push(SystemsInst::Label(done));
        Ok(())
    }

    fn materialize_call_pack(
        &mut self,
        arguments: &[PackedArgument<'_>],
        span: Span,
    ) -> NativeResult<NativeLocalId> {
        let total = packed_size(arguments, self.program)?;
        let pack = self.allocate_system_temp(
            SystemType::Array(Box::new(SystemType::U8), total),
            total,
            4,
            "call_pack",
        )?;
        self.instructions.push(SystemsInst::LocalAddress(pack));
        self.instructions
            .push(SystemsInst::AggregateZero { size: total });
        let sizes = arguments
            .iter()
            .map(|argument| packed_argument_size(argument, self.program))
            .collect::<NativeResult<Vec<_>>>()?;
        for (index, argument) in arguments.iter().enumerate() {
            let offset = sizes[index + 1..].iter().copied().sum::<u32>();
            self.instructions.push(SystemsInst::LocalAddress(pack));
            if offset != 0 {
                self.instructions.push(SystemsInst::FieldAddress { offset });
            }
            match argument {
                PackedArgument::Address(local) => {
                    self.instructions.push(SystemsInst::LocalAddress(*local));
                    self.instructions
                        .push(SystemsInst::Store(SystemType::Pointer(Box::new(
                            SystemType::U8,
                        ))));
                }
                PackedArgument::Value(value) => {
                    self.lower_expr(value)?;
                    self.store_value(value.ty, span)?;
                }
            }
        }
        Ok(pack)
    }

    fn store_value(&mut self, ty: TypeId, span: Span) -> NativeResult<()> {
        let lowered = lower_type(self.program, ty, span)?;
        if lowered.is_scalar() {
            self.instructions.push(SystemsInst::Store(lowered));
        } else {
            self.instructions.push(SystemsInst::AggregateCopy {
                size: self.layout(ty, span)?.size,
            });
        }
        Ok(())
    }

    fn allocate_temp(
        &mut self,
        ty: TypeId,
        purpose: &str,
        span: Span,
    ) -> NativeResult<NativeLocalId> {
        let layout = self.layout(ty, span)?;
        self.allocate_system_temp(
            lower_type(self.program, ty, span)?,
            layout.size,
            layout.align,
            purpose,
        )
    }

    fn allocate_system_temp(
        &mut self,
        ty: SystemType,
        size: u32,
        alignment: u32,
        purpose: &str,
    ) -> NativeResult<NativeLocalId> {
        if size == 0 {
            return Err(self.error(
                DiagnosticCategory::InvalidType,
                "zero-size temporary has no native storage",
                self.function.span,
            ));
        }
        let id = NativeLocalId(self.slots.len() as u32);
        self.slots.push(SystemsSlot {
            id,
            name: format!("__dbyte_{purpose}_{}", id.0),
            ty,
            storage_size: size,
            alignment,
            is_parameter: false,
        });
        Ok(id)
    }

    fn layout(&self, ty: TypeId, _span: Span) -> NativeResult<dbyte_hir::Layout> {
        layout_of(&self.program.semantic, ty).map_err(hir_diagnostic)
    }

    fn is_scalar(&self, ty: TypeId) -> bool {
        lower_type(self.program, ty, self.function.span).is_ok_and(|ty| ty.is_scalar())
    }

    fn require_scalar(&self, ty: &SystemType, span: Span) -> NativeResult<()> {
        if ty.is_scalar() {
            Ok(())
        } else {
            Err(self.unsupported("aggregate value ABI", span))
        }
    }

    fn label(&mut self) -> LabelId {
        let label = LabelId(self.labels);
        self.labels += 1;
        label
    }

    fn unsupported(&self, capability: &str, span: Span) -> Diagnostic {
        self.error(
            DiagnosticCategory::UnsupportedFeature,
            format!(
                "E_NATIVE_HIR_UNSUPPORTED: {capability} is not implemented for i686 native HIR lowering"
            ),
            span,
        )
    }

    fn internal_hir(&self, message: &str, span: Span) -> Diagnostic {
        self.error(
            DiagnosticCategory::UnsupportedFeature,
            format!("E_NATIVE_HIR_INVARIANT: {message}"),
            span,
        )
    }

    fn global(
        &self,
        symbol: dbyte_hir::SymbolId,
        span: Span,
    ) -> NativeResult<&dbyte_hir::HirGlobal> {
        self.program
            .globals
            .iter()
            .find(|global| global.symbol == symbol && !global.constant)
            .ok_or_else(|| self.internal_hir("global symbol has no concrete storage", span))
    }

    fn error(
        &self,
        category: DiagnosticCategory,
        message: impl Into<String>,
        span: Span,
    ) -> Diagnostic {
        Diagnostic::new(category, message, span)
    }
}

fn lower_global(
    program: &ConcreteHirProgram,
    global: &dbyte_hir::HirGlobal,
) -> NativeResult<SystemsGlobal> {
    let layout = layout_of(&program.semantic, global.ty).map_err(hir_diagnostic)?;
    let bytes = match &global.initializer {
        Some(initializer) => {
            encode_global_initializer(program, initializer, global.ty, global.span)?
        }
        None => Vec::new(),
    };
    Ok(SystemsGlobal {
        source_name: global.name.clone(),
        symbol: global.name.clone(),
        visibility: if global.public {
            Visibility::Public
        } else {
            Visibility::Private
        },
        ty: lower_type(program, global.ty, global.span)?,
        mutable: global.mutable,
        section: if bytes.is_empty() {
            GlobalSection::Bss
        } else if global.mutable {
            GlobalSection::Data
        } else {
            GlobalSection::ReadOnly
        },
        bytes,
        size: layout.size,
        alignment: layout.align,
        span: global.span,
    })
}

fn encode_global_initializer(
    program: &ConcreteHirProgram,
    initializer: &HirExpr,
    ty: TypeId,
    span: Span,
) -> NativeResult<Vec<u8>> {
    let layout = layout_of(&program.semantic, ty).map_err(hir_diagnostic)?;
    if let (CanonicalType::Array { element, length }, HirExprKind::List(values)) =
        (program.semantic.type_of(ty), &initializer.kind)
    {
        if values.len() != *length as usize {
            return Err(Diagnostic::new(
                DiagnosticCategory::InvalidType,
                "E_NATIVE_HIR_INVARIANT: concrete array initializer length changed",
                initializer.span,
            ));
        }
        let element_layout = layout_of(&program.semantic, *element).map_err(hir_diagnostic)?;
        let mut bytes = vec![0; layout.size as usize];
        for (index, value) in values.iter().enumerate() {
            let encoded = encode_global_initializer(program, value, *element, value.span)?;
            let offset = index * element_layout.size as usize;
            bytes[offset..offset + encoded.len()].copy_from_slice(&encoded);
        }
        return Ok(bytes);
    }
    if let (CanonicalType::Struct { .. }, HirExprKind::Struct { fields, .. }) =
        (program.semantic.type_of(ty), &initializer.kind)
    {
        let mut bytes = vec![0; layout.size as usize];
        for (field, value) in fields {
            let offset = hir_field_offset(&program.semantic, ty, *field, value.span)?;
            let encoded = encode_global_initializer(program, value, value.ty, value.span)?;
            let offset = offset as usize;
            bytes[offset..offset + encoded.len()].copy_from_slice(&encoded);
        }
        return Ok(bytes);
    }
    let bits = hir_static_bits(program, initializer, ty, span)?;
    Ok(bits.to_le_bytes()[..layout.size as usize].to_vec())
}

fn hir_static_bits(
    program: &ConcreteHirProgram,
    initializer: &HirExpr,
    ty: TypeId,
    span: Span,
) -> NativeResult<u64> {
    match &initializer.kind {
        HirExprKind::Integer(value) => {
            let lowered = lower_type(program, ty, span)?;
            if lowered.is_wide_integer() {
                integer_bits64(*value, &lowered, span)
            } else {
                integer_bits(*value, &lowered, span).map(u64::from)
            }
        }
        HirExprKind::Bool(value) => Ok(u64::from(*value)),
        HirExprKind::Null => Ok(0),
        HirExprKind::Cast { value, .. } | HirExprKind::Constant { value, .. } => {
            hir_static_bits(program, value, ty, span)
        }
        _ => Err(Diagnostic::new(
            DiagnosticCategory::UnsupportedFeature,
            "E_NATIVE_HIR_UNSUPPORTED: non-scalar static initializer",
            initializer.span,
        )),
    }
}

fn hir_constant_u16(expression: &HirExpr) -> Option<u16> {
    let value = match &expression.kind {
        HirExprKind::Integer(value) => *value,
        HirExprKind::Cast { value, .. } | HirExprKind::Constant { value, .. } => {
            return hir_constant_u16(value)
        }
        _ => return None,
    };
    u16::try_from(value).ok()
}

fn packed_argument_size(
    argument: &PackedArgument<'_>,
    program: &ConcreteHirProgram,
) -> NativeResult<u32> {
    let size = match argument {
        PackedArgument::Address(_) => 4,
        PackedArgument::Value(value) => {
            if matches!(program.semantic.type_of(value.ty), CanonicalType::Unit) {
                // Unit has zero semantic storage, but the legacy DBYTE32 call
                // ABI materializes it as the same deterministic i32 zero used
                // by scalar calls. Packed calls must reserve that physical
                // slot or every preceding argument is shifted at the callee.
                4
            } else {
                layout_of(&program.semantic, value.ty)
                    .map_err(hir_diagnostic)?
                    .size
            }
        }
    };
    Ok((size + 3) & !3)
}

fn packed_size(
    arguments: &[PackedArgument<'_>],
    program: &ConcreteHirProgram,
) -> NativeResult<u32> {
    arguments.iter().try_fold(0u32, |total, argument| {
        total
            .checked_add(packed_argument_size(argument, program)?)
            .ok_or_else(|| {
                Diagnostic::without_span(
                    DiagnosticCategory::LayoutOverflow,
                    "packed DByte call argument area overflow",
                )
            })
    })
}

fn lower_type(program: &ConcreteHirProgram, ty: TypeId, span: Span) -> NativeResult<SystemType> {
    Ok(match program.semantic.type_of(ty) {
        CanonicalType::Unit => SystemType::I32,
        CanonicalType::Bool => SystemType::Bool,
        CanonicalType::Integer(integer) => match integer {
            IntegerType::I8 => SystemType::I8,
            IntegerType::U8 => SystemType::U8,
            IntegerType::I16 => SystemType::I16,
            IntegerType::U16 => SystemType::U16,
            IntegerType::I32 | IntegerType::LegacyInt => SystemType::I32,
            IntegerType::U32 => SystemType::U32,
            IntegerType::I64 => SystemType::I64,
            IntegerType::U64 => SystemType::U64,
            IntegerType::Usize => SystemType::Usize,
        },
        CanonicalType::LegacyStr => SystemType::Str,
        CanonicalType::Pointer { pointee, .. } | CanonicalType::Borrow { pointee, .. } => {
            SystemType::Pointer(Box::new(lower_type(program, *pointee, span)?))
        }
        CanonicalType::Array { element, length } => {
            SystemType::Array(Box::new(lower_type(program, *element, span)?), *length)
        }
        CanonicalType::Struct { symbol, args } => {
            SystemType::Struct(mangle_aggregate("struct", symbol.0, args))
        }
        CanonicalType::Enum { symbol, args } => {
            SystemType::Struct(mangle_aggregate("enum", symbol.0, args))
        }
        CanonicalType::Slice { .. }
        | CanonicalType::StringView
        | CanonicalType::ClosureShape { .. } => {
            SystemType::Struct(format!("__dbyte_hir_aggregate_t{}", ty.0))
        }
        CanonicalType::Function { .. } => SystemType::Pointer(Box::new(SystemType::U8)),
        CanonicalType::GenericParam { .. } => {
            return Err(Diagnostic::new(
                DiagnosticCategory::UnsupportedFeature,
                format!(
                    "E_HIR_OPEN_GENERIC: generic type t{} reached native adapter",
                    ty.0
                ),
                span,
            ));
        }
        CanonicalType::Float
        | CanonicalType::F32
        | CanonicalType::F64
        | CanonicalType::LegacyBytes
        | CanonicalType::LegacyBuffer
        | CanonicalType::LegacyList(_) => {
            return Err(Diagnostic::new(
                DiagnosticCategory::UnsupportedFeature,
                "E_NATIVE_HIR_UNSUPPORTED: hosted compatibility type is unavailable in native HIR lowering",
                span,
            ));
        }
    })
}

fn hir_field_offset(
    semantic: &dbyte_hir::HirProgram,
    ty: TypeId,
    field: dbyte_hir::FieldId,
    span: Span,
) -> NativeResult<u32> {
    let CanonicalType::Struct { symbol, args } = semantic.type_of(ty) else {
        return Err(Diagnostic::new(
            DiagnosticCategory::InvalidType,
            "E_NATIVE_HIR_INVARIANT: field owner is not a concrete struct",
            span,
        ));
    };
    let definition = semantic
        .definitions
        .iter()
        .find(|definition| definition.symbol == *symbol)
        .ok_or_else(|| {
            Diagnostic::new(
                DiagnosticCategory::InvalidType,
                "E_NATIVE_HIR_INVARIANT: field owner definition is missing",
                span,
            )
        })?;
    let dbyte_hir::TypeDefKind::Struct { fields, .. } = &definition.kind else {
        unreachable!()
    };
    let index = fields
        .iter()
        .position(|candidate| candidate.id == field)
        .ok_or_else(|| {
            Diagnostic::new(
                DiagnosticCategory::InvalidType,
                "E_NATIVE_HIR_INVARIANT: field identity is missing",
                span,
            )
        })?;
    let (_, offsets) = dbyte_hir::layout_struct(semantic, *symbol, args).map_err(hir_diagnostic)?;
    Ok(offsets[index])
}

fn closure_environment_layout(
    program: &ConcreteHirProgram,
    captures: &[dbyte_hir::HirCapture],
    span: Span,
) -> NativeResult<(
    dbyte_hir::Layout,
    BTreeMap<dbyte_hir::LocalId, CaptureAccess>,
)> {
    let mut size = 0u32;
    let mut align = 1u32;
    let mut output = BTreeMap::new();
    for capture in captures {
        let layout = if capture.mode == CaptureMode::Move {
            layout_of(&program.semantic, capture.ty).map_err(hir_diagnostic)?
        } else {
            dbyte_hir::Layout { size: 4, align: 4 }
        };
        let mask = layout.align.checked_sub(1).ok_or_else(|| {
            Diagnostic::new(
                DiagnosticCategory::LayoutOverflow,
                "closure capture alignment is zero",
                span,
            )
        })?;
        size = size
            .checked_add(mask)
            .map(|value| value & !mask)
            .ok_or_else(|| {
                Diagnostic::new(
                    DiagnosticCategory::LayoutOverflow,
                    "closure environment offset overflow",
                    span,
                )
            })?;
        output.insert(
            capture.local,
            CaptureAccess {
                offset: size,
                mode: capture.mode,
                ty: capture.ty,
            },
        );
        size = size.checked_add(layout.size).ok_or_else(|| {
            Diagnostic::new(
                DiagnosticCategory::LayoutOverflow,
                "closure environment size overflow",
                span,
            )
        })?;
        align = align.max(layout.align);
    }
    let mask = align - 1;
    size = size
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or_else(|| {
            Diagnostic::new(
                DiagnosticCategory::LayoutOverflow,
                "closure environment tail padding overflow",
                span,
            )
        })?;
    Ok((dbyte_hir::Layout { size, align }, output))
}

fn mangle_aggregate(prefix: &str, symbol: u32, args: &[TypeId]) -> String {
    let mut output = format!("__dbyte_{prefix}_s{symbol}");
    for argument in args {
        output.push_str(&format!("_t{}", argument.0));
    }
    output
}

fn integer_bits(value: i64, ty: &SystemType, span: Span) -> NativeResult<u32> {
    let fits = match ty {
        SystemType::I8 => i8::try_from(value).is_ok(),
        SystemType::U8 => u8::try_from(value).is_ok(),
        SystemType::I16 => i16::try_from(value).is_ok(),
        SystemType::U16 => u16::try_from(value).is_ok(),
        SystemType::I32 => i32::try_from(value).is_ok(),
        SystemType::U32 | SystemType::Usize => u32::try_from(value).is_ok(),
        _ => false,
    };
    if !fits {
        return Err(Diagnostic::new(
            DiagnosticCategory::IntegerOutOfRange,
            format!("integer literal `{value}` is outside `{ty}`"),
            span,
        ));
    }
    Ok(value as u32)
}

fn integer_bits64(value: i64, ty: &SystemType, span: Span) -> NativeResult<u64> {
    let fits = match ty {
        SystemType::I64 => true,
        SystemType::U64 => value >= 0,
        _ => false,
    };
    if !fits {
        return Err(Diagnostic::new(
            DiagnosticCategory::IntegerOutOfRange,
            format!("integer literal `{value}` is outside `{ty}`"),
            span,
        ));
    }
    Ok(value as u64)
}

fn compare(op: BinOp) -> CompareOp {
    match op {
        BinOp::EqEq => CompareOp::Eq,
        BinOp::NotEq => CompareOp::NotEq,
        BinOp::Lt => CompareOp::Lt,
        BinOp::Gt => CompareOp::Gt,
        BinOp::LtEq => CompareOp::LtEq,
        BinOp::GtEq => CompareOp::GtEq,
        _ => unreachable!("comparison caller filters operators"),
    }
}

fn hir_diagnostic(error: dbyte_hir::Diagnostic) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCategory::InvalidType,
        format!("{}: {}", error.code, error.message),
        error.span,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;

    fn concrete(source: &str) -> ConcreteHirProgram {
        concrete_profile(source, dbyte_hir::Profile::I686None)
    }

    fn concrete_profile(source: &str, profile: dbyte_hir::Profile) -> ConcreteHirProgram {
        let tokens = Lexer::new(source).tokenize().unwrap();
        let program = Parser::new(tokens).parse_program().unwrap();
        let executable =
            dbyte_hir::analyze_executable(&program, profile, dbyte_hir::TargetDataLayout::I686)
                .unwrap();
        dbyte_hir::monomorphize(&executable).unwrap()
    }

    #[test]
    fn generic_call_reaches_existing_systems_ir_without_open_types() {
        let program = concrete("fn id[T](value: T) -> T:\n    return value\n\npub fn kernel_main() -> int:\n    let answer: i32 = id[i32](42)\n    return answer as int\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        assert_eq!(lowered.functions.len(), 2);
        assert!(lowered.dump().contains("__dbyte_mono_id_t"));
        assert!(!lowered.dump().contains("GenericParam"));
        let machine = crate::systems_lower::lower_systems_module(&lowered).unwrap();
        let object = crate::object::write_relocatable(&machine.image).unwrap();
        let linked = crate::linker::link_object_bytes(&[crate::linker::LinkInput {
            name: "kernel.o".into(),
            bytes: object.bytes,
        }])
        .unwrap();
        assert!(linked.elf.starts_with(b"\x7fELF"));
    }

    #[test]
    fn payload_enum_construction_reaches_machine_lowering() {
        let program = concrete("enum Value:\n    Item(i32)\n\npub fn kernel_main() -> int:\n    let value = Value.Item(1)\n    return 0\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        assert!(lowered.dump().contains("aggregate.copy 8"));
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn aggregate_return_and_payload_match_use_one_dbyte32_image() {
        let program = concrete("fn id[T](value: T) -> T:\n    return value\n\nfn compute() -> Result[u32, u32]:\n    let value: u32 = id[u32](40)\n    return Result[u32, u32].Ok(value + 2)\n\npub fn kernel_main() -> int:\n    let result = compute()\n    match result:\n        case Result.Ok(value):\n            return value as int\n        case Result.Err(error):\n            return error as int\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        assert!(dump.contains("__dbyte_sret"));
        assert!(dump.contains("call.packed.DByte compute"));
        assert!(dump.contains("compare.eq.u32"));
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn borrowed_enum_payload_match_loads_the_stored_pointer() {
        let program = concrete(
            "fn select(value: ptr[u32]) -> Option[borrow u32]:\n    return Option[borrow u32].Some(borrow *value)\n\npub fn kernel_main() -> int:\n    let answer: u32 = 42 as u32\n    let selected = select(&answer)\n    match selected:\n        case Option.Some(value):\n            return *value as int\n        case Option.None:\n            return 0\n",
        );
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        assert!(
            dump.contains("address.field +4\n")
                && dump.contains("load.ptr[u32]\n")
                && dump.contains("local.store.ptr[u32]"),
            "{dump}"
        );
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn result_propagation_is_explicit_hir_control_flow() {
        let program = concrete("fn attempt(input: Result[i32, i32]) -> Result[i32, i32]:\n    let value = input?\n    return Result[i32, i32].Ok(value + 1)\n\npub fn kernel_main() -> int:\n    let result = attempt(Result[i32, i32].Err(7))\n    match result:\n        case Result.Ok(value):\n            return value as int\n        case Result.Err(error):\n            return error as int\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        assert!(lowered.dump().contains("aggregate.copy"));
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn function_value_and_indirect_call_reach_machine_lowering() {
        let program = concrete("fn helper() -> int:\n    return 1\n\npub fn kernel_main() -> int:\n    let callback: fn() -> int = helper\n    return callback()\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        assert!(lowered.dump().contains("call.indirect.DByte"));
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn capturing_closure_uses_hidden_environment_and_indirect_call() {
        let program = concrete("pub fn kernel_main() -> int:\n    let factor: int = 2\n    let callback = fn [borrow factor](value: int) -> int:\n        return value + factor\n    return callback(40)\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        assert!(dump.contains("__dbyte_closure_kernel_c0"));
        assert!(dump.contains("call.indirect.DByte 2"));
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn string_view_and_borrowed_slice_have_two_word_native_representations() {
        let program = concrete("fn first(data: slice[u8]) -> u8:\n    return data[0]\n\npub fn kernel_main() -> int:\n    let bytes: array[u8, 2] = [41 as u8, 42 as u8]\n    let view: slice[u8] = borrow bytes\n    let text: StringView = \"OK\"\n    return (first(view) as int) + (text[1] as int)\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        assert!(dump.contains("trap.ud2"));
        assert!(dump.contains("call.packed.DByte first bytes=8"), "{dump}");
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn packed_dbyte32_unit_arguments_reserve_a_physical_slot() {
        let program = concrete(
            "fn first(data: slice[u8], marker: unit) -> Option[u8]:\n    return Option[u8].Some(data[0])\n\npub fn kernel_main() -> int:\n    let bytes: array[u8, 1] = [42 as u8]\n    let view: slice[u8] = borrow bytes\n    let result = first(view, unit)\n    match result:\n        case Option.Some(value):\n            return value as int\n        case Option.None:\n            return 0\n",
        );
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        assert!(dump.contains("call.packed.DByte first bytes=16"), "{dump}");
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn canonical_intrinsics_reach_the_existing_bounded_encoder() {
        let program = concrete("pub fn kernel_main() -> int:\n    native.io_out8(1016 as u16, 65 as u8)\n    return native.cpu_read_eflags() as int\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        assert!(dump.contains("intrinsic native.io_out8"));
        assert!(dump.contains("intrinsic native.cpu_read_eflags"));
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn canonical_user_syscall_is_profile_checked_and_lowered() {
        let program = concrete_profile(
            "pub fn user_main() -> int:\n    return user.syscall0(2 as u32) as int\n",
            dbyte_hir::Profile::I686User,
        );
        let lowered = lower_hir_module_to_native_ir("user", &program).unwrap();
        assert!(lowered.dump().contains("intrinsic user.syscall0"));
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn canonical_constants_and_static_places_reach_object_storage() {
        let program = concrete("const PORT: u16 = 1016 as u16\nstatic mut VALUE: u32 = 7 as u32\n\npub fn kernel_main() -> int:\n    VALUE = VALUE + 1 as u32\n    native.io_out8(PORT, VALUE as u8)\n    return VALUE as int\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        assert!(dump.contains("private static VALUE: u32 section=data"));
        assert!(!dump.contains("static PORT"));
        assert!(dump.contains("address.global VALUE"));
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn canonical_c_declarations_and_indirect_calls_use_the_aligned_path() {
        let program = concrete("extern \"C\" fn c_add(left: i32, right: i32) -> i32\n\npub fn kernel_main() -> int:\n    let callback: extern \"C\" fn(i32, i32) -> i32 = c_add\n    return callback(40 as i32, 2 as i32) as int\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        assert_eq!(lowered.external_functions.len(), 1);
        assert_eq!(lowered.functions.len(), 1);
        assert!(lowered.dump().contains("call.indirect.C"));
        let machine = crate::systems_lower::lower_systems_module(&lowered).unwrap();
        let crate::object::SectionContents::Bytes(text) =
            &machine.image.sections[&crate::object::ObjectSection::Text].contents
        else {
            panic!("text must be initialized")
        };
        assert!(text.windows(2).any(|window| window == [0xff, 0xd0]));
    }

    #[test]
    fn canonical_legacy_string_index_keeps_v11_native_representation() {
        let program = concrete("fn write(value: str, length: usize):\n    let index: usize = 0 as usize\n    while index < length:\n        native.io_out8(1016 as u16, value[index] as u8)\n        index = index + 1 as usize\n\npub fn kernel_main() -> int:\n    write(\"OK\", 2 as usize)\n    return 0\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        assert!(dump.contains("address.index scale=1"));
        assert!(dump.contains("address.string"));
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn scalar_borrow_dereference_loads_the_pointee_width() {
        let program = concrete(
            "fn read(value: borrow u32) -> u32:\n    return *value\n\npub fn kernel_main() -> int:\n    let value: u32 = 42 as u32\n    return read(borrow value) as int\n",
        );
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        assert!(dump.contains("private fn read(ptr[u32]) -> u32"), "{dump}");
        assert!(
            dump.contains("local.load.ptr[u32] %l0\n  0001 load.u32"),
            "{dump}"
        );
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn v121_custom_drop_glue_reaches_native_calls_exactly_at_cleanup_edges() {
        let program = concrete("struct Resource:\n    value: u32\n\nstatic mut DROPS: u32 = 0 as u32\n\ndrop fn destroy(value: borrow mut Resource):\n    DROPS = DROPS + 1 as u32\n    (*value).value = 0 as u32\n\npub fn kernel_main() -> int:\n    let resource = Resource(value = 42 as u32)\n    drop(resource)\n    return DROPS as int\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        assert!(dump.contains("call destroy 1 -> i32"), "{dump}");
        assert_eq!(dump.matches("call destroy 1 -> i32").count(), 1, "{dump}");
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn v121_monomorphization_materializes_generic_structural_drop_glue() {
        let program = concrete("struct Child:\n    value: u32\n\nstatic mut DROPS: u32 = 0 as u32\n\ndrop fn destroy_child(value: borrow mut Child):\n    DROPS = DROPS + 1 as u32\n\nfn consume[T](value: T) -> unit:\n    return unit\n\npub fn kernel_main() -> int:\n    let child = Child(value = 42 as u32)\n    consume[Child](child)\n    return DROPS as int\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        assert!(dump.contains("__dbyte_mono_consume"), "{dump}");
        assert!(dump.contains("call destroy_child 1 -> i32"), "{dump}");
        crate::systems_lower::lower_systems_module(&lowered)
            .unwrap_or_else(|error| panic!("{error:?}\n{dump}"));
    }

    #[test]
    fn v121_reassignment_materializes_rhs_before_old_value_cleanup() {
        let program = concrete("struct Resource:\n    value: u32\n\nstatic mut DROPS: u32 = 0 as u32\n\ndrop fn destroy(value: borrow mut Resource):\n    DROPS = DROPS + 1 as u32\n\nfn make(value: u32) -> Resource:\n    return Resource(value = value)\n\npub fn kernel_main() -> int:\n    let resource = make(1 as u32)\n    resource = make(2 as u32)\n    return DROPS as int\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        let replacement = dump.find("call.packed.DByte make").unwrap();
        let replacement = dump[replacement + 1..]
            .find("call.packed.DByte make")
            .map(|offset| replacement + 1 + offset)
            .unwrap();
        let cleanup = dump[replacement..]
            .find("call destroy 1 -> i32")
            .map(|offset| replacement + offset)
            .unwrap();
        assert!(replacement < cleanup, "{dump}");
        crate::systems_lower::lower_systems_module(&lowered)
            .unwrap_or_else(|error| panic!("{error:?}\n{dump}"));
    }

    #[test]
    fn v121_generic_layout_queries_materialize_after_substitution() {
        let program = concrete("fn element_size[T]() -> usize:\n    return size_of(T)\n\npub fn kernel_main() -> int:\n    return element_size[u32]() as int\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        assert!(dump.contains("const.usize 0x00000004"), "{dump}");
        assert!(!dump.contains("LayoutOf"), "{dump}");
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn v121_indirect_dbyte_result_return_uses_hidden_sret() {
        let program = concrete("enum Failure:\n    Out\n\nfn provider(context: ptr[u8], size: usize, alignment: usize) -> Result[ptr[u8], Failure]:\n    return Result[ptr[u8], Failure].Ok(context)\n\npub fn kernel_main() -> int:\n    let callback: fn(ptr[u8], usize, usize) -> Result[ptr[u8], Failure] = provider\n    let result = callback(null, 4 as usize, 4 as usize)\n    match result:\n        case Result.Ok(pointer):\n            return 0\n        case Result.Err(error):\n            return 1\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        assert!(dump.contains("call.indirect.DByte 4 -> i32"), "{dump}");
        assert!(dump.contains("__dbyte_indirect_call_result"), "{dump}");
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn v121_string_view_literal_and_len_use_the_two_word_representation() {
        let program = concrete(
            "pub fn kernel_main() -> int:\n    let view: StringView = \"runtime\"\n    return len(view) as int\n",
        );
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        assert!(dump.contains("address.string"), "{dump}");
        assert!(dump.contains("address.field +4"), "{dump}");
        assert!(dump.contains("load.usize"), "{dump}");
        crate::systems_lower::lower_systems_module(&lowered).unwrap();
    }

    #[test]
    fn v121_unit_result_payload_neither_loads_nor_stores_phantom_bytes() {
        let program = concrete("fn ready() -> Result[unit, u32]:\n    return Result[unit, u32].Ok(unit)\n\npub fn kernel_main() -> int:\n    let result = ready()\n    match result:\n        case Result.Ok(value):\n            return 0\n        case Result.Err(error):\n            return error as int\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        crate::systems_lower::lower_systems_module(&lowered)
            .unwrap_or_else(|error| panic!("{error:?}\n{}", lowered.dump()));
    }

    #[test]
    fn v121_enum_drop_dispatches_only_the_active_payload() {
        let program = concrete("struct Resource:\n    value: u32\n\nstatic mut DROPS: u32 = 0 as u32\n\ndrop fn destroy(value: borrow mut Resource):\n    DROPS = DROPS + 1 as u32\n\nenum Maybe:\n    Some(Resource)\n    None\n\npub fn kernel_main() -> int:\n    let value = Maybe.Some(Resource(value = 1 as u32))\n    drop(value)\n    return DROPS as int\n");
        let lowered = lower_hir_module_to_native_ir("kernel", &program).unwrap();
        let dump = lowered.dump();
        assert!(dump.contains("compare.eq.u32"), "{dump}");
        assert_eq!(dump.matches("call destroy 1 -> i32").count(), 1, "{dump}");
        crate::systems_lower::lower_systems_module(&lowered)
            .unwrap_or_else(|error| panic!("{error:?}\n{dump}"));
    }
}
