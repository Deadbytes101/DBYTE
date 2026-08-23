use crate::types::{Intrinsic, NativeType};
use dbyte_ast::Span;
use std::fmt::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LabelId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StringId(pub u32);

impl LocalId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl LabelId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl StringId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for LocalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%l{}", self.0)
    }
}

impl fmt::Display for LabelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bb{}", self.0)
    }
}

impl fmt::Display for StringId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@str{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompareOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
}

impl fmt::Display for CompareOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let op = match self {
            Self::Eq => "eq",
            Self::NotEq => "ne",
            Self::Lt => "lt",
            Self::LtEq => "le",
            Self::Gt => "gt",
            Self::GtEq => "ge",
        };
        f.write_str(op)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrProgram {
    pub functions: Vec<IrFunction>,
    /// Deduplicated ASCII string bytes in deterministic first-use order.
    pub strings: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IrFunction {
    pub name: String,
    pub params: Vec<IrParam>,
    pub return_type: NativeType,
    pub slots: Vec<IrSlot>,
    pub instructions: Vec<IrInst>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrParam {
    pub name: String,
    pub ty: NativeType,
    pub slot: LocalId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrSlot {
    pub name: String,
    pub ty: NativeType,
    pub is_param: bool,
}

/// Typed stack IR. Expression instructions leave their value on the evaluation
/// stack. Stores, conditional jumps, pops, and returns consume a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrInst {
    ConstI32(i32),
    ConstBool(bool),
    ConstStr(StringId),
    LocalLoad(LocalId),
    LocalStore(LocalId),
    AddI32,
    SubI32,
    MulI32,
    DivI32,
    RemI32,
    NegI32,
    BoolNot,
    Compare(CompareOp),
    StrIndex8,
    Label(LabelId),
    Jump(LabelId),
    JumpIfFalse(LabelId),
    Call {
        function: String,
        argument_count: usize,
        result: NativeType,
    },
    Intrinsic {
        intrinsic: Intrinsic,
        argument_count: usize,
        result: Option<NativeType>,
    },
    Pop,
    Return,
}

impl IrProgram {
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
            strings: Vec::new(),
        }
    }

    pub fn function(&self, name: &str) -> Option<&IrFunction> {
        self.functions.iter().find(|function| function.name == name)
    }

    pub fn dump(&self) -> String {
        let mut output = String::new();
        let _ = writeln!(output, "native-ir");
        let _ = writeln!(output, "strings {}", self.strings.len());
        for (index, bytes) in self.strings.iter().enumerate() {
            let _ = writeln!(output, "  @str{} = \"{}\"", index, escape_bytes(bytes));
        }
        let _ = writeln!(output, "functions {}", self.functions.len());
        for function in &self.functions {
            let _ = write!(output, "fn {}(", function.name);
            for (index, param) in function.params.iter().enumerate() {
                if index > 0 {
                    let _ = write!(output, ", ");
                }
                let _ = write!(output, "{}: {} {}", param.name, param.ty, param.slot);
            }
            let _ = writeln!(output, ") -> {} {{", function.return_type);
            for (index, slot) in function.slots.iter().enumerate() {
                let role = if slot.is_param { "param" } else { "local" };
                let _ = writeln!(
                    output,
                    "  slot %l{} {}: {} [{}]",
                    index, slot.name, slot.ty, role
                );
            }
            for (index, instruction) in function.instructions.iter().enumerate() {
                let _ = writeln!(output, "  {:04} {}", index, instruction);
            }
            let _ = writeln!(output, "}}");
        }
        output
    }
}

impl Default for IrProgram {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for IrProgram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.dump())
    }
}

impl fmt::Display for IrInst {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConstI32(value) => write!(f, "const.i32 {}", value),
            Self::ConstBool(value) => write!(f, "const.bool {}", value),
            Self::ConstStr(id) => write!(f, "const.str {}", id),
            Self::LocalLoad(id) => write!(f, "local.load {}", id),
            Self::LocalStore(id) => write!(f, "local.store {}", id),
            Self::AddI32 => f.write_str("add.i32"),
            Self::SubI32 => f.write_str("sub.i32"),
            Self::MulI32 => f.write_str("mul.i32"),
            Self::DivI32 => f.write_str("div.i32"),
            Self::RemI32 => f.write_str("rem.i32"),
            Self::NegI32 => f.write_str("neg.i32"),
            Self::BoolNot => f.write_str("not.bool"),
            Self::Compare(op) => write!(f, "compare.{}", op),
            Self::StrIndex8 => f.write_str("str.index8"),
            Self::Label(label) => write!(f, "label {}", label),
            Self::Jump(label) => write!(f, "jump {}", label),
            Self::JumpIfFalse(label) => write!(f, "jump_if_false {}", label),
            Self::Call {
                function,
                argument_count,
                result,
            } => write!(f, "call {} {} -> {}", function, argument_count, result),
            Self::Intrinsic {
                intrinsic,
                argument_count,
                result,
            } => {
                write!(f, "intrinsic {} {}", intrinsic, argument_count)?;
                if let Some(result) = result {
                    write!(f, " -> {}", result)?;
                }
                Ok(())
            }
            Self::Pop => f.write_str("pop"),
            Self::Return => f.write_str("return"),
        }
    }
}

fn escape_bytes(bytes: &[u8]) -> String {
    let mut escaped = String::new();
    for byte in bytes {
        for part in std::ascii::escape_default(*byte) {
            escaped.push(part as char);
        }
    }
    escaped
}
