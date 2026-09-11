//! Concrete HIR to Native MIR lowering.
//!
//! The first vertical slice covers canonical scalar control flow and calls.
//! Unsupported representation operations fail precisely; callers must never
//! fall back to an AST or backend-specific semantic path.

use crate::native_mir::{
    BlockId, MachineType, NativeAggregateLayout, NativeBinaryOp, NativeCallee, NativeCompareOp,
    NativeMirBlock, NativeMirError, NativeMirFunction, NativeMirGlobal, NativeMirInst,
    NativeMirProject, NativeStackObject, NativeTerminator, NativeUnaryOp, NativeValue,
    SpannedNativeInst, StackObjectId, VirtualRegister,
};
use dbyte_ast::{BinOp, Span, UnaryOp};
use dbyte_hir::{
    CanonicalType, ConcreteHirFunction, ConcreteHirProgram, HirBlock, HirExpr, HirExprKind,
    HirPlaceBase, HirStatement, IntegerType, LocalId, TypeId,
};
use std::collections::{BTreeMap, BTreeSet};

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

#[derive(Clone, Copy)]
struct CaptureAccess {
    offset: u32,
    mode: dbyte_ast::CaptureMode,
    ty: TypeId,
}

fn lower_global(
    program: &ConcreteHirProgram,
    global: &dbyte_hir::HirGlobal,
) -> Result<NativeMirGlobal, NativeMirError> {
    let layout = dbyte_hir::layout_of(&program.semantic, global.ty)
        .map_err(|diagnostic| NativeMirError::new(diagnostic.code, diagnostic.message))?;
    if layout.size == 0 {
        return Err(unsupported(global.span, "zero-sized native global"));
    }
    let bytes = if let Some(initializer) = &global.initializer {
        static_value_bytes(program, initializer, global.ty)?
    } else {
        Vec::new()
    };
    Ok(NativeMirGlobal {
        symbol: global.name.clone(),
        bytes,
        size: layout.size,
        alignment: layout.align,
        mutable: global.mutable,
    })
}

fn static_value_bytes(
    program: &ConcreteHirProgram,
    expression: &HirExpr,
    ty: TypeId,
) -> Result<Vec<u8>, NativeMirError> {
    let layout = dbyte_hir::layout_of(&program.semantic, ty)
        .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
    match (program.semantic.type_of(ty), &expression.kind) {
        (CanonicalType::Array { element, length }, HirExprKind::List(values)) => {
            if values.len() != *length as usize {
                return Err(unsupported(
                    expression.span,
                    "malformed static array initializer",
                ));
            }
            let element_layout = dbyte_hir::layout_of(&program.semantic, *element)
                .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
            let stride = element_layout
                .size
                .checked_add(element_layout.align - 1)
                .map(|value| value & !(element_layout.align - 1))
                .ok_or_else(|| unsupported(expression.span, "static array stride overflow"))?;
            let mut bytes = vec![0; layout.size as usize];
            for (index, value) in values.iter().enumerate() {
                let encoded = static_value_bytes(program, value, *element)?;
                let offset = usize::try_from(stride)
                    .ok()
                    .and_then(|stride| stride.checked_mul(index))
                    .ok_or_else(|| unsupported(expression.span, "static array offset overflow"))?;
                let end = offset
                    .checked_add(encoded.len())
                    .ok_or_else(|| unsupported(expression.span, "static array range overflow"))?;
                let destination = bytes.get_mut(offset..end).ok_or_else(|| {
                    unsupported(expression.span, "static array initializer exceeds layout")
                })?;
                destination.copy_from_slice(&encoded);
            }
            Ok(bytes)
        }
        (_, HirExprKind::Struct { fields, .. }) => {
            let mut bytes = vec![0; layout.size as usize];
            for (field, value) in fields {
                let (field_ty, offset) =
                    dbyte_hir::struct_field_layout(&program.semantic, ty, *field)
                        .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
                let encoded = static_value_bytes(program, value, field_ty)?;
                let offset = usize::try_from(offset)
                    .map_err(|_| unsupported(value.span, "static struct offset exceeds usize"))?;
                let end = offset
                    .checked_add(encoded.len())
                    .ok_or_else(|| unsupported(value.span, "static struct range overflow"))?;
                let destination = bytes.get_mut(offset..end).ok_or_else(|| {
                    unsupported(value.span, "static struct initializer exceeds layout")
                })?;
                destination.copy_from_slice(&encoded);
            }
            Ok(bytes)
        }
        (
            _,
            HirExprKind::Enum {
                variant,
                discriminant,
                payload,
                ..
            },
        ) => {
            if layout.size < 4 {
                return Err(unsupported(
                    expression.span,
                    "static enum layout is smaller than tag",
                ));
            }
            let mut bytes = vec![0; layout.size as usize];
            bytes[..4].copy_from_slice(&discriminant.to_le_bytes());
            let (_, _, fields) = dbyte_hir::enum_variant_layout(&program.semantic, ty, *variant)
                .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
            if fields.len() != payload.len() {
                return Err(unsupported(expression.span, "static enum payload arity"));
            }
            for (value, (payload_ty, offset)) in payload.iter().zip(fields) {
                let encoded = static_value_bytes(program, value, payload_ty)?;
                let offset = usize::try_from(offset)
                    .map_err(|_| unsupported(value.span, "static enum offset exceeds usize"))?;
                let end = offset
                    .checked_add(encoded.len())
                    .ok_or_else(|| unsupported(value.span, "static enum range overflow"))?;
                let destination = bytes.get_mut(offset..end).ok_or_else(|| {
                    unsupported(value.span, "static enum initializer exceeds layout")
                })?;
                destination.copy_from_slice(&encoded);
            }
            Ok(bytes)
        }
        (_, HirExprKind::Constant { value, .. }) => static_value_bytes(program, value, ty),
        _ if layout.size <= 8 => {
            let bits = static_scalar_bits(expression)?;
            Ok(bits.to_le_bytes()[..layout.size as usize].to_vec())
        }
        _ => Err(unsupported(expression.span, "aggregate static initializer")),
    }
}

fn static_scalar_bits(expression: &HirExpr) -> Result<u64, NativeMirError> {
    match &expression.kind {
        HirExprKind::Integer(value) => Ok(*value as u64),
        HirExprKind::Bool(value) => Ok(u64::from(*value)),
        HirExprKind::Null | HirExprKind::Unit => Ok(0),
        HirExprKind::Cast { value, .. }
        | HirExprKind::Bitcast { value, .. }
        | HirExprKind::Constant { value, .. } => static_scalar_bits(value),
        _ => Err(unsupported(
            expression.span,
            "non-scalar static initializer",
        )),
    }
}

fn collect_strings_block(block: &HirBlock, output: &mut Vec<Vec<u8>>) {
    for statement in &block.statements {
        match statement {
            HirStatement::Let { value, .. } | HirStatement::Expression(value) => {
                collect_strings_expr(value, output)
            }
            HirStatement::Assign {
                value,
                cleanup_before_commit,
                ..
            } => {
                collect_strings_expr(value, output);
                if let Some(cleanup) = cleanup_before_commit {
                    collect_strings_block(cleanup, output);
                }
            }
            HirStatement::Return { value, cleanup, .. } => {
                if let Some(value) = value {
                    collect_strings_expr(value, output);
                }
                for cleanup in cleanup {
                    collect_strings_block(cleanup, output);
                }
            }
            HirStatement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                collect_strings_expr(condition, output);
                collect_strings_block(then_block, output);
                if let Some(else_block) = else_block {
                    collect_strings_block(else_block, output);
                }
            }
            HirStatement::While {
                condition, body, ..
            } => {
                collect_strings_expr(condition, output);
                collect_strings_block(body, output);
            }
            HirStatement::For { iterable, body, .. } => {
                collect_strings_expr(iterable, output);
                collect_strings_block(body, output);
            }
            HirStatement::Match { value, arms, .. } => {
                collect_strings_expr(value, output);
                for arm in arms {
                    collect_strings_block(&arm.body, output);
                }
            }
            HirStatement::Defer { body, .. } => collect_strings_block(body, output),
            HirStatement::Break { cleanup, .. } | HirStatement::Continue { cleanup, .. } => {
                for cleanup in cleanup {
                    collect_strings_block(cleanup, output);
                }
            }
        }
    }
}

fn collect_strings_expr(expression: &HirExpr, output: &mut Vec<Vec<u8>>) {
    match &expression.kind {
        HirExprKind::String(bytes) => {
            if !output.contains(bytes) {
                output.push(bytes.clone());
            }
        }
        HirExprKind::Length { value }
        | HirExprKind::Unary { value, .. }
        | HirExprKind::Cast { value, .. }
        | HirExprKind::Bitcast { value, .. }
        | HirExprKind::Constant { value, .. }
        | HirExprKind::Try { value, .. } => collect_strings_expr(value, output),
        HirExprKind::Binary { left, right, .. } => {
            collect_strings_expr(left, output);
            collect_strings_expr(right, output);
        }
        HirExprKind::DirectCall { arguments, .. }
        | HirExprKind::IntrinsicCall { arguments, .. }
        | HirExprKind::HostCall { arguments, .. } => {
            for argument in arguments {
                collect_strings_expr(argument, output);
            }
        }
        HirExprKind::IndirectCall {
            callee, arguments, ..
        } => {
            collect_strings_expr(callee, output);
            for argument in arguments {
                collect_strings_expr(argument, output);
            }
        }
        HirExprKind::FormatString(parts) => {
            for part in parts {
                if let dbyte_hir::HirFormatPart::Value(value) = part {
                    collect_strings_expr(value, output);
                }
            }
        }
        HirExprKind::List(values)
        | HirExprKind::Enum {
            payload: values, ..
        } => {
            for value in values {
                collect_strings_expr(value, output);
            }
        }
        HirExprKind::Struct { fields, .. } => {
            for (_, value) in fields {
                collect_strings_expr(value, output);
            }
        }
        HirExprKind::Place(place) | HirExprKind::Borrow { place, .. } => {
            if let HirPlaceBase::Pointer(pointer) = &place.base {
                collect_strings_expr(pointer, output);
            }
            for projection in &place.projections {
                if let dbyte_hir::HirProjection::Index { index, .. } = projection {
                    collect_strings_expr(index, output);
                }
            }
        }
        HirExprKind::Drop { cleanup, .. } | HirExprKind::Closure { body: cleanup, .. } => {
            collect_strings_block(cleanup, output)
        }
        HirExprKind::Unit
        | HirExprKind::Integer(_)
        | HirExprKind::LayoutOf { .. }
        | HirExprKind::Float(_)
        | HirExprKind::Bool(_)
        | HirExprKind::Null
        | HirExprKind::Local(_)
        | HirExprKind::Global(_)
        | HirExprKind::Function(_)
        | HirExprKind::InterruptAddress { .. }
        | HirExprKind::TaskAddress { .. }
        | HirExprKind::Bytes(_) => {}
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
                if let Some(other) = else_block {
                    collect_closures_block(other, parent, output);
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
                if let dbyte_hir::HirProjection::Index { index, .. } = projection {
                    collect_closures_expr(index, parent, output);
                }
            }
        }
        _ => {}
    }
}

fn collect_address_taken_block(block: &HirBlock, output: &mut BTreeSet<LocalId>) {
    for statement in &block.statements {
        match statement {
            HirStatement::Let { value, .. } | HirStatement::Expression(value) => {
                collect_address_taken_expr(value, output);
            }
            HirStatement::Assign {
                place,
                value,
                cleanup_before_commit,
                ..
            } => {
                collect_address_taken_place(place, !place.projections.is_empty(), output);
                collect_address_taken_expr(value, output);
                if let Some(cleanup) = cleanup_before_commit {
                    collect_address_taken_block(cleanup, output);
                }
            }
            HirStatement::Return { value, cleanup, .. } => {
                if let Some(value) = value {
                    collect_address_taken_expr(value, output);
                }
                for cleanup in cleanup {
                    collect_address_taken_block(cleanup, output);
                }
            }
            HirStatement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                collect_address_taken_expr(condition, output);
                collect_address_taken_block(then_block, output);
                if let Some(else_block) = else_block {
                    collect_address_taken_block(else_block, output);
                }
            }
            HirStatement::While {
                condition, body, ..
            } => {
                collect_address_taken_expr(condition, output);
                collect_address_taken_block(body, output);
            }
            HirStatement::For { iterable, body, .. } => {
                collect_address_taken_expr(iterable, output);
                collect_address_taken_block(body, output);
            }
            HirStatement::Match { value, arms, .. } => {
                collect_address_taken_expr(value, output);
                for arm in arms {
                    collect_address_taken_block(&arm.body, output);
                }
            }
            HirStatement::Defer { body, .. } => collect_address_taken_block(body, output),
            HirStatement::Break { cleanup, .. } | HirStatement::Continue { cleanup, .. } => {
                for cleanup in cleanup {
                    collect_address_taken_block(cleanup, output);
                }
            }
        }
    }
}

fn collect_address_taken_place(
    place: &dbyte_hir::HirPlace,
    force_address: bool,
    output: &mut BTreeSet<LocalId>,
) {
    match &place.base {
        HirPlaceBase::Local(local) if force_address => {
            output.insert(*local);
        }
        HirPlaceBase::Pointer(pointer) => collect_address_taken_expr(pointer, output),
        HirPlaceBase::Local(_) | HirPlaceBase::Global(_) => {}
    }
    for projection in &place.projections {
        if let dbyte_hir::HirProjection::Index { index, .. } = projection {
            collect_address_taken_expr(index, output);
        }
    }
}

fn collect_address_taken_expr(expression: &HirExpr, output: &mut BTreeSet<LocalId>) {
    match &expression.kind {
        HirExprKind::Borrow { place, .. } => collect_address_taken_place(place, true, output),
        HirExprKind::Place(place) => {
            collect_address_taken_place(place, !place.projections.is_empty(), output);
        }
        HirExprKind::Length { value }
        | HirExprKind::Unary { value, .. }
        | HirExprKind::Cast { value, .. }
        | HirExprKind::Bitcast { value, .. }
        | HirExprKind::Constant { value, .. }
        | HirExprKind::Try { value, .. } => collect_address_taken_expr(value, output),
        HirExprKind::Binary { left, right, .. } => {
            collect_address_taken_expr(left, output);
            collect_address_taken_expr(right, output);
        }
        HirExprKind::DirectCall { arguments, .. }
        | HirExprKind::IntrinsicCall { arguments, .. }
        | HirExprKind::HostCall { arguments, .. } => {
            for argument in arguments {
                collect_address_taken_expr(argument, output);
            }
        }
        HirExprKind::IndirectCall {
            callee, arguments, ..
        } => {
            collect_address_taken_expr(callee, output);
            for argument in arguments {
                collect_address_taken_expr(argument, output);
            }
        }
        HirExprKind::FormatString(parts) => {
            for part in parts {
                if let dbyte_hir::HirFormatPart::Value(value) = part {
                    collect_address_taken_expr(value, output);
                }
            }
        }
        HirExprKind::List(values)
        | HirExprKind::Enum {
            payload: values, ..
        } => {
            for value in values {
                collect_address_taken_expr(value, output);
            }
        }
        HirExprKind::Struct { fields, .. } => {
            for (_, value) in fields {
                collect_address_taken_expr(value, output);
            }
        }
        HirExprKind::Drop { cleanup, .. } => collect_address_taken_block(cleanup, output),
        HirExprKind::Closure { captures, body, .. } => {
            for capture in captures {
                if capture.mode != dbyte_ast::CaptureMode::Move {
                    output.insert(capture.local);
                }
            }
            collect_address_taken_block(body, output);
        }
        HirExprKind::Unit
        | HirExprKind::Integer(_)
        | HirExprKind::Float(_)
        | HirExprKind::Bool(_)
        | HirExprKind::String(_)
        | HirExprKind::Bytes(_)
        | HirExprKind::Null
        | HirExprKind::Local(_)
        | HirExprKind::Global(_)
        | HirExprKind::Function(_)
        | HirExprKind::LayoutOf { .. }
        | HirExprKind::InterruptAddress { .. }
        | HirExprKind::TaskAddress { .. } => {}
    }
}

pub fn lower_program_to_native_mir(
    program: &ConcreteHirProgram,
) -> Result<NativeMirProject, NativeMirError> {
    lower_program_to_native_mir_with_link_names(program, None)
}

pub fn lower_program_to_native_mir_with_link_names(
    program: &ConcreteHirProgram,
    link_names: Option<&BTreeMap<dbyte_hir::SymbolId, String>>,
) -> Result<NativeMirProject, NativeMirError> {
    let symbols: BTreeMap<_, _> = program
        .functions
        .iter()
        .map(|function| {
            let symbol = if function.key.arguments.is_empty()
                && function.convention == dbyte_ast::CallConvention::C
            {
                link_names
                    .and_then(|names| names.get(&function.key.symbol))
                    .cloned()
                    .or_else(|| {
                        program
                            .semantic
                            .functions
                            .iter()
                            .find(|definition| definition.symbol == function.key.symbol)
                            .map(|definition| definition.name.clone())
                    })
                    .unwrap_or_else(|| function.symbol_name.clone())
            } else {
                function.symbol_name.clone()
            };
            (function.key.clone(), symbol)
        })
        .collect();
    let global_names: BTreeMap<_, _> = program
        .globals
        .iter()
        .map(|global| (global.symbol, global.name.clone()))
        .collect();
    let globals = program
        .globals
        .iter()
        .map(|global| lower_global(program, global))
        .collect::<Result<Vec<_>, _>>()?;
    let mut string_bytes = Vec::<Vec<u8>>::new();
    let mut closures = Vec::new();
    for function in &program.functions {
        collect_strings_block(&function.body, &mut string_bytes);
        collect_closures_block(&function.body, function, &mut closures);
    }
    closures.sort_by_key(|closure| closure.id);
    let closure_symbols = closures
        .iter()
        .map(|closure| (closure.id, format!("__dbyte_closure_{}", closure.id.0)))
        .collect::<BTreeMap<_, _>>();
    let string_names = string_bytes
        .iter()
        .enumerate()
        .map(|(index, bytes)| (bytes.clone(), format!("__dbyte_string_{index:04}")))
        .collect::<BTreeMap<_, _>>();
    let mut globals = globals;
    for (index, bytes) in string_bytes.into_iter().enumerate() {
        let empty = bytes.is_empty();
        let size = if empty {
            1
        } else {
            u32::try_from(bytes.len()).map_err(|_| {
                NativeMirError::new(
                    "E_NATIVE_MIR_STRING_SIZE",
                    "native string literal exceeds target object limits",
                )
            })?
        };
        globals.push(NativeMirGlobal {
            symbol: format!("__dbyte_string_{index:04}"),
            size,
            bytes: if empty { vec![0] } else { bytes },
            alignment: 1,
            mutable: false,
        });
    }
    let mut functions = Vec::new();
    for function in &program.functions {
        if function.declaration_only {
            continue;
        }
        functions.push(
            FunctionLowerer::new(
                program,
                function,
                &symbols,
                &closure_symbols,
                &global_names,
                &string_names,
            )?
            .lower()?,
        );
    }
    for closure in &closures {
        functions.push(FunctionLowerer::lower_closure(
            program,
            closure,
            &symbols,
            &closure_symbols,
            &global_names,
            &string_names,
        )?);
    }
    let project = NativeMirProject {
        pointer_bytes: program.semantic.layout.pointer_size as u8,
        globals,
        functions,
    };
    project.validate()?;
    Ok(project)
}

struct FunctionLowerer<'a> {
    program: &'a ConcreteHirProgram,
    function: ConcreteHirFunction,
    symbols: &'a BTreeMap<dbyte_hir::FunctionInstanceKey, String>,
    closure_symbols: &'a BTreeMap<dbyte_hir::ClosureId, String>,
    global_names: &'a BTreeMap<dbyte_hir::SymbolId, String>,
    string_names: &'a BTreeMap<Vec<u8>, String>,
    register_types: Vec<MachineType>,
    local_registers: BTreeMap<LocalId, VirtualRegister>,
    parameter_registers: BTreeMap<LocalId, VirtualRegister>,
    local_stack_objects: BTreeMap<LocalId, StackObjectId>,
    stack_objects: Vec<NativeStackObject>,
    blocks: Vec<NativeMirBlock>,
    current: BlockId,
    loops: Vec<(BlockId, BlockId)>,
    hidden_environment: Option<VirtualRegister>,
    aggregate_result_register: Option<VirtualRegister>,
    capture_accesses: BTreeMap<LocalId, CaptureAccess>,
}

impl<'a> FunctionLowerer<'a> {
    fn new(
        program: &'a ConcreteHirProgram,
        function: &ConcreteHirFunction,
        symbols: &'a BTreeMap<dbyte_hir::FunctionInstanceKey, String>,
        closure_symbols: &'a BTreeMap<dbyte_hir::ClosureId, String>,
        global_names: &'a BTreeMap<dbyte_hir::SymbolId, String>,
        string_names: &'a BTreeMap<Vec<u8>, String>,
    ) -> Result<Self, NativeMirError> {
        let mut address_taken = BTreeSet::new();
        collect_address_taken_block(&function.body, &mut address_taken);
        let mut register_types = Vec::new();
        let mut local_registers = BTreeMap::new();
        let mut parameter_registers = BTreeMap::new();
        let mut local_stack_objects = BTreeMap::new();
        let mut stack_objects = Vec::new();
        for local in &function.locals {
            let scalar = machine_type(program, local.ty, local.span).ok();
            if local.parameter && scalar.is_none() {
                // Aggregate source parameters are address values naming the
                // canonical by-value incoming stack copy. The selector owns
                // the physical ABI placement; MIR owns only its layout.
                let register = VirtualRegister(register_types.len() as u32);
                register_types.push(MachineType::Pointer);
                local_registers.insert(local.id, register);
                parameter_registers.insert(local.id, register);
            } else if address_taken.contains(&local.id) || scalar.is_none() {
                let layout = dbyte_hir::layout_of(&program.semantic, local.ty)
                    .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
                let object = StackObjectId(stack_objects.len() as u32);
                stack_objects.push(NativeStackObject {
                    id: object,
                    bytes: layout.size,
                    alignment: layout.align,
                    address_taken: address_taken.contains(&local.id),
                });
                local_stack_objects.insert(local.id, object);
                if local.parameter {
                    let ty = scalar.ok_or_else(|| unsupported(local.span, "parameter ABI"))?;
                    let register = VirtualRegister(register_types.len() as u32);
                    register_types.push(ty);
                    parameter_registers.insert(local.id, register);
                }
            } else {
                let scalar = scalar.ok_or_else(|| {
                    NativeMirError::new(
                        "E_NATIVE_MIR_LOCAL_TYPE",
                        "scalar local lost its canonical machine type",
                    )
                })?;
                let register = VirtualRegister(register_types.len() as u32);
                register_types.push(scalar);
                local_registers.insert(local.id, register);
                if local.parameter {
                    parameter_registers.insert(local.id, register);
                }
            }
        }
        let aggregate_result_register =
            if aggregate_layout(program, function.result, function.span)?.is_some() {
                let register = VirtualRegister(register_types.len() as u32);
                register_types.push(MachineType::Pointer);
                Some(register)
            } else {
                None
            };
        let mut lowerer = Self {
            program,
            function: function.clone(),
            symbols,
            closure_symbols,
            global_names,
            string_names,
            register_types,
            local_registers,
            parameter_registers,
            local_stack_objects,
            stack_objects,
            blocks: vec![NativeMirBlock {
                id: BlockId(0),
                instructions: Vec::new(),
                terminator: NativeTerminator::Unreachable,
            }],
            current: BlockId(0),
            loops: Vec::new(),
            hidden_environment: None,
            aggregate_result_register,
            capture_accesses: BTreeMap::new(),
        };
        for parameter in &function.parameters {
            if let Some(object) = lowerer.local_stack_objects.get(&parameter.id).copied() {
                let value = lowerer.parameter_registers[&parameter.id];
                let address = lowerer.stack_address(object, parameter.span);
                let ty = lowerer.register_types[value.0 as usize];
                lowerer.emit(
                    NativeMirInst::Store {
                        address,
                        value,
                        ty,
                        volatile: false,
                    },
                    parameter.span,
                );
            }
        }
        Ok(lowerer)
    }

    fn lower_closure(
        program: &'a ConcreteHirProgram,
        closure: &ClosureInfo,
        symbols: &'a BTreeMap<dbyte_hir::FunctionInstanceKey, String>,
        closure_symbols: &'a BTreeMap<dbyte_hir::ClosureId, String>,
        global_names: &'a BTreeMap<dbyte_hir::SymbolId, String>,
        string_names: &'a BTreeMap<Vec<u8>, String>,
    ) -> Result<NativeMirFunction, NativeMirError> {
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
        let mut lowerer = Self::new(
            program,
            &function,
            symbols,
            closure_symbols,
            global_names,
            string_names,
        )?;
        let environment = lowerer.new_register(MachineType::Pointer);
        lowerer.hidden_environment = Some(environment);
        lowerer.capture_accesses =
            closure_environment_layout(program, &closure.captures, closure.span)?.1;
        let mut lowered = lowerer.lower()?;
        lowered.symbol = closure_symbols[&closure.id].clone();
        Ok(lowered)
    }

    fn lower(mut self) -> Result<NativeMirFunction, NativeMirError> {
        if let Some(class) = self.function.interrupt {
            self.emit(
                NativeMirInst::InterruptDefinition { class },
                self.function.span,
            );
        }
        let body = self.function.body.clone();
        self.lower_block(&body)?;
        if !self.current_is_terminated() {
            if matches!(
                self.program.semantic.type_of(self.function.result),
                CanonicalType::Unit
            ) {
                self.terminate(NativeTerminator::Return(None))?;
            } else if aggregate_layout(self.program, self.function.result, self.function.span)?
                .is_some()
            {
                return Err(unsupported(
                    self.function.span,
                    &format!(
                        "implicit aggregate return without a value in function '{}'",
                        self.function.symbol_name
                    ),
                ));
            } else {
                let result = machine_type(self.program, self.function.result, self.function.span)?;
                let zero = self.new_register(result);
                self.emit(
                    NativeMirInst::Const {
                        destination: zero,
                        value: 0,
                        ty: result,
                    },
                    self.function.span,
                );
                self.terminate(NativeTerminator::Return(Some(zero)))?;
            }
        }
        let mut parameters = self
            .function
            .parameters
            .iter()
            .map(|parameter| {
                self.parameter_registers
                    .get(&parameter.id)
                    .copied()
                    .ok_or_else(|| {
                        NativeMirError::new(
                            "E_NATIVE_MIR_PARAMETER",
                            "resolved HIR parameter has no incoming virtual register",
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut parameter_aggregates = self
            .function
            .parameters
            .iter()
            .map(|parameter| aggregate_layout(self.program, parameter.ty, parameter.span))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(environment) = self.hidden_environment {
            parameters.insert(0, environment);
            parameter_aggregates.insert(0, None);
        }
        let (result, aggregate_result) = match self.program.semantic.type_of(self.function.result) {
            CanonicalType::Unit => (None, None),
            _ => match machine_type(self.program, self.function.result, self.function.span) {
                Ok(result) => (Some(result), None),
                Err(_) => (
                    None,
                    aggregate_layout(self.program, self.function.result, self.function.span)?,
                ),
            },
        };
        let symbol = self
            .symbols
            .get(&self.function.key)
            .cloned()
            .ok_or_else(|| {
                NativeMirError::new(
                    "E_NATIVE_MIR_SYMBOL",
                    "concrete HIR function has no resolved machine symbol",
                )
            })?;
        Ok(NativeMirFunction {
            instance: self.function.instance,
            symbol,
            convention: self.function.convention,
            parameters,
            parameter_aggregates,
            result,
            aggregate_result,
            aggregate_result_register: self.aggregate_result_register,
            register_types: self.register_types,
            stack_objects: self.stack_objects,
            entry: BlockId(0),
            blocks: self.blocks,
        })
    }

    fn lower_block(&mut self, block: &HirBlock) -> Result<(), NativeMirError> {
        for statement in &block.statements {
            if self.current_is_terminated() {
                break;
            }
            self.lower_statement(statement)?;
        }
        Ok(())
    }

    fn lower_statement(&mut self, statement: &HirStatement) -> Result<(), NativeMirError> {
        match statement {
            HirStatement::Let { local, value, span } => {
                if let Some(object) = self.local_stack_objects.get(local).copied() {
                    if matches!(
                        &value.kind,
                        HirExprKind::DirectCall { .. } | HirExprKind::IndirectCall { .. }
                    ) {
                        let destination = self.stack_address(object, *span);
                        self.lower_aggregate_call_into(value, destination)?;
                        return Ok(());
                    }
                    if matches!(value.kind, HirExprKind::Struct { .. }) {
                        return self.lower_struct_initializer(*local, value, *span);
                    }
                }
                let value = self.lower_expr(value)?;
                self.assign_local(*local, value, *span)
            }
            HirStatement::Assign {
                place,
                value,
                cleanup_before_commit,
                span,
            } if cleanup_before_commit.is_none() => {
                if place.projections.is_empty() {
                    if let HirPlaceBase::Local(local) = &place.base {
                        let value = self.lower_expr(value)?;
                        return self.assign_local(*local, value, *span);
                    }
                }
                let address = self.lower_place_address(place)?;
                let value = self.lower_expr(value)?;
                if is_zero_sized(self.program, place.ty)? {
                    return Ok(());
                }
                if let Some(layout) = aggregate_layout(self.program, place.ty, place.span)? {
                    self.emit(
                        NativeMirInst::AggregateCopy {
                            destination: address,
                            source: value,
                            bytes: layout.bytes,
                            alignment: layout.alignment,
                        },
                        *span,
                    );
                    return Ok(());
                }
                let ty = machine_type(self.program, place.ty, place.span)?;
                self.emit(
                    NativeMirInst::Store {
                        address,
                        value,
                        ty,
                        volatile: false,
                    },
                    *span,
                );
                Ok(())
            }
            HirStatement::Expression(expression) => {
                self.lower_expr(expression)?;
                Ok(())
            }
            HirStatement::Return {
                value,
                cleanup,
                span,
            } => {
                let aggregate = aggregate_layout(self.program, self.function.result, *span)?;
                let value = if let Some(value) = value.as_ref() {
                    if aggregate.is_some() {
                        let destination = self.aggregate_result_register.ok_or_else(|| {
                            NativeMirError::new(
                                "E_NATIVE_MIR_ABI",
                                "aggregate return has no hidden result register",
                            )
                        })?;
                        if let Some(direct) =
                            self.lower_direct_aggregate_expr_into(value, destination)?
                        {
                            Some(direct)
                        } else {
                            Some(self.lower_expr(value)?)
                        }
                    } else {
                        Some(self.lower_expr(value)?)
                    }
                } else {
                    None
                };
                for cleanup in cleanup {
                    self.lower_block(cleanup)?;
                }
                if matches!(
                    self.program.semantic.type_of(self.function.result),
                    CanonicalType::Unit
                ) {
                    return self.terminate(NativeTerminator::Return(None));
                }
                match (value, aggregate) {
                    (Some(source), Some(layout)) => {
                        self.terminate(NativeTerminator::ReturnAggregate { source, layout })
                    }
                    (value, None) => self.terminate(NativeTerminator::Return(value)),
                    (None, Some(_)) => Err(unsupported(*span, "missing aggregate return value")),
                }
            }
            HirStatement::If {
                condition,
                then_block,
                else_block,
                ..
            } => self.lower_if(condition, then_block, else_block.as_ref()),
            HirStatement::While {
                condition, body, ..
            } => self.lower_while(condition, body),
            HirStatement::Break { cleanup, span } => {
                for cleanup in cleanup {
                    self.lower_block(cleanup)?;
                }
                let (_, done) = self
                    .loops
                    .last()
                    .copied()
                    .ok_or_else(|| unsupported(*span, "break outside a scalar MIR loop"))?;
                self.terminate(NativeTerminator::Jump(done))
            }
            HirStatement::Continue { cleanup, span } => {
                for cleanup in cleanup {
                    self.lower_block(cleanup)?;
                }
                let (head, _) = self
                    .loops
                    .last()
                    .copied()
                    .ok_or_else(|| unsupported(*span, "continue outside a scalar MIR loop"))?;
                self.terminate(NativeTerminator::Jump(head))
            }
            HirStatement::Defer { .. } => Ok(()),
            HirStatement::Match {
                mode,
                value,
                arms,
                exhaustive,
                span,
            } => self.lower_match(*mode, value, arms, *exhaustive, *span),
            HirStatement::Assign { span, .. } | HirStatement::For { span, .. } => {
                Err(unsupported(*span, "aggregate/place control flow"))
            }
        }
    }

    fn lower_match(
        &mut self,
        mode: dbyte_ast::MatchMode,
        value: &HirExpr,
        arms: &[dbyte_hir::HirMatchArm],
        exhaustive: bool,
        span: Span,
    ) -> Result<(), NativeMirError> {
        if !exhaustive {
            return Err(unsupported(span, "non-exhaustive canonical match"));
        }
        let enum_address = self.lower_expr(value)?;
        let tag = self.new_register(MachineType::I32);
        self.emit(
            NativeMirInst::Load {
                destination: tag,
                address: enum_address,
                ty: MachineType::I32,
                volatile: false,
            },
            span,
        );
        let done = self.new_block();
        let arm_blocks = (0..arms.len())
            .map(|_| self.new_block())
            .collect::<Vec<_>>();
        let mut check = self.current;
        let mut default_bound = false;
        for (index, arm) in arms.iter().enumerate() {
            self.current = check;
            if let Some(variant) = arm.variant {
                let next = self.new_block();
                let expected = self.new_register(MachineType::I32);
                self.emit(
                    NativeMirInst::Const {
                        destination: expected,
                        value: u64::from(variant.discriminant),
                        ty: MachineType::I32,
                    },
                    arm.span,
                );
                let matches = self.new_register(MachineType::Bool);
                self.emit(
                    NativeMirInst::Compare {
                        destination: matches,
                        op: NativeCompareOp::Equal,
                        left: NativeValue::Register(tag),
                        right: NativeValue::Register(expected),
                        ty: MachineType::I32,
                    },
                    arm.span,
                );
                self.terminate(NativeTerminator::Branch {
                    condition: NativeValue::Register(matches),
                    then_block: arm_blocks[index],
                    else_block: next,
                })?;
                check = next;
            } else {
                self.terminate(NativeTerminator::Jump(arm_blocks[index]))?;
                default_bound = true;
                break;
            }
        }
        if !default_bound {
            self.current = check;
            self.terminate(NativeTerminator::Trap)?;
        }
        let mut all_terminal = true;
        for (arm, block) in arms.iter().zip(arm_blocks.iter().copied()) {
            self.current = block;
            if let Some(variant) = arm.variant {
                let (_, _, payload) = dbyte_hir::enum_variant_layout(
                    &self.program.semantic,
                    value.ty,
                    variant.variant,
                )
                .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
                if payload.len() != arm.bindings.len() {
                    return Err(unsupported(arm.span, "match payload binding arity"));
                }
                for (binding, (payload_ty, offset)) in arm.bindings.iter().zip(payload) {
                    let address = self.offset_address(enum_address, offset, arm.span)?;
                    let incoming = match mode {
                        dbyte_ast::MatchMode::Borrow | dbyte_ast::MatchMode::BorrowMut => address,
                        dbyte_ast::MatchMode::Move => {
                            if is_zero_sized(self.program, payload_ty)? {
                                self.unit_value(arm.span)
                            } else if aggregate_layout(self.program, payload_ty, arm.span)?
                                .is_some()
                            {
                                address
                            } else {
                                let value = self.new_register(machine_type(
                                    self.program,
                                    payload_ty,
                                    arm.span,
                                )?);
                                self.emit(
                                    NativeMirInst::Load {
                                        destination: value,
                                        address,
                                        ty: machine_type(self.program, payload_ty, arm.span)?,
                                        volatile: false,
                                    },
                                    arm.span,
                                );
                                value
                            }
                        }
                    };
                    self.assign_local(binding.id, incoming, binding.span)?;
                }
            }
            self.lower_block(&arm.body)?;
            if !self.current_is_terminated() {
                all_terminal = false;
                self.terminate(NativeTerminator::Jump(done))?;
            }
        }
        self.current = done;
        if all_terminal {
            // Exhaustive terminal arms leave the join unreachable. Marking it
            // as a trap preserves fail-closed invalid control flow while also
            // preventing an invented aggregate fallthrough return.
            self.terminate(NativeTerminator::Trap)?;
        }
        Ok(())
    }

    fn lower_if(
        &mut self,
        condition: &HirExpr,
        then_body: &HirBlock,
        else_body: Option<&HirBlock>,
    ) -> Result<(), NativeMirError> {
        let condition = self.lower_expr(condition)?;
        let then_block = self.new_block();
        let else_block = self.new_block();
        let done = self.new_block();
        self.terminate(NativeTerminator::Branch {
            condition: NativeValue::Register(condition),
            then_block,
            else_block,
        })?;

        self.current = then_block;
        self.lower_block(then_body)?;
        if !self.current_is_terminated() {
            self.terminate(NativeTerminator::Jump(done))?;
        }
        self.current = else_block;
        if let Some(else_body) = else_body {
            self.lower_block(else_body)?;
        }
        if !self.current_is_terminated() {
            self.terminate(NativeTerminator::Jump(done))?;
        }
        self.current = done;
        Ok(())
    }

    fn lower_while(&mut self, condition: &HirExpr, body: &HirBlock) -> Result<(), NativeMirError> {
        let head = self.new_block();
        let body_block = self.new_block();
        let done = self.new_block();
        self.terminate(NativeTerminator::Jump(head))?;
        self.current = head;
        let condition = self.lower_expr(condition)?;
        self.terminate(NativeTerminator::Branch {
            condition: NativeValue::Register(condition),
            then_block: body_block,
            else_block: done,
        })?;
        self.current = body_block;
        self.loops.push((head, done));
        self.lower_block(body)?;
        self.loops.pop();
        if !self.current_is_terminated() {
            self.terminate(NativeTerminator::Jump(head))?;
        }
        self.current = done;
        Ok(())
    }

    fn lower_expr(&mut self, expression: &HirExpr) -> Result<VirtualRegister, NativeMirError> {
        if aggregate_layout(self.program, expression.ty, expression.span)?.is_some() {
            return self.lower_aggregate_expr(expression);
        }
        let ty = machine_type(self.program, expression.ty, expression.span)?;
        match &expression.kind {
            HirExprKind::Integer(value) => {
                let destination = self.new_register(ty);
                self.emit(
                    NativeMirInst::Const {
                        destination,
                        value: *value as u64,
                        ty,
                    },
                    expression.span,
                );
                Ok(destination)
            }
            HirExprKind::Bool(value) => {
                let destination = self.new_register(MachineType::Bool);
                self.emit(
                    NativeMirInst::Const {
                        destination,
                        value: u64::from(*value),
                        ty: MachineType::Bool,
                    },
                    expression.span,
                );
                Ok(destination)
            }
            HirExprKind::String(bytes) => {
                let symbol = self.string_names.get(bytes).cloned().ok_or_else(|| {
                    NativeMirError::new(
                        "E_NATIVE_MIR_STRING_SYMBOL",
                        "canonical string literal has no pooled native symbol",
                    )
                })?;
                Ok(self.symbol_address(symbol, expression.span))
            }
            HirExprKind::Null | HirExprKind::Unit => {
                let destination = self.new_register(ty);
                self.emit(
                    NativeMirInst::Const {
                        destination,
                        value: 0,
                        ty,
                    },
                    expression.span,
                );
                Ok(destination)
            }
            HirExprKind::Function(key) => {
                let symbol = self.symbols.get(key).cloned().ok_or_else(|| {
                    NativeMirError::new(
                        "E_NATIVE_MIR_FUNCTION_ADDRESS",
                        "resolved function value has no native link symbol",
                    )
                })?;
                Ok(self.symbol_address(symbol, expression.span))
            }
            HirExprKind::Local(local) => self.lower_local_value(*local, expression.span),
            HirExprKind::Global(symbol) => {
                let address = self.global_address(*symbol, expression.span)?;
                let destination = self.new_register(ty);
                self.emit(
                    NativeMirInst::Load {
                        destination,
                        address,
                        ty,
                        volatile: false,
                    },
                    expression.span,
                );
                Ok(destination)
            }
            HirExprKind::Constant { value, .. } => self.lower_expr(value),
            HirExprKind::Binary { left, op, right } => {
                let signed = is_signed(self.program, left.ty);
                let left = self.lower_expr(left)?;
                let right = self.lower_expr(right)?;
                let destination = self.new_register(ty);
                if let Some(op) = compare_operation(op.clone(), signed) {
                    self.emit(
                        NativeMirInst::Compare {
                            destination,
                            op,
                            left: NativeValue::Register(left),
                            right: NativeValue::Register(right),
                            ty: self.register_types[left.0 as usize],
                        },
                        expression.span,
                    );
                } else {
                    self.emit(
                        NativeMirInst::Binary {
                            destination,
                            op: binary_operation(op.clone(), signed)?,
                            left: NativeValue::Register(left),
                            right: NativeValue::Register(right),
                            ty,
                        },
                        expression.span,
                    );
                }
                Ok(destination)
            }
            HirExprKind::Unary { op, value } => {
                let value = self.lower_expr(value)?;
                let destination = self.new_register(ty);
                let op = match op {
                    UnaryOp::Neg => NativeUnaryOp::Negate,
                    UnaryOp::Not => NativeUnaryOp::BoolNot,
                    UnaryOp::BitNot => NativeUnaryOp::BitNot,
                    UnaryOp::AddressOf | UnaryOp::Deref => {
                        return Err(unsupported(expression.span, "raw unary address operation"));
                    }
                };
                self.emit(
                    NativeMirInst::Unary {
                        destination,
                        op,
                        value: NativeValue::Register(value),
                        ty,
                    },
                    expression.span,
                );
                Ok(destination)
            }
            HirExprKind::Cast { value, .. } => {
                // An explicit integer-literal cast is target-typed before machine
                // lowering. Do not first normalize the literal through its default
                // source integer width: that would truncate values above u32 on i686.
                if ty == MachineType::I64
                    || (ty == MachineType::Pointer
                        && self.program.semantic.layout.pointer_size == 8)
                {
                    if let HirExprKind::Integer(bits) = value.kind {
                        let target_is_valid_wide_integer = matches!(
                            self.program.semantic.type_of(expression.ty),
                            CanonicalType::Integer(IntegerType::I64)
                        ) || matches!(
                            self.program.semantic.type_of(expression.ty),
                            CanonicalType::Integer(IntegerType::U64) if bits >= 0
                        ) || matches!(
                            self.program.semantic.type_of(expression.ty),
                            CanonicalType::Integer(IntegerType::Usize) if bits >= 0
                        );
                        if target_is_valid_wide_integer {
                            let destination = self.new_register(ty);
                            self.emit(
                                NativeMirInst::Const {
                                    destination,
                                    value: bits as u64,
                                    ty,
                                },
                                expression.span,
                            );
                            return Ok(destination);
                        }
                    }
                }
                let value_register = self.lower_expr(value)?;
                let from = self.register_types[value_register.0 as usize];
                let destination = self.new_register(ty);
                self.emit(
                    NativeMirInst::Cast {
                        destination,
                        value: value_register,
                        from,
                        to: ty,
                        signed: is_signed(self.program, value.ty),
                    },
                    expression.span,
                );
                Ok(destination)
            }
            HirExprKind::Bitcast { value, .. } => {
                let source = self.lower_expr(value)?;
                let source_ty = self.register_types[source.0 as usize];
                let pointer_bytes = self.program.semantic.layout.pointer_size as u8;
                if source_ty.bytes(pointer_bytes) != ty.bytes(pointer_bytes) {
                    return Err(NativeMirError::new(
                        "E_NATIVE_MIR_BITCAST_WIDTH",
                        "canonical exact bitcast reached native MIR with unequal widths",
                    ));
                }
                let destination = self.new_register(ty);
                self.emit(
                    NativeMirInst::Copy {
                        destination,
                        source,
                        ty,
                    },
                    expression.span,
                );
                Ok(destination)
            }
            HirExprKind::Length { value } => {
                let descriptor = self.lower_expr(value)?;
                if !matches!(
                    self.program.semantic.type_of(value.ty),
                    CanonicalType::Slice { .. } | CanonicalType::StringView
                ) {
                    return Err(unsupported(expression.span, "length of non-view value"));
                }
                let address = self.offset_address(
                    descriptor,
                    self.program.semantic.layout.pointer_size,
                    expression.span,
                )?;
                let destination = self.new_register(MachineType::Pointer);
                self.emit(
                    NativeMirInst::Load {
                        destination,
                        address,
                        ty: MachineType::Pointer,
                        volatile: false,
                    },
                    expression.span,
                );
                Ok(destination)
            }
            HirExprKind::DirectCall {
                callee,
                convention,
                arguments,
            } => {
                let symbol = self.symbols.get(callee).cloned().ok_or_else(|| {
                    NativeMirError::new(
                        "E_NATIVE_MIR_CALL",
                        "resolved HIR call has no native link symbol",
                    )
                })?;
                let (arguments, argument_aggregates) =
                    self.lower_call_arguments(arguments, *convention)?;
                let unit = matches!(
                    self.program.semantic.type_of(expression.ty),
                    CanonicalType::Unit
                );
                let destination = (!unit).then(|| self.new_register(ty));
                self.emit(
                    NativeMirInst::Call {
                        destination,
                        callee: NativeCallee::Direct { symbol },
                        arguments,
                        argument_aggregates,
                        convention: *convention,
                        result: (!unit).then_some(ty),
                        aggregate_result: None,
                    },
                    expression.span,
                );
                if let Some(destination) = destination {
                    Ok(destination)
                } else {
                    Ok(self.unit_value(expression.span))
                }
            }
            HirExprKind::IndirectCall {
                callee,
                convention,
                arguments,
            } => {
                let callee = self.lower_native_callee(callee)?;
                let (arguments, argument_aggregates) =
                    self.lower_call_arguments(arguments, *convention)?;
                let unit = matches!(
                    self.program.semantic.type_of(expression.ty),
                    CanonicalType::Unit
                );
                let destination = (!unit).then(|| self.new_register(ty));
                self.emit(
                    NativeMirInst::Call {
                        destination,
                        callee,
                        arguments,
                        argument_aggregates,
                        convention: *convention,
                        result: (!unit).then_some(ty),
                        aggregate_result: None,
                    },
                    expression.span,
                );
                if let Some(destination) = destination {
                    Ok(destination)
                } else {
                    Ok(self.unit_value(expression.span))
                }
            }
            HirExprKind::IntrinsicCall {
                intrinsic,
                arguments,
            } => {
                let linker_symbol = match intrinsic {
                    dbyte_hir::HirIntrinsic::KernelImageStart => Some("__dbyte_image_start"),
                    dbyte_hir::HirIntrinsic::KernelImageEnd => Some("__dbyte_image_end"),
                    dbyte_hir::HirIntrinsic::BootstrapStackStart => Some("__dbyte_stack_bottom"),
                    dbyte_hir::HirIntrinsic::BootstrapStackEnd => Some("__dbyte_stack_top"),
                    dbyte_hir::HirIntrinsic::TaskTrampolineAddress => {
                        Some("__dbyte_task_trampoline")
                    }
                    _ => None,
                };
                if let Some(symbol) = linker_symbol {
                    if !arguments.is_empty() {
                        return Err(unsupported(
                            expression.span,
                            "linker address intrinsic retained arguments",
                        ));
                    }
                    return Ok(self.symbol_address(symbol.to_owned(), expression.span));
                }
                let arguments = arguments
                    .iter()
                    .map(|argument| self.lower_expr(argument))
                    .collect::<Result<Vec<_>, _>>()?;
                let unit = matches!(
                    self.program.semantic.type_of(expression.ty),
                    CanonicalType::Unit
                );
                let destination = (!unit).then(|| self.new_register(ty));
                self.emit(
                    NativeMirInst::Intrinsic {
                        destination,
                        intrinsic: *intrinsic,
                        arguments,
                        result: (!unit).then_some(ty),
                        volatile: true,
                    },
                    expression.span,
                );
                if let Some(destination) = destination {
                    Ok(destination)
                } else {
                    Ok(self.unit_value(expression.span))
                }
            }
            HirExprKind::InterruptAddress {
                vector,
                handler,
                class,
            } => {
                let symbol = self.symbols.get(handler).cloned().ok_or_else(|| {
                    NativeMirError::new(
                        "E_NATIVE_MIR_INTERRUPT_ADDRESS",
                        "resolved interrupt handler has no native link symbol",
                    )
                })?;
                let destination = self.new_register(MachineType::Pointer);
                self.emit(
                    NativeMirInst::InterruptAddress {
                        destination,
                        vector: *vector,
                        symbol,
                        class: *class,
                    },
                    expression.span,
                );
                Ok(destination)
            }
            HirExprKind::TaskAddress { function } => {
                let symbol = self.symbols.get(function).cloned().ok_or_else(|| {
                    NativeMirError::new(
                        "E_NATIVE_MIR_TASK_ADDRESS",
                        "resolved task function has no native link symbol",
                    )
                })?;
                let destination = self.new_register(MachineType::Pointer);
                self.emit(
                    NativeMirInst::TaskAddress {
                        destination,
                        symbol,
                    },
                    expression.span,
                );
                Ok(destination)
            }
            HirExprKind::Place(place) => self.lower_place_value(place),
            HirExprKind::Borrow { place, .. } => self.lower_place_address(place),
            HirExprKind::Drop { cleanup, .. } => {
                self.lower_block(cleanup)?;
                Ok(self.unit_value(expression.span))
            }
            HirExprKind::Try {
                value,
                ok_variant,
                err_variant,
                cleanup,
            } => self.lower_try_expr(value, *ok_variant, *err_variant, cleanup, expression),
            _ => Err(unsupported(expression.span, "non-scalar HIR expression")),
        }
    }

    fn lower_try_expr(
        &mut self,
        value: &HirExpr,
        ok_variant: dbyte_hir::VariantId,
        err_variant: dbyte_hir::VariantId,
        cleanup: &[HirBlock],
        expression: &HirExpr,
    ) -> Result<VirtualRegister, NativeMirError> {
        let result_address = self.lower_expr(value)?;
        let (ok_discriminant, _, ok_payload) =
            dbyte_hir::enum_variant_layout(&self.program.semantic, value.ty, ok_variant)
                .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
        let (err_discriminant, _, _) =
            dbyte_hir::enum_variant_layout(&self.program.semantic, value.ty, err_variant)
                .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
        if ok_payload.len() != 1 {
            return Err(unsupported(expression.span, "Result Ok payload arity"));
        }
        let tag = self.new_register(MachineType::I32);
        self.emit(
            NativeMirInst::Load {
                destination: tag,
                address: result_address,
                ty: MachineType::I32,
                volatile: false,
            },
            expression.span,
        );
        let expected = self.new_register(MachineType::I32);
        self.emit(
            NativeMirInst::Const {
                destination: expected,
                value: u64::from(ok_discriminant),
                ty: MachineType::I32,
            },
            expression.span,
        );
        let is_ok = self.new_register(MachineType::Bool);
        self.emit(
            NativeMirInst::Compare {
                destination: is_ok,
                op: NativeCompareOp::Equal,
                left: NativeValue::Register(tag),
                right: NativeValue::Register(expected),
                ty: MachineType::I32,
            },
            expression.span,
        );
        let ok_block = self.new_block();
        let err_block = self.new_block();
        self.terminate(NativeTerminator::Branch {
            condition: NativeValue::Register(is_ok),
            then_block: ok_block,
            else_block: err_block,
        })?;
        self.current = err_block;
        let err_expected = self.new_register(MachineType::I32);
        self.emit(
            NativeMirInst::Const {
                destination: err_expected,
                value: u64::from(err_discriminant),
                ty: MachineType::I32,
            },
            expression.span,
        );
        let is_err = self.new_register(MachineType::Bool);
        self.emit(
            NativeMirInst::Compare {
                destination: is_err,
                op: NativeCompareOp::Equal,
                left: NativeValue::Register(tag),
                right: NativeValue::Register(err_expected),
                ty: MachineType::I32,
            },
            expression.span,
        );
        let propagate = self.new_block();
        let invalid = self.new_block();
        self.terminate(NativeTerminator::Branch {
            condition: NativeValue::Register(is_err),
            then_block: propagate,
            else_block: invalid,
        })?;
        self.current = invalid;
        self.terminate(NativeTerminator::Trap)?;
        self.current = propagate;
        for cleanup in cleanup {
            self.lower_block(cleanup)?;
        }
        let return_layout = aggregate_layout(self.program, self.function.result, expression.span)?
            .ok_or_else(|| {
                unsupported(expression.span, "Result propagation from scalar function")
            })?;
        self.terminate(NativeTerminator::ReturnAggregate {
            source: result_address,
            layout: return_layout,
        })?;
        self.current = ok_block;
        let (payload_ty, offset) = ok_payload[0];
        let payload_address = self.offset_address(result_address, offset, expression.span)?;
        if is_zero_sized(self.program, payload_ty)? {
            Ok(self.unit_value(expression.span))
        } else if aggregate_layout(self.program, payload_ty, expression.span)?.is_some() {
            Ok(payload_address)
        } else {
            let ty = machine_type(self.program, payload_ty, expression.span)?;
            let output = self.new_register(ty);
            self.emit(
                NativeMirInst::Load {
                    destination: output,
                    address: payload_address,
                    ty,
                    volatile: false,
                },
                expression.span,
            );
            Ok(output)
        }
    }

    fn lower_call_arguments(
        &mut self,
        arguments: &[HirExpr],
        convention: dbyte_ast::CallConvention,
    ) -> Result<(Vec<VirtualRegister>, Vec<Option<NativeAggregateLayout>>), NativeMirError> {
        let mut values = Vec::with_capacity(arguments.len());
        let mut aggregates = Vec::with_capacity(arguments.len());
        for argument in arguments {
            let aggregate = aggregate_layout(self.program, argument.ty, argument.span)?;
            let mut value = self.lower_expr(argument)?;
            if aggregate.is_none() && convention == dbyte_ast::CallConvention::C {
                let signed_narrow = matches!(
                    self.program.semantic.type_of(argument.ty),
                    CanonicalType::Integer(IntegerType::I8 | IntegerType::I16)
                );
                let c_promoted = signed_narrow
                    || matches!(
                        self.program.semantic.type_of(argument.ty),
                        CanonicalType::Bool
                            | CanonicalType::Integer(IntegerType::U8 | IntegerType::U16)
                    );
                if c_promoted {
                    let from = self.register_types[value.0 as usize];
                    let promoted = self.new_register(MachineType::I32);
                    self.emit(
                        NativeMirInst::Cast {
                            destination: promoted,
                            value,
                            from,
                            to: MachineType::I32,
                            signed: signed_narrow,
                        },
                        argument.span,
                    );
                    value = promoted;
                }
            }
            values.push(value);
            aggregates.push(aggregate);
        }
        Ok((values, aggregates))
    }

    fn lower_native_callee(&mut self, callee: &HirExpr) -> Result<NativeCallee, NativeMirError> {
        let value = self.lower_expr(callee)?;
        if matches!(
            self.program.semantic.type_of(callee.ty),
            CanonicalType::ClosureShape { .. }
        ) {
            let code = self.new_register(MachineType::Pointer);
            self.emit(
                NativeMirInst::Load {
                    destination: code,
                    address: value,
                    ty: MachineType::Pointer,
                    volatile: false,
                },
                callee.span,
            );
            let environment_address = self.offset_address(
                value,
                self.program.semantic.layout.pointer_size,
                callee.span,
            )?;
            let environment = self.new_register(MachineType::Pointer);
            self.emit(
                NativeMirInst::Load {
                    destination: environment,
                    address: environment_address,
                    ty: MachineType::Pointer,
                    volatile: false,
                },
                callee.span,
            );
            Ok(NativeCallee::Closure { code, environment })
        } else {
            Ok(NativeCallee::Indirect { address: value })
        }
    }

    fn lower_aggregate_call_into(
        &mut self,
        expression: &HirExpr,
        destination: VirtualRegister,
    ) -> Result<(), NativeMirError> {
        let layout = aggregate_layout(self.program, expression.ty, expression.span)?
            .ok_or_else(|| unsupported(expression.span, "aggregate call result layout"))?;
        match &expression.kind {
            HirExprKind::DirectCall {
                callee,
                convention,
                arguments,
            } => {
                let symbol = self.symbols.get(callee).cloned().ok_or_else(|| {
                    NativeMirError::new(
                        "E_NATIVE_MIR_CALL",
                        "resolved HIR call has no native link symbol",
                    )
                })?;
                let (arguments, argument_aggregates) =
                    self.lower_call_arguments(arguments, *convention)?;
                self.emit(
                    NativeMirInst::Call {
                        destination: Some(destination),
                        callee: NativeCallee::Direct { symbol },
                        arguments,
                        argument_aggregates,
                        convention: *convention,
                        result: None,
                        aggregate_result: Some(layout),
                    },
                    expression.span,
                );
                Ok(())
            }
            HirExprKind::IndirectCall {
                callee,
                convention,
                arguments,
            } => {
                let callee = self.lower_native_callee(callee)?;
                let (arguments, argument_aggregates) =
                    self.lower_call_arguments(arguments, *convention)?;
                self.emit(
                    NativeMirInst::Call {
                        destination: Some(destination),
                        callee,
                        arguments,
                        argument_aggregates,
                        convention: *convention,
                        result: None,
                        aggregate_result: Some(layout),
                    },
                    expression.span,
                );
                Ok(())
            }
            _ => Err(unsupported(expression.span, "aggregate call initializer")),
        }
    }

    fn lower_direct_aggregate_expr_into(
        &mut self,
        expression: &HirExpr,
        destination: VirtualRegister,
    ) -> Result<Option<VirtualRegister>, NativeMirError> {
        let layout = aggregate_layout(self.program, expression.ty, expression.span)?
            .ok_or_else(|| unsupported(expression.span, "direct aggregate return layout"))?;
        match &expression.kind {
            HirExprKind::Constant { value, .. } => {
                self.lower_direct_aggregate_expr_into(value, destination)
            }
            HirExprKind::Struct { fields, .. } => {
                self.emit(
                    NativeMirInst::AggregateZero {
                        destination,
                        bytes: layout.bytes,
                        alignment: layout.alignment,
                    },
                    expression.span,
                );
                for (field, value) in fields {
                    let (field_ty, offset) = dbyte_hir::struct_field_layout(
                        &self.program.semantic,
                        expression.ty,
                        *field,
                    )
                    .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
                    let value_register = self.lower_expr(value)?;
                    if is_zero_sized(self.program, field_ty)? {
                        continue;
                    }
                    let address = self.offset_address(destination, offset, value.span)?;
                    if let Some(field_layout) =
                        aggregate_layout(self.program, field_ty, value.span)?
                    {
                        self.emit(
                            NativeMirInst::AggregateCopy {
                                destination: address,
                                source: value_register,
                                bytes: field_layout.bytes,
                                alignment: field_layout.alignment,
                            },
                            value.span,
                        );
                    } else {
                        self.emit(
                            NativeMirInst::Store {
                                address,
                                value: value_register,
                                ty: machine_type(self.program, field_ty, value.span)?,
                                volatile: false,
                            },
                            value.span,
                        );
                    }
                }
                Ok(Some(destination))
            }
            HirExprKind::Enum {
                variant,
                discriminant,
                payload,
                ..
            } => {
                self.emit(
                    NativeMirInst::AggregateZero {
                        destination,
                        bytes: layout.bytes,
                        alignment: layout.alignment,
                    },
                    expression.span,
                );
                let tag = self.new_register(MachineType::I32);
                self.emit(
                    NativeMirInst::Const {
                        destination: tag,
                        value: u64::from(*discriminant),
                        ty: MachineType::I32,
                    },
                    expression.span,
                );
                self.emit(
                    NativeMirInst::Store {
                        address: destination,
                        value: tag,
                        ty: MachineType::I32,
                        volatile: false,
                    },
                    expression.span,
                );
                let (_, _, fields) =
                    dbyte_hir::enum_variant_layout(&self.program.semantic, expression.ty, *variant)
                        .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
                if fields.len() != payload.len() {
                    return Err(unsupported(expression.span, "enum payload arity"));
                }
                for (value, (payload_ty, offset)) in payload.iter().zip(fields) {
                    let value_register = self.lower_expr(value)?;
                    if is_zero_sized(self.program, payload_ty)? {
                        continue;
                    }
                    let address = self.offset_address(destination, offset, value.span)?;
                    if let Some(field_layout) =
                        aggregate_layout(self.program, payload_ty, value.span)?
                    {
                        self.emit(
                            NativeMirInst::AggregateCopy {
                                destination: address,
                                source: value_register,
                                bytes: field_layout.bytes,
                                alignment: field_layout.alignment,
                            },
                            value.span,
                        );
                    } else {
                        self.emit(
                            NativeMirInst::Store {
                                address,
                                value: value_register,
                                ty: machine_type(self.program, payload_ty, value.span)?,
                                volatile: false,
                            },
                            value.span,
                        );
                    }
                }
                Ok(Some(destination))
            }
            _ => Ok(None),
        }
    }

    fn lower_aggregate_expr(
        &mut self,
        expression: &HirExpr,
    ) -> Result<VirtualRegister, NativeMirError> {
        let layout = aggregate_layout(self.program, expression.ty, expression.span)?
            .ok_or_else(|| unsupported(expression.span, "aggregate expression layout"))?;
        match &expression.kind {
            HirExprKind::Closure {
                closure, captures, ..
            } => {
                let destination = self.allocate_temporary(expression.ty, expression.span)?;
                self.emit(
                    NativeMirInst::AggregateZero {
                        destination,
                        bytes: layout.bytes,
                        alignment: layout.alignment,
                    },
                    expression.span,
                );
                let symbol = self.closure_symbols.get(closure).cloned().ok_or_else(|| {
                    NativeMirError::new(
                        "E_NATIVE_MIR_CLOSURE",
                        "resolved closure has no native code symbol",
                    )
                })?;
                let code = self.symbol_address(symbol, expression.span);
                self.emit(
                    NativeMirInst::Store {
                        address: destination,
                        value: code,
                        ty: MachineType::Pointer,
                        volatile: false,
                    },
                    expression.span,
                );
                let (environment_layout, accesses) =
                    closure_environment_layout(self.program, captures, expression.span)?;
                let environment = if captures.is_empty() {
                    let null = self.new_register(MachineType::Pointer);
                    self.emit(
                        NativeMirInst::Const {
                            destination: null,
                            value: 0,
                            ty: MachineType::Pointer,
                        },
                        expression.span,
                    );
                    null
                } else {
                    let object = StackObjectId(self.stack_objects.len() as u32);
                    self.stack_objects.push(NativeStackObject {
                        id: object,
                        bytes: environment_layout.size,
                        alignment: environment_layout.align,
                        address_taken: true,
                    });
                    let environment = self.stack_address(object, expression.span);
                    self.emit(
                        NativeMirInst::AggregateZero {
                            destination: environment,
                            bytes: environment_layout.size,
                            alignment: environment_layout.align,
                        },
                        expression.span,
                    );
                    for capture in captures {
                        let access = accesses[&capture.local];
                        let address =
                            self.offset_address(environment, access.offset, capture.span)?;
                        if capture.mode == dbyte_ast::CaptureMode::Move {
                            let value = self.lower_local_value(capture.local, capture.span)?;
                            if is_zero_sized(self.program, capture.ty)? {
                                continue;
                            }
                            if let Some(layout) =
                                aggregate_layout(self.program, capture.ty, capture.span)?
                            {
                                self.emit(
                                    NativeMirInst::AggregateCopy {
                                        destination: address,
                                        source: value,
                                        bytes: layout.bytes,
                                        alignment: layout.alignment,
                                    },
                                    capture.span,
                                );
                            } else {
                                self.emit(
                                    NativeMirInst::Store {
                                        address,
                                        value,
                                        ty: machine_type(self.program, capture.ty, capture.span)?,
                                        volatile: false,
                                    },
                                    capture.span,
                                );
                            }
                        } else {
                            let value = self.lower_local_address(capture.local, capture.span)?;
                            self.emit(
                                NativeMirInst::Store {
                                    address,
                                    value,
                                    ty: MachineType::Pointer,
                                    volatile: false,
                                },
                                capture.span,
                            );
                        }
                    }
                    environment
                };
                let env_address = self.offset_address(
                    destination,
                    self.program.semantic.layout.pointer_size,
                    expression.span,
                )?;
                self.emit(
                    NativeMirInst::Store {
                        address: env_address,
                        value: environment,
                        ty: MachineType::Pointer,
                        volatile: false,
                    },
                    expression.span,
                );
                Ok(destination)
            }
            HirExprKind::String(bytes) => {
                if !matches!(
                    self.program.semantic.type_of(expression.ty),
                    CanonicalType::StringView
                ) {
                    return Err(unsupported(
                        expression.span,
                        "string literal aggregate context",
                    ));
                }
                let destination = self.allocate_temporary(expression.ty, expression.span)?;
                self.emit(
                    NativeMirInst::AggregateZero {
                        destination,
                        bytes: layout.bytes,
                        alignment: layout.alignment,
                    },
                    expression.span,
                );
                let symbol = self.string_names.get(bytes).cloned().ok_or_else(|| {
                    NativeMirError::new(
                        "E_NATIVE_MIR_STRING_SYMBOL",
                        "canonical string literal has no pooled native symbol",
                    )
                })?;
                let pointer = self.symbol_address(symbol, expression.span);
                self.emit(
                    NativeMirInst::Store {
                        address: destination,
                        value: pointer,
                        ty: MachineType::Pointer,
                        volatile: false,
                    },
                    expression.span,
                );
                let length_address = self.offset_address(
                    destination,
                    self.program.semantic.layout.pointer_size,
                    expression.span,
                )?;
                let length = self.new_register(MachineType::Pointer);
                self.emit(
                    NativeMirInst::Const {
                        destination: length,
                        value: bytes.len() as u64,
                        ty: MachineType::Pointer,
                    },
                    expression.span,
                );
                self.emit(
                    NativeMirInst::Store {
                        address: length_address,
                        value: length,
                        ty: MachineType::Pointer,
                        volatile: false,
                    },
                    expression.span,
                );
                Ok(destination)
            }
            HirExprKind::IntrinsicCall {
                intrinsic:
                    dbyte_hir::HirIntrinsic::RuntimeSliceFromRaw
                    | dbyte_hir::HirIntrinsic::RuntimeMutSliceFromRaw
                    | dbyte_hir::HirIntrinsic::RuntimeStringViewFromRaw,
                arguments,
            } => {
                let [pointer, length] = arguments.as_slice() else {
                    return Err(unsupported(
                        expression.span,
                        "runtime view constructor arity",
                    ));
                };
                let destination = self.allocate_temporary(expression.ty, expression.span)?;
                self.emit(
                    NativeMirInst::AggregateZero {
                        destination,
                        bytes: layout.bytes,
                        alignment: layout.alignment,
                    },
                    expression.span,
                );
                let pointer = self.lower_expr(pointer)?;
                self.emit(
                    NativeMirInst::Store {
                        address: destination,
                        value: pointer,
                        ty: MachineType::Pointer,
                        volatile: false,
                    },
                    expression.span,
                );
                let length_address = self.offset_address(
                    destination,
                    self.program.semantic.layout.pointer_size,
                    expression.span,
                )?;
                let length = self.lower_expr(length)?;
                self.emit(
                    NativeMirInst::Store {
                        address: length_address,
                        value: length,
                        ty: MachineType::Pointer,
                        volatile: false,
                    },
                    expression.span,
                );
                Ok(destination)
            }
            HirExprKind::Local(local) => self.lower_local_value(*local, expression.span),
            HirExprKind::Place(place) => self.lower_place_address(place),
            HirExprKind::Constant { value, .. } => self.lower_aggregate_expr(value),
            HirExprKind::Try {
                value,
                ok_variant,
                err_variant,
                cleanup,
            } => self.lower_try_expr(value, *ok_variant, *err_variant, cleanup, expression),
            HirExprKind::Borrow { place, .. } => {
                let length = match self.program.semantic.type_of(place.ty) {
                    CanonicalType::Array { length, .. } => *length,
                    _ => {
                        return Err(unsupported(
                            expression.span,
                            "slice borrow source is not an array",
                        ))
                    }
                };
                let destination = self.allocate_temporary(expression.ty, expression.span)?;
                self.emit(
                    NativeMirInst::AggregateZero {
                        destination,
                        bytes: layout.bytes,
                        alignment: layout.alignment,
                    },
                    expression.span,
                );
                let pointer = self.lower_place_address(place)?;
                self.emit(
                    NativeMirInst::Store {
                        address: destination,
                        value: pointer,
                        ty: MachineType::Pointer,
                        volatile: false,
                    },
                    expression.span,
                );
                let length_address = self.offset_address(
                    destination,
                    self.program.semantic.layout.pointer_size,
                    expression.span,
                )?;
                let length_value = self.new_register(MachineType::Pointer);
                self.emit(
                    NativeMirInst::Const {
                        destination: length_value,
                        value: u64::from(length),
                        ty: MachineType::Pointer,
                    },
                    expression.span,
                );
                self.emit(
                    NativeMirInst::Store {
                        address: length_address,
                        value: length_value,
                        ty: MachineType::Pointer,
                        volatile: false,
                    },
                    expression.span,
                );
                Ok(destination)
            }
            HirExprKind::List(values) => {
                let (element, length) = match self.program.semantic.type_of(expression.ty) {
                    CanonicalType::Array { element, length } => (*element, *length),
                    _ => {
                        return Err(unsupported(
                            expression.span,
                            "aggregate list is not an array",
                        ))
                    }
                };
                if values.len() != length as usize {
                    return Err(unsupported(expression.span, "malformed array initializer"));
                }
                let element_layout = dbyte_hir::layout_of(&self.program.semantic, element)
                    .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
                let stride = element_layout
                    .size
                    .checked_add(element_layout.align - 1)
                    .map(|value| value & !(element_layout.align - 1))
                    .ok_or_else(|| unsupported(expression.span, "array stride overflow"))?;
                let destination = self.allocate_temporary(expression.ty, expression.span)?;
                self.emit(
                    NativeMirInst::AggregateZero {
                        destination,
                        bytes: layout.bytes,
                        alignment: layout.alignment,
                    },
                    expression.span,
                );
                for (index, value) in values.iter().enumerate() {
                    let value_register = self.lower_expr(value)?;
                    let offset = stride
                        .checked_mul(index as u32)
                        .ok_or_else(|| unsupported(value.span, "array element offset overflow"))?;
                    let address = self.offset_address(destination, offset, value.span)?;
                    if let Some(element_aggregate) =
                        aggregate_layout(self.program, element, value.span)?
                    {
                        self.emit(
                            NativeMirInst::AggregateCopy {
                                destination: address,
                                source: value_register,
                                bytes: element_aggregate.bytes,
                                alignment: element_aggregate.alignment,
                            },
                            value.span,
                        );
                    } else {
                        self.emit(
                            NativeMirInst::Store {
                                address,
                                value: value_register,
                                ty: machine_type(self.program, element, value.span)?,
                                volatile: false,
                            },
                            value.span,
                        );
                    }
                }
                Ok(destination)
            }
            HirExprKind::Struct { fields, .. } => {
                let destination = self.allocate_temporary(expression.ty, expression.span)?;
                self.emit(
                    NativeMirInst::AggregateZero {
                        destination,
                        bytes: layout.bytes,
                        alignment: layout.alignment,
                    },
                    expression.span,
                );
                for (field, value) in fields {
                    let (field_ty, offset) = dbyte_hir::struct_field_layout(
                        &self.program.semantic,
                        expression.ty,
                        *field,
                    )
                    .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
                    let value_register = self.lower_expr(value)?;
                    if is_zero_sized(self.program, field_ty)? {
                        continue;
                    }
                    let address = self.offset_address(destination, offset, expression.span)?;
                    if let Some(field_layout) =
                        aggregate_layout(self.program, field_ty, value.span)?
                    {
                        self.emit(
                            NativeMirInst::AggregateCopy {
                                destination: address,
                                source: value_register,
                                bytes: field_layout.bytes,
                                alignment: field_layout.alignment,
                            },
                            value.span,
                        );
                    } else {
                        self.emit(
                            NativeMirInst::Store {
                                address,
                                value: value_register,
                                ty: machine_type(self.program, field_ty, value.span)?,
                                volatile: false,
                            },
                            value.span,
                        );
                    }
                }
                Ok(destination)
            }
            HirExprKind::Enum {
                variant,
                discriminant,
                payload,
                ..
            } => {
                let destination = self.allocate_temporary(expression.ty, expression.span)?;
                self.emit(
                    NativeMirInst::AggregateZero {
                        destination,
                        bytes: layout.bytes,
                        alignment: layout.alignment,
                    },
                    expression.span,
                );
                let tag = self.new_register(MachineType::I32);
                self.emit(
                    NativeMirInst::Const {
                        destination: tag,
                        value: u64::from(*discriminant),
                        ty: MachineType::I32,
                    },
                    expression.span,
                );
                self.emit(
                    NativeMirInst::Store {
                        address: destination,
                        value: tag,
                        ty: MachineType::I32,
                        volatile: false,
                    },
                    expression.span,
                );
                let (_, _, fields) =
                    dbyte_hir::enum_variant_layout(&self.program.semantic, expression.ty, *variant)
                        .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
                if fields.len() != payload.len() {
                    return Err(unsupported(expression.span, "enum payload arity"));
                }
                for (value, (payload_ty, offset)) in payload.iter().zip(fields) {
                    let value_register = self.lower_expr(value)?;
                    if is_zero_sized(self.program, payload_ty)? {
                        continue;
                    }
                    let address = self.offset_address(destination, offset, value.span)?;
                    if let Some(field_layout) =
                        aggregate_layout(self.program, payload_ty, value.span)?
                    {
                        self.emit(
                            NativeMirInst::AggregateCopy {
                                destination: address,
                                source: value_register,
                                bytes: field_layout.bytes,
                                alignment: field_layout.alignment,
                            },
                            value.span,
                        );
                    } else {
                        self.emit(
                            NativeMirInst::Store {
                                address,
                                value: value_register,
                                ty: machine_type(self.program, payload_ty, value.span)?,
                                volatile: false,
                            },
                            value.span,
                        );
                    }
                }
                Ok(destination)
            }
            HirExprKind::DirectCall {
                callee,
                convention,
                arguments,
            } => {
                let symbol = self.symbols.get(callee).cloned().ok_or_else(|| {
                    NativeMirError::new(
                        "E_NATIVE_MIR_CALL",
                        "resolved HIR call has no native link symbol",
                    )
                })?;
                let (arguments, argument_aggregates) =
                    self.lower_call_arguments(arguments, *convention)?;
                let destination = self.allocate_temporary(expression.ty, expression.span)?;
                self.emit(
                    NativeMirInst::Call {
                        destination: Some(destination),
                        callee: NativeCallee::Direct { symbol },
                        arguments,
                        argument_aggregates,
                        convention: *convention,
                        result: None,
                        aggregate_result: Some(layout),
                    },
                    expression.span,
                );
                Ok(destination)
            }
            HirExprKind::IndirectCall {
                callee,
                convention,
                arguments,
            } => {
                let callee = self.lower_native_callee(callee)?;
                let (arguments, argument_aggregates) =
                    self.lower_call_arguments(arguments, *convention)?;
                let destination = self.allocate_temporary(expression.ty, expression.span)?;
                self.emit(
                    NativeMirInst::Call {
                        destination: Some(destination),
                        callee,
                        arguments,
                        argument_aggregates,
                        convention: *convention,
                        result: None,
                        aggregate_result: Some(layout),
                    },
                    expression.span,
                );
                Ok(destination)
            }
            _ => Err(unsupported(expression.span, "aggregate HIR expression")),
        }
    }

    fn allocate_temporary(
        &mut self,
        ty: TypeId,
        span: Span,
    ) -> Result<VirtualRegister, NativeMirError> {
        let layout = dbyte_hir::layout_of(&self.program.semantic, ty)
            .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
        let object = StackObjectId(self.stack_objects.len() as u32);
        self.stack_objects.push(NativeStackObject {
            id: object,
            bytes: layout.size,
            alignment: layout.align,
            address_taken: true,
        });
        Ok(self.stack_address(object, span))
    }

    fn offset_address(
        &mut self,
        base: VirtualRegister,
        offset: u32,
        span: Span,
    ) -> Result<VirtualRegister, NativeMirError> {
        if offset == 0 {
            return Ok(base);
        }
        let destination = self.new_register(MachineType::Pointer);
        self.emit(
            NativeMirInst::AddressOffset {
                destination,
                base,
                index: None,
                scale: 1,
                displacement: i32::try_from(offset)
                    .map_err(|_| unsupported(span, "aggregate field offset outside i32"))?,
            },
            span,
        );
        Ok(destination)
    }

    fn assign_local(
        &mut self,
        local: LocalId,
        value: VirtualRegister,
        span: Span,
    ) -> Result<(), NativeMirError> {
        if self.capture_accesses.contains_key(&local) {
            let address = self.lower_local_address(local, span)?;
            let access = self.capture_accesses[&local];
            if is_zero_sized(self.program, access.ty)? {
                return Ok(());
            }
            if let Some(layout) = aggregate_layout(self.program, access.ty, span)? {
                self.emit(
                    NativeMirInst::AggregateCopy {
                        destination: address,
                        source: value,
                        bytes: layout.bytes,
                        alignment: layout.alignment,
                    },
                    span,
                );
            } else {
                self.emit(
                    NativeMirInst::Store {
                        address,
                        value,
                        ty: machine_type(self.program, access.ty, span)?,
                        volatile: false,
                    },
                    span,
                );
            }
            return Ok(());
        }
        if let Some(object) = self.local_stack_objects.get(&local).copied() {
            let address = self.stack_address(object, span);
            let local_ty = self
                .function
                .locals
                .iter()
                .find(|item| item.id == local)
                .ok_or_else(|| NativeMirError::new("E_NATIVE_MIR_LOCAL", "unknown HIR local"))?
                .ty;
            if is_zero_sized(self.program, local_ty)? {
                return Ok(());
            }
            if let Some(layout) = aggregate_layout(self.program, local_ty, span)? {
                self.emit(
                    NativeMirInst::AggregateCopy {
                        destination: address,
                        source: value,
                        bytes: layout.bytes,
                        alignment: layout.alignment,
                    },
                    span,
                );
            } else {
                let ty = self.register_types[value.0 as usize];
                self.emit(
                    NativeMirInst::Store {
                        address,
                        value,
                        ty,
                        volatile: false,
                    },
                    span,
                );
            }
            return Ok(());
        }
        let destination = self.local_register(local)?;
        if destination != value {
            let ty = self.register_types[destination.0 as usize];
            self.emit(
                NativeMirInst::Copy {
                    destination,
                    source: value,
                    ty,
                },
                span,
            );
        }
        Ok(())
    }

    fn lower_struct_initializer(
        &mut self,
        local: LocalId,
        value: &HirExpr,
        span: Span,
    ) -> Result<(), NativeMirError> {
        let object = self
            .local_stack_objects
            .get(&local)
            .copied()
            .ok_or_else(|| {
                NativeMirError::new(
                    "E_NATIVE_MIR_LOCAL_ADDRESS",
                    "aggregate local has no stack object",
                )
            })?;
        let local_ty = self
            .function
            .locals
            .iter()
            .find(|candidate| candidate.id == local)
            .map(|candidate| candidate.ty)
            .ok_or_else(|| NativeMirError::new("E_NATIVE_MIR_LOCAL", "unknown aggregate local"))?;
        let layout = dbyte_hir::layout_of(&self.program.semantic, local_ty)
            .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
        let base = self.stack_address(object, span);
        self.emit(
            NativeMirInst::AggregateZero {
                destination: base,
                bytes: layout.size,
                alignment: layout.align,
            },
            span,
        );
        let HirExprKind::Struct { fields, .. } = &value.kind else {
            return Err(unsupported(value.span, "non-struct aggregate initializer"));
        };
        for (field, field_value) in fields {
            let (field_ty, offset) =
                dbyte_hir::struct_field_layout(&self.program.semantic, local_ty, *field)
                    .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
            let field_span = field_value.span;
            let field_value = self.lower_expr(field_value)?;
            if is_zero_sized(self.program, field_ty)? {
                continue;
            }
            let field_machine_ty = machine_type(self.program, field_ty, field_span)?;
            let address = if offset == 0 {
                base
            } else {
                let address = self.new_register(MachineType::Pointer);
                self.emit(
                    NativeMirInst::AddressOffset {
                        destination: address,
                        base,
                        index: None,
                        scale: 1,
                        displacement: i32::try_from(offset)
                            .map_err(|_| unsupported(span, "struct field offset outside i32"))?,
                    },
                    value.span,
                );
                address
            };
            self.emit(
                NativeMirInst::Store {
                    address,
                    value: field_value,
                    ty: field_machine_ty,
                    volatile: false,
                },
                value.span,
            );
        }
        Ok(())
    }

    fn lower_place_value(
        &mut self,
        place: &dbyte_hir::HirPlace,
    ) -> Result<VirtualRegister, NativeMirError> {
        if place.projections.is_empty() {
            if let HirPlaceBase::Local(local) = &place.base {
                return self.lower_local_value(*local, place.span);
            }
        }
        let address = self.lower_place_address(place)?;
        if is_zero_sized(self.program, place.ty)? {
            return Ok(self.unit_value(place.span));
        }
        let ty = machine_type(self.program, place.ty, place.span)?;
        let destination = self.new_register(ty);
        self.emit(
            NativeMirInst::Load {
                destination,
                address,
                ty,
                volatile: false,
            },
            place.span,
        );
        Ok(destination)
    }

    fn lower_place_address(
        &mut self,
        place: &dbyte_hir::HirPlace,
    ) -> Result<VirtualRegister, NativeMirError> {
        let mut current_ty = match &place.base {
            HirPlaceBase::Local(local) => self
                .function
                .locals
                .iter()
                .find(|candidate| candidate.id == *local)
                .map(|candidate| candidate.ty)
                .ok_or_else(|| NativeMirError::new("E_NATIVE_MIR_LOCAL", "unknown HIR local"))?,
            HirPlaceBase::Global(symbol) => self
                .program
                .globals
                .iter()
                .find(|candidate| candidate.symbol == *symbol)
                .map(|candidate| candidate.ty)
                .ok_or_else(|| NativeMirError::new("E_NATIVE_MIR_GLOBAL", "unknown HIR global"))?,
            HirPlaceBase::Pointer(pointer)
                if matches!(
                    place.projections.first(),
                    Some(dbyte_hir::HirProjection::Index { .. })
                ) =>
            {
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
        };
        let mut address = match &place.base {
            HirPlaceBase::Local(local) => self.lower_local_address(*local, place.span)?,
            HirPlaceBase::Global(symbol) => self.global_address(*symbol, place.span)?,
            HirPlaceBase::Pointer(pointer) => self.lower_expr(pointer)?,
        };
        for projection in &place.projections {
            match projection {
                dbyte_hir::HirProjection::Field { ty, offset, .. }
                | dbyte_hir::HirProjection::Payload { ty, offset, .. } => {
                    address = self.offset_address(address, *offset, place.span)?;
                    current_ty = *ty;
                }
                dbyte_hir::HirProjection::Index {
                    index,
                    element,
                    element_size,
                } => {
                    let index_register = self.lower_expr(index)?;
                    match self.program.semantic.type_of(current_ty).clone() {
                        CanonicalType::Slice { .. } | CanonicalType::StringView => {
                            let length_address = self.offset_address(
                                address,
                                self.program.semantic.layout.pointer_size,
                                place.span,
                            )?;
                            let length = self.new_register(MachineType::Pointer);
                            self.emit(
                                NativeMirInst::Load {
                                    destination: length,
                                    address: length_address,
                                    ty: MachineType::Pointer,
                                    volatile: false,
                                },
                                place.span,
                            );
                            let in_bounds = self.new_register(MachineType::Bool);
                            self.emit(
                                NativeMirInst::Compare {
                                    destination: in_bounds,
                                    op: NativeCompareOp::UnsignedLess,
                                    left: NativeValue::Register(index_register),
                                    right: NativeValue::Register(length),
                                    ty: MachineType::Pointer,
                                },
                                place.span,
                            );
                            let valid = self.new_block();
                            let invalid = self.new_block();
                            self.terminate(NativeTerminator::Branch {
                                condition: NativeValue::Register(in_bounds),
                                then_block: valid,
                                else_block: invalid,
                            })?;
                            self.current = invalid;
                            self.terminate(NativeTerminator::Trap)?;
                            self.current = valid;
                            let data = self.new_register(MachineType::Pointer);
                            self.emit(
                                NativeMirInst::Load {
                                    destination: data,
                                    address,
                                    ty: MachineType::Pointer,
                                    volatile: false,
                                },
                                place.span,
                            );
                            address = self.lower_indexed_address(
                                data,
                                index_register,
                                *element_size,
                                place.span,
                            )?;
                        }
                        CanonicalType::Array { .. }
                        | CanonicalType::Pointer { .. }
                        | CanonicalType::LegacyStr => {
                            address = self.lower_indexed_address(
                                address,
                                index_register,
                                *element_size,
                                place.span,
                            )?;
                        }
                        _ => {
                            return Err(unsupported(place.span, "index projection representation"))
                        }
                    }
                    current_ty = *element;
                }
            }
        }
        Ok(address)
    }

    fn lower_indexed_address(
        &mut self,
        base: VirtualRegister,
        mut index: VirtualRegister,
        element_size: u32,
        span: Span,
    ) -> Result<VirtualRegister, NativeMirError> {
        let scale = if matches!(element_size, 1 | 2 | 4 | 8) {
            element_size
        } else {
            let scaled = self.new_register(MachineType::Pointer);
            self.emit(
                NativeMirInst::Binary {
                    destination: scaled,
                    op: NativeBinaryOp::Mul,
                    left: NativeValue::Register(index),
                    right: NativeValue::Immediate(u64::from(element_size)),
                    ty: MachineType::Pointer,
                },
                span,
            );
            index = scaled;
            1
        };
        let destination = self.new_register(MachineType::Pointer);
        self.emit(
            NativeMirInst::AddressOffset {
                destination,
                base,
                index: Some(index),
                scale,
                displacement: 0,
            },
            span,
        );
        Ok(destination)
    }

    fn lower_local_value(
        &mut self,
        local: LocalId,
        span: Span,
    ) -> Result<VirtualRegister, NativeMirError> {
        if let Some(access) = self.capture_accesses.get(&local).copied() {
            let address = self.lower_local_address(local, span)?;
            if is_zero_sized(self.program, access.ty)? {
                return Ok(self.unit_value(span));
            }
            if aggregate_layout(self.program, access.ty, span)?.is_some() {
                return Ok(address);
            }
            let ty = machine_type(self.program, access.ty, span)?;
            let destination = self.new_register(ty);
            self.emit(
                NativeMirInst::Load {
                    destination,
                    address,
                    ty,
                    volatile: false,
                },
                span,
            );
            return Ok(destination);
        }
        if let Some(object) = self.local_stack_objects.get(&local).copied() {
            let address = self.stack_address(object, span);
            let local = self
                .function
                .locals
                .iter()
                .find(|candidate| candidate.id == local)
                .ok_or_else(|| NativeMirError::new("E_NATIVE_MIR_LOCAL", "unknown HIR local"))?;
            if is_zero_sized(self.program, local.ty)? {
                return Ok(self.unit_value(span));
            }
            if aggregate_layout(self.program, local.ty, span)?.is_some() {
                return Ok(address);
            }
            let ty = machine_type(self.program, local.ty, span)?;
            let destination = self.new_register(ty);
            self.emit(
                NativeMirInst::Load {
                    destination,
                    address,
                    ty,
                    volatile: false,
                },
                span,
            );
            Ok(destination)
        } else {
            self.local_register(local)
        }
    }

    fn lower_local_address(
        &mut self,
        local: LocalId,
        span: Span,
    ) -> Result<VirtualRegister, NativeMirError> {
        if let Some(access) = self.capture_accesses.get(&local).copied() {
            let environment = self.hidden_environment.ok_or_else(|| {
                NativeMirError::new(
                    "E_NATIVE_MIR_CLOSURE",
                    "captured local has no hidden environment parameter",
                )
            })?;
            let field = self.offset_address(environment, access.offset, span)?;
            if access.mode == dbyte_ast::CaptureMode::Move {
                return Ok(field);
            }
            let pointer = self.new_register(MachineType::Pointer);
            self.emit(
                NativeMirInst::Load {
                    destination: pointer,
                    address: field,
                    ty: MachineType::Pointer,
                    volatile: false,
                },
                span,
            );
            return Ok(pointer);
        }
        if let Some(object) = self.local_stack_objects.get(&local).copied() {
            return Ok(self.stack_address(object, span));
        }
        if let Some(register) = self.local_registers.get(&local).copied() {
            let local_ty = self
                .function
                .locals
                .iter()
                .find(|candidate| candidate.id == local)
                .map(|candidate| candidate.ty)
                .ok_or_else(|| NativeMirError::new("E_NATIVE_MIR_LOCAL", "unknown HIR local"))?;
            if aggregate_layout(self.program, local_ty, span)?.is_some() {
                return Ok(register);
            }
        }
        Err(NativeMirError::new(
            "E_NATIVE_MIR_LOCAL_ADDRESS",
            format!(
                "resolved local l{} has no addressable storage at {}",
                local.0, span
            ),
        ))
    }

    fn stack_address(&mut self, object: StackObjectId, span: Span) -> VirtualRegister {
        let destination = self.new_register(MachineType::Pointer);
        self.emit(
            NativeMirInst::StackAddress {
                destination,
                object,
            },
            span,
        );
        destination
    }

    fn global_address(
        &mut self,
        symbol: dbyte_hir::SymbolId,
        span: Span,
    ) -> Result<VirtualRegister, NativeMirError> {
        let symbol = self.global_names.get(&symbol).cloned().ok_or_else(|| {
            NativeMirError::new(
                "E_NATIVE_MIR_GLOBAL",
                "resolved HIR global has no link symbol",
            )
        })?;
        Ok(self.symbol_address(symbol, span))
    }

    fn symbol_address(&mut self, symbol: String, span: Span) -> VirtualRegister {
        let destination = self.new_register(MachineType::Pointer);
        self.emit(
            NativeMirInst::GlobalAddress {
                destination,
                symbol,
            },
            span,
        );
        destination
    }

    fn local_register(&self, local: LocalId) -> Result<VirtualRegister, NativeMirError> {
        self.local_registers.get(&local).copied().ok_or_else(|| {
            NativeMirError::new(
                "E_NATIVE_MIR_LOCAL",
                format!("resolved HIR references unknown local l{}", local.0),
            )
        })
    }

    fn new_register(&mut self, ty: MachineType) -> VirtualRegister {
        let register = VirtualRegister(self.register_types.len() as u32);
        self.register_types.push(ty);
        register
    }

    fn unit_value(&mut self, span: Span) -> VirtualRegister {
        let register = self.new_register(MachineType::I8);
        self.emit(
            NativeMirInst::Const {
                destination: register,
                value: 0,
                ty: MachineType::I8,
            },
            span,
        );
        register
    }

    fn new_block(&mut self) -> BlockId {
        let block = BlockId(self.blocks.len() as u32);
        self.blocks.push(NativeMirBlock {
            id: block,
            instructions: Vec::new(),
            terminator: NativeTerminator::Unreachable,
        });
        block
    }

    fn emit(&mut self, instruction: NativeMirInst, span: Span) {
        self.blocks[self.current.0 as usize]
            .instructions
            .push(SpannedNativeInst { instruction, span });
    }

    fn terminate(&mut self, terminator: NativeTerminator) -> Result<(), NativeMirError> {
        if self.current_is_terminated() {
            return Err(NativeMirError::new(
                "E_NATIVE_MIR_CFG",
                "native MIR block received more than one terminator",
            ));
        }
        self.blocks[self.current.0 as usize].terminator = terminator;
        Ok(())
    }

    fn current_is_terminated(&self) -> bool {
        !matches!(
            self.blocks[self.current.0 as usize].terminator,
            NativeTerminator::Unreachable
        )
    }
}

fn is_zero_sized(program: &ConcreteHirProgram, ty: TypeId) -> Result<bool, NativeMirError> {
    let layout = dbyte_hir::layout_of(&program.semantic, ty)
        .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
    Ok(layout.size == 0)
}

fn machine_type(
    program: &ConcreteHirProgram,
    ty: TypeId,
    span: Span,
) -> Result<MachineType, NativeMirError> {
    Ok(match program.semantic.type_of(ty) {
        CanonicalType::Unit => MachineType::I8,
        CanonicalType::Bool => MachineType::Bool,
        CanonicalType::Integer(IntegerType::I8 | IntegerType::U8) => MachineType::I8,
        CanonicalType::Integer(IntegerType::I16 | IntegerType::U16) => MachineType::I16,
        CanonicalType::Integer(IntegerType::I32 | IntegerType::U32 | IntegerType::LegacyInt) => {
            MachineType::I32
        }
        CanonicalType::Integer(IntegerType::I64 | IntegerType::U64) => MachineType::I64,
        CanonicalType::F32 => MachineType::F32,
        CanonicalType::F64 => MachineType::F64,
        CanonicalType::Integer(IntegerType::Usize)
        | CanonicalType::Pointer { .. }
        | CanonicalType::Borrow { .. }
        | CanonicalType::Function { .. }
        | CanonicalType::LegacyStr => MachineType::Pointer,
        _ => return Err(unsupported(span, "aggregate or hosted-only machine type")),
    })
}

fn closure_environment_layout(
    program: &ConcreteHirProgram,
    captures: &[dbyte_hir::HirCapture],
    span: Span,
) -> Result<(dbyte_hir::Layout, BTreeMap<LocalId, CaptureAccess>), NativeMirError> {
    let mut size = 0u32;
    let mut align = 1u32;
    let mut accesses = BTreeMap::new();
    for capture in captures {
        let layout = if capture.mode == dbyte_ast::CaptureMode::Move {
            dbyte_hir::layout_of(&program.semantic, capture.ty)
                .map_err(|failure| NativeMirError::new(failure.code, failure.message))?
        } else {
            dbyte_hir::Layout {
                size: program.semantic.layout.pointer_size,
                align: program.semantic.layout.pointer_align,
            }
        };
        size = align_up_layout(size, layout.align, span)?;
        accesses.insert(
            capture.local,
            CaptureAccess {
                offset: size,
                mode: capture.mode,
                ty: capture.ty,
            },
        );
        size = size
            .checked_add(layout.size)
            .ok_or_else(|| unsupported(span, "closure environment size overflow"))?;
        align = align.max(layout.align);
    }
    size = align_up_layout(size, align, span)?;
    Ok((dbyte_hir::Layout { size, align }, accesses))
}

fn align_up_layout(value: u32, alignment: u32, span: Span) -> Result<u32, NativeMirError> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| unsupported(span, "closure environment layout overflow"))
}

fn aggregate_layout(
    program: &ConcreteHirProgram,
    ty: TypeId,
    span: Span,
) -> Result<Option<NativeAggregateLayout>, NativeMirError> {
    if machine_type(program, ty, span).is_ok()
        || matches!(program.semantic.type_of(ty), CanonicalType::Unit)
    {
        return Ok(None);
    }
    let layout = dbyte_hir::layout_of(&program.semantic, ty)
        .map_err(|failure| NativeMirError::new(failure.code, failure.message))?;
    let layout = NativeAggregateLayout {
        bytes: layout.size,
        alignment: layout.align,
    };
    if !layout.is_valid() {
        return Err(unsupported(
            span,
            "zero-sized or malformed aggregate layout",
        ));
    }
    Ok(Some(layout))
}

fn is_signed(program: &ConcreteHirProgram, ty: TypeId) -> bool {
    matches!(
        program.semantic.type_of(ty),
        CanonicalType::Integer(
            IntegerType::I8
                | IntegerType::I16
                | IntegerType::I32
                | IntegerType::I64
                | IntegerType::LegacyInt
        )
    )
}

fn compare_operation(op: BinOp, signed: bool) -> Option<NativeCompareOp> {
    Some(match (op, signed) {
        (BinOp::EqEq, _) => NativeCompareOp::Equal,
        (BinOp::NotEq, _) => NativeCompareOp::NotEqual,
        (BinOp::Lt, true) => NativeCompareOp::SignedLess,
        (BinOp::LtEq, true) => NativeCompareOp::SignedLessEqual,
        (BinOp::Gt, true) => NativeCompareOp::SignedGreater,
        (BinOp::GtEq, true) => NativeCompareOp::SignedGreaterEqual,
        (BinOp::Lt, false) => NativeCompareOp::UnsignedLess,
        (BinOp::LtEq, false) => NativeCompareOp::UnsignedLessEqual,
        (BinOp::Gt, false) => NativeCompareOp::UnsignedGreater,
        (BinOp::GtEq, false) => NativeCompareOp::UnsignedGreaterEqual,
        _ => return None,
    })
}

fn binary_operation(op: BinOp, signed: bool) -> Result<NativeBinaryOp, NativeMirError> {
    Ok(match (op, signed) {
        (BinOp::Add, _) => NativeBinaryOp::Add,
        (BinOp::Sub, _) => NativeBinaryOp::Sub,
        (BinOp::Mul, _) => NativeBinaryOp::Mul,
        (BinOp::Div, true) => NativeBinaryOp::SignedDiv,
        (BinOp::Div, false) => NativeBinaryOp::UnsignedDiv,
        (BinOp::Rem, true) => NativeBinaryOp::SignedRem,
        (BinOp::Rem, false) => NativeBinaryOp::UnsignedRem,
        (BinOp::BitAnd, _) => NativeBinaryOp::BitAnd,
        (BinOp::BitOr, _) => NativeBinaryOp::BitOr,
        (BinOp::BitXor, _) => NativeBinaryOp::BitXor,
        (BinOp::Shl, _) => NativeBinaryOp::ShiftLeft,
        (BinOp::Shr, true) => NativeBinaryOp::SignedShiftRight,
        (BinOp::Shr, false) => NativeBinaryOp::UnsignedShiftRight,
        _ => {
            return Err(NativeMirError::new(
                "E_NATIVE_MIR_BINARY",
                "comparison reached binary lowering",
            ))
        }
    })
}

fn unsupported(span: Span, operation: &str) -> NativeMirError {
    NativeMirError::new(
        "E_NATIVE_MIR_UNSUPPORTED",
        format!("native MIR does not yet represent {operation} at {span}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbyte_hir::{Profile, TargetDataLayout};
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;

    fn concrete(source: &str) -> ConcreteHirProgram {
        let program = Parser::new(Lexer::new(source).tokenize().unwrap())
            .parse_program()
            .unwrap();
        let executable =
            dbyte_hir::analyze_executable(&program, Profile::X86_64Linux, TargetDataLayout::X86_64)
                .unwrap();
        dbyte_hir::monomorphize(&executable).unwrap()
    }

    fn concrete_i686(source: &str) -> ConcreteHirProgram {
        let program = Parser::new(Lexer::new(source).tokenize().unwrap())
            .parse_program()
            .unwrap();
        let executable =
            dbyte_hir::analyze_executable(&program, Profile::I686None, TargetDataLayout::I686)
                .unwrap();
        dbyte_hir::monomorphize(&executable).unwrap()
    }

    #[test]
    fn scalar_hir_lowers_to_explicit_blocks_vregs_and_calls() {
        let concrete = concrete(
            "pub fn add(a: i32, b: i32) -> i32:\n    return a + b\n\npub fn main() -> int:\n    let value: i32 = 0\n    while value < 4:\n        value = add(value, 1)\n    return value\n",
        );
        let project = lower_program_to_native_mir(&concrete).unwrap();
        project.validate().unwrap();
        assert_eq!(project.functions.len(), 2);
        let main = project
            .functions
            .iter()
            .find(|function| function.symbol == "main")
            .unwrap();
        assert!(main.blocks.len() >= 4);
        assert!(main
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .any(|instruction| matches!(instruction.instruction, NativeMirInst::Call { .. })));
        assert!(
            project
                .dump()
                .contains("Native MIR".to_ascii_lowercase().as_str())
                || project.dump().contains("native-mir")
        );
    }

    #[test]
    fn named_scalar_structs_lower_to_explicit_stack_layout_without_semantic_fallback() {
        let concrete = concrete(
            "struct Pair:\n    left: i32\n    right: i32\n\npub fn main() -> int:\n    let pair = Pair(left = 1, right = 2)\n    return pair.left\n",
        );
        let project = lower_program_to_native_mir(&concrete).unwrap();
        let main = &project.functions[0];
        assert_eq!(main.stack_objects.len(), 1);
        let instructions = main
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .map(|instruction| &instruction.instruction)
            .collect::<Vec<_>>();
        assert!(instructions
            .iter()
            .any(|instruction| matches!(instruction, NativeMirInst::AggregateZero { .. })));
        assert!(
            instructions
                .iter()
                .filter(|instruction| matches!(instruction, NativeMirInst::Store { .. }))
                .count()
                >= 2
        );
    }

    #[test]
    fn direct_aggregate_return_does_not_stage_through_a_stack_temporary() {
        let concrete = concrete(
            "struct Pair:\n    left: u64\n    right: u64\n\npub fn make() -> Pair:\n    return Pair(left = 40 as u64, right = 2 as u64)\n",
        );
        let project = lower_program_to_native_mir(&concrete).unwrap();
        let make = project
            .functions
            .iter()
            .find(|function| function.symbol == "make")
            .unwrap();
        assert!(make.aggregate_result.is_some());
        assert!(make.stack_objects.is_empty());
    }

    #[test]
    fn o1_reuses_immediate_loop_bound_for_string_view_indexing() {
        let concrete = concrete(
            "pub fn main() -> i64:\n    let view = \"abcd\"\n    let index: usize = 0 as usize\n    let total: u64 = 0 as u64\n    while index < len(view):\n        total = total + (view[index] as u64)\n        index = index + (1 as usize)\n    return total as i64\n",
        );
        let mut project = lower_program_to_native_mir(&concrete).unwrap();
        project
            .optimize(crate::native_mir::NativeOptimizationLevel::O1)
            .unwrap();
        let main = project
            .functions
            .iter()
            .find(|function| function.symbol == "main")
            .unwrap();
        let bounds = main
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter(|instruction| {
                matches!(
                    instruction.instruction,
                    NativeMirInst::Compare {
                        op: NativeCompareOp::UnsignedLess,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(bounds, 1);
    }

    #[test]
    fn aggregate_call_initializes_local_storage_without_a_temporary() {
        let concrete = concrete(
            "struct Pair:\n    left: u64\n    right: u64\n\nfn make() -> Pair:\n    return Pair(left = 40 as u64, right = 2 as u64)\n\npub fn main() -> i64:\n    let pair = make()\n    return pair.right as i64\n",
        );
        let project = lower_program_to_native_mir(&concrete).unwrap();
        let main = project
            .functions
            .iter()
            .find(|function| function.symbol == "main")
            .unwrap();
        assert_eq!(main.stack_objects.len(), 1);
    }

    #[test]
    fn unit_calls_have_no_phantom_machine_result() {
        let concrete = concrete(
            "fn touch() -> unit:\n    return\n\npub fn main() -> int:\n    touch()\n    return 0\n",
        );
        let project = lower_program_to_native_mir(&concrete).unwrap();
        let touch = project
            .functions
            .iter()
            .find(|function| function.symbol == "touch")
            .unwrap();
        assert_eq!(touch.result, None);
        assert!(matches!(
            touch.blocks[0].terminator,
            NativeTerminator::Return(None)
        ));
        let main = project
            .functions
            .iter()
            .find(|function| function.symbol == "main")
            .unwrap();
        assert!(main.blocks[0]
            .instructions
            .iter()
            .any(|instruction| matches!(
                instruction.instruction,
                NativeMirInst::Call {
                    destination: None,
                    result: None,
                    ..
                }
            )));
    }

    #[test]
    fn scalar_globals_lower_to_explicit_addresses_loads_and_stores() {
        let concrete = concrete(
            "static mut VALUE: u32\n\npub fn main() -> int:\n    VALUE = 42 as u32\n    return VALUE as int\n",
        );
        let project = lower_program_to_native_mir(&concrete).unwrap();
        assert_eq!(project.globals.len(), 1);
        assert_eq!(project.globals[0].symbol, "VALUE");
        assert_eq!(project.globals[0].size, 4);
        let instructions = project.functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .map(|instruction| &instruction.instruction)
            .collect::<Vec<_>>();
        assert!(instructions
            .iter()
            .any(|instruction| matches!(instruction, NativeMirInst::GlobalAddress { .. })));
        assert!(instructions
            .iter()
            .any(|instruction| matches!(instruction, NativeMirInst::Store { .. })));
        assert!(instructions
            .iter()
            .any(|instruction| matches!(instruction, NativeMirInst::Load { .. })));
    }

    #[test]
    fn closure_definition_keeps_generated_symbol_distinct_from_parent() {
        let concrete = concrete(
            "fn closure_check() -> u64:\n    let total: u64 = 1 as u64\n    let add = fn [borrow mut total](value: u64) -> u64:\n        total = total + value\n        return total\n    return add(41 as u64)\n\npub fn main() -> int:\n    return closure_check() as int\n",
        );
        let project = lower_program_to_native_mir(&concrete).unwrap();
        let symbols = project
            .functions
            .iter()
            .map(|function| function.symbol.as_str())
            .collect::<BTreeSet<_>>();
        assert!(symbols.contains("closure_check"));
        assert!(symbols
            .iter()
            .any(|symbol| symbol.starts_with("__dbyte_closure_")));
        assert_eq!(symbols.len(), project.functions.len());
    }

    #[test]
    fn static_array_of_structs_preserves_canonical_layout_bytes() {
        let concrete = concrete(
            "struct Pair:\n    left: u32\n    right: u32\n\nstatic PAIRS: array[Pair, 2] = [Pair(left = 1 as u32, right = 2 as u32), Pair(left = 3 as u32, right = 4 as u32)]\n\npub fn main() -> int:\n    return 0\n",
        );
        let project = lower_program_to_native_mir(&concrete).unwrap();
        assert_eq!(project.globals.len(), 1);
        assert_eq!(project.globals[0].symbol, "PAIRS");
        assert_eq!(project.globals[0].size, 16);
        assert_eq!(
            project.globals[0].bytes,
            [1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0,]
        );
    }

    #[test]
    fn aggregate_global_assignment_lowers_to_explicit_copy() {
        let concrete = concrete(
            "struct Pair:\n    left: u64\n    right: u64\n\nstatic mut VALUE: Pair\n\npub fn main() -> int:\n    let pair = Pair(left = 40 as u64, right = 2 as u64)\n    VALUE = pair\n    return VALUE.right as int\n",
        );
        let project = lower_program_to_native_mir(&concrete).unwrap();
        let instructions = project.functions[0]
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .map(|instruction| &instruction.instruction)
            .collect::<Vec<_>>();
        assert!(instructions.iter().any(|instruction| matches!(
            instruction,
            NativeMirInst::AggregateCopy {
                bytes: 16,
                alignment: 8,
                ..
            }
        )));
        assert!(instructions.iter().any(|instruction| matches!(
            instruction,
            NativeMirInst::GlobalAddress { symbol, .. } if symbol == "VALUE"
        )));
    }

    #[test]
    fn zero_sized_enum_payload_does_not_emit_phantom_memory_access() {
        let concrete = concrete(
            "enum Maybe:\n    Some(unit)\n    None\n\nfn make() -> Maybe:\n    return Maybe.Some(unit)\n\npub fn main() -> int:\n    let value = make()\n    match value:\n        case Maybe.Some(payload):\n            return 0\n        case Maybe.None:\n            return 1\n",
        );
        let project = lower_program_to_native_mir(&concrete).unwrap();
        let make = project
            .functions
            .iter()
            .find(|function| function.symbol == "make")
            .unwrap();
        let layout = make
            .aggregate_result
            .expect("Maybe must use aggregate return");
        assert_eq!((layout.bytes, layout.alignment), (4, 4));
        for function in [
            make,
            project
                .functions
                .iter()
                .find(|function| function.symbol == "main")
                .unwrap(),
        ] {
            let offset_four = function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter_map(|instruction| match instruction.instruction {
                    NativeMirInst::AddressOffset {
                        destination,
                        displacement: 4,
                        ..
                    } => Some(destination),
                    _ => None,
                })
                .collect::<BTreeSet<_>>();
            assert!(
                !function
                    .blocks
                    .iter()
                    .flat_map(|block| &block.instructions)
                    .any(|instruction| matches!(
                        instruction.instruction,
                        NativeMirInst::Load { address, .. } | NativeMirInst::Store { address, .. }
                            if offset_four.contains(&address)
                    )),
                "zero-sized enum payload emitted a memory access beyond its four-byte tag in {}",
                function.symbol
            );
        }
    }

    #[test]
    fn zero_sized_struct_field_does_not_emit_phantom_memory_access() {
        let concrete = concrete(
            "struct Holder:\n    head: u64\n    marker: unit\n\nfn make() -> Holder:\n    return Holder(head = 42 as u64, marker = unit)\n\npub fn main() -> int:\n    let holder = make()\n    let marker = holder.marker\n    return 0\n",
        );
        let project = lower_program_to_native_mir(&concrete).unwrap();
        for function in &project.functions {
            let offset_eight = function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .filter_map(|instruction| match instruction.instruction {
                    NativeMirInst::AddressOffset {
                        destination,
                        displacement: 8,
                        ..
                    } => Some(destination),
                    _ => None,
                })
                .collect::<BTreeSet<_>>();
            assert!(
                !function
                    .blocks
                    .iter()
                    .flat_map(|block| &block.instructions)
                    .any(|instruction| matches!(
                        instruction.instruction,
                        NativeMirInst::Load { address, .. } | NativeMirInst::Store { address, .. }
                            if offset_eight.contains(&address)
                    )),
                "zero-sized struct field emitted a memory access beyond Holder layout in {}",
                function.symbol
            );
        }
    }

    #[test]
    fn string_literals_and_linux_write_lower_to_rodata_and_resolved_intrinsics() {
        let concrete = concrete(
            "fn linux_write_raw(fd: i32, text: str, length: usize) -> i64:\n    return linux.write(fd, text, length)\n\npub fn main() -> i64:\n    return linux_write_raw(1 as i32, \"MIR OK\\n\", 7 as usize)\n",
        );
        let project = lower_program_to_native_mir(&concrete).unwrap();
        assert_eq!(project.globals.len(), 1);
        assert_eq!(project.globals[0].bytes, b"MIR OK\n");
        assert!(!project.globals[0].mutable);
        let instructions = project
            .functions
            .iter()
            .flat_map(|function| &function.blocks)
            .flat_map(|block| &block.instructions)
            .map(|instruction| &instruction.instruction)
            .collect::<Vec<_>>();
        assert!(instructions.iter().any(|instruction| matches!(
            instruction,
            NativeMirInst::GlobalAddress { symbol, .. } if symbol == "__dbyte_string_0000"
        )));
        assert!(instructions.iter().any(|instruction| matches!(
            instruction,
            NativeMirInst::Intrinsic {
                intrinsic: dbyte_hir::HirIntrinsic::LinuxWrite,
                ..
            }
        )));
    }

    #[test]
    fn exact_bitcast_is_a_same_width_copy_not_a_numeric_cast() {
        let source = "pub fn round32(bits: u32) -> u32:\n    return (bits as! f32) as! u32\n\npub fn round64(bits: u64) -> u64:\n    return (bits as! f64) as! u64\n";
        for concrete in [concrete(source), concrete_i686(source)] {
            let project = lower_program_to_native_mir(&concrete).unwrap();
            let instructions = project
                .functions
                .iter()
                .flat_map(|function| &function.blocks)
                .flat_map(|block| &block.instructions)
                .map(|instruction| &instruction.instruction)
                .collect::<Vec<_>>();
            assert_eq!(
                instructions
                    .iter()
                    .filter(|instruction| matches!(instruction, NativeMirInst::Copy { .. }))
                    .count(),
                4,
                "each exact bitcast must remain an explicit raw-bit copy before optimization"
            );
            assert!(
                !instructions
                    .iter()
                    .any(|instruction| matches!(instruction, NativeMirInst::Cast { .. })),
                "exact bitcast must not enter numeric-cast lowering: {project:#?}"
            );
        }
    }

    #[test]
    fn explicit_float_c_abi_preserves_float_machine_types() {
        let concrete = concrete(
            "pub extern \"C\" fn foreign32(value: f32) -> f32:\n    return value\n\npub extern \"C\" fn callback64(value: f64) -> f64:\n    return value\n",
        );
        let project = lower_program_to_native_mir(&concrete)
            .expect("explicit f32/f64 C ABI must reach Native MIR as float machine types");
        let foreign32 = project
            .functions
            .iter()
            .find(|function| function.symbol == "foreign32")
            .unwrap();
        assert_eq!(foreign32.convention, dbyte_ast::CallConvention::C);
        assert_eq!(foreign32.parameters.len(), 1);
        assert_eq!(
            foreign32.register_types[foreign32.parameters[0].0 as usize],
            MachineType::F32
        );
        assert_eq!(foreign32.result, Some(MachineType::F32));
        let callback64 = project
            .functions
            .iter()
            .find(|function| function.symbol == "callback64")
            .unwrap();
        assert_eq!(callback64.convention, dbyte_ast::CallConvention::C);
        assert_eq!(callback64.parameters.len(), 1);
        assert_eq!(
            callback64.register_types[callback64.parameters[0].0 as usize],
            MachineType::F64
        );
        assert_eq!(callback64.result, Some(MachineType::F64));
    }

    #[test]
    fn x64_usize_literal_keeps_target_width_before_optimization() {
        let concrete = concrete("pub fn main() -> usize:\n    return 4294967296 as usize\n");
        for level in [
            crate::native_mir::NativeOptimizationLevel::O0,
            crate::native_mir::NativeOptimizationLevel::O1,
        ] {
            let mut project = lower_program_to_native_mir(&concrete).unwrap();
            project.optimize(level).unwrap();
            assert!(
                project
                    .functions
                    .iter()
                    .flat_map(|f| &f.blocks)
                    .flat_map(|b| &b.instructions)
                    .any(|i| matches!(
                        i.instruction,
                        NativeMirInst::Const {
                            value: 0x0000_0001_0000_0000,
                            ty: MachineType::Pointer,
                            ..
                        }
                    )),
                "usize literal narrowed before {level}: {project:#?}"
            );
        }
    }

    #[test]
    fn i686_explicit_wide_literal_casts_preserve_all_64_bits_through_o1() {
        let concrete = concrete_i686(
            "pub fn wide_u() -> u64:\n    return 4294967303 as u64\n\npub fn wide_i() -> i64:\n    return 12884901909 as i64\n\npub fn wide_boundary() -> i64:\n    return 4294967296 as i64\n",
        );
        let mut project = lower_program_to_native_mir(&concrete).unwrap();
        project
            .optimize(crate::native_mir::NativeOptimizationLevel::O1)
            .unwrap();

        let returned_const = |symbol: &str| {
            let function = project
                .functions
                .iter()
                .find(|function| function.symbol == symbol)
                .unwrap();
            function
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .find_map(|instruction| match instruction.instruction {
                    NativeMirInst::Const {
                        value,
                        ty: MachineType::I64,
                        ..
                    } => Some(value),
                    _ => None,
                })
                .unwrap()
        };

        assert_eq!(returned_const("wide_u"), 0x0000_0001_0000_0007);
        assert_eq!(returned_const("wide_i"), 0x0000_0003_0000_0015);
        assert_eq!(returned_const("wide_boundary"), 0x0000_0001_0000_0000);
    }

    #[test]
    fn borrowed_scalar_local_uses_one_explicit_addressable_stack_object() {
        let concrete = concrete(
            "fn read(value: borrow u32) -> u32:\n    return *value\n\npub fn main() -> i64:\n    let value: u32 = 42 as u32\n    return read(borrow value) as i64\n",
        );
        let project = lower_program_to_native_mir(&concrete).unwrap();
        let main = project
            .functions
            .iter()
            .find(|function| function.symbol == "main")
            .unwrap();
        assert_eq!(main.stack_objects.len(), 1);
        assert!(main.stack_objects[0].address_taken);
        let instructions = main
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .map(|instruction| &instruction.instruction)
            .collect::<Vec<_>>();
        assert!(instructions
            .iter()
            .any(|instruction| matches!(instruction, NativeMirInst::StackAddress { .. })));
        assert!(instructions
            .iter()
            .any(|instruction| matches!(instruction, NativeMirInst::Store { .. })));
    }
}
