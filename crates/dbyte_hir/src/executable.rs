//! Executable, backend-neutral HIR bodies.
//!
//! The semantic analyzer remains the only authority for names, types,
//! ownership, variants and layouts.  This module turns those decisions into a
//! representation that execution backends can consume without inspecting the
//! source AST again.

use super::{
    CanonicalType, ClosureId, Diagnostic, FieldId, FunctionDef, GenericDefId, HirExpression,
    HirIntrinsic, HirProgram, ModuleId, OwnershipAction, SymbolId, TypeDefKind, TypeId,
    ValueCategory,
};
use dbyte_ast::{
    BinOp, CallConvention, CaptureMode, Expr, FunctionKind, MatchMode, Pattern, Program, Span,
    Stmt, UnaryOp,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FunctionInstanceId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FunctionInstanceKey {
    pub symbol: SymbolId,
    pub arguments: Vec<TypeId>,
}

#[derive(Debug, Clone)]
pub struct ExecutableHirProgram {
    pub semantic: HirProgram,
    pub functions: Vec<HirFunction>,
    pub globals: Vec<HirGlobal>,
    pub top_level: HirBlock,
}

#[derive(Debug, Clone)]
pub struct ConcreteHirProgram {
    pub semantic: HirProgram,
    pub functions: Vec<ConcreteHirFunction>,
    pub globals: Vec<HirGlobal>,
    pub top_level: HirBlock,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirGlobal {
    pub symbol: SymbolId,
    pub name: String,
    pub public: bool,
    pub mutable: bool,
    pub constant: bool,
    pub ty: TypeId,
    pub initializer: Option<HirExpr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirFunction {
    pub symbol: SymbolId,
    pub generic: Option<GenericDefId>,
    pub name: String,
    pub public: bool,
    pub kind: FunctionKind,
    pub drop_target: Option<SymbolId>,
    pub convention: CallConvention,
    pub interrupt: Option<dbyte_ast::InterruptClass>,
    pub declaration_only: bool,
    pub parameters: Vec<HirLocal>,
    pub result: TypeId,
    pub locals: Vec<HirLocal>,
    pub body: HirBlock,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConcreteHirFunction {
    pub instance: FunctionInstanceId,
    pub key: FunctionInstanceKey,
    pub symbol_name: String,
    pub public: bool,
    pub kind: FunctionKind,
    pub drop_target: Option<SymbolId>,
    pub convention: CallConvention,
    pub interrupt: Option<dbyte_ast::InterruptClass>,
    pub declaration_only: bool,
    pub parameters: Vec<HirLocal>,
    pub result: TypeId,
    pub locals: Vec<HirLocal>,
    pub body: HirBlock,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirLocal {
    pub id: LocalId,
    pub name: String,
    pub ty: TypeId,
    pub mutable: bool,
    pub parameter: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HirBlock {
    pub statements: Vec<HirStatement>,
}

fn block_is_terminal(block: &HirBlock) -> bool {
    block
        .statements
        .last()
        .is_some_and(|statement| match statement {
            HirStatement::Return { .. }
            | HirStatement::Break { .. }
            | HirStatement::Continue { .. } => true,
            HirStatement::If {
                then_block,
                else_block: Some(else_block),
                ..
            } => block_is_terminal(then_block) && block_is_terminal(else_block),
            HirStatement::Match { arms, .. } => {
                !arms.is_empty() && arms.iter().all(|arm| block_is_terminal(&arm.body))
            }
            _ => false,
        })
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirStatement {
    Let {
        local: LocalId,
        value: HirExpr,
        span: Span,
    },
    Assign {
        place: HirPlace,
        value: HirExpr,
        cleanup_before_commit: Option<HirBlock>,
        span: Span,
    },
    Expression(HirExpr),
    Return {
        value: Option<HirExpr>,
        cleanup: Vec<HirBlock>,
        span: Span,
    },
    If {
        condition: HirExpr,
        then_block: HirBlock,
        else_block: Option<HirBlock>,
        span: Span,
    },
    While {
        condition: HirExpr,
        body: HirBlock,
        span: Span,
    },
    For {
        local: HirLocal,
        iterable: HirExpr,
        body: HirBlock,
        span: Span,
    },
    Match {
        mode: MatchMode,
        value: HirExpr,
        arms: Vec<HirMatchArm>,
        exhaustive: bool,
        span: Span,
    },
    Defer {
        captures: Vec<HirCapture>,
        body: HirBlock,
        span: Span,
    },
    Break {
        cleanup: Vec<HirBlock>,
        span: Span,
    },
    Continue {
        cleanup: Vec<HirBlock>,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirMatchArm {
    pub variant: Option<FieldedVariant>,
    pub bindings: Vec<HirLocal>,
    pub body: HirBlock,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldedVariant {
    pub symbol: SymbolId,
    pub variant: super::VariantId,
    pub discriminant: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirCapture {
    pub local: LocalId,
    pub mode: CaptureMode,
    pub ty: TypeId,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirExpr {
    pub ty: TypeId,
    pub category: ValueCategory,
    pub ownership: OwnershipAction,
    pub allocator_origin: Option<super::AllocatorOriginId>,
    pub borrow_origin: Option<super::BorrowOriginId>,
    pub kind: HirExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostCapabilityId {
    Print,
    MathAbs,
    MathMin,
    MathMax,
    FsReadText,
    FsWriteText,
    FsReadBytes,
    FsWriteBytes,
    FsExists,
    FsMkdir,
    FsRemove,
    EncodingHexEncode,
    EncodingHexDecode,
    HashSha256,
    EnvArgs,
    ProcessRun,
    BufferNew,
    BufferFromBytes,
    BufferToBytes,
    BufferLen,
    BufferGet,
    BufferSet,
    BufferSlice,
    BufferLoad,
    BufferSave,
    BufferFind,
    BufferReplace,
    BinaryU8,
    BinaryI8,
    BinaryU16Le,
    BinaryU16Be,
    BinaryI16Le,
    BinaryI16Be,
    BinaryU32Le,
    BinaryU32Be,
    BinaryI32Le,
    BinaryI32Be,
    BinaryPackU16Le,
    BinaryPackU16Be,
    BinaryPackU32Le,
    BinaryPackU32Be,
    BinaryWriteU16Le,
    BinaryWriteU16Be,
    BinaryWriteU32Le,
    BinaryWriteU32Be,
}

impl HostCapabilityId {
    pub fn resolve(module: &str, member: &str) -> Option<Self> {
        Some(match (module, member) {
            ("std.math", "abs") => Self::MathAbs,
            ("std.math", "min") => Self::MathMin,
            ("std.math", "max") => Self::MathMax,
            ("std.fs", "read_text") => Self::FsReadText,
            ("std.fs", "write_text") => Self::FsWriteText,
            ("std.fs", "read_bytes") => Self::FsReadBytes,
            ("std.fs", "write_bytes") => Self::FsWriteBytes,
            ("std.fs", "exists") => Self::FsExists,
            ("std.fs", "mkdir") => Self::FsMkdir,
            ("std.fs", "remove") => Self::FsRemove,
            ("std.encoding", "hex_encode") => Self::EncodingHexEncode,
            ("std.encoding", "hex_decode") => Self::EncodingHexDecode,
            ("std.hash", "sha256") => Self::HashSha256,
            ("std.env", "args") => Self::EnvArgs,
            ("std.process", "run") => Self::ProcessRun,
            ("std.buffer", "new") => Self::BufferNew,
            ("std.buffer", "from_bytes") => Self::BufferFromBytes,
            ("std.buffer", "to_bytes") => Self::BufferToBytes,
            ("std.buffer", "len") => Self::BufferLen,
            ("std.buffer", "get") => Self::BufferGet,
            ("std.buffer", "set") => Self::BufferSet,
            ("std.buffer", "slice") => Self::BufferSlice,
            ("std.buffer", "load") => Self::BufferLoad,
            ("std.buffer", "save") => Self::BufferSave,
            ("std.buffer", "find") => Self::BufferFind,
            ("std.buffer", "replace") => Self::BufferReplace,
            ("std.binary", "u8") => Self::BinaryU8,
            ("std.binary", "i8") => Self::BinaryI8,
            ("std.binary", "u16_le") => Self::BinaryU16Le,
            ("std.binary", "u16_be") => Self::BinaryU16Be,
            ("std.binary", "i16_le") => Self::BinaryI16Le,
            ("std.binary", "i16_be") => Self::BinaryI16Be,
            ("std.binary", "u32_le") => Self::BinaryU32Le,
            ("std.binary", "u32_be") => Self::BinaryU32Be,
            ("std.binary", "i32_le") => Self::BinaryI32Le,
            ("std.binary", "i32_be") => Self::BinaryI32Be,
            ("std.binary", "pack_u16_le") => Self::BinaryPackU16Le,
            ("std.binary", "pack_u16_be") => Self::BinaryPackU16Be,
            ("std.binary", "pack_u32_le") => Self::BinaryPackU32Le,
            ("std.binary", "pack_u32_be") => Self::BinaryPackU32Be,
            ("std.binary", "write_u16_le") => Self::BinaryWriteU16Le,
            ("std.binary", "write_u16_be") => Self::BinaryWriteU16Be,
            ("std.binary", "write_u32_le") => Self::BinaryWriteU32Le,
            ("std.binary", "write_u32_be") => Self::BinaryWriteU32Be,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirFormatPart {
    Literal(Vec<u8>),
    Value(HirExpr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirExprKind {
    Unit,
    Integer(i64),
    LayoutOf {
        target: TypeId,
        alignment: bool,
    },
    Length {
        value: Box<HirExpr>,
    },
    Float(f64),
    Bool(bool),
    String(Vec<u8>),
    Bytes(Vec<u8>),
    Null,
    Local(LocalId),
    Global(SymbolId),
    Constant {
        symbol: SymbolId,
        value: Box<HirExpr>,
    },
    Function(FunctionInstanceKey),
    List(Vec<HirExpr>),
    Binary {
        left: Box<HirExpr>,
        op: BinOp,
        right: Box<HirExpr>,
    },
    Unary {
        op: UnaryOp,
        value: Box<HirExpr>,
    },
    Cast {
        value: Box<HirExpr>,
        target: TypeId,
    },
    DirectCall {
        callee: FunctionInstanceKey,
        convention: CallConvention,
        arguments: Vec<HirExpr>,
    },
    IndirectCall {
        callee: Box<HirExpr>,
        convention: CallConvention,
        arguments: Vec<HirExpr>,
    },
    IntrinsicCall {
        intrinsic: HirIntrinsic,
        arguments: Vec<HirExpr>,
    },
    HostCall {
        capability: HostCapabilityId,
        arguments: Vec<HirExpr>,
    },
    FormatString(Vec<HirFormatPart>),
    InterruptAddress {
        vector: u8,
        handler: FunctionInstanceKey,
        class: dbyte_ast::InterruptClass,
    },
    TaskAddress {
        function: FunctionInstanceKey,
    },
    Struct {
        symbol: SymbolId,
        fields: Vec<(FieldId, HirExpr)>,
    },
    Enum {
        symbol: SymbolId,
        variant: super::VariantId,
        discriminant: u32,
        payload: Vec<HirExpr>,
    },
    Place(HirPlace),
    Borrow {
        mutable: bool,
        place: HirPlace,
    },
    Try {
        value: Box<HirExpr>,
        ok_variant: super::VariantId,
        err_variant: super::VariantId,
        cleanup: Vec<HirBlock>,
    },
    Drop {
        place: HirPlace,
        cleanup: HirBlock,
    },
    Closure {
        closure: ClosureId,
        captures: Vec<HirCapture>,
        parameters: Vec<HirLocal>,
        result: TypeId,
        body: HirBlock,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct HirPlace {
    pub base: HirPlaceBase,
    pub projections: Vec<HirProjection>,
    pub ty: TypeId,
    pub mutable: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirPlaceBase {
    Local(LocalId),
    Global(SymbolId),
    Pointer(Box<HirExpr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum HirProjection {
    Field {
        field: FieldId,
        ty: TypeId,
        offset: u32,
    },
    Payload {
        variant: super::VariantId,
        index: u32,
        ty: TypeId,
        offset: u32,
    },
    Index {
        index: HirExpr,
        element: TypeId,
        element_size: u32,
    },
}

/// Build executable bodies only after canonical semantic analysis succeeds.
/// The builder consumes the analyzer's resolved expression facts and never
/// performs type inference or ownership validation.
pub fn executable_from_analyzed(
    program: &Program,
    semantic: HirProgram,
) -> Result<ExecutableHirProgram, Diagnostic> {
    ExecutableBuilder::new(&semantic).build(program, semantic.clone())
}

/// Analyze one source module and produce executable HIR.  Project-level users
/// call this in canonical module order; the project API is added without
/// changing the single-module compatibility entry point.
pub fn analyze_executable(
    program: &Program,
    profile: super::Profile,
    layout: super::TargetDataLayout,
) -> Result<ExecutableHirProgram, Vec<Diagnostic>> {
    let semantic = super::analyze(program, profile, layout)?;
    executable_from_analyzed(program, semantic).map_err(|diagnostic| vec![diagnostic])
}

#[derive(Debug, Clone)]
pub struct ProjectModuleInput {
    pub id: ModuleId,
    pub name: String,
    pub program: Program,
    pub imports: Vec<(String, ModuleId)>,
}

#[derive(Debug, Clone)]
pub struct ProjectInput {
    pub root: ModuleId,
    pub modules: Vec<ProjectModuleInput>,
}

#[derive(Debug, Clone)]
pub struct CanonicalHirProject {
    pub root: ModuleId,
    pub module_order: Vec<ModuleId>,
    pub source_modules: BTreeMap<ModuleId, String>,
    pub executable: ExecutableHirProgram,
    pub symbol_owners: BTreeMap<SymbolId, ModuleId>,
    pub link_names: BTreeMap<SymbolId, String>,
}

#[derive(Debug, Clone)]
pub struct ConcreteHirProject {
    pub root: ModuleId,
    pub module_order: Vec<ModuleId>,
    pub source_modules: BTreeMap<ModuleId, String>,
    pub executable: ConcreteHirProgram,
    pub symbol_owners: BTreeMap<SymbolId, ModuleId>,
    pub link_names: BTreeMap<SymbolId, String>,
}

pub fn analyze_project(
    project: &ProjectInput,
    profile: super::Profile,
    layout: super::TargetDataLayout,
) -> Result<CanonicalHirProject, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    for (expected, module) in project.modules.iter().enumerate() {
        if module.id.0 != expected as u32 {
            diagnostics.push(Diagnostic {
                code: "E_MODULE_ORDER",
                message: "project modules must use dense canonical ModuleId order".into(),
                span: Span::zero(),
            });
            break;
        }
        if module
            .imports
            .iter()
            .any(|(_, target)| target.0 as usize >= project.modules.len())
        {
            diagnostics.push(Diagnostic {
                code: "E_MODULE_IMPORT_TARGET",
                message: "module import target is outside the canonical project".into(),
                span: Span::zero(),
            });
        }
    }
    diagnostics
        .sort_by_key(|diagnostic| (diagnostic.span.line, diagnostic.span.col, diagnostic.code));
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    let resolved = super::project::resolve_project_ast(project)?;
    let executable = analyze_executable(&resolved.program, profile, layout)?;
    let mut symbol_owners = BTreeMap::new();
    let mut link_names = BTreeMap::new();
    for (internal, (owner, source)) in resolved.owners {
        let symbol = executable
            .semantic
            .functions
            .iter()
            .map(|function| (function.name.as_str(), function.symbol))
            .chain(
                executable
                    .semantic
                    .definitions
                    .iter()
                    .map(|definition| (definition.name.as_str(), definition.symbol)),
            )
            .chain(
                executable
                    .semantic
                    .globals
                    .iter()
                    .map(|global| (global.name.as_str(), global.symbol)),
            )
            .find_map(|(name, symbol)| (name == internal).then_some(symbol))
            .ok_or_else(|| {
                vec![Diagnostic {
                    code: "E_PROJECT_SYMBOL_ID",
                    message: format!("resolved project symbol `{internal}` has no SymbolId"),
                    span: Span::zero(),
                }]
            })?;
        symbol_owners.insert(symbol, owner);
        link_names.insert(symbol, source);
    }
    Ok(CanonicalHirProject {
        root: project.root,
        module_order: project.modules.iter().map(|module| module.id).collect(),
        source_modules: project
            .modules
            .iter()
            .map(|module| (module.id, module.name.clone()))
            .collect(),
        executable,
        symbol_owners,
        link_names,
    })
}

pub fn monomorphize_project(
    project: &CanonicalHirProject,
) -> Result<ConcreteHirProject, Diagnostic> {
    let executable = monomorphize(&project.executable)?;
    validate_profile(&executable, executable.semantic.profile)?;
    Ok(ConcreteHirProject {
        root: project.root,
        module_order: project.module_order.clone(),
        source_modules: project.source_modules.clone(),
        executable,
        symbol_owners: project.symbol_owners.clone(),
        link_names: project.link_names.clone(),
    })
}

pub fn validate_project_profile(
    project: &ConcreteHirProject,
    profile: super::Profile,
) -> Result<(), Diagnostic> {
    validate_profile(&project.executable, profile)
}

pub fn monomorphize(program: &ExecutableHirProgram) -> Result<ConcreteHirProgram, Diagnostic> {
    let mut pending = BTreeSet::new();
    for function in &program.functions {
        let exported_drop = function.kind == FunctionKind::Drop
            && function.generic.is_none()
            && function.drop_target.is_some_and(|target| {
                program
                    .semantic
                    .definitions
                    .iter()
                    .any(|definition| definition.symbol == target && definition.public)
            });
        if function.generic.is_none()
            && (function.public
                || exported_drop
                || function.name == "kernel_main"
                || function.name == "user_main")
        {
            pending.insert(FunctionInstanceKey {
                symbol: function.symbol,
                arguments: Vec::new(),
            });
        }
    }
    collect_called_instances_block(&program.top_level, &mut pending);
    for specialization in &program.semantic.specializations {
        if specialization.arguments.iter().any(|argument| {
            concrete_type_is_open(&program.semantic, *argument, &mut BTreeSet::new())
        }) {
            continue;
        }
        if let Some(function) = program
            .functions
            .iter()
            .find(|function| function.generic == Some(specialization.definition))
        {
            pending.insert(FunctionInstanceKey {
                symbol: function.symbol,
                arguments: specialization.arguments.clone(),
            });
        }
    }

    let mut functions = Vec::new();
    let mut emitted = BTreeSet::new();
    let mut types = TypeSubstituter::new(&program.semantic.types);
    while let Some(key) = pending.iter().next().cloned() {
        pending.remove(&key);
        if !emitted.insert(key.clone()) {
            continue;
        }
        let source = program
            .functions
            .iter()
            .find(|function| function.symbol == key.symbol)
            .ok_or_else(|| Diagnostic {
                code: "E_HIR_UNRESOLVED_FUNCTION",
                message: format!(
                    "concrete function refers to unknown symbol s{}",
                    key.symbol.0
                ),
                span: Span::zero(),
            })?;
        let expected = source.generic.map_or(0, |_| {
            program
                .semantic
                .functions
                .iter()
                .find(|function| function.symbol == source.symbol)
                .map_or(0, |function| function.generic_params.len())
        });
        if key.arguments.len() != expected {
            return Err(Diagnostic {
                code: "E_HIR_OPEN_GENERIC",
                message: format!(
                    "function `{}` requires {expected} concrete type arguments, found {}",
                    source.name,
                    key.arguments.len()
                ),
                span: source.span,
            });
        }
        let substitution = source.generic.map(|owner| (owner, key.arguments.clone()));
        if substitution.is_some() {
            let source_type_count = program.semantic.types.len();
            for index in 0..source_type_count {
                match types.substitute(TypeId(index as u32), substitution.as_ref(), source.span) {
                    Ok(_) => {}
                    Err(error) if error.code == "E_HIR_OPEN_GENERIC" => {}
                    Err(error) => return Err(error),
                }
            }
        }
        let mut concrete = source.clone();
        substitute_function(&mut concrete, substitution.as_ref(), &mut types)?;
        let mut concrete_semantic = program.semantic.clone();
        concrete_semantic.types = types.types.clone();
        materialize_concrete_drop_block(&mut concrete.body, &concrete_semantic)?;
        let mut discovered = BTreeSet::new();
        collect_called_instances_block(&concrete.body, &mut discovered);
        for instance in discovered {
            if !emitted.contains(&instance) {
                pending.insert(instance);
            }
        }
        functions.push(ConcreteHirFunction {
            instance: FunctionInstanceId((functions.len()) as u32),
            key: key.clone(),
            symbol_name: mangle_instance(&source.name, &key.arguments),
            public: source.public && key.arguments.is_empty(),
            kind: source.kind,
            drop_target: source.drop_target,
            convention: source.convention,
            interrupt: source.interrupt,
            declaration_only: source.declaration_only,
            parameters: concrete.parameters,
            result: concrete.result,
            locals: concrete.locals,
            body: concrete.body,
            span: source.span,
        });
    }
    let mut semantic = program.semantic.clone();
    semantic.types = types.types;
    let concrete = ConcreteHirProgram {
        semantic,
        functions,
        globals: program.globals.clone(),
        top_level: program.top_level.clone(),
    };
    validate_concrete(&concrete)?;
    Ok(concrete)
}

pub fn validate_profile(
    program: &ConcreteHirProgram,
    profile: super::Profile,
) -> Result<(), Diagnostic> {
    if program.semantic.profile != profile {
        return Err(Diagnostic {
            code: "E_PROFILE_MISMATCH",
            message: "concrete HIR profile differs from the requested backend profile".into(),
            span: Span::zero(),
        });
    }
    validate_concrete(program)
}

fn mangle_instance(name: &str, arguments: &[TypeId]) -> String {
    if arguments.is_empty() {
        return name.to_owned();
    }
    let mut output = format!("__dbyte_mono_{name}");
    for argument in arguments {
        output.push_str(&format!("_t{}", argument.0));
    }
    output
}

fn collect_called_instances_block(block: &HirBlock, output: &mut BTreeSet<FunctionInstanceKey>) {
    for statement in &block.statements {
        match statement {
            HirStatement::Let { value, .. } | HirStatement::Expression(value) => {
                collect_called_instances_expr(value, output)
            }
            HirStatement::Assign {
                value,
                cleanup_before_commit,
                ..
            } => {
                collect_called_instances_expr(value, output);
                if let Some(cleanup) = cleanup_before_commit {
                    collect_called_instances_block(cleanup, output);
                }
            }
            HirStatement::Return { value, cleanup, .. } => {
                if let Some(value) = value {
                    collect_called_instances_expr(value, output);
                }
                for block in cleanup {
                    collect_called_instances_block(block, output);
                }
            }
            HirStatement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                collect_called_instances_expr(condition, output);
                collect_called_instances_block(then_block, output);
                if let Some(block) = else_block {
                    collect_called_instances_block(block, output);
                }
            }
            HirStatement::While {
                condition, body, ..
            } => {
                collect_called_instances_expr(condition, output);
                collect_called_instances_block(body, output);
            }
            HirStatement::For { iterable, body, .. } => {
                collect_called_instances_expr(iterable, output);
                collect_called_instances_block(body, output);
            }
            HirStatement::Match { value, arms, .. } => {
                collect_called_instances_expr(value, output);
                for arm in arms {
                    collect_called_instances_block(&arm.body, output);
                }
            }
            HirStatement::Defer { body, .. } => collect_called_instances_block(body, output),
            HirStatement::Break { cleanup, .. } | HirStatement::Continue { cleanup, .. } => {
                for block in cleanup {
                    collect_called_instances_block(block, output);
                }
            }
        }
    }
}

fn collect_called_instances_expr(expression: &HirExpr, output: &mut BTreeSet<FunctionInstanceKey>) {
    match &expression.kind {
        HirExprKind::Function(key)
        | HirExprKind::DirectCall { callee: key, .. }
        | HirExprKind::InterruptAddress { handler: key, .. }
        | HirExprKind::TaskAddress { function: key } => {
            output.insert(key.clone());
        }
        _ => {}
    }
    match &expression.kind {
        HirExprKind::Binary { left, right, .. } => {
            collect_called_instances_expr(left, output);
            collect_called_instances_expr(right, output);
        }
        HirExprKind::Unary { value, .. }
        | HirExprKind::Cast { value, .. }
        | HirExprKind::Length { value }
        | HirExprKind::Try { value, .. }
        | HirExprKind::Constant { value, .. } => collect_called_instances_expr(value, output),
        HirExprKind::DirectCall { arguments, .. }
        | HirExprKind::IntrinsicCall { arguments, .. }
        | HirExprKind::HostCall { arguments, .. }
        | HirExprKind::List(arguments)
        | HirExprKind::Enum {
            payload: arguments, ..
        } => {
            for argument in arguments {
                collect_called_instances_expr(argument, output);
            }
        }
        HirExprKind::IndirectCall {
            callee, arguments, ..
        } => {
            collect_called_instances_expr(callee, output);
            for argument in arguments {
                collect_called_instances_expr(argument, output);
            }
        }
        HirExprKind::Struct { fields, .. } => {
            for (_, value) in fields {
                collect_called_instances_expr(value, output);
            }
        }
        HirExprKind::FormatString(parts) => {
            for part in parts {
                if let HirFormatPart::Value(value) = part {
                    collect_called_instances_expr(value, output);
                }
            }
        }
        HirExprKind::Closure { body, .. } => collect_called_instances_block(body, output),
        HirExprKind::Drop { cleanup, .. } => collect_called_instances_block(cleanup, output),
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
        | HirExprKind::TaskAddress { .. }
        | HirExprKind::Place(..)
        | HirExprKind::Borrow { .. } => {}
    }
}

fn substitute_function(
    function: &mut HirFunction,
    substitution: Option<&(GenericDefId, Vec<TypeId>)>,
    types: &mut TypeSubstituter,
) -> Result<(), Diagnostic> {
    for local in function
        .parameters
        .iter_mut()
        .chain(function.locals.iter_mut())
    {
        local.ty = types.substitute(local.ty, substitution, local.span)?;
    }
    function.result = types.substitute(function.result, substitution, function.span)?;
    substitute_block(&mut function.body, substitution, types)
}

fn substitute_block(
    block: &mut HirBlock,
    substitution: Option<&(GenericDefId, Vec<TypeId>)>,
    types: &mut TypeSubstituter,
) -> Result<(), Diagnostic> {
    for statement in &mut block.statements {
        match statement {
            HirStatement::Let { value, .. } | HirStatement::Expression(value) => {
                substitute_expr(value, substitution, types)?
            }
            HirStatement::Assign {
                place,
                value,
                cleanup_before_commit,
                ..
            } => {
                substitute_place(place, substitution, types)?;
                substitute_expr(value, substitution, types)?;
                if let Some(cleanup) = cleanup_before_commit {
                    substitute_block(cleanup, substitution, types)?;
                }
            }
            HirStatement::Return { value, cleanup, .. } => {
                if let Some(value) = value {
                    substitute_expr(value, substitution, types)?;
                }
                for block in cleanup {
                    substitute_block(block, substitution, types)?;
                }
            }
            HirStatement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                substitute_expr(condition, substitution, types)?;
                substitute_block(then_block, substitution, types)?;
                if let Some(block) = else_block {
                    substitute_block(block, substitution, types)?;
                }
            }
            HirStatement::While {
                condition, body, ..
            } => {
                substitute_expr(condition, substitution, types)?;
                substitute_block(body, substitution, types)?;
            }
            HirStatement::For {
                local,
                iterable,
                body,
                ..
            } => {
                local.ty = types.substitute(local.ty, substitution, local.span)?;
                substitute_expr(iterable, substitution, types)?;
                substitute_block(body, substitution, types)?;
            }
            HirStatement::Match { value, arms, .. } => {
                substitute_expr(value, substitution, types)?;
                for arm in arms {
                    for binding in &mut arm.bindings {
                        binding.ty = types.substitute(binding.ty, substitution, binding.span)?;
                    }
                    substitute_block(&mut arm.body, substitution, types)?;
                }
            }
            HirStatement::Defer { captures, body, .. } => {
                for capture in captures {
                    capture.ty = types.substitute(capture.ty, substitution, capture.span)?;
                }
                substitute_block(body, substitution, types)?;
            }
            HirStatement::Break { cleanup, .. } | HirStatement::Continue { cleanup, .. } => {
                for block in cleanup {
                    substitute_block(block, substitution, types)?;
                }
            }
        }
    }
    Ok(())
}

fn substitute_expr(
    expression: &mut HirExpr,
    substitution: Option<&(GenericDefId, Vec<TypeId>)>,
    types: &mut TypeSubstituter,
) -> Result<(), Diagnostic> {
    expression.ty = types.substitute(expression.ty, substitution, expression.span)?;
    match &mut expression.kind {
        HirExprKind::LayoutOf { target, .. } => {
            *target = types.substitute(*target, substitution, expression.span)?;
        }
        HirExprKind::Binary { left, right, .. } => {
            substitute_expr(left, substitution, types)?;
            substitute_expr(right, substitution, types)?;
        }
        HirExprKind::Unary { value, .. } | HirExprKind::Cast { value, .. } => {
            substitute_expr(value, substitution, types)?
        }
        HirExprKind::Length { value } => substitute_expr(value, substitution, types)?,
        HirExprKind::Try { value, cleanup, .. } => {
            substitute_expr(value, substitution, types)?;
            for block in cleanup {
                substitute_block(block, substitution, types)?;
            }
        }
        HirExprKind::DirectCall {
            callee, arguments, ..
        } => {
            for argument in &mut callee.arguments {
                *argument = types.substitute(*argument, substitution, expression.span)?;
            }
            for argument in arguments {
                substitute_expr(argument, substitution, types)?;
            }
        }
        HirExprKind::Function(callee) => {
            for argument in &mut callee.arguments {
                *argument = types.substitute(*argument, substitution, expression.span)?;
            }
        }
        HirExprKind::InterruptAddress { handler, .. } => {
            handler.arguments = handler
                .arguments
                .iter()
                .map(|argument| types.substitute(*argument, substitution, expression.span))
                .collect::<Result<_, _>>()?;
        }
        HirExprKind::TaskAddress { function } => {
            function.arguments = function
                .arguments
                .iter()
                .map(|argument| types.substitute(*argument, substitution, expression.span))
                .collect::<Result<_, _>>()?;
        }
        HirExprKind::IntrinsicCall { arguments, .. }
        | HirExprKind::HostCall { arguments, .. }
        | HirExprKind::List(arguments)
        | HirExprKind::Enum {
            payload: arguments, ..
        } => {
            for argument in arguments {
                substitute_expr(argument, substitution, types)?;
            }
        }
        HirExprKind::IndirectCall {
            callee, arguments, ..
        } => {
            substitute_expr(callee, substitution, types)?;
            for argument in arguments {
                substitute_expr(argument, substitution, types)?;
            }
        }
        HirExprKind::Struct { fields, .. } => {
            for (_, value) in fields {
                substitute_expr(value, substitution, types)?;
            }
        }
        HirExprKind::FormatString(parts) => {
            for part in parts {
                if let HirFormatPart::Value(value) = part {
                    substitute_expr(value, substitution, types)?;
                }
            }
        }
        HirExprKind::Closure {
            captures,
            parameters,
            result,
            body,
            ..
        } => {
            for capture in captures {
                capture.ty = types.substitute(capture.ty, substitution, capture.span)?;
            }
            for parameter in parameters {
                parameter.ty = types.substitute(parameter.ty, substitution, parameter.span)?;
            }
            *result = types.substitute(*result, substitution, expression.span)?;
            substitute_block(body, substitution, types)?;
        }
        HirExprKind::Drop { place, cleanup } => {
            substitute_place(place, substitution, types)?;
            substitute_block(cleanup, substitution, types)?;
        }
        HirExprKind::Place(place) | HirExprKind::Borrow { place, .. } => {
            substitute_place(place, substitution, types)?
        }
        _ => {}
    }
    Ok(())
}

fn substitute_place(
    place: &mut HirPlace,
    substitution: Option<&(GenericDefId, Vec<TypeId>)>,
    types: &mut TypeSubstituter,
) -> Result<(), Diagnostic> {
    place.ty = types.substitute(place.ty, substitution, place.span)?;
    if let HirPlaceBase::Pointer(pointer) = &mut place.base {
        substitute_expr(pointer, substitution, types)?;
    }
    for projection in &mut place.projections {
        match projection {
            HirProjection::Field { ty, .. } | HirProjection::Payload { ty, .. } => {
                *ty = types.substitute(*ty, substitution, place.span)?;
            }
            HirProjection::Index { index, element, .. } => {
                substitute_expr(index, substitution, types)?;
                *element = types.substitute(*element, substitution, place.span)?;
            }
        }
    }
    Ok(())
}

fn materialize_concrete_drop_block(
    block: &mut HirBlock,
    semantic: &HirProgram,
) -> Result<(), Diagnostic> {
    for statement in &mut block.statements {
        match statement {
            HirStatement::Let { value, .. } | HirStatement::Expression(value) => {
                materialize_concrete_drop_expr(value, semantic)?
            }
            HirStatement::Assign {
                place,
                value,
                cleanup_before_commit,
                ..
            } => {
                materialize_concrete_drop_place(place, semantic)?;
                materialize_concrete_drop_expr(value, semantic)?;
                if let Some(cleanup) = cleanup_before_commit {
                    materialize_concrete_drop_block(cleanup, semantic)?;
                }
            }
            HirStatement::Return { value, cleanup, .. } => {
                if let Some(value) = value {
                    materialize_concrete_drop_expr(value, semantic)?;
                }
                for cleanup in cleanup {
                    materialize_concrete_drop_block(cleanup, semantic)?;
                }
            }
            HirStatement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                materialize_concrete_drop_expr(condition, semantic)?;
                materialize_concrete_drop_block(then_block, semantic)?;
                if let Some(else_block) = else_block {
                    materialize_concrete_drop_block(else_block, semantic)?;
                }
            }
            HirStatement::While {
                condition, body, ..
            } => {
                materialize_concrete_drop_expr(condition, semantic)?;
                materialize_concrete_drop_block(body, semantic)?;
            }
            HirStatement::For { iterable, body, .. } => {
                materialize_concrete_drop_expr(iterable, semantic)?;
                materialize_concrete_drop_block(body, semantic)?;
            }
            HirStatement::Match { value, arms, .. } => {
                materialize_concrete_drop_expr(value, semantic)?;
                for arm in arms {
                    materialize_concrete_drop_block(&mut arm.body, semantic)?;
                }
            }
            HirStatement::Defer { body, .. } => materialize_concrete_drop_block(body, semantic)?,
            HirStatement::Break { cleanup, .. } | HirStatement::Continue { cleanup, .. } => {
                for cleanup in cleanup {
                    materialize_concrete_drop_block(cleanup, semantic)?;
                }
            }
        }
    }
    Ok(())
}

fn materialize_concrete_drop_expr(
    expression: &mut HirExpr,
    semantic: &HirProgram,
) -> Result<(), Diagnostic> {
    if let HirExprKind::LayoutOf { target, alignment } = &expression.kind {
        let layout = super::layout_of(semantic, *target)?;
        expression.kind = HirExprKind::Integer(i64::from(if *alignment {
            layout.align
        } else {
            layout.size
        }));
        return Ok(());
    }
    match &mut expression.kind {
        HirExprKind::Length { value } => materialize_concrete_drop_expr(value, semantic)?,
        HirExprKind::Drop { place, cleanup } => {
            if cleanup.statements.is_empty()
                && concrete_type_needs_drop(semantic, place.ty, &mut BTreeSet::new())?
            {
                *cleanup = concrete_drop_block(semantic, place.clone(), place.ty, expression.span)?;
            }
            materialize_concrete_drop_block(cleanup, semantic)?;
        }
        HirExprKind::Binary { left, right, .. } => {
            materialize_concrete_drop_expr(left, semantic)?;
            materialize_concrete_drop_expr(right, semantic)?;
        }
        HirExprKind::Unary { value, .. }
        | HirExprKind::Cast { value, .. }
        | HirExprKind::Try { value, .. } => materialize_concrete_drop_expr(value, semantic)?,
        HirExprKind::DirectCall { arguments, .. }
        | HirExprKind::IntrinsicCall { arguments, .. }
        | HirExprKind::HostCall { arguments, .. }
        | HirExprKind::List(arguments)
        | HirExprKind::Enum {
            payload: arguments, ..
        } => {
            for argument in arguments {
                materialize_concrete_drop_expr(argument, semantic)?;
            }
        }
        HirExprKind::IndirectCall {
            callee, arguments, ..
        } => {
            materialize_concrete_drop_expr(callee, semantic)?;
            for argument in arguments {
                materialize_concrete_drop_expr(argument, semantic)?;
            }
        }
        HirExprKind::Struct { fields, .. } => {
            for (_, value) in fields {
                materialize_concrete_drop_expr(value, semantic)?;
            }
        }
        HirExprKind::FormatString(parts) => {
            for part in parts {
                if let HirFormatPart::Value(value) = part {
                    materialize_concrete_drop_expr(value, semantic)?;
                }
            }
        }
        HirExprKind::Closure { body, .. } => materialize_concrete_drop_block(body, semantic)?,
        HirExprKind::Place(place) | HirExprKind::Borrow { place, .. } => {
            materialize_concrete_drop_place(place, semantic)?
        }
        _ => {}
    }
    Ok(())
}

fn materialize_concrete_drop_place(
    place: &mut HirPlace,
    semantic: &HirProgram,
) -> Result<(), Diagnostic> {
    if let HirPlaceBase::Pointer(pointer) = &mut place.base {
        materialize_concrete_drop_expr(pointer, semantic)?;
    }
    for projection in &mut place.projections {
        if let HirProjection::Index {
            index,
            element,
            element_size,
        } = projection
        {
            materialize_concrete_drop_expr(index, semantic)?;
            *element_size = super::layout_of(semantic, *element)?.size;
        }
    }
    Ok(())
}

fn concrete_type_needs_drop(
    semantic: &HirProgram,
    ty: TypeId,
    active: &mut BTreeSet<TypeId>,
) -> Result<bool, Diagnostic> {
    if !active.insert(ty) {
        return Ok(false);
    }
    let result = match semantic.type_of(ty) {
        CanonicalType::Struct { symbol, args } | CanonicalType::Enum { symbol, args } => {
            if semantic.functions.iter().any(|function| {
                function.kind == FunctionKind::Drop && function.drop_target == Some(*symbol)
            }) {
                true
            } else {
                let definition = semantic
                    .definitions
                    .iter()
                    .find(|definition| definition.symbol == *symbol)
                    .ok_or_else(|| Diagnostic {
                        code: "E_HIR_DROP_TYPE",
                        message: "drop glue type definition is missing".into(),
                        span: Span::zero(),
                    })?;
                let substitutions: BTreeMap<_, _> = definition
                    .generic_params
                    .iter()
                    .cloned()
                    .zip(args.iter().copied())
                    .collect();
                match &definition.kind {
                    TypeDefKind::Struct { fields, .. } => fields.iter().any(|field| {
                        semantic
                            .types
                            .iter()
                            .position(|candidate| {
                                annotation_matches(
                                    semantic,
                                    candidate,
                                    &field.annotation,
                                    &substitutions,
                                )
                            })
                            .is_some_and(|index| {
                                concrete_type_needs_drop(semantic, TypeId(index as u32), active)
                                    .unwrap_or(true)
                            })
                    }),
                    TypeDefKind::Enum { variants } => variants.iter().any(|variant| {
                        variant.payload.iter().any(|payload| {
                            semantic
                                .types
                                .iter()
                                .position(|candidate| {
                                    annotation_matches(semantic, candidate, payload, &substitutions)
                                })
                                .is_some_and(|index| {
                                    concrete_type_needs_drop(semantic, TypeId(index as u32), active)
                                        .unwrap_or(true)
                                })
                        })
                    }),
                }
            }
        }
        CanonicalType::Array { element, .. } => {
            concrete_type_needs_drop(semantic, *element, active)?
        }
        CanonicalType::GenericParam { .. } => {
            return Err(Diagnostic {
                code: "E_HIR_OPEN_GENERIC",
                message: "open generic reached concrete drop-glue materialization".into(),
                span: Span::zero(),
            })
        }
        _ => false,
    };
    active.remove(&ty);
    Ok(result)
}

fn concrete_drop_block(
    semantic: &HirProgram,
    place: HirPlace,
    ty: TypeId,
    span: Span,
) -> Result<HirBlock, Diagnostic> {
    let mut statements = Vec::new();
    let nominal = match semantic.type_of(ty) {
        CanonicalType::Struct { symbol, args } | CanonicalType::Enum { symbol, args } => {
            Some((*symbol, args.clone()))
        }
        _ => None,
    };
    if let Some((symbol, args)) = nominal.clone() {
        if let Some(function) = semantic.functions.iter().find(|function| {
            function.kind == FunctionKind::Drop && function.drop_target == Some(symbol)
        }) {
            let borrow_ty = semantic
                .types
                .iter()
                .position(|candidate| {
                    *candidate
                        == CanonicalType::Borrow {
                            mutable: true,
                            pointee: ty,
                        }
                })
                .map(|index| TypeId(index as u32))
                .ok_or_else(|| Diagnostic {
                    code: "E_HIR_DROP_BORROW",
                    message: "concrete drop glue lacks its mutable borrow type".into(),
                    span,
                })?;
            statements.push(HirStatement::Expression(HirExpr {
                ty: TypeId(0),
                category: ValueCategory::RValue,
                ownership: OwnershipAction::Copy,
                allocator_origin: None,
                borrow_origin: None,
                kind: HirExprKind::DirectCall {
                    callee: FunctionInstanceKey {
                        symbol: function.symbol,
                        arguments: args,
                    },
                    convention: CallConvention::DByte,
                    arguments: vec![HirExpr {
                        ty: borrow_ty,
                        category: ValueCategory::RValue,
                        ownership: OwnershipAction::BorrowMut,
                        allocator_origin: None,
                        borrow_origin: None,
                        kind: HirExprKind::Borrow {
                            mutable: true,
                            place: place.clone(),
                        },
                        span,
                    }],
                },
                span,
            }));
        }
    }
    match semantic.type_of(ty) {
        CanonicalType::Struct { symbol, args } => {
            let definition = semantic
                .definitions
                .iter()
                .find(|definition| definition.symbol == *symbol)
                .ok_or_else(|| Diagnostic {
                    code: "E_HIR_DROP_TYPE",
                    message: "drop glue struct definition is missing".into(),
                    span,
                })?;
            if let TypeDefKind::Struct { fields, .. } = &definition.kind {
                let (_, offsets) = super::layout_struct(semantic, *symbol, args)?;
                let substitutions: BTreeMap<_, _> = definition
                    .generic_params
                    .iter()
                    .cloned()
                    .zip(args.iter().copied())
                    .collect();
                for (field, offset) in fields.iter().zip(offsets).rev() {
                    if !annotation_can_need_drop(semantic, &field.annotation, &substitutions) {
                        continue;
                    }
                    let field_ty =
                        super::resolved_annotation(semantic, &field.annotation, &substitutions)?;
                    if concrete_type_needs_drop(semantic, field_ty, &mut BTreeSet::new())? {
                        let mut field_place = place.clone();
                        field_place.projections.push(HirProjection::Field {
                            field: field.id,
                            ty: field_ty,
                            offset,
                        });
                        field_place.ty = field_ty;
                        statements.extend(
                            concrete_drop_block(semantic, field_place, field_ty, span)?.statements,
                        );
                    }
                }
            }
        }
        CanonicalType::Array { element, length } => {
            let layout = super::layout_of(semantic, *element)?;
            let usize_ty = semantic
                .types
                .iter()
                .position(|candidate| {
                    matches!(candidate, CanonicalType::Integer(super::IntegerType::Usize))
                })
                .map(|index| TypeId(index as u32))
                .ok_or_else(|| Diagnostic {
                    code: "E_HIR_DROP_INDEX",
                    message: "canonical usize type is missing".into(),
                    span,
                })?;
            for index in (0..*length).rev() {
                let mut element_place = place.clone();
                element_place.projections.push(HirProjection::Index {
                    index: HirExpr {
                        ty: usize_ty,
                        category: ValueCategory::RValue,
                        ownership: OwnershipAction::Copy,
                        allocator_origin: None,
                        borrow_origin: None,
                        kind: HirExprKind::Integer(i64::from(index)),
                        span,
                    },
                    element: *element,
                    element_size: layout.size,
                });
                element_place.ty = *element;
                statements.extend(
                    concrete_drop_block(semantic, element_place, *element, span)?.statements,
                );
            }
        }
        CanonicalType::Enum { symbol, .. } => {
            let definition = semantic
                .definitions
                .iter()
                .find(|definition| definition.symbol == *symbol)
                .ok_or_else(|| Diagnostic {
                    code: "E_HIR_DROP_TYPE",
                    message: "drop glue enum definition is missing".into(),
                    span,
                })?;
            let TypeDefKind::Enum { variants } = &definition.kind else {
                unreachable!()
            };
            let mut arms = Vec::with_capacity(variants.len());
            for variant in variants {
                let (_, _, payload) = super::enum_variant_layout(semantic, ty, variant.id)?;
                let mut body = Vec::new();
                for (index, (payload_ty, offset)) in payload.iter().copied().enumerate().rev() {
                    if !concrete_type_needs_drop(semantic, payload_ty, &mut BTreeSet::new())? {
                        continue;
                    }
                    let mut payload_place = place.clone();
                    payload_place.projections.push(HirProjection::Payload {
                        variant: variant.id,
                        index: index as u32,
                        ty: payload_ty,
                        offset,
                    });
                    payload_place.ty = payload_ty;
                    body.extend(
                        concrete_drop_block(semantic, payload_place, payload_ty, span)?.statements,
                    );
                }
                arms.push(HirMatchArm {
                    variant: Some(FieldedVariant {
                        symbol: *symbol,
                        variant: variant.id,
                        discriminant: variant.discriminant,
                    }),
                    bindings: Vec::new(),
                    body: HirBlock { statements: body },
                    span,
                });
            }
            let value_kind = match &place.base {
                HirPlaceBase::Local(local) if place.projections.is_empty() => {
                    HirExprKind::Local(*local)
                }
                HirPlaceBase::Global(symbol) if place.projections.is_empty() => {
                    HirExprKind::Global(*symbol)
                }
                _ => HirExprKind::Place(place.clone()),
            };
            statements.push(HirStatement::Match {
                mode: MatchMode::BorrowMut,
                value: HirExpr {
                    ty,
                    category: ValueCategory::LValue,
                    ownership: OwnershipAction::BorrowMut,
                    allocator_origin: None,
                    borrow_origin: None,
                    kind: value_kind,
                    span,
                },
                arms,
                exhaustive: true,
                span,
            });
        }
        _ => {}
    }
    Ok(HirBlock { statements })
}

fn annotation_can_need_drop(
    semantic: &HirProgram,
    annotation: &dbyte_ast::TypeAnnotation,
    substitutions: &BTreeMap<String, TypeId>,
) -> bool {
    use dbyte_ast::TypeAnnotation;
    match annotation {
        TypeAnnotation::Named(name) => substitutions.get(name).map_or_else(
            || {
                semantic
                    .definitions
                    .iter()
                    .any(|definition| definition.name == *name)
            },
            |ty| concrete_type_needs_drop(semantic, *ty, &mut BTreeSet::new()).unwrap_or(true),
        ),
        TypeAnnotation::Generic { .. } | TypeAnnotation::Array { .. } => true,
        _ => false,
    }
}

struct TypeSubstituter {
    types: Vec<CanonicalType>,
    ids: BTreeMap<CanonicalType, TypeId>,
}

impl TypeSubstituter {
    fn new(types: &[CanonicalType]) -> Self {
        Self {
            types: types.to_vec(),
            ids: types
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, ty)| (ty, TypeId(index as u32)))
                .collect(),
        }
    }

    fn intern(&mut self, ty: CanonicalType) -> TypeId {
        if let Some(id) = self.ids.get(&ty) {
            return *id;
        }
        let id = TypeId(self.types.len() as u32);
        self.types.push(ty.clone());
        self.ids.insert(ty, id);
        id
    }

    fn substitute(
        &mut self,
        ty: TypeId,
        substitution: Option<&(GenericDefId, Vec<TypeId>)>,
        span: Span,
    ) -> Result<TypeId, Diagnostic> {
        let current = self.types[ty.0 as usize].clone();
        let concrete = match current {
            CanonicalType::GenericParam { owner, index } => {
                let Some((expected, arguments)) = substitution else {
                    return Err(Diagnostic {
                        code: "E_HIR_OPEN_GENERIC",
                        message: "open generic type reached concrete HIR".into(),
                        span,
                    });
                };
                if owner != *expected {
                    return Err(Diagnostic {
                        code: "E_HIR_OPEN_GENERIC",
                        message: "foreign generic parameter reached concrete HIR".into(),
                        span,
                    });
                }
                return arguments
                    .get(index as usize)
                    .copied()
                    .ok_or_else(|| Diagnostic {
                        code: "E_HIR_OPEN_GENERIC",
                        message: "generic substitution is missing a canonical argument".into(),
                        span,
                    });
            }
            CanonicalType::LegacyList(element) => {
                CanonicalType::LegacyList(self.substitute(element, substitution, span)?)
            }
            CanonicalType::Pointer { mutable, pointee } => CanonicalType::Pointer {
                mutable,
                pointee: self.substitute(pointee, substitution, span)?,
            },
            CanonicalType::Borrow { mutable, pointee } => CanonicalType::Borrow {
                mutable,
                pointee: self.substitute(pointee, substitution, span)?,
            },
            CanonicalType::Slice { mutable, element } => CanonicalType::Slice {
                mutable,
                element: self.substitute(element, substitution, span)?,
            },
            CanonicalType::Array { element, length } => CanonicalType::Array {
                element: self.substitute(element, substitution, span)?,
                length,
            },
            CanonicalType::Struct { symbol, args } => CanonicalType::Struct {
                symbol,
                args: args
                    .into_iter()
                    .map(|arg| self.substitute(arg, substitution, span))
                    .collect::<Result<_, _>>()?,
            },
            CanonicalType::Enum { symbol, args } => CanonicalType::Enum {
                symbol,
                args: args
                    .into_iter()
                    .map(|arg| self.substitute(arg, substitution, span))
                    .collect::<Result<_, _>>()?,
            },
            CanonicalType::Function {
                convention,
                params,
                result,
            } => CanonicalType::Function {
                convention,
                params: params
                    .into_iter()
                    .map(|param| self.substitute(param, substitution, span))
                    .collect::<Result<_, _>>()?,
                result: self.substitute(result, substitution, span)?,
            },
            CanonicalType::ClosureShape { params, result } => CanonicalType::ClosureShape {
                params: params
                    .into_iter()
                    .map(|param| self.substitute(param, substitution, span))
                    .collect::<Result<_, _>>()?,
                result: self.substitute(result, substitution, span)?,
            },
            other => other,
        };
        Ok(self.intern(concrete))
    }
}

fn validate_concrete(program: &ConcreteHirProgram) -> Result<(), Diagnostic> {
    for function in &program.functions {
        for (label, ty) in function
            .parameters
            .iter()
            .map(|parameter| (parameter.name.as_str(), parameter.ty))
            .chain(
                function
                    .locals
                    .iter()
                    .map(|local| (local.name.as_str(), local.ty)),
            )
            .chain(std::iter::once(("return", function.result)))
        {
            if concrete_type_is_open(&program.semantic, ty, &mut BTreeSet::new()) {
                return Err(Diagnostic {
                    code: "E_HIR_OPEN_GENERIC",
                    message: format!(
                        "open generic type reached concrete function `{}` slot `{label}`",
                        function.symbol_name
                    ),
                    span: function.span,
                });
            }
        }
    }
    Ok(())
}

fn concrete_type_is_open(semantic: &HirProgram, ty: TypeId, active: &mut BTreeSet<TypeId>) -> bool {
    if !active.insert(ty) {
        return false;
    }
    let open = match semantic.type_of(ty) {
        CanonicalType::GenericParam { .. } => true,
        CanonicalType::LegacyList(element)
        | CanonicalType::Pointer {
            pointee: element, ..
        }
        | CanonicalType::Borrow {
            pointee: element, ..
        }
        | CanonicalType::Slice { element, .. }
        | CanonicalType::Array { element, .. } => concrete_type_is_open(semantic, *element, active),
        CanonicalType::Struct { args, .. } | CanonicalType::Enum { args, .. } => args
            .iter()
            .copied()
            .any(|argument| concrete_type_is_open(semantic, argument, active)),
        CanonicalType::Function { params, result, .. }
        | CanonicalType::ClosureShape { params, result } => params
            .iter()
            .copied()
            .chain(std::iter::once(*result))
            .any(|part| concrete_type_is_open(semantic, part, active)),
        _ => false,
    };
    active.remove(&ty);
    open
}

struct ExecutableBuilder<'a> {
    semantic: &'a HirProgram,
    facts: BTreeMap<(usize, usize), HirExpression>,
    symbols: BTreeMap<String, SymbolId>,
    scopes: Vec<BTreeMap<String, LocalId>>,
    locals: Vec<HirLocal>,
    next_local: u32,
    next_closure: u32,
    active_cleanups: Vec<ActiveCleanup>,
    current_generics: BTreeMap<String, TypeId>,
    host_modules: BTreeMap<String, String>,
    runtime_drop_enabled: bool,
}

#[derive(Debug, Clone)]
struct ActiveCleanup {
    owner: Option<LocalId>,
    block: HirBlock,
}

impl<'a> ExecutableBuilder<'a> {
    fn new(semantic: &'a HirProgram) -> Self {
        let facts = semantic
            .expressions
            .iter()
            .cloned()
            .map(|fact| ((fact.span.line, fact.span.col), fact))
            .collect();
        let mut symbols = BTreeMap::new();
        for definition in &semantic.definitions {
            symbols.insert(definition.name.clone(), definition.symbol);
        }
        for function in &semantic.functions {
            symbols.insert(function.name.clone(), function.symbol);
        }
        for global in &semantic.globals {
            symbols.insert(global.name.clone(), global.symbol);
        }
        Self {
            semantic,
            facts,
            symbols,
            scopes: Vec::new(),
            locals: Vec::new(),
            next_local: 0,
            next_closure: 0,
            active_cleanups: Vec::new(),
            current_generics: BTreeMap::new(),
            host_modules: BTreeMap::new(),
            runtime_drop_enabled: semantic
                .functions
                .iter()
                .any(|function| function.kind == FunctionKind::Drop),
        }
    }

    fn build(
        mut self,
        program: &Program,
        semantic: HirProgram,
    ) -> Result<ExecutableHirProgram, Diagnostic> {
        for statement in &program.stmts {
            if let Stmt::Import { path, alias, .. } = statement {
                if path.starts_with("std.") {
                    self.host_modules.insert(alias.clone(), path.clone());
                }
            }
        }
        self.reset_function();
        self.push_scope();
        let mut globals = Vec::new();
        for global in &self.semantic.globals {
            let ty = self.resolve_annotation(&global.annotation, &BTreeMap::new(), global.span)?;
            let initializer = global
                .initializer
                .as_ref()
                .map(|value| self.expr(value))
                .transpose()?;
            globals.push(HirGlobal {
                symbol: global.symbol,
                name: global.name.clone(),
                public: global.public,
                mutable: global.mutable,
                constant: global.constant,
                ty,
                initializer,
                span: global.span,
            });
        }
        self.pop_scope();
        let mut functions = Vec::new();
        for statement in &program.stmts {
            if let Stmt::FnDef {
                name,
                params,
                body,
                span,
                ..
            } = statement
            {
                let definition = self
                    .semantic
                    .functions
                    .iter()
                    .find(|function| function.name == *name)
                    .ok_or_else(|| {
                        self.error(
                            "E_HIR_UNRESOLVED_FUNCTION",
                            "function definition has no canonical symbol",
                            *span,
                        )
                    })?
                    .clone();
                self.reset_function();
                self.push_scope();
                let generic_types = self.generic_types(&definition);
                self.current_generics = generic_types.clone();
                let mut parameters = Vec::new();
                for (index, parameter) in params.iter().enumerate() {
                    let ty =
                        self.resolve_annotation(&parameter.ty, &generic_types, parameter.span)?;
                    let local = self.define_local(parameter.name.clone(), ty, true, parameter.span);
                    self.register_drop(local.id, ty, parameter.span)?;
                    debug_assert_eq!(index, parameters.len());
                    parameters.push(local);
                }
                let result =
                    self.resolve_annotation(&definition.result, &generic_types, definition.span)?;
                let mut body = self.block(body)?;
                for cleanup in self.active_cleanups.iter().rev() {
                    body.statements.extend(cleanup.block.statements.clone());
                }
                self.pop_scope();
                functions.push(HirFunction {
                    symbol: definition.symbol,
                    generic: definition.generic,
                    name: definition.name,
                    public: definition.public,
                    kind: definition.kind,
                    drop_target: definition.drop_target,
                    convention: definition.convention,
                    interrupt: definition.interrupt,
                    declaration_only: definition.declaration_only,
                    parameters,
                    result,
                    locals: self.locals.clone(),
                    body,
                    span: *span,
                });
            }
        }
        self.reset_function();
        self.push_scope();
        let top_source: Vec<_> = program
            .stmts
            .iter()
            .filter(|statement| {
                !matches!(
                    statement,
                    Stmt::FnDef { .. }
                        | Stmt::StructDef { .. }
                        | Stmt::EnumDef { .. }
                        | Stmt::Import { .. }
                        | Stmt::Const { .. }
                        | Stmt::Static { .. }
                )
            })
            .cloned()
            .collect();
        let top_level = self.block(&top_source)?;
        self.pop_scope();
        Ok(ExecutableHirProgram {
            semantic,
            functions,
            globals,
            top_level,
        })
    }

    fn reset_function(&mut self) {
        self.scopes.clear();
        self.locals.clear();
        self.next_local = 0;
        self.active_cleanups.clear();
        self.current_generics.clear();
    }
    fn push_scope(&mut self) {
        self.scopes.push(BTreeMap::new());
    }
    fn pop_scope(&mut self) {
        self.scopes.pop();
    }
    fn define_local(&mut self, name: String, ty: TypeId, parameter: bool, span: Span) -> HirLocal {
        let local = HirLocal {
            id: LocalId(self.next_local),
            name: name.clone(),
            ty,
            mutable: true,
            parameter,
            span,
        };
        self.next_local += 1;
        self.scopes
            .last_mut()
            .expect("HIR scope")
            .insert(name, local.id);
        self.locals.push(local.clone());
        local
    }
    fn lookup_local(&self, name: &str) -> Option<LocalId> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn deactivate_drop(&mut self, local: LocalId) {
        self.active_cleanups
            .retain(|cleanup| cleanup.owner != Some(local));
    }

    fn live_drop_owners(cleanups: &[ActiveCleanup]) -> BTreeSet<LocalId> {
        cleanups
            .iter()
            .filter_map(|cleanup| cleanup.owner)
            .collect()
    }

    fn branch_block(
        &mut self,
        statements: &[Stmt],
        entry: &[ActiveCleanup],
    ) -> Result<(HirBlock, Vec<ActiveCleanup>), Diagnostic> {
        self.active_cleanups = entry.to_vec();
        let block = self.block(statements)?;
        let exit = self.active_cleanups.clone();
        Ok((block, exit))
    }

    fn require_same_drop_state(
        &self,
        expected: &[ActiveCleanup],
        actual: &[ActiveCleanup],
        span: Span,
    ) -> Result<(), Diagnostic> {
        if Self::live_drop_owners(expected) != Self::live_drop_owners(actual) {
            return Err(self.error(
                "E_OWNERSHIP_MAYBE_MOVED",
                "control-flow join would make an owned local maybe-live or maybe-moved",
                span,
            ));
        }
        Ok(())
    }

    fn type_needs_drop(&mut self, ty: TypeId, active: &mut BTreeSet<TypeId>) -> bool {
        if !active.insert(ty) {
            return false;
        }
        let result = match self.semantic.type_of(ty).clone() {
            CanonicalType::Struct { symbol, args } | CanonicalType::Enum { symbol, args } => {
                if self.semantic.functions.iter().any(|function| {
                    function.kind == FunctionKind::Drop && function.drop_target == Some(symbol)
                }) {
                    true
                } else {
                    let Some(definition) = self
                        .semantic
                        .definitions
                        .iter()
                        .find(|definition| definition.symbol == symbol)
                        .cloned()
                    else {
                        active.remove(&ty);
                        return false;
                    };
                    let substitutions: BTreeMap<_, _> = definition
                        .generic_params
                        .iter()
                        .cloned()
                        .zip(args)
                        .collect();
                    match definition.kind {
                        TypeDefKind::Struct { fields, .. } => fields.into_iter().any(|field| {
                            self.resolve_annotation(&field.annotation, &substitutions, field.span)
                                .is_ok_and(|field_ty| self.type_needs_drop(field_ty, active))
                        }),
                        TypeDefKind::Enum { variants } => variants.into_iter().any(|variant| {
                            variant.payload.into_iter().any(|payload| {
                                self.resolve_annotation(&payload, &substitutions, variant.span)
                                    .is_ok_and(|payload_ty| {
                                        self.type_needs_drop(payload_ty, active)
                                    })
                            })
                        }),
                    }
                }
            }
            CanonicalType::Array { element, .. } => self.type_needs_drop(element, active),
            CanonicalType::GenericParam { .. } => true,
            _ => false,
        };
        active.remove(&ty);
        result
    }

    fn drop_block(
        &mut self,
        place: HirPlace,
        ty: TypeId,
        span: Span,
    ) -> Result<HirBlock, Diagnostic> {
        if matches!(
            self.semantic.type_of(ty),
            CanonicalType::GenericParam { .. }
        ) {
            return Ok(HirBlock {
                statements: vec![HirStatement::Expression(HirExpr {
                    ty: TypeId(0),
                    category: ValueCategory::RValue,
                    ownership: OwnershipAction::Copy,
                    allocator_origin: None,
                    borrow_origin: None,
                    kind: HirExprKind::Drop {
                        place,
                        cleanup: HirBlock::default(),
                    },
                    span,
                })],
            });
        }
        let mut statements = Vec::new();
        let nominal = match self.semantic.type_of(ty) {
            CanonicalType::Struct { symbol, args } | CanonicalType::Enum { symbol, args } => {
                Some((*symbol, args.clone()))
            }
            _ => None,
        };
        if let Some((symbol, args)) = nominal.clone() {
            if let Some(function) = self.semantic.functions.iter().find(|function| {
                function.kind == FunctionKind::Drop && function.drop_target == Some(symbol)
            }) {
                let borrow_ty = self
                    .semantic
                    .types
                    .iter()
                    .position(|candidate| {
                        *candidate
                            == CanonicalType::Borrow {
                                mutable: true,
                                pointee: ty,
                            }
                    })
                    .map(|index| TypeId(index as u32))
                    .ok_or_else(|| {
                        self.error(
                            "E_HIR_DROP_BORROW",
                            "drop glue is missing its canonical mutable borrow type",
                            span,
                        )
                    })?;
                statements.push(HirStatement::Expression(HirExpr {
                    ty: TypeId(0),
                    category: ValueCategory::RValue,
                    ownership: OwnershipAction::Copy,
                    allocator_origin: None,
                    borrow_origin: None,
                    kind: HirExprKind::DirectCall {
                        callee: FunctionInstanceKey {
                            symbol: function.symbol,
                            arguments: args,
                        },
                        convention: CallConvention::DByte,
                        arguments: vec![HirExpr {
                            ty: borrow_ty,
                            category: ValueCategory::RValue,
                            ownership: OwnershipAction::BorrowMut,
                            allocator_origin: None,
                            borrow_origin: None,
                            kind: HirExprKind::Borrow {
                                mutable: true,
                                place: place.clone(),
                            },
                            span,
                        }],
                    },
                    span,
                }));
            }
        }
        if let Some((symbol, args)) = nominal {
            let definition = self
                .semantic
                .definitions
                .iter()
                .find(|definition| definition.symbol == symbol)
                .cloned();
            if let Some(definition) = definition {
                if let TypeDefKind::Struct { fields, .. } = definition.kind {
                    let (_, offsets) = super::layout_struct(self.semantic, symbol, &args)?;
                    let substitutions: BTreeMap<_, _> = definition
                        .generic_params
                        .iter()
                        .cloned()
                        .zip(args.iter().copied())
                        .collect();
                    for (field, offset) in fields.iter().zip(offsets).rev() {
                        if !annotation_can_need_drop(
                            self.semantic,
                            &field.annotation,
                            &substitutions,
                        ) {
                            continue;
                        }
                        let field_ty = super::resolved_annotation(
                            self.semantic,
                            &field.annotation,
                            &substitutions,
                        )?;
                        if self.type_needs_drop(field_ty, &mut BTreeSet::new()) {
                            let mut field_place = place.clone();
                            field_place.projections.push(HirProjection::Field {
                                field: field.id,
                                ty: field_ty,
                                offset,
                            });
                            field_place.ty = field_ty;
                            statements
                                .extend(self.drop_block(field_place, field_ty, span)?.statements);
                        }
                    }
                }
            }
        }
        Ok(HirBlock { statements })
    }

    fn register_drop(&mut self, local: LocalId, ty: TypeId, span: Span) -> Result<(), Diagnostic> {
        if self.type_needs_drop(ty, &mut BTreeSet::new()) {
            let place = HirPlace {
                base: HirPlaceBase::Local(local),
                projections: Vec::new(),
                ty,
                mutable: true,
                span,
            };
            let block = self.drop_block(place, ty, span)?;
            self.active_cleanups.push(ActiveCleanup {
                owner: Some(local),
                block,
            });
        }
        Ok(())
    }
    fn fact(&self, span: Span) -> Result<HirExpression, Diagnostic> {
        self.facts
            .get(&(span.line, span.col))
            .cloned()
            .ok_or_else(|| {
                let columns = self
                    .facts
                    .keys()
                    .filter_map(|(line, col)| (*line == span.line).then_some(*col))
                    .collect::<Vec<_>>();
                self.error(
                    "E_HIR_MISSING_EXPRESSION",
                    format!(
                        "canonical expression fact is missing (recorded columns on line: {columns:?})"
                    ),
                    span,
                )
            })
    }
    fn error(&self, code: &'static str, message: impl Into<String>, span: Span) -> Diagnostic {
        Diagnostic {
            code,
            message: message.into(),
            span,
        }
    }

    fn generic_types(&self, function: &FunctionDef) -> BTreeMap<String, TypeId> {
        let mut output = BTreeMap::new();
        if let Some(owner) = function.generic {
            for (index, name) in function.generic_params.iter().enumerate() {
                if let Some((id, _)) = self.semantic.types.iter().enumerate().find(|(_, ty)| matches!(ty, CanonicalType::GenericParam { owner: candidate, index: candidate_index } if *candidate == owner && *candidate_index == index as u32)) {
                    output.insert(name.clone(), TypeId(id as u32));
                }
            }
        }
        output
    }

    fn resolve_annotation(
        &self,
        annotation: &dbyte_ast::TypeAnnotation,
        generics: &BTreeMap<String, TypeId>,
        span: Span,
    ) -> Result<TypeId, Diagnostic> {
        if let dbyte_ast::TypeAnnotation::Named(name) = annotation {
            if let Some(ty) = generics.get(name) {
                return Ok(*ty);
            }
        }
        self.semantic
            .types
            .iter()
            .position(|candidate| {
                annotation_matches(self.semantic, candidate, annotation, generics)
            })
            .map(|index| TypeId(index as u32))
            .ok_or_else(|| {
                self.error(
                    "E_HIR_UNRESOLVED_TYPE",
                    format!("type `{annotation}` has no canonical TypeId"),
                    span,
                )
            })
    }

    fn block(&mut self, statements: &[Stmt]) -> Result<HirBlock, Diagnostic> {
        self.push_scope();
        let defer_base = self.active_cleanups.len();
        let mut output = Vec::new();
        for statement in statements {
            if let Some(statement) = self.statement(statement, defer_base)? {
                output.push(statement);
            }
        }
        // Preserve the frozen v12.0 cleanup image for programs that do not
        // opt into affine destruction. v12.1 drop programs use the unified
        // graph and must not emit unreachable cleanup after terminal edges.
        let falls_through = !self.runtime_drop_enabled
            || !matches!(
                statements.last(),
                Some(Stmt::Return { .. } | Stmt::Break { .. } | Stmt::Continue { .. })
            );
        if falls_through {
            for cleanup in self
                .active_cleanups
                .get(defer_base..)
                .unwrap_or(&[])
                .iter()
                .rev()
            {
                output.extend(cleanup.block.statements.clone());
            }
        }
        self.active_cleanups.truncate(defer_base);
        self.pop_scope();
        Ok(HirBlock { statements: output })
    }

    fn statement(
        &mut self,
        statement: &Stmt,
        defer_base: usize,
    ) -> Result<Option<HirStatement>, Diagnostic> {
        Ok(Some(match statement {
            Stmt::Let {
                name,
                ty,
                value,
                span,
                ..
            } => {
                let mut value = self.expr(value)?;
                let local_ty = if matches!(ty, dbyte_ast::TypeAnnotation::Inferred) {
                    value.ty
                } else {
                    self.resolve_annotation(ty, &self.current_generics, *span)?
                };
                if value.ty != local_ty {
                    value.ty = local_ty;
                }
                let local = self.define_local(name.clone(), local_ty, false, *span);
                self.register_drop(local.id, local_ty, *span)?;
                HirStatement::Let {
                    local: local.id,
                    value,
                    span: *span,
                }
            }
            Stmt::Assign { name, value, span } => {
                let place = self.named_place(name, *span)?;
                let cleanup_before_commit = match &place.base {
                    HirPlaceBase::Local(local) if place.projections.is_empty() => self
                        .active_cleanups
                        .iter()
                        .find(|cleanup| cleanup.owner == Some(*local))
                        .map(|cleanup| cleanup.block.clone()),
                    _ => None,
                };
                let mut value = self.expr(value)?;
                if value.ty != place.ty {
                    value.ty = place.ty;
                }
                if let HirPlaceBase::Local(local) = &place.base {
                    if place.projections.is_empty() {
                        self.deactivate_drop(*local);
                        self.register_drop(*local, place.ty, *span)?;
                    }
                }
                HirStatement::Assign {
                    place,
                    value,
                    cleanup_before_commit,
                    span: *span,
                }
            }
            Stmt::Store {
                target,
                value,
                span,
            } => HirStatement::Assign {
                place: self.place(target)?,
                value: self.expr(value)?,
                cleanup_before_commit: None,
                span: *span,
            },
            Stmt::Expr(value) => HirStatement::Expression(self.expr(value)?),
            Stmt::Return { value, span } => HirStatement::Return {
                value: value.as_ref().map(|value| self.expr(value)).transpose()?,
                cleanup: self
                    .active_cleanups
                    .iter()
                    .rev()
                    .map(|cleanup| cleanup.block.clone())
                    .collect(),
                span: *span,
            },
            Stmt::If {
                cond,
                then_body,
                else_body,
                span,
            } => {
                let condition = self.expr(cond)?;
                let entry = self.active_cleanups.clone();
                let (then_block, then_exit) = self.branch_block(then_body, &entry)?;
                let (else_block, else_exit) = if let Some(body) = else_body {
                    let (block, exit) = self.branch_block(body, &entry)?;
                    (Some(block), exit)
                } else {
                    (None, entry.clone())
                };
                self.require_same_drop_state(&then_exit, &else_exit, *span)?;
                self.active_cleanups = then_exit;
                HirStatement::If {
                    condition,
                    then_block,
                    else_block,
                    span: *span,
                }
            }
            Stmt::While { cond, body, span } => {
                let condition = self.expr(cond)?;
                let entry = self.active_cleanups.clone();
                let (body, exit) = self.branch_block(body, &entry)?;
                self.require_same_drop_state(&entry, &exit, *span)?;
                self.active_cleanups = entry;
                HirStatement::While {
                    condition,
                    body,
                    span: *span,
                }
            }
            Stmt::Match {
                mode,
                value,
                arms,
                span,
            } => {
                let value = self.expr(value)?;
                let enum_ty = value.ty;
                let (enum_symbol, _) = match self.semantic.type_of(enum_ty) {
                    CanonicalType::Enum { symbol, args } => (*symbol, args.clone()),
                    _ => {
                        return Err(self.error(
                            "E_HIR_MATCH_TYPE",
                            "validated match lost its enum type",
                            *span,
                        ))
                    }
                };
                let definition = self
                    .semantic
                    .definitions
                    .iter()
                    .find(|definition| definition.symbol == enum_symbol)
                    .ok_or_else(|| {
                        self.error("E_HIR_MATCH_TYPE", "enum definition is missing", *span)
                    })?;
                let variants = match &definition.kind {
                    TypeDefKind::Enum { variants } => variants,
                    _ => {
                        return Err(self.error(
                            "E_HIR_MATCH_TYPE",
                            "match symbol is not an enum",
                            *span,
                        ))
                    }
                };
                let mut lowered = Vec::new();
                let entry = self.active_cleanups.clone();
                let mut common_exit: Option<Vec<ActiveCleanup>> = None;
                for arm in arms {
                    self.active_cleanups = entry.clone();
                    self.push_scope();
                    let (variant, bindings) = match &arm.pattern {
                        Pattern::Wildcard(_) => (None, Vec::new()),
                        Pattern::Variant {
                            path,
                            bindings,
                            span,
                        } => {
                            let name = path.rsplit('.').next().unwrap_or(path);
                            let variant = variants
                                .iter()
                                .find(|variant| variant.name == name)
                                .ok_or_else(|| {
                                    self.error(
                                        "E_HIR_UNKNOWN_VARIANT",
                                        "validated variant is missing",
                                        *span,
                                    )
                                })?;
                            let (_, _, payload_layout) =
                                super::enum_variant_layout(self.semantic, enum_ty, variant.id)?;
                            let mut locals = Vec::new();
                            for (binding, (payload_ty, _)) in bindings.iter().zip(payload_layout) {
                                let ty = match mode {
                                    MatchMode::Move => payload_ty,
                                    MatchMode::Borrow | MatchMode::BorrowMut => {
                                        let expected = CanonicalType::Borrow {
                                            mutable: *mode == MatchMode::BorrowMut,
                                            pointee: payload_ty,
                                        };
                                        self.semantic
                                            .types
                                            .iter()
                                            .position(|candidate| *candidate == expected)
                                            .map(|index| TypeId(index as u32))
                                            .ok_or_else(|| {
                                                self.error(
                                                    "E_HIR_UNRESOLVED_TYPE",
                                                    "match borrow type is missing",
                                                    *span,
                                                )
                                            })?
                                    }
                                };
                                let local = self.define_local(binding.clone(), ty, false, *span);
                                if *mode == MatchMode::Move {
                                    self.register_drop(local.id, ty, *span)?;
                                }
                                locals.push(local);
                            }
                            (
                                Some(FieldedVariant {
                                    symbol: enum_symbol,
                                    variant: variant.id,
                                    discriminant: variant.discriminant,
                                }),
                                locals,
                            )
                        }
                    };
                    let body = self.block(&arm.body)?;
                    self.pop_scope();
                    let exit = self.active_cleanups.clone();
                    if !block_is_terminal(&body) {
                        if let Some(expected) = &common_exit {
                            self.require_same_drop_state(expected, &exit, arm.span)?;
                        } else {
                            common_exit = Some(exit);
                        }
                    }
                    lowered.push(HirMatchArm {
                        variant,
                        bindings,
                        body,
                        span: arm.span,
                    });
                }
                self.active_cleanups = common_exit.unwrap_or(entry);
                HirStatement::Match {
                    mode: *mode,
                    value,
                    arms: lowered,
                    exhaustive: true,
                    span: *span,
                }
            }
            Stmt::Defer {
                captures,
                body,
                span,
            } => {
                let captures = captures
                    .iter()
                    .map(|capture| self.capture(capture))
                    .collect::<Result<Vec<_>, _>>()?;
                let body = self.block(body)?;
                self.active_cleanups.push(ActiveCleanup {
                    owner: None,
                    block: body.clone(),
                });
                HirStatement::Defer {
                    captures,
                    body,
                    span: *span,
                }
            }
            Stmt::Break { span } => HirStatement::Break {
                cleanup: self
                    .active_cleanups
                    .get(defer_base..)
                    .unwrap_or(&[])
                    .iter()
                    .rev()
                    .map(|cleanup| cleanup.block.clone())
                    .collect(),
                span: *span,
            },
            Stmt::Continue { span } => HirStatement::Continue {
                cleanup: self
                    .active_cleanups
                    .get(defer_base..)
                    .unwrap_or(&[])
                    .iter()
                    .rev()
                    .map(|cleanup| cleanup.block.clone())
                    .collect(),
                span: *span,
            },
            Stmt::For {
                var,
                iterable,
                body,
                span,
            } => {
                let iterable = self.expr(iterable)?;
                let element = match self.semantic.type_of(iterable.ty) {
                    CanonicalType::LegacyList(element)
                    | CanonicalType::Array { element, .. }
                    | CanonicalType::Slice { element, .. } => *element,
                    CanonicalType::LegacyStr => iterable.ty,
                    _ => {
                        return Err(self.error(
                            "E_HIR_FOR_TYPE",
                            "validated for iterable has no canonical element type",
                            *span,
                        ))
                    }
                };
                let entry = self.active_cleanups.clone();
                self.push_scope();
                let local = self.define_local(var.clone(), element, false, *span);
                let (body, exit) = self.branch_block(body, &entry)?;
                self.pop_scope();
                self.require_same_drop_state(&entry, &exit, *span)?;
                self.active_cleanups = entry;
                HirStatement::For {
                    local,
                    iterable,
                    body,
                    span: *span,
                }
            }
            Stmt::FnDef { .. }
            | Stmt::StructDef { .. }
            | Stmt::EnumDef { .. }
            | Stmt::Import { .. }
            | Stmt::Const { .. }
            | Stmt::Static { .. } => return Ok(None),
        }))
    }

    fn capture(&self, capture: &dbyte_ast::Capture) -> Result<HirCapture, Diagnostic> {
        let local = self.lookup_local(&capture.name).ok_or_else(|| {
            self.error(
                "E_HIR_UNKNOWN_CAPTURE",
                "validated capture has no local",
                capture.span,
            )
        })?;
        let ty = self
            .locals
            .iter()
            .find(|candidate| candidate.id == local)
            .map(|candidate| candidate.ty)
            .ok_or_else(|| {
                self.error(
                    "E_HIR_UNKNOWN_CAPTURE",
                    "capture local metadata is missing",
                    capture.span,
                )
            })?;
        Ok(HirCapture {
            local,
            mode: capture.mode,
            ty,
            span: capture.span,
        })
    }

    fn named_place(&self, name: &str, span: Span) -> Result<HirPlace, Diagnostic> {
        if let Some(local) = self.lookup_local(name) {
            let ty = self
                .locals
                .iter()
                .find(|candidate| candidate.id == local)
                .map(|candidate| candidate.ty)
                .ok_or_else(|| {
                    self.error("E_HIR_UNKNOWN_LOCAL", "local metadata is missing", span)
                })?;
            return Ok(HirPlace {
                base: HirPlaceBase::Local(local),
                projections: Vec::new(),
                ty,
                mutable: true,
                span,
            });
        }
        let symbol = self.symbols.get(name).copied().ok_or_else(|| {
            self.error(
                "E_HIR_UNKNOWN_SYMBOL",
                "validated place has no symbol",
                span,
            )
        })?;
        let global = self
            .semantic
            .globals
            .iter()
            .find(|global| global.symbol == symbol)
            .ok_or_else(|| {
                self.error("E_HIR_UNKNOWN_GLOBAL", "place symbol is not a global", span)
            })?;
        let ty = self.resolve_annotation(&global.annotation, &BTreeMap::new(), span)?;
        Ok(HirPlace {
            base: HirPlaceBase::Global(symbol),
            projections: Vec::new(),
            ty,
            mutable: global.mutable,
            span,
        })
    }

    fn place(&mut self, expression: &Expr) -> Result<HirPlace, Diagnostic> {
        match expression {
            Expr::Ident(name, span) => self.named_place(name, *span),
            Expr::Unary {
                op: UnaryOp::Deref,
                expr,
                span,
            } => {
                let pointer = self.expr(expr)?;
                let fact = self.fact(*span)?;
                Ok(HirPlace {
                    base: HirPlaceBase::Pointer(Box::new(pointer)),
                    projections: Vec::new(),
                    ty: fact.ty,
                    mutable: true,
                    span: *span,
                })
            }
            Expr::Member {
                object,
                property,
                span,
            } => {
                let mut place = self.place(object)?;
                let (field, offset, ty) = resolve_field(self.semantic, place.ty, property, *span)?;
                place
                    .projections
                    .push(HirProjection::Field { field, ty, offset });
                place.ty = ty;
                place.span = *span;
                Ok(place)
            }
            Expr::Index {
                target,
                index,
                span,
            } => {
                let target_fact = self.fact(target.span())?;
                let mut place = if matches!(
                    self.semantic.type_of(target_fact.ty),
                    CanonicalType::Pointer { .. } | CanonicalType::LegacyStr
                ) {
                    HirPlace {
                        base: HirPlaceBase::Pointer(Box::new(self.expr(target)?)),
                        projections: Vec::new(),
                        ty: target_fact.ty,
                        mutable: matches!(
                            self.semantic.type_of(target_fact.ty),
                            CanonicalType::Pointer { mutable: true, .. }
                        ),
                        span: target.span(),
                    }
                } else {
                    self.place(target)?
                };
                let index = self.expr(index)?;
                let (element, size) = resolve_element(self.semantic, place.ty, *span)?;
                place.projections.push(HirProjection::Index {
                    index,
                    element,
                    element_size: size,
                });
                place.ty = element;
                place.span = *span;
                Ok(place)
            }
            _ => Err(self.error(
                "E_HIR_NOT_PLACE",
                "validated lvalue cannot be represented as a HIR place",
                expression.span(),
            )),
        }
    }

    fn expr(&mut self, expression: &Expr) -> Result<HirExpr, Diagnostic> {
        let fact = self.fact(expression.span())?;
        // Expected-context checking represents a signed minimum literal as a
        // unary expression but intentionally does not create a separate fact
        // for its out-of-range positive operand. Materialize that one literal
        // form here. Ordinary arithmetic remains executable HIR so frozen
        // native profiles retain their instruction-level behavior and v12
        // backends all observe the same operations.
        if matches!(
            expression,
            Expr::Unary {
                op: UnaryOp::Neg,
                expr,
                ..
            } if matches!(expr.as_ref(), Expr::IntLit(..))
        ) {
            if let Some(value) = super::constant_integer_expression(expression) {
                return Ok(HirExpr {
                    ty: fact.ty,
                    category: fact.category,
                    ownership: fact.ownership,
                    allocator_origin: fact.allocator_origin,
                    borrow_origin: fact.borrow_origin,
                    kind: HirExprKind::Integer(value),
                    span: expression.span(),
                });
            }
        }
        let kind = match expression {
            Expr::UnitLit(_) => HirExprKind::Unit,
            Expr::IntLit(value, _) => HirExprKind::Integer(*value),
            Expr::FloatLit(value, _) => HirExprKind::Float(*value),
            Expr::BoolLit(value, _) => HirExprKind::Bool(*value),
            Expr::StrLit(value, _) => HirExprKind::String(value.as_bytes().to_vec()),
            Expr::BytesLit(value, _) => HirExprKind::Bytes(value.clone()),
            Expr::NullLit(_) => HirExprKind::Null,
            Expr::Ident(name, span) => {
                if let Some(local) = self.lookup_local(name) {
                    HirExprKind::Local(local)
                } else if let Some(symbol) = self.symbols.get(name).copied() {
                    if self
                        .semantic
                        .functions
                        .iter()
                        .any(|function| function.symbol == symbol)
                    {
                        HirExprKind::Function(FunctionInstanceKey {
                            symbol,
                            arguments: Vec::new(),
                        })
                    } else if let Some(initializer) = self
                        .semantic
                        .globals
                        .iter()
                        .find(|global| global.symbol == symbol && global.constant)
                        .and_then(|global| global.initializer.clone())
                    {
                        HirExprKind::Constant {
                            symbol,
                            value: Box::new(self.expr(&initializer)?),
                        }
                    } else {
                        HirExprKind::Global(symbol)
                    }
                } else {
                    return Err(self.error(
                        "E_HIR_UNKNOWN_SYMBOL",
                        "validated identifier has no canonical symbol",
                        *span,
                    ));
                }
            }
            Expr::List(values, _) => HirExprKind::List(
                values
                    .iter()
                    .map(|value| self.expr(value))
                    .collect::<Result<_, _>>()?,
            ),
            Expr::Binary {
                left, op, right, ..
            } => HirExprKind::Binary {
                left: Box::new(self.expr(left)?),
                op: op.clone(),
                right: Box::new(self.expr(right)?),
            },
            Expr::Unary {
                op: UnaryOp::AddressOf,
                expr,
                ..
            } => HirExprKind::Borrow {
                mutable: true,
                place: self.place(expr)?,
            },
            Expr::Unary { op, expr, .. } => HirExprKind::Unary {
                op: op.clone(),
                value: Box::new(self.expr(expr)?),
            },
            Expr::Borrow { mutable, expr, .. } => HirExprKind::Borrow {
                mutable: *mutable,
                place: self.place(expr)?,
            },
            Expr::Cast { expr, .. } => HirExprKind::Cast {
                value: Box::new(self.expr(expr)?),
                target: fact.ty,
            },
            Expr::Index { .. } | Expr::Member { .. } => HirExprKind::Place(self.place(expression)?),
            Expr::Call { name, args, span } => self.call(name, &[], args, &[], *span)?,
            Expr::Invoke {
                callee,
                args,
                named_args,
                span,
            } => {
                if let Some((name, type_args)) = generic_target(callee) {
                    self.call(name, type_args, args, named_args, *span)?
                } else {
                    HirExprKind::IndirectCall {
                        convention: call_convention(self.semantic, callee)?,
                        callee: Box::new(self.expr(callee)?),
                        arguments: args
                            .iter()
                            .map(|argument| self.expr(argument))
                            .collect::<Result<_, _>>()?,
                    }
                }
            }
            Expr::MemberCall {
                object,
                property,
                args,
                span,
            } => {
                if let Expr::Ident(namespace, _) = object.as_ref() {
                    if let Some(module) = self.host_modules.get(namespace) {
                        let capability =
                            HostCapabilityId::resolve(module, property).ok_or_else(|| {
                                self.error(
                                    "E_HIR_HOST_CAPABILITY",
                                    "validated host capability lost its canonical identity",
                                    *span,
                                )
                            })?;
                        HirExprKind::HostCall {
                            capability,
                            arguments: args
                                .iter()
                                .map(|argument| self.expr(argument))
                                .collect::<Result<_, _>>()?,
                        }
                    } else if let Some(intrinsic) = HirIntrinsic::resolve(namespace, property) {
                        HirExprKind::IntrinsicCall {
                            intrinsic,
                            arguments: args
                                .iter()
                                .map(|argument| self.expr(argument))
                                .collect::<Result<_, _>>()?,
                        }
                    } else if let Some(kind) = self.variant_call(object, property, args, *span)? {
                        kind
                    } else {
                        return Err(self.error(
                            "E_HIR_UNRESOLVED_MEMBER_CALL",
                            "member call has no resolved canonical identity",
                            *span,
                        ));
                    }
                } else if let Some(kind) = self.variant_call(object, property, args, *span)? {
                    kind
                } else {
                    return Err(self.error(
                        "E_HIR_UNSUPPORTED",
                        "member call is not a resolved enum constructor",
                        *span,
                    ));
                }
            }
            Expr::GenericApply { target, args, span } => {
                let Expr::Ident(name, _) = target.as_ref() else {
                    return Err(self.error(
                        "E_HIR_GENERIC_TARGET",
                        "generic target is not a canonical symbol",
                        *span,
                    ));
                };
                let function = self
                    .semantic
                    .functions
                    .iter()
                    .find(|function| function.name == *name)
                    .ok_or_else(|| {
                        self.error(
                            "E_HIR_GENERIC_TARGET",
                            "generic target is not a function",
                            *span,
                        )
                    })?;
                HirExprKind::Function(FunctionInstanceKey {
                    symbol: function.symbol,
                    arguments: args
                        .iter()
                        .map(|arg| self.resolve_annotation(arg, &BTreeMap::new(), *span))
                        .collect::<Result<_, _>>()?,
                })
            }
            Expr::Try { expr, .. } => {
                let value = self.expr(expr)?;
                let (ok, err) = result_variants(self.semantic, value.ty, expression.span())?;
                HirExprKind::Try {
                    value: Box::new(value),
                    ok_variant: ok,
                    err_variant: err,
                    cleanup: self
                        .active_cleanups
                        .iter()
                        .rev()
                        .map(|cleanup| cleanup.block.clone())
                        .collect(),
                }
            }
            Expr::Drop { expr, span } => {
                let place = self.place(expr)?;
                let cleanup = self.drop_block(place.clone(), place.ty, *span)?;
                if let HirPlaceBase::Local(local) = place.base {
                    if place.projections.is_empty() {
                        self.deactivate_drop(local);
                    }
                }
                HirExprKind::Drop { place, cleanup }
            }
            Expr::Closure {
                captures,
                params,
                ret_ty,
                body,
                span,
            } => {
                let closure = ClosureId(self.next_closure);
                self.next_closure += 1;
                let captures = captures
                    .iter()
                    .map(|capture| self.capture(capture))
                    .collect::<Result<Vec<_>, _>>()?;
                self.push_scope();
                let parameters = params
                    .iter()
                    .map(|parameter| {
                        let ty = self.resolve_annotation(
                            &parameter.ty,
                            &BTreeMap::new(),
                            parameter.span,
                        )?;
                        Ok(self.define_local(parameter.name.clone(), ty, true, parameter.span))
                    })
                    .collect::<Result<Vec<_>, Diagnostic>>()?;
                let result = self.resolve_annotation(ret_ty, &BTreeMap::new(), *span)?;
                let body = self.block(body)?;
                self.pop_scope();
                HirExprKind::Closure {
                    closure,
                    captures,
                    parameters,
                    result,
                    body,
                }
            }
            Expr::FStr(parts, span) => HirExprKind::FormatString(
                parts
                    .iter()
                    .map(|part| match part {
                        dbyte_ast::FStrPart::Literal(value) => {
                            Ok(HirFormatPart::Literal(value.as_bytes().to_vec()))
                        }
                        dbyte_ast::FStrPart::Interp(name) => self
                            .expr(&Expr::Ident(name.clone(), *span))
                            .map(HirFormatPart::Value),
                    })
                    .collect::<Result<_, _>>()?,
            ),
        };
        let lowered = HirExpr {
            ty: fact.ty,
            category: fact.category,
            ownership: fact.ownership,
            allocator_origin: fact.allocator_origin,
            borrow_origin: fact.borrow_origin,
            kind,
            span: expression.span(),
        };
        if fact.ownership == OwnershipAction::Move {
            match &lowered.kind {
                HirExprKind::Local(local) => self.deactivate_drop(*local),
                HirExprKind::Place(HirPlace {
                    base: HirPlaceBase::Local(local),
                    projections,
                    ..
                }) if projections.is_empty() => self.deactivate_drop(*local),
                _ => {}
            }
        }
        Ok(lowered)
    }

    fn call(
        &mut self,
        name: &str,
        type_args: &[dbyte_ast::TypeAnnotation],
        args: &[Expr],
        named_args: &[dbyte_ast::NamedArgument],
        span: Span,
    ) -> Result<HirExprKind, Diagnostic> {
        if name == "print" {
            return Ok(HirExprKind::HostCall {
                capability: HostCapabilityId::Print,
                arguments: args
                    .iter()
                    .map(|argument| self.expr(argument))
                    .collect::<Result<_, _>>()?,
            });
        }
        if name == "interrupt_address" || name == "task_address" {
            let resolved = self
                .semantic
                .resolved_addresses
                .get(&(span.line, span.col))
                .ok_or_else(|| {
                    self.error(
                        "E_HIR_UNRESOLVED_ADDRESS",
                        "validated address reference lost its canonical identity",
                        span,
                    )
                })?;
            return Ok(match resolved {
                super::ResolvedAddress::Interrupt {
                    vector,
                    handler,
                    class,
                } => HirExprKind::InterruptAddress {
                    vector: *vector,
                    handler: FunctionInstanceKey {
                        symbol: *handler,
                        arguments: Vec::new(),
                    },
                    class: *class,
                },
                super::ResolvedAddress::Task { function } => HirExprKind::TaskAddress {
                    function: FunctionInstanceKey {
                        symbol: *function,
                        arguments: Vec::new(),
                    },
                },
            });
        }
        if name == "size_of" || name == "align_of" {
            let annotation = args
                .first()
                .and_then(type_expression_annotation)
                .ok_or_else(|| {
                    self.error(
                        "E_HIR_LAYOUT_TYPE",
                        "validated layout query lost its canonical type",
                        span,
                    )
                })?;
            let ty = self.resolve_annotation(&annotation, &self.current_generics, span)?;
            return Ok(HirExprKind::LayoutOf {
                target: ty,
                alignment: name == "align_of",
            });
        }
        if name == "len" {
            let value = args.first().ok_or_else(|| {
                self.error(
                    "E_HIR_LENGTH_ARGUMENT",
                    "validated len call lost its argument",
                    span,
                )
            })?;
            return Ok(HirExprKind::Length {
                value: Box::new(self.expr(value)?),
            });
        }
        if let Some(local) = self.lookup_local(name) {
            let local_ty = self
                .locals
                .iter()
                .find(|candidate| candidate.id == local)
                .map(|candidate| candidate.ty)
                .ok_or_else(|| {
                    self.error(
                        "E_HIR_UNKNOWN_LOCAL",
                        "callable local metadata is missing",
                        span,
                    )
                })?;
            let convention = match self.semantic.type_of(local_ty) {
                CanonicalType::Function { convention, .. } => *convention,
                CanonicalType::ClosureShape { .. } => CallConvention::DByte,
                _ => {
                    return Err(self.error(
                        "E_HIR_NOT_CALLABLE",
                        "validated local is not callable",
                        span,
                    ))
                }
            };
            let callee = HirExpr {
                ty: local_ty,
                category: ValueCategory::LValue,
                ownership: OwnershipAction::Copy,
                allocator_origin: None,
                borrow_origin: None,
                kind: HirExprKind::Local(local),
                span,
            };
            return Ok(HirExprKind::IndirectCall {
                callee: Box::new(callee),
                convention,
                arguments: args
                    .iter()
                    .map(|argument| self.expr(argument))
                    .collect::<Result<_, _>>()?,
            });
        }
        if let Some(function) = self
            .semantic
            .functions
            .iter()
            .find(|function| function.name == name)
        {
            let arguments = type_args
                .iter()
                .map(|arg| self.resolve_annotation(arg, &self.current_generics, span))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(HirExprKind::DirectCall {
                callee: FunctionInstanceKey {
                    symbol: function.symbol,
                    arguments,
                },
                convention: function.convention,
                arguments: args
                    .iter()
                    .map(|argument| self.expr(argument))
                    .collect::<Result<_, _>>()?,
            });
        }
        if let Some(definition) = self
            .semantic
            .definitions
            .iter()
            .find(|definition| definition.name == name)
        {
            let TypeDefKind::Struct { fields, .. } = &definition.kind else {
                return Err(self.error("E_HIR_NOT_CALLABLE", "enum type requires a variant", span));
            };
            let mut values = Vec::new();
            for field in fields {
                let argument = named_args
                    .iter()
                    .find(|argument| argument.name == field.name)
                    .ok_or_else(|| {
                        self.error(
                            "E_HIR_MISSING_FIELD",
                            "validated struct initializer lost a field",
                            span,
                        )
                    })?;
                values.push((field.id, self.expr(&argument.value)?));
            }
            return Ok(HirExprKind::Struct {
                symbol: definition.symbol,
                fields: values,
            });
        }
        Err(self.error(
            "E_HIR_UNRESOLVED_CALL",
            format!("call `{name}` has no resolved canonical identity"),
            span,
        ))
    }

    fn variant_call(
        &mut self,
        object: &Expr,
        property: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<Option<HirExprKind>, Diagnostic> {
        let (name, _) = generic_target(object).unwrap_or(match object {
            Expr::Ident(name, _) => (name.as_str(), &[]),
            _ => ("", &[]),
        });
        let Some(definition) = self
            .semantic
            .definitions
            .iter()
            .find(|definition| definition.name == name)
        else {
            return Ok(None);
        };
        let TypeDefKind::Enum { variants } = &definition.kind else {
            return Ok(None);
        };
        let variant = variants
            .iter()
            .find(|variant| variant.name == property)
            .ok_or_else(|| {
                self.error(
                    "E_HIR_UNKNOWN_VARIANT",
                    "validated enum variant is missing",
                    span,
                )
            })?;
        Ok(Some(HirExprKind::Enum {
            symbol: definition.symbol,
            variant: variant.id,
            discriminant: variant.discriminant,
            payload: args
                .iter()
                .map(|argument| self.expr(argument))
                .collect::<Result<_, _>>()?,
        }))
    }
}

fn type_expression_annotation(expression: &Expr) -> Option<dbyte_ast::TypeAnnotation> {
    match expression {
        Expr::Ident(name, _) => Some(dbyte_ast::TypeAnnotation::Named(name.clone())),
        Expr::GenericApply { target, args, .. } => match target.as_ref() {
            Expr::Ident(name, _) => Some(dbyte_ast::TypeAnnotation::Generic {
                base: name.clone(),
                args: args.clone(),
            }),
            _ => None,
        },
        _ => None,
    }
}

fn generic_target(expression: &Expr) -> Option<(&str, &[dbyte_ast::TypeAnnotation])> {
    match expression {
        Expr::Ident(name, _) => Some((name, &[])),
        Expr::GenericApply { target, args, .. } => match target.as_ref() {
            Expr::Ident(name, _) => Some((name, args)),
            _ => None,
        },
        _ => None,
    }
}

fn call_convention(program: &HirProgram, callee: &Expr) -> Result<CallConvention, Diagnostic> {
    let fact = program
        .expressions
        .iter()
        .find(|fact| fact.span == callee.span())
        .ok_or_else(|| Diagnostic {
            code: "E_HIR_MISSING_EXPRESSION",
            message: "callee expression fact is missing".into(),
            span: callee.span(),
        })?;
    match program.type_of(fact.ty) {
        CanonicalType::Function { convention, .. } => Ok(*convention),
        CanonicalType::ClosureShape { .. } => Ok(CallConvention::DByte),
        _ => Err(Diagnostic {
            code: "E_HIR_NOT_CALLABLE",
            message: "validated callee is not callable".into(),
            span: callee.span(),
        }),
    }
}

fn annotation_matches(
    program: &HirProgram,
    candidate: &CanonicalType,
    annotation: &dbyte_ast::TypeAnnotation,
    generics: &BTreeMap<String, TypeId>,
) -> bool {
    use dbyte_ast::TypeAnnotation as A;
    if let A::Named(name) = annotation {
        if let Some(substitution) = generics.get(name) {
            return program.type_of(*substitution) == candidate;
        }
    }
    match (candidate, annotation) {
        (CanonicalType::Unit, A::Unit | A::Inferred) => true,
        (CanonicalType::Integer(super::IntegerType::LegacyInt), A::Int) => true,
        (CanonicalType::Integer(super::IntegerType::I8), A::I8) => true,
        (CanonicalType::Integer(super::IntegerType::U8), A::U8) => true,
        (CanonicalType::Integer(super::IntegerType::I16), A::I16) => true,
        (CanonicalType::Integer(super::IntegerType::U16), A::U16) => true,
        (CanonicalType::Integer(super::IntegerType::I32), A::I32) => true,
        (CanonicalType::Integer(super::IntegerType::U32), A::U32) => true,
        (CanonicalType::Integer(super::IntegerType::I64), A::I64) => true,
        (CanonicalType::Integer(super::IntegerType::U64), A::U64) => true,
        (CanonicalType::Integer(super::IntegerType::Usize), A::Usize) => true,
        (CanonicalType::Float, A::Float)
        | (CanonicalType::Bool, A::Bool)
        | (CanonicalType::LegacyStr, A::Str)
        | (CanonicalType::LegacyBytes, A::Bytes)
        | (CanonicalType::LegacyBuffer, A::Buffer)
        | (CanonicalType::StringView, A::StringView) => true,
        (CanonicalType::LegacyList(element), A::List(inner)) => {
            annotation_matches(program, program.type_of(*element), inner, generics)
        }
        (
            CanonicalType::Pointer {
                mutable: true,
                pointee,
            },
            A::Pointer(inner),
        )
        | (
            CanonicalType::Pointer {
                mutable: false,
                pointee,
            },
            A::ConstPointer(inner),
        ) => annotation_matches(program, program.type_of(*pointee), inner, generics),
        (
            CanonicalType::Slice {
                mutable: false,
                element,
            },
            A::Slice(inner),
        )
        | (
            CanonicalType::Slice {
                mutable: true,
                element,
            },
            A::MutSlice(inner),
        ) => annotation_matches(program, program.type_of(*element), inner, generics),
        (
            CanonicalType::Borrow { mutable, pointee },
            A::Borrow {
                mutable: expected,
                pointee: inner,
            },
        ) => {
            mutable == expected
                && annotation_matches(program, program.type_of(*pointee), inner, generics)
        }
        (
            CanonicalType::Array { element, length },
            A::Array {
                element: inner,
                length: expected,
            },
        ) => {
            length == expected
                && annotation_matches(program, program.type_of(*element), inner, generics)
        }
        (
            CanonicalType::Struct { symbol, .. } | CanonicalType::Enum { symbol, .. },
            A::Named(name),
        ) => program
            .definitions
            .iter()
            .any(|definition| definition.symbol == *symbol && definition.name == *name),
        (
            CanonicalType::Struct { symbol, args } | CanonicalType::Enum { symbol, args },
            A::Generic {
                base,
                args: annotations,
            },
        ) => {
            program
                .definitions
                .iter()
                .any(|definition| definition.symbol == *symbol && definition.name == *base)
                && args.len() == annotations.len()
                && args.iter().zip(annotations).all(|(ty, annotation)| {
                    annotation_matches(program, program.type_of(*ty), annotation, generics)
                })
        }
        (
            CanonicalType::Function {
                convention,
                params,
                result,
            },
            A::Function {
                convention: expected,
                params: annotations,
                ret,
            },
        ) => {
            convention == expected
                && params.len() == annotations.len()
                && params.iter().zip(annotations).all(|(ty, annotation)| {
                    annotation_matches(program, program.type_of(*ty), annotation, generics)
                })
                && annotation_matches(program, program.type_of(*result), ret, generics)
        }
        (
            CanonicalType::ClosureShape { params, result },
            A::Closure {
                params: annotations,
                ret,
            },
        ) => {
            params.len() == annotations.len()
                && params.iter().zip(annotations).all(|(ty, annotation)| {
                    annotation_matches(program, program.type_of(*ty), annotation, generics)
                })
                && annotation_matches(program, program.type_of(*result), ret, generics)
        }
        _ => false,
    }
}

fn resolve_field(
    program: &HirProgram,
    base: TypeId,
    name: &str,
    span: Span,
) -> Result<(FieldId, u32, TypeId), Diagnostic> {
    let (symbol, args) = match program.type_of(base) {
        CanonicalType::Struct { symbol, args } => (*symbol, args.clone()),
        CanonicalType::Pointer { pointee, .. } | CanonicalType::Borrow { pointee, .. } => {
            match program.type_of(*pointee) {
                CanonicalType::Struct { symbol, args } => (*symbol, args.clone()),
                _ => {
                    return Err(Diagnostic {
                        code: "E_HIR_FIELD_BASE",
                        message: "field base is not a struct".into(),
                        span,
                    })
                }
            }
        }
        _ => {
            return Err(Diagnostic {
                code: "E_HIR_FIELD_BASE",
                message: "field base is not a struct".into(),
                span,
            })
        }
    };
    let definition = program
        .definitions
        .iter()
        .find(|definition| definition.symbol == symbol)
        .ok_or_else(|| Diagnostic {
            code: "E_HIR_FIELD_BASE",
            message: "struct definition is missing".into(),
            span,
        })?;
    let TypeDefKind::Struct { fields, .. } = &definition.kind else {
        unreachable!()
    };
    let field = fields
        .iter()
        .find(|field| field.name == name)
        .ok_or_else(|| Diagnostic {
            code: "E_HIR_UNKNOWN_FIELD",
            message: "field definition is missing".into(),
            span,
        })?;
    let (_, offsets) = super::layout_struct(program, symbol, &args)?;
    let field_index = fields
        .iter()
        .position(|candidate| candidate.id == field.id)
        .ok_or_else(|| Diagnostic {
            code: "E_HIR_UNKNOWN_FIELD",
            message: "field layout is missing".into(),
            span,
        })?;
    let substitutions: BTreeMap<_, _> = definition
        .generic_params
        .iter()
        .cloned()
        .zip(args)
        .collect();
    let field_ty = program
        .types
        .iter()
        .position(|candidate| {
            annotation_matches(program, candidate, &field.annotation, &substitutions)
        })
        .map(|index| TypeId(index as u32))
        .ok_or_else(|| Diagnostic {
            code: "E_HIR_UNRESOLVED_TYPE",
            message: "field type is missing".into(),
            span,
        })?;
    Ok((field.id, offsets[field_index], field_ty))
}

fn resolve_element(
    program: &HirProgram,
    base: TypeId,
    span: Span,
) -> Result<(TypeId, u32), Diagnostic> {
    let element = match program.type_of(base) {
        CanonicalType::Array { element, .. }
        | CanonicalType::Slice { element, .. }
        | CanonicalType::LegacyList(element) => *element,
        CanonicalType::Pointer { pointee, .. } => *pointee,
        CanonicalType::LegacyStr => program
            .types
            .iter()
            .position(|ty| matches!(ty, CanonicalType::Integer(super::IntegerType::U8)))
            .map(|index| TypeId(index as u32))
            .ok_or_else(|| Diagnostic {
                code: "E_HIR_UNRESOLVED_TYPE",
                message: "legacy string byte type is missing".into(),
                span,
            })?,
        CanonicalType::LegacyBytes | CanonicalType::LegacyBuffer => program
            .types
            .iter()
            .position(|ty| matches!(ty, CanonicalType::Integer(super::IntegerType::LegacyInt)))
            .map(|index| TypeId(index as u32))
            .ok_or_else(|| Diagnostic {
                code: "E_HIR_UNRESOLVED_TYPE",
                message: "legacy byte index type is missing".into(),
                span,
            })?,
        CanonicalType::StringView => program
            .types
            .iter()
            .position(|ty| matches!(ty, CanonicalType::Integer(super::IntegerType::U8)))
            .map(|index| TypeId(index as u32))
            .ok_or_else(|| Diagnostic {
                code: "E_HIR_UNRESOLVED_TYPE",
                message: "StringView byte type is missing".into(),
                span,
            })?,
        _ => {
            return Err(Diagnostic {
                code: "E_HIR_INDEX_BASE",
                message: "index base has no canonical element type".into(),
                span,
            })
        }
    };
    let size = if matches!(
        program.type_of(base),
        CanonicalType::LegacyBytes | CanonicalType::LegacyBuffer
    ) {
        1
    } else {
        match super::layout_of(program, element) {
            Ok(layout) => layout.size,
            Err(error) if error.code == "E_LAYOUT_UNAVAILABLE" => 0,
            Err(error) => return Err(error),
        }
    };
    Ok((element, size))
}

fn result_variants(
    program: &HirProgram,
    ty: TypeId,
    span: Span,
) -> Result<(super::VariantId, super::VariantId), Diagnostic> {
    let CanonicalType::Enum { symbol, .. } = program.type_of(ty) else {
        return Err(Diagnostic {
            code: "E_HIR_TRY_TYPE",
            message: "try value is not Result".into(),
            span,
        });
    };
    let definition = program
        .definitions
        .iter()
        .find(|definition| definition.symbol == *symbol && definition.name == "Result")
        .ok_or_else(|| Diagnostic {
            code: "E_HIR_TRY_TYPE",
            message: "try value is not canonical Result".into(),
            span,
        })?;
    let TypeDefKind::Enum { variants } = &definition.kind else {
        unreachable!()
    };
    let ok = variants
        .iter()
        .find(|variant| variant.name == "Ok")
        .map(|variant| variant.id)
        .ok_or_else(|| Diagnostic {
            code: "E_HIR_TRY_TYPE",
            message: "Result.Ok is missing".into(),
            span,
        })?;
    let err = variants
        .iter()
        .find(|variant| variant.name == "Err")
        .map(|variant| variant.id)
        .ok_or_else(|| Diagnostic {
            code: "E_HIR_TRY_TYPE",
            message: "Result.Err is missing".into(),
            span,
        })?;
    Ok((ok, err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;

    fn parse(source: &str) -> Program {
        let tokens = Lexer::new(source).tokenize().unwrap();
        Parser::new(tokens).parse_program().unwrap()
    }

    #[test]
    fn executable_hir_resolves_locals_and_direct_generic_calls() {
        let source = "fn id[T](value: T) -> T:\n    return value\n\npub fn kernel_main() -> int:\n    let answer: i32 = id[i32](42)\n    return answer as int\n";
        let executable = analyze_executable(
            &parse(source),
            super::super::Profile::I686None,
            super::super::TargetDataLayout::I686,
        )
        .unwrap();
        let concrete = monomorphize(&executable).unwrap();
        assert_eq!(concrete.functions.len(), 2);
        assert!(concrete
            .functions
            .iter()
            .any(|function| function.symbol_name.starts_with("__dbyte_mono_id_t")));
        assert!(concrete.functions.iter().all(|function| {
            function
                .parameters
                .iter()
                .chain(&function.locals)
                .all(|local| {
                    !matches!(
                        concrete.semantic.type_of(local.ty),
                        CanonicalType::GenericParam { .. }
                    )
                })
        }));
    }

    #[test]
    fn monomorphization_recomputes_generic_index_projection_scale() {
        let source = "fn write[T](data: ptr[T], index: usize, value: T) -> unit:\n    data[index] = value\n    return unit\n\npub fn kernel_main() -> int:\n    let values: array[u32, 2] = [0 as u32, 0 as u32]\n    write[u32](&values as usize as ptr[u32], 1 as usize, 42 as u32)\n    return values[1] as int\n";
        let executable = analyze_executable(
            &parse(source),
            super::super::Profile::I686None,
            super::super::TargetDataLayout::I686,
        )
        .unwrap();
        let concrete = monomorphize(&executable).unwrap();
        let write = concrete
            .functions
            .iter()
            .find(|function| function.symbol_name.starts_with("__dbyte_mono_write_t"))
            .unwrap();
        let HirStatement::Assign { place, .. } = &write.body.statements[0] else {
            panic!("generic pointer write did not remain an assignment");
        };
        assert!(matches!(
            place.projections.last(),
            Some(HirProjection::Index {
                element_size: 4,
                ..
            })
        ));
    }

    #[test]
    fn v121_executable_hir_registers_custom_drop_on_fallthrough() {
        let source = "struct Resource:\n    id: u32\n\ndrop fn destroy(value: borrow mut Resource):\n    value.id = 0\n\nfn run() -> unit:\n    let resource = Resource(id = 7)\n    return unit\n";
        let executable = analyze_executable(
            &parse(source),
            super::super::Profile::Hosted,
            super::super::TargetDataLayout::I686,
        )
        .expect("drop program builds executable HIR");
        let run = executable
            .functions
            .iter()
            .find(|function| function.name == "run")
            .expect("run function");
        let HirStatement::Return { cleanup, .. } = run
            .body
            .statements
            .iter()
            .find(|statement| matches!(statement, HirStatement::Return { .. }))
            .unwrap()
        else {
            panic!("expected return");
        };
        assert_eq!(cleanup.len(), 1);
        assert!(matches!(
            cleanup[0].statements[0],
            HirStatement::Expression(HirExpr {
                kind: HirExprKind::DirectCall { .. },
                ..
            })
        ));
    }

    #[test]
    fn project_module_order_is_fail_closed_and_deterministic() {
        let program = parse("pub fn kernel_main() -> int:\n    return 0\n");
        let project = ProjectInput {
            root: ModuleId(0),
            modules: vec![ProjectModuleInput {
                id: ModuleId(1),
                name: "kernel".into(),
                program,
                imports: Vec::new(),
            }],
        };
        let error = analyze_project(
            &project,
            super::super::Profile::I686None,
            super::super::TargetDataLayout::I686,
        )
        .unwrap_err();
        assert_eq!(error[0].code, "E_MODULE_ORDER");
    }

    #[test]
    fn project_analysis_resolves_imported_public_symbols_before_body_hir() {
        let project = ProjectInput {
            root: ModuleId(1),
            modules: vec![
                ProjectModuleInput {
                    id: ModuleId(0),
                    name: "numbers".into(),
                    program: parse("pub fn add(left: u32, right: u32) -> u32:\n    return left + right\n"),
                    imports: Vec::new(),
                },
                ProjectModuleInput {
                    id: ModuleId(1),
                    name: "kernel".into(),
                    program: parse("import \"numbers.dby\" as numbers\n\npub fn kernel_main() -> int:\n    return numbers.add(40 as u32, 2 as u32) as int\n"),
                    imports: vec![("numbers".into(), ModuleId(0))],
                },
            ],
        };
        let analyzed = analyze_project(
            &project,
            super::super::Profile::I686None,
            super::super::TargetDataLayout::I686,
        )
        .unwrap();
        let concrete = monomorphize_project(&analyzed).unwrap();
        assert_eq!(concrete.executable.functions.len(), 2);
        assert_eq!(concrete.module_order, [ModuleId(0), ModuleId(1)]);
        assert_eq!(concrete.source_modules[&ModuleId(0)], "numbers");
        validate_project_profile(&concrete, super::super::Profile::I686None).unwrap();
        let add = analyzed
            .executable
            .semantic
            .functions
            .iter()
            .find(|function| function.name == "add")
            .unwrap();
        assert_eq!(analyzed.symbol_owners[&add.symbol], ModuleId(0));
        assert_eq!(analyzed.link_names[&add.symbol], "add");
    }
}
