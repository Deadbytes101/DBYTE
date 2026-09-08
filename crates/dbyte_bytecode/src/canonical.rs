//! Flat bytecode whose operands are canonical HIR identities rather than source names.

use dbyte_ast::{BinOp, CallConvention, CaptureMode, MatchMode, Span, UnaryOp};
use dbyte_hir::{
    ClosureId, FieldId, FunctionInstanceId, FunctionInstanceKey, HirIntrinsic, HirLocal,
    HostCapabilityId, IntegerType, LocalId, ModuleId, OwnershipAction, SymbolId, TypeId, VariantId,
};
use std::collections::BTreeMap;
use std::fmt::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmOptimizationLevel {
    O0,
    O1,
}

#[derive(Debug, Clone)]
pub struct CanonicalBytecodeProject {
    pub root: ModuleId,
    pub module_order: Vec<ModuleId>,
    pub symbol_owners: BTreeMap<SymbolId, ModuleId>,
    pub link_names: BTreeMap<SymbolId, String>,
    pub semantic: dbyte_hir::HirProgram,
    pub functions: Vec<CanonicalBytecodeFunction>,
    pub closures: Vec<CanonicalBytecodeClosure>,
    pub globals: Vec<CanonicalBytecodeGlobal>,
    pub entry: CanonicalBytecodeBody,
}

#[derive(Debug, Clone)]
pub struct CanonicalBytecodeFunction {
    pub instance: FunctionInstanceId,
    pub key: FunctionInstanceKey,
    pub declaration_only: bool,
    pub parameters: Vec<HirLocal>,
    pub result: TypeId,
    pub body: CanonicalBytecodeBody,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct CanonicalBytecodeClosure {
    pub closure: ClosureId,
    pub parameters: Vec<HirLocal>,
    pub result: TypeId,
    pub body: CanonicalBytecodeBody,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct CanonicalBytecodeGlobal {
    pub symbol: SymbolId,
    pub mutable: bool,
    pub ty: TypeId,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct CanonicalFrameLayout {
    pub local_count: usize,
    /// Locals whose address may escape the instruction that accesses them.
    pub stable_cells: Vec<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct CanonicalBytecodeBody {
    pub instructions: Vec<CanonicalInstruction>,
    pub frame: CanonicalFrameLayout,
}

#[derive(Debug, Clone)]
pub struct CanonicalInstruction {
    pub opcode: CanonicalOpcode,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum CanonicalOpcode {
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
    PushFunction {
        instance: FunctionInstanceId,
        key: FunctionInstanceKey,
    },
    MakeList(usize),
    Binary {
        op: BinOp,
        operand: CanonicalOperandType,
    },
    Unary {
        op: UnaryOp,
        operand: CanonicalOperandType,
    },
    Cast(TypeId),
    Bitcast {
        source: TypeId,
        target: TypeId,
    },
    Length,
    CallDirect {
        callee: FunctionInstanceId,
        key: FunctionInstanceKey,
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
    ResolvePlace(CanonicalPlace),
    ReadPlace(OwnershipAction),
    BorrowPlace {
        mutable: bool,
    },
    StorePlace,
    ClearPlace,
    StoreLocal(LocalId),
    StoreGlobal(SymbolId),
    Pop,
    Jump(usize),
    JumpIfFalse(usize),
    ResultUnwrapOrJump {
        ok_variant: VariantId,
        err_variant: VariantId,
        error_target: usize,
    },
    IteratorBegin {
        local: LocalId,
        end_target: usize,
    },
    IteratorNext {
        local: LocalId,
        end_target: usize,
    },
    IteratorEnd {
        local: LocalId,
    },
    MatchDispatch {
        mode: MatchMode,
        arms: Vec<CanonicalMatchTarget>,
    },
    MatchRestore,
    MakeClosure {
        closure: ClosureId,
        captures: Vec<CanonicalCapture>,
    },
    Return,
    Halt,
}

#[derive(Debug, Clone)]
pub struct CanonicalMatchTarget {
    pub variant: Option<VariantId>,
    pub bindings: Vec<HirLocal>,
    pub target: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalOperandType {
    Integer(IntegerType),
    Bool,
    Float,
    Pointer,
    String,
    Other(TypeId),
}

impl std::fmt::Display for CanonicalOperandType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Integer(kind) => write!(formatter, "{}", format!("{kind:?}").to_uppercase()),
            Self::Bool => formatter.write_str("BOOL"),
            Self::Float => formatter.write_str("F64"),
            Self::Pointer => formatter.write_str("PTR"),
            Self::String => formatter.write_str("STRING"),
            Self::Other(ty) => write!(formatter, "TYPE{}", ty.0),
        }
    }
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

/// A place descriptor contains no executable expression. Pointer bases and
/// index operands are evaluated before `ResolvePlace` and consumed from the
/// operand stack in source order.
#[derive(Debug, Clone)]
pub struct CanonicalPlace {
    pub base: CanonicalPlaceBase,
    pub projections: Vec<CanonicalProjection>,
    pub ty: TypeId,
    pub mutable: bool,
}

#[derive(Debug, Clone)]
pub enum CanonicalPlaceBase {
    Local(LocalId),
    Global(SymbolId),
    StackPointer,
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
    StackIndex {
        element: TypeId,
        element_size: u32,
    },
}

pub fn disassemble_canonical_bytecode(
    project: &CanonicalBytecodeProject,
    display_name: &str,
) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "== {display_name} ==");
    disassemble_body(project, "entry", &project.entry, &mut output);
    for function in &project.functions {
        let name = project
            .link_names
            .get(&function.key.symbol)
            .cloned()
            .unwrap_or_else(|| format!("symbol#{}", function.key.symbol.0));
        disassemble_body(
            project,
            &format!("fn {name} instance#{}", function.instance.0),
            &function.body,
            &mut output,
        );
    }
    for closure in &project.closures {
        disassemble_body(
            project,
            &format!("closure#{}", closure.closure.0),
            &closure.body,
            &mut output,
        );
    }
    output.truncate(output.trim_end().len());
    output
}

fn disassemble_body(
    project: &CanonicalBytecodeProject,
    name: &str,
    body: &CanonicalBytecodeBody,
    output: &mut String,
) {
    let _ = writeln!(output, "{name} slots={}", body.frame.local_count);
    for (ip, instruction) in body.instructions.iter().enumerate() {
        let _ = writeln!(
            output,
            "{ip:04} {}",
            display_opcode(project, &instruction.opcode)
        );
    }
}

fn display_opcode(project: &CanonicalBytecodeProject, opcode: &CanonicalOpcode) -> String {
    match opcode {
        CanonicalOpcode::PushUnit => "CONST_UNIT".into(),
        CanonicalOpcode::PushInteger(value) => format!("CONST_I64 {value}"),
        CanonicalOpcode::PushLayout { target, alignment } => format!(
            "CONST_{} type#{}",
            if *alignment { "ALIGN" } else { "SIZE" },
            target.0
        ),
        CanonicalOpcode::PushFloat(value) => format!("CONST_F64 {value}"),
        CanonicalOpcode::PushBool(value) => format!("CONST_BOOL {value}"),
        CanonicalOpcode::PushString(value) => format!("CONST_STRING {} bytes", value.len()),
        CanonicalOpcode::PushBytes(value) => format!("CONST_BYTES {} bytes", value.len()),
        CanonicalOpcode::PushNull => "CONST_NULL".into(),
        CanonicalOpcode::LoadLocal { local, action } => {
            format!("LOAD_LOCAL {} {action:?}", local.0)
        }
        CanonicalOpcode::LoadGlobal { symbol, action } => {
            format!(
                "LOAD_GLOBAL {} {action:?}",
                display_symbol(project, *symbol)
            )
        }
        CanonicalOpcode::PushFunction { key, .. } => {
            format!("FUNCTION {}", display_symbol(project, key.symbol))
        }
        CanonicalOpcode::MakeList(count) => format!("MAKE_LIST {count}"),
        CanonicalOpcode::Binary { op, operand } => {
            format!("{}_{operand}", format!("{op:?}").to_uppercase())
        }
        CanonicalOpcode::Unary { op, operand } => format!("UNARY_{op:?}_{operand}"),
        CanonicalOpcode::Cast(target) => format!("CAST type#{}", target.0),
        CanonicalOpcode::Bitcast { source, target } => {
            format!("BITCAST type#{} -> type#{}", source.0, target.0)
        }
        CanonicalOpcode::Length => "LENGTH".into(),
        CanonicalOpcode::CallDirect {
            key,
            convention,
            argument_count,
            ..
        } => format!(
            "CALL_DIRECT {} {convention:?} {argument_count}",
            display_symbol(project, key.symbol)
        ),
        CanonicalOpcode::CallIndirect {
            convention,
            argument_count,
        } => {
            format!("CALL_INDIRECT {convention:?} {argument_count}")
        }
        CanonicalOpcode::Intrinsic {
            intrinsic,
            argument_count,
            ..
        } => {
            format!("INTRINSIC {intrinsic:?} {argument_count}")
        }
        CanonicalOpcode::HostCall {
            capability,
            argument_count,
        } => {
            format!("HOST_CALL {capability:?} {argument_count}")
        }
        CanonicalOpcode::Format { value_count, .. } => format!("FORMAT {value_count}"),
        CanonicalOpcode::MakeStruct { symbol, fields } => {
            format!(
                "MAKE_STRUCT {} {}",
                display_symbol(project, *symbol),
                fields.len()
            )
        }
        CanonicalOpcode::MakeEnum {
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
        CanonicalOpcode::ResolvePlace(place) => format!("RESOLVE_PLACE {}", display_place(place)),
        CanonicalOpcode::ReadPlace(action) => format!("READ_PLACE {action:?}"),
        CanonicalOpcode::BorrowPlace { mutable } => {
            format!("BORROW{}_PLACE", if *mutable { "_MUT" } else { "" })
        }
        CanonicalOpcode::StorePlace => "STORE_PLACE".into(),
        CanonicalOpcode::ClearPlace => "CLEAR_PLACE".into(),
        CanonicalOpcode::StoreLocal(local) => format!("STORE_LOCAL {}", local.0),
        CanonicalOpcode::StoreGlobal(symbol) => format!("STORE_GLOBAL {}", symbol.0),
        CanonicalOpcode::Pop => "POP".into(),
        CanonicalOpcode::Jump(target) => format!("JUMP {target:04}"),
        CanonicalOpcode::JumpIfFalse(target) => format!("JUMP_IF_FALSE {target:04}"),
        CanonicalOpcode::ResultUnwrapOrJump {
            ok_variant,
            err_variant,
            error_target,
        } => {
            format!(
                "RESULT_UNWRAP {} {} ELSE {error_target:04}",
                ok_variant.0, err_variant.0
            )
        }
        CanonicalOpcode::IteratorBegin { local, end_target } => {
            format!("ITER_BEGIN {} {end_target:04}", local.0)
        }
        CanonicalOpcode::IteratorNext { local, end_target } => {
            format!("ITER_NEXT {} {end_target:04}", local.0)
        }
        CanonicalOpcode::IteratorEnd { local } => format!("ITER_END {}", local.0),
        CanonicalOpcode::MatchDispatch { arms, .. } => format!("MATCH_DISPATCH {}", arms.len()),
        CanonicalOpcode::MatchRestore => "MATCH_RESTORE".into(),
        CanonicalOpcode::MakeClosure { closure, captures } => {
            format!("MAKE_CLOSURE {} captures={}", closure.0, captures.len())
        }
        CanonicalOpcode::Return => "RETURN".into(),
        CanonicalOpcode::Halt => "HALT".into(),
    }
}

fn display_symbol(project: &CanonicalBytecodeProject, symbol: SymbolId) -> String {
    project
        .link_names
        .get(&symbol)
        .cloned()
        .unwrap_or_else(|| format!("symbol#{}", symbol.0))
}

fn display_place(place: &CanonicalPlace) -> String {
    let mut text = match place.base {
        CanonicalPlaceBase::Local(local) => format!("local#{}", local.0),
        CanonicalPlaceBase::Global(symbol) => format!("global#{}", symbol.0),
        CanonicalPlaceBase::StackPointer => "stack-pointer".into(),
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
            CanonicalProjection::StackIndex { element_size, .. } => {
                let _ = write!(text, "[stack-index]*{}", element_size);
            }
        }
    }
    text
}
