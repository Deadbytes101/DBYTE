//! Machine-independent typed IR for the native systems profile.

use crate::ir::{CompareOp, LabelId, LocalId, StringId};
use crate::system_types::SystemType;
pub use dbyte_ast::InterruptClass;
use dbyte_ast::Span;
use std::fmt::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Visibility {
    Private,
    Public,
}

impl fmt::Display for Visibility {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Private => formatter.write_str("private"),
            Self::Public => formatter.write_str("public"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GlobalSection {
    ReadOnly,
    Data,
    Bss,
}

impl fmt::Display for GlobalSection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadOnly => formatter.write_str("rodata"),
            Self::Data => formatter.write_str("data"),
            Self::Bss => formatter.write_str("bss"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SystemIntrinsic {
    VolatileLoad8,
    VolatileStore8,
    VolatileLoad16,
    VolatileStore16,
    VolatileLoad32,
    VolatileStore32,
    IoOut8,
    IoIn8,
    IoOut16,
    IoIn16,
    CpuHlt,
    CpuCli,
    CpuSti,
    CpuInt3,
    CpuReadEflags,
    CpuReadCr0,
    CpuWriteCr0,
    CpuReadCr2,
    CpuReadCr3,
    CpuWriteCr3,
    CpuInvlpg,
    CpuLgdt,
    CpuLidt,
    TaskBlockTrap,
    InterruptResume,
}

impl SystemIntrinsic {
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.strip_prefix("native.").unwrap_or(name);
        match name {
            "volatile_load8" => Some(Self::VolatileLoad8),
            "volatile_store8" => Some(Self::VolatileStore8),
            "volatile_load16" => Some(Self::VolatileLoad16),
            "volatile_store16" => Some(Self::VolatileStore16),
            "volatile_load32" => Some(Self::VolatileLoad32),
            "volatile_store32" => Some(Self::VolatileStore32),
            "io_out8" => Some(Self::IoOut8),
            "io_in8" => Some(Self::IoIn8),
            "io_out16" => Some(Self::IoOut16),
            "io_in16" => Some(Self::IoIn16),
            "cpu_hlt" => Some(Self::CpuHlt),
            "cpu_cli" => Some(Self::CpuCli),
            "cpu_sti" => Some(Self::CpuSti),
            "cpu_int3" => Some(Self::CpuInt3),
            "cpu_read_eflags" => Some(Self::CpuReadEflags),
            "cpu_read_cr0" => Some(Self::CpuReadCr0),
            "cpu_write_cr0" => Some(Self::CpuWriteCr0),
            "cpu_read_cr2" => Some(Self::CpuReadCr2),
            "cpu_read_cr3" => Some(Self::CpuReadCr3),
            "cpu_write_cr3" => Some(Self::CpuWriteCr3),
            "cpu_invlpg" => Some(Self::CpuInvlpg),
            "cpu_lgdt" => Some(Self::CpuLgdt),
            "cpu_lidt" => Some(Self::CpuLidt),
            "task_block_trap" => Some(Self::TaskBlockTrap),
            "interrupt_resume" => Some(Self::InterruptResume),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::VolatileLoad8 => "native.volatile_load8",
            Self::VolatileStore8 => "native.volatile_store8",
            Self::VolatileLoad16 => "native.volatile_load16",
            Self::VolatileStore16 => "native.volatile_store16",
            Self::VolatileLoad32 => "native.volatile_load32",
            Self::VolatileStore32 => "native.volatile_store32",
            Self::IoOut8 => "native.io_out8",
            Self::IoIn8 => "native.io_in8",
            Self::IoOut16 => "native.io_out16",
            Self::IoIn16 => "native.io_in16",
            Self::CpuHlt => "native.cpu_hlt",
            Self::CpuCli => "native.cpu_cli",
            Self::CpuSti => "native.cpu_sti",
            Self::CpuInt3 => "native.cpu_int3",
            Self::CpuReadEflags => "native.cpu_read_eflags",
            Self::CpuReadCr0 => "native.cpu_read_cr0",
            Self::CpuWriteCr0 => "native.cpu_write_cr0",
            Self::CpuReadCr2 => "native.cpu_read_cr2",
            Self::CpuReadCr3 => "native.cpu_read_cr3",
            Self::CpuWriteCr3 => "native.cpu_write_cr3",
            Self::CpuInvlpg => "native.cpu_invlpg",
            Self::CpuLgdt => "native.cpu_lgdt",
            Self::CpuLidt => "native.cpu_lidt",
            Self::TaskBlockTrap => "native.task_block_trap",
            Self::InterruptResume => "native.interrupt_resume",
        }
    }

    pub fn parameter_types(self) -> Vec<SystemType> {
        match self {
            Self::VolatileLoad8 => vec![SystemType::Pointer(Box::new(SystemType::U8))],
            Self::VolatileStore8 => vec![
                SystemType::Pointer(Box::new(SystemType::U8)),
                SystemType::U8,
            ],
            Self::VolatileLoad16 => vec![SystemType::Pointer(Box::new(SystemType::U16))],
            Self::VolatileStore16 => vec![
                SystemType::Pointer(Box::new(SystemType::U16)),
                SystemType::U16,
            ],
            Self::VolatileLoad32 => vec![SystemType::Pointer(Box::new(SystemType::U32))],
            Self::VolatileStore32 => vec![
                SystemType::Pointer(Box::new(SystemType::U32)),
                SystemType::U32,
            ],
            Self::IoOut8 => vec![SystemType::U16, SystemType::U8],
            Self::IoIn8 => vec![SystemType::U16],
            Self::IoOut16 => vec![SystemType::U16, SystemType::U16],
            Self::IoIn16 => vec![SystemType::U16],
            Self::CpuHlt
            | Self::CpuCli
            | Self::CpuSti
            | Self::CpuInt3
            | Self::CpuReadEflags
            | Self::CpuReadCr0
            | Self::CpuReadCr2
            | Self::CpuReadCr3 => Vec::new(),
            Self::CpuWriteCr0 => vec![SystemType::U32],
            Self::CpuWriteCr3 => vec![SystemType::Usize],
            Self::CpuInvlpg => vec![SystemType::Pointer(Box::new(SystemType::U8))],
            Self::CpuLgdt | Self::CpuLidt => vec![SystemType::Usize],
            Self::TaskBlockTrap => vec![SystemType::U32, SystemType::Usize],
            Self::InterruptResume => vec![SystemType::Usize],
        }
    }

    pub fn result_type(self) -> Option<SystemType> {
        match self {
            Self::VolatileLoad8 | Self::IoIn8 => Some(SystemType::U8),
            Self::VolatileLoad16 | Self::IoIn16 => Some(SystemType::U16),
            Self::VolatileLoad32 => Some(SystemType::U32),
            Self::VolatileStore8
            | Self::VolatileStore16
            | Self::VolatileStore32
            | Self::IoOut8
            | Self::IoOut16
            | Self::CpuHlt
            | Self::CpuCli
            | Self::CpuSti
            | Self::CpuInt3
            | Self::CpuWriteCr0
            | Self::CpuWriteCr3
            | Self::CpuInvlpg
            | Self::CpuLgdt
            | Self::CpuLidt => None,
            Self::TaskBlockTrap => Some(SystemType::I32),
            Self::InterruptResume => None,
            Self::CpuReadEflags | Self::CpuReadCr0 => Some(SystemType::U32),
            Self::CpuReadCr2 | Self::CpuReadCr3 => Some(SystemType::Usize),
        }
    }
}

impl fmt::Display for SystemIntrinsic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SystemsModuleIr {
    pub module_name: String,
    pub functions: Vec<SystemsFunction>,
    pub globals: Vec<SystemsGlobal>,
    pub strings: Vec<Vec<u8>>,
    pub external_functions: Vec<ExternalFunction>,
    pub external_globals: Vec<ExternalGlobal>,
}

impl SystemsModuleIr {
    pub fn function(&self, symbol: &str) -> Option<&SystemsFunction> {
        self.functions
            .iter()
            .find(|function| function.symbol == symbol)
    }

    pub fn function_signature(&self, symbol: &str) -> Option<(&[SystemType], &SystemType)> {
        if let Some(function) = self.function(symbol) {
            return Some((&function.parameter_types, &function.return_type));
        }
        self.external_functions
            .iter()
            .find(|function| function.symbol == symbol)
            .map(|function| (function.parameter_types.as_slice(), &function.return_type))
    }

    pub fn dump(&self) -> String {
        let mut output = String::new();
        let _ = writeln!(output, "native-systems-ir module {}", self.module_name);
        for external in &self.external_functions {
            let _ = write!(output, "extern fn {}(", external.symbol);
            write_types(&mut output, &external.parameter_types);
            let _ = writeln!(output, ") -> {}", external.return_type);
        }
        for external in &self.external_globals {
            let _ = writeln!(
                output,
                "extern static {}: {} mutable={}",
                external.symbol, external.ty, external.mutable
            );
        }
        for global in &self.globals {
            let _ = writeln!(
                output,
                "{} static {}: {} section={} size={} align={} mutable={}",
                global.visibility,
                global.symbol,
                global.ty,
                global.section,
                global.size,
                global.alignment,
                global.mutable
            );
        }
        for (index, bytes) in self.strings.iter().enumerate() {
            let _ = writeln!(output, "private string @str{index} bytes={}", bytes.len());
        }
        for function in &self.functions {
            let _ = write!(output, "{} fn {}(", function.visibility, function.symbol);
            write_types(&mut output, &function.parameter_types);
            let _ = writeln!(output, ") -> {} {{", function.return_type);
            for slot in &function.slots {
                let _ = writeln!(
                    output,
                    "  slot {} {}: {} size={} align={} param={}",
                    slot.id,
                    slot.name,
                    slot.ty,
                    slot.storage_size,
                    slot.alignment,
                    slot.is_parameter
                );
            }
            for (index, instruction) in function.instructions.iter().enumerate() {
                let _ = writeln!(output, "  {index:04} {instruction}");
            }
            let _ = writeln!(output, "}}");
        }
        output
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalFunction {
    pub symbol: String,
    pub parameter_types: Vec<SystemType>,
    pub return_type: SystemType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalGlobal {
    pub symbol: String,
    pub ty: SystemType,
    pub mutable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SystemsFunction {
    pub source_name: String,
    pub symbol: String,
    pub visibility: Visibility,
    pub interrupt: Option<InterruptClass>,
    pub parameter_types: Vec<SystemType>,
    pub params: Vec<SystemsParam>,
    pub return_type: SystemType,
    pub slots: Vec<SystemsSlot>,
    pub instructions: Vec<SystemsInst>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemsParam {
    pub name: String,
    pub ty: SystemType,
    pub slot: LocalId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemsSlot {
    pub id: LocalId,
    pub name: String,
    pub ty: SystemType,
    pub storage_size: u32,
    pub alignment: u32,
    pub is_parameter: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SystemsGlobal {
    pub source_name: String,
    pub symbol: String,
    pub visibility: Visibility,
    pub ty: SystemType,
    pub mutable: bool,
    pub section: GlobalSection,
    pub bytes: Vec<u8>,
    pub size: u32,
    pub alignment: u32,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemsInst {
    Const {
        bits: u32,
        ty: SystemType,
    },
    ConstBool(bool),
    StringAddress(StringId),
    LocalAddress(LocalId),
    LocalLoad {
        local: LocalId,
        ty: SystemType,
    },
    LocalStore {
        local: LocalId,
        ty: SystemType,
    },
    GlobalAddress {
        symbol: String,
        pointee: SystemType,
    },
    Load(SystemType),
    Store(SystemType),
    Add(SystemType),
    Sub(SystemType),
    Mul(SystemType),
    Div(SystemType),
    Rem(SystemType),
    Neg(SystemType),
    BitAnd(SystemType),
    BitOr(SystemType),
    BitXor(SystemType),
    BitNot(SystemType),
    ShiftLeft(SystemType),
    ShiftRight(SystemType),
    Cast {
        from: SystemType,
        to: SystemType,
    },
    BoolNot,
    Compare {
        op: CompareOp,
        operand_type: SystemType,
    },
    IndexAddress {
        element_size: u32,
    },
    FieldAddress {
        offset: u32,
    },
    Label(LabelId),
    Jump(LabelId),
    JumpIfFalse(LabelId),
    Call {
        symbol: String,
        argument_count: usize,
        result: SystemType,
    },
    InterruptAddress {
        vector: u8,
        symbol: String,
        class: InterruptClass,
    },
    TaskAddress {
        symbol: String,
    },
    LinkerAddress {
        symbol: String,
    },
    ReloadSegments {
        code_selector: u16,
        data_selector: u16,
    },
    Intrinsic {
        intrinsic: SystemIntrinsic,
        argument_count: usize,
        result: Option<SystemType>,
    },
    Pop,
    Return,
}

impl fmt::Display for SystemsInst {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Const { bits, ty } => write!(formatter, "const.{ty} 0x{bits:08x}"),
            Self::ConstBool(value) => write!(formatter, "const.bool {value}"),
            Self::StringAddress(id) => write!(formatter, "address.string {id}"),
            Self::LocalAddress(id) => write!(formatter, "address.local {id}"),
            Self::LocalLoad { local, ty } => write!(formatter, "local.load.{ty} {local}"),
            Self::LocalStore { local, ty } => write!(formatter, "local.store.{ty} {local}"),
            Self::GlobalAddress { symbol, pointee } => {
                write!(formatter, "address.global {symbol}: ptr[{pointee}]")
            }
            Self::Load(ty) => write!(formatter, "load.{ty}"),
            Self::Store(ty) => write!(formatter, "store.{ty}"),
            Self::Add(ty) => write!(formatter, "add.{ty}"),
            Self::Sub(ty) => write!(formatter, "sub.{ty}"),
            Self::Mul(ty) => write!(formatter, "mul.{ty}"),
            Self::Div(ty) => write!(formatter, "div.{ty}"),
            Self::Rem(ty) => write!(formatter, "rem.{ty}"),
            Self::Neg(ty) => write!(formatter, "neg.{ty}"),
            Self::BitAnd(ty) => write!(formatter, "and.{ty}"),
            Self::BitOr(ty) => write!(formatter, "or.{ty}"),
            Self::BitXor(ty) => write!(formatter, "xor.{ty}"),
            Self::BitNot(ty) => write!(formatter, "bitnot.{ty}"),
            Self::ShiftLeft(ty) => write!(formatter, "shl.{ty}"),
            Self::ShiftRight(ty) => write!(formatter, "shr.{ty}"),
            Self::Cast { from, to } => write!(formatter, "cast {from} -> {to}"),
            Self::BoolNot => formatter.write_str("not.bool"),
            Self::Compare { op, operand_type } => {
                write!(formatter, "compare.{op}.{operand_type}")
            }
            Self::IndexAddress { element_size } => {
                write!(formatter, "address.index scale={element_size}")
            }
            Self::FieldAddress { offset } => write!(formatter, "address.field +{offset}"),
            Self::Label(label) => write!(formatter, "label {label}"),
            Self::Jump(label) => write!(formatter, "jump {label}"),
            Self::JumpIfFalse(label) => write!(formatter, "jump_if_false {label}"),
            Self::Call {
                symbol,
                argument_count,
                result,
            } => write!(formatter, "call {symbol} {argument_count} -> {result}"),
            Self::InterruptAddress {
                vector,
                symbol,
                class,
            } => write!(
                formatter,
                "interrupt.address vector={vector} {symbol} class={class}"
            ),
            Self::TaskAddress { symbol } => write!(formatter, "task.address {symbol}"),
            Self::LinkerAddress { symbol } => write!(formatter, "linker.address {symbol}"),
            Self::ReloadSegments {
                code_selector,
                data_selector,
            } => write!(
                formatter,
                "cpu.reload_segments cs=0x{code_selector:04x} ds=0x{data_selector:04x}"
            ),
            Self::Intrinsic {
                intrinsic,
                argument_count,
                result,
            } => {
                write!(formatter, "intrinsic {intrinsic} {argument_count}")?;
                if let Some(result) = result {
                    write!(formatter, " -> {result}")?;
                }
                Ok(())
            }
            Self::Pop => formatter.write_str("pop"),
            Self::Return => formatter.write_str("return"),
        }
    }
}

fn write_types(output: &mut String, types: &[SystemType]) {
    for (index, ty) in types.iter().enumerate() {
        if index != 0 {
            let _ = output.write_str(", ");
        }
        let _ = write!(output, "{ty}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_exposes_widths_addresses_aggregates_and_external_symbols() {
        let module = SystemsModuleIr {
            module_name: "memory".into(),
            functions: vec![SystemsFunction {
                source_name: "check".into(),
                symbol: "check".into(),
                visibility: Visibility::Public,
                interrupt: None,
                parameter_types: vec![],
                params: vec![],
                return_type: SystemType::U32,
                slots: vec![SystemsSlot {
                    id: LocalId(0),
                    name: "bytes".into(),
                    ty: SystemType::Array(Box::new(SystemType::U8), 4),
                    storage_size: 4,
                    alignment: 1,
                    is_parameter: false,
                }],
                instructions: vec![
                    SystemsInst::LocalAddress(LocalId(0)),
                    SystemsInst::Const {
                        bits: 1,
                        ty: SystemType::Usize,
                    },
                    SystemsInst::IndexAddress { element_size: 1 },
                    SystemsInst::Load(SystemType::U8),
                    SystemsInst::BitNot(SystemType::U8),
                    SystemsInst::Return,
                ],
                span: Span::new(1, 1),
            }],
            globals: vec![],
            strings: vec![],
            external_functions: vec![ExternalFunction {
                symbol: "add".into(),
                parameter_types: vec![SystemType::U32, SystemType::U32],
                return_type: SystemType::U32,
            }],
            external_globals: vec![],
        };
        let dump = module.dump();
        assert!(dump.contains("extern fn add(u32, u32) -> u32"));
        assert!(dump.contains("array[u8, 4]"));
        assert!(dump.contains("address.index scale=1"));
        assert!(dump.contains("bitnot.u8"));
    }
}
