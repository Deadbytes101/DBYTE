//! Bytecode whose operands are canonical HIR identities rather than source names.

use dbyte_ast::{BinOp, CallConvention, CaptureMode, MatchMode, Span, UnaryOp};
use dbyte_hir::{
    ClosureId, FieldId, FunctionInstanceId, FunctionInstanceKey, HirIntrinsic, HirLocal,
    HostCapabilityId, LocalId, ModuleId, OwnershipAction, SymbolId, TypeId, VariantId,
};
use std::collections::BTreeMap;
use std::fmt::Write;

#[derive(Debug, Clone)]
pub struct CanonicalBytecodeProject {
    pub root: ModuleId,
    pub module_order: Vec<ModuleId>,
    pub symbol_owners: BTreeMap<SymbolId, ModuleId>,
    pub link_names: BTreeMap<SymbolId, String>,
    pub semantic: dbyte_hir::HirProgram,
    pub functions: Vec<CanonicalBytecodeFunction>,
    pub globals: Vec<CanonicalBytecodeGlobal>,
    pub top_level: CanonicalBytecodeBlock,
}

#[derive(Debug, Clone)]
pub struct CanonicalBytecodeFunction {
    pub instance: FunctionInstanceId,
    pub key: FunctionInstanceKey,
    pub declaration_only: bool,
    pub parameters: Vec<HirLocal>,
    pub locals: Vec<HirLocal>,
    pub result: TypeId,
    pub body: CanonicalBytecodeBlock,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct CanonicalBytecodeGlobal {
    pub symbol: SymbolId,
    pub mutable: bool,
    pub ty: TypeId,
    pub initializer: Option<CanonicalExprCode>,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct CanonicalBytecodeBlock {
    pub statements: Vec<CanonicalBytecodeStatement>,
}

#[derive(Debug, Clone)]
pub enum CanonicalBytecodeStatement {
    Let {
        local: LocalId,
        value: CanonicalExprCode,
        span: Span,
    },
    Assign {
        place: CanonicalPlace,
        value: CanonicalExprCode,
        cleanup_before_commit: Option<CanonicalBytecodeBlock>,
        span: Span,
    },
    Expression(CanonicalExprCode),
    Return {
        value: Option<CanonicalExprCode>,
        cleanup: Vec<CanonicalBytecodeBlock>,
        span: Span,
    },
    If {
        condition: CanonicalExprCode,
        then_block: CanonicalBytecodeBlock,
        else_block: Option<CanonicalBytecodeBlock>,
        span: Span,
    },
    While {
        condition: CanonicalExprCode,
        body: CanonicalBytecodeBlock,
        span: Span,
    },
    For {
        local: HirLocal,
        iterable: CanonicalExprCode,
        body: CanonicalBytecodeBlock,
        span: Span,
    },
    Match {
        mode: MatchMode,
        value: CanonicalExprCode,
        arms: Vec<CanonicalMatchArm>,
        span: Span,
    },
    RegisterDefer,
    Break {
        cleanup: Vec<CanonicalBytecodeBlock>,
        span: Span,
    },
    Continue {
        cleanup: Vec<CanonicalBytecodeBlock>,
        span: Span,
    },
}

#[derive(Debug, Clone)]
pub struct CanonicalMatchArm {
    pub variant: Option<VariantId>,
    pub bindings: Vec<HirLocal>,
    pub body: CanonicalBytecodeBlock,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct CanonicalExprCode {
    pub ty: TypeId,
    pub span: Span,
    pub ops: Vec<CanonicalOp>,
}

#[derive(Debug, Clone)]
pub enum CanonicalOp {
    PushUnit,
    PushInteger(i64),
    PushLayout {
        target: TypeId,
        alignment: bool,
    },
    PushFloat(f64),
    PushBool(bool),
    PushString(Vec<u8>),
    PushBytes(Vec<u8>),
    PushNull,
    LoadLocal {
        local: LocalId,
        action: OwnershipAction,
    },
    LoadGlobal {
        symbol: SymbolId,
        action: OwnershipAction,
    },
    PushFunction(FunctionInstanceKey),
    MakeList(usize),
    Binary(BinOp),
    Unary(UnaryOp),
    Cast(TypeId),
    Length,
    CallDirect {
        callee: FunctionInstanceKey,
        convention: CallConvention,
        argument_count: usize,
    },
    CallIndirect {
        convention: CallConvention,
        argument_count: usize,
    },
    Intrinsic {
        intrinsic: HirIntrinsic,
        argument_count: usize,
        result: TypeId,
    },
    HostCall {
        capability: HostCapabilityId,
        argument_count: usize,
    },
    Format {
        parts: Vec<CanonicalFormatPart>,
        value_count: usize,
    },
    MakeStruct {
        symbol: SymbolId,
        fields: Vec<FieldId>,
    },
    MakeEnum {
        symbol: SymbolId,
        variant: VariantId,
        discriminant: u32,
        payload_count: usize,
    },
    LoadPlace {
        place: CanonicalPlace,
        action: OwnershipAction,
    },
    BorrowPlace {
        place: CanonicalPlace,
        mutable: bool,
    },
    PropagateResult {
        ok_variant: VariantId,
        err_variant: VariantId,
        cleanup: Vec<CanonicalBytecodeBlock>,
    },
    DropPlace {
        place: CanonicalPlace,
        cleanup: CanonicalBytecodeBlock,
    },
    MakeClosure {
        closure: ClosureId,
        captures: Vec<CanonicalCapture>,
        parameters: Vec<HirLocal>,
        result: TypeId,
        body: CanonicalBytecodeBlock,
    },
}

#[derive(Debug, Clone)]
pub enum CanonicalFormatPart {
    Literal(Vec<u8>),
    Value,
}

#[derive(Debug, Clone)]
pub struct CanonicalCapture {
    pub local: LocalId,
    pub mode: CaptureMode,
    pub ty: TypeId,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct CanonicalPlace {
    pub base: CanonicalPlaceBase,
    pub projections: Vec<CanonicalProjection>,
    pub ty: TypeId,
    pub mutable: bool,
    pub span: Span,
}

/// Render the executable canonical bytecode in a compact, stable form.
///
/// This is intentionally a view over canonical identities and operations.  It
/// must never recover names or semantic decisions from the source AST.
pub fn disassemble_canonical_bytecode(
    project: &CanonicalBytecodeProject,
    display_name: &str,
) -> String {
    let mut output = String::new();
    let mut instruction = 0usize;
    let _ = writeln!(output, "== {display_name} ==");
    disassemble_block(project, &project.top_level, &mut instruction, &mut output);
    emit(&mut output, &mut instruction, "HALT");
    for function in &project.functions {
        let name = project
            .link_names
            .get(&function.key.symbol)
            .cloned()
            .unwrap_or_else(|| format!("symbol#{}", function.key.symbol.0));
        let _ = writeln!(output, "fn {name} instance#{}", function.instance.0);
        disassemble_block(project, &function.body, &mut instruction, &mut output);
        if !block_is_terminal(&function.body) {
            emit(&mut output, &mut instruction, "RETURN_UNIT");
        }
    }
    output.truncate(output.trim_end().len());
    output
}

fn block_is_terminal(block: &CanonicalBytecodeBlock) -> bool {
    matches!(
        block.statements.last(),
        Some(CanonicalBytecodeStatement::Return { .. })
            | Some(CanonicalBytecodeStatement::Break { .. })
            | Some(CanonicalBytecodeStatement::Continue { .. })
    )
}

fn emit(output: &mut String, instruction: &mut usize, text: impl AsRef<str>) {
    let _ = writeln!(output, "{:04} {}", *instruction, text.as_ref());
    *instruction += 1;
}

fn disassemble_block(
    project: &CanonicalBytecodeProject,
    block: &CanonicalBytecodeBlock,
    instruction: &mut usize,
    output: &mut String,
) {
    for statement in &block.statements {
        match statement {
            CanonicalBytecodeStatement::Let { local, value, .. } => {
                disassemble_expr(project, value, instruction, output);
                emit(
                    output,
                    instruction,
                    format!("STORE_LOCAL {} ; {}", local.0, local_name(project, *local)),
                );
            }
            CanonicalBytecodeStatement::Assign {
                place,
                value,
                cleanup_before_commit,
                ..
            } => {
                disassemble_expr(project, value, instruction, output);
                if let Some(cleanup) = cleanup_before_commit {
                    emit(output, instruction, "CLEANUP_OLD_BEGIN");
                    disassemble_block(project, cleanup, instruction, output);
                    emit(output, instruction, "CLEANUP_OLD_END");
                }
                emit(
                    output,
                    instruction,
                    format!("STORE_PLACE {}", display_place(project, place)),
                );
            }
            CanonicalBytecodeStatement::Expression(value) => {
                disassemble_expr(project, value, instruction, output);
                emit(output, instruction, "POP");
            }
            CanonicalBytecodeStatement::Return { value, cleanup, .. } => {
                if let Some(value) = value {
                    disassemble_expr(project, value, instruction, output);
                }
                for block in cleanup {
                    disassemble_block(project, block, instruction, output);
                }
                emit(
                    output,
                    instruction,
                    if value.is_some() {
                        "RETURN"
                    } else {
                        "RETURN_UNIT"
                    },
                );
            }
            CanonicalBytecodeStatement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                disassemble_expr(project, condition, instruction, output);
                emit(output, instruction, "IF_BEGIN");
                disassemble_block(project, then_block, instruction, output);
                if let Some(else_block) = else_block {
                    emit(output, instruction, "ELSE_BEGIN");
                    disassemble_block(project, else_block, instruction, output);
                }
                emit(output, instruction, "IF_END");
            }
            CanonicalBytecodeStatement::While {
                condition, body, ..
            } => {
                emit(output, instruction, "WHILE_BEGIN");
                disassemble_expr(project, condition, instruction, output);
                emit(output, instruction, "WHILE_CONDITION");
                disassemble_block(project, body, instruction, output);
                emit(output, instruction, "WHILE_END");
            }
            CanonicalBytecodeStatement::For {
                local,
                iterable,
                body,
                ..
            } => {
                disassemble_expr(project, iterable, instruction, output);
                emit(
                    output,
                    instruction,
                    format!("FOR_BEGIN {} ; {}", local.id.0, local.name),
                );
                disassemble_block(project, body, instruction, output);
                emit(output, instruction, "FOR_END");
            }
            CanonicalBytecodeStatement::Match { value, arms, .. } => {
                disassemble_expr(project, value, instruction, output);
                emit(output, instruction, "MATCH_TAG");
                for arm in arms {
                    emit(
                        output,
                        instruction,
                        arm.variant.map_or_else(
                            || "MATCH_DEFAULT".to_string(),
                            |variant| format!("MATCH_VARIANT {}", variant.0),
                        ),
                    );
                    disassemble_block(project, &arm.body, instruction, output);
                }
                emit(output, instruction, "MATCH_END");
            }
            CanonicalBytecodeStatement::RegisterDefer => {
                emit(output, instruction, "REGISTER_DEFER")
            }
            CanonicalBytecodeStatement::Break { cleanup, .. } => {
                for block in cleanup {
                    disassemble_block(project, block, instruction, output);
                }
                emit(output, instruction, "BREAK");
            }
            CanonicalBytecodeStatement::Continue { cleanup, .. } => {
                for block in cleanup {
                    disassemble_block(project, block, instruction, output);
                }
                emit(output, instruction, "CONTINUE");
            }
        }
    }
}

fn disassemble_expr(
    project: &CanonicalBytecodeProject,
    expression: &CanonicalExprCode,
    instruction: &mut usize,
    output: &mut String,
) {
    for op in &expression.ops {
        let text = match op {
            CanonicalOp::PushUnit => "CONST_UNIT".to_string(),
            CanonicalOp::PushInteger(value) => format!("CONST_I64 {value}"),
            CanonicalOp::PushLayout { target, alignment } => format!(
                "CONST_{} type#{}",
                if *alignment { "ALIGN" } else { "SIZE" },
                target.0
            ),
            CanonicalOp::PushFloat(value) => format!("CONST_F64 {value}"),
            CanonicalOp::PushBool(value) => format!("CONST_BOOL {value}"),
            CanonicalOp::PushString(value) => format!("CONST_STRING {} bytes", value.len()),
            CanonicalOp::PushBytes(value) => format!("CONST_BYTES {} bytes", value.len()),
            CanonicalOp::PushNull => "CONST_NULL".to_string(),
            CanonicalOp::LoadLocal { local, action } => format!(
                "LOAD_LOCAL {} {:?} ; {}",
                local.0,
                action,
                local_name(project, *local)
            ),
            CanonicalOp::LoadGlobal { symbol, action } => {
                format!(
                    "LOAD_GLOBAL {} {:?}",
                    display_symbol(project, *symbol),
                    action
                )
            }
            CanonicalOp::PushFunction(key) => {
                format!("FUNCTION {}", display_symbol(project, key.symbol))
            }
            CanonicalOp::MakeList(count) => format!("MAKE_LIST {count}"),
            CanonicalOp::Binary(op) => format!("{}_I64", format!("{op:?}").to_uppercase()),
            CanonicalOp::Unary(op) => format!("UNARY_{op:?}"),
            CanonicalOp::Cast(target) => format!("CAST type#{}", target.0),
            CanonicalOp::Length => "LENGTH".to_string(),
            CanonicalOp::CallDirect {
                callee,
                convention,
                argument_count,
            } => format!(
                "CALL_DIRECT {} {:?} {}",
                display_symbol(project, callee.symbol),
                convention,
                argument_count
            ),
            CanonicalOp::CallIndirect {
                convention,
                argument_count,
            } => format!("CALL_INDIRECT {:?} {}", convention, argument_count),
            CanonicalOp::Intrinsic {
                intrinsic,
                argument_count,
                ..
            } => format!("INTRINSIC {intrinsic:?} {argument_count}"),
            CanonicalOp::HostCall {
                capability,
                argument_count,
            } => {
                if *capability == HostCapabilityId::Print {
                    format!("CALL print {argument_count}")
                } else {
                    format!("HOST_CALL {capability:?} {argument_count}")
                }
            }
            CanonicalOp::Format { value_count, .. } => format!("FORMAT {value_count}"),
            CanonicalOp::MakeStruct { symbol, fields } => format!(
                "MAKE_STRUCT {} {}",
                display_symbol(project, *symbol),
                fields.len()
            ),
            CanonicalOp::MakeEnum {
                symbol,
                variant,
                discriminant,
                payload_count,
            } => format!(
                "MAKE_ENUM {} variant#{} tag={} payloads={}",
                display_symbol(project, *symbol),
                variant.0,
                discriminant,
                payload_count
            ),
            CanonicalOp::LoadPlace { place, action } => {
                format!("LOAD_PLACE {} {:?}", display_place(project, place), action)
            }
            CanonicalOp::BorrowPlace { place, mutable } => format!(
                "BORROW{}_PLACE {}",
                if *mutable { "_MUT" } else { "" },
                display_place(project, place)
            ),
            CanonicalOp::PropagateResult {
                ok_variant,
                err_variant,
                ..
            } => format!("PROPAGATE_RESULT {} {}", ok_variant.0, err_variant.0),
            CanonicalOp::DropPlace { place, .. } => {
                format!("DROP_PLACE {}", display_place(project, place))
            }
            CanonicalOp::MakeClosure {
                closure, captures, ..
            } => format!("MAKE_CLOSURE {} captures={}", closure.0, captures.len()),
        };
        emit(output, instruction, text);
    }
}

fn local_name(project: &CanonicalBytecodeProject, local: LocalId) -> &str {
    project
        .functions
        .iter()
        .flat_map(|function| function.parameters.iter().chain(function.locals.iter()))
        .find(|candidate| candidate.id == local)
        .map_or("local", |candidate| candidate.name.as_str())
}

fn display_symbol(project: &CanonicalBytecodeProject, symbol: SymbolId) -> String {
    project
        .link_names
        .get(&symbol)
        .cloned()
        .unwrap_or_else(|| format!("symbol#{}", symbol.0))
}

fn display_place(project: &CanonicalBytecodeProject, place: &CanonicalPlace) -> String {
    let mut text = match &place.base {
        CanonicalPlaceBase::Local(local) => {
            format!("local#{}({})", local.0, local_name(project, *local))
        }
        CanonicalPlaceBase::Global(symbol) => display_symbol(project, *symbol),
        CanonicalPlaceBase::Pointer(_) => "pointer".to_string(),
    };
    for projection in &place.projections {
        match projection {
            CanonicalProjection::Field { field, offset, .. } => {
                let _ = write!(text, ".field#{}@{}", field.0, offset);
            }
            CanonicalProjection::Payload {
                variant,
                index,
                offset,
                ..
            } => {
                let _ = write!(text, ".payload#{}:{}@{}", variant.0, index, offset);
            }
            CanonicalProjection::Index { element_size, .. } => {
                let _ = write!(text, "[index]*{}", element_size);
            }
        }
    }
    text
}

#[derive(Debug, Clone)]
pub enum CanonicalPlaceBase {
    Local(LocalId),
    Global(SymbolId),
    Pointer(Box<CanonicalExprCode>),
}

#[derive(Debug, Clone)]
pub enum CanonicalProjection {
    Field {
        field: FieldId,
        ty: TypeId,
        offset: u32,
    },
    Payload {
        variant: VariantId,
        index: u32,
        ty: TypeId,
        offset: u32,
    },
    Index {
        index: CanonicalExprCode,
        element: TypeId,
        element_size: u32,
    },
}
