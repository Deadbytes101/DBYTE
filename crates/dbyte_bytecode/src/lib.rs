use dbyte_ast::FStrPart;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeFn {
    MathAbs,
    MathMin,
    MathMax,
    FsReadText,
    FsWriteText,
    FsReadBytes,
    FsWriteBytes,
    EncodingHexEncode,
    EncodingHexDecode,
    HashSha256,
    EnvArgs,
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

impl NativeFn {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "std.math.abs" => Some(Self::MathAbs),
            "std.math.min" => Some(Self::MathMin),
            "std.math.max" => Some(Self::MathMax),
            "std.fs.read_text" => Some(Self::FsReadText),
            "std.fs.write_text" => Some(Self::FsWriteText),
            "std.fs.read_bytes" => Some(Self::FsReadBytes),
            "std.fs.write_bytes" => Some(Self::FsWriteBytes),
            "std.encoding.hex_encode" => Some(Self::EncodingHexEncode),
            "std.encoding.hex_decode" => Some(Self::EncodingHexDecode),
            "std.hash.sha256" => Some(Self::HashSha256),
            "std.env.args" => Some(Self::EnvArgs),
            "std.buffer.new" => Some(Self::BufferNew),
            "std.buffer.from_bytes" => Some(Self::BufferFromBytes),
            "std.buffer.to_bytes" => Some(Self::BufferToBytes),
            "std.buffer.len" => Some(Self::BufferLen),
            "std.buffer.get" => Some(Self::BufferGet),
            "std.buffer.set" => Some(Self::BufferSet),
            "std.buffer.slice" => Some(Self::BufferSlice),
            "std.buffer.load" => Some(Self::BufferLoad),
            "std.buffer.save" => Some(Self::BufferSave),
            "std.buffer.find" => Some(Self::BufferFind),
            "std.buffer.replace" => Some(Self::BufferReplace),
            "std.binary.u8" => Some(Self::BinaryU8),
            "std.binary.i8" => Some(Self::BinaryI8),
            "std.binary.u16_le" => Some(Self::BinaryU16Le),
            "std.binary.u16_be" => Some(Self::BinaryU16Be),
            "std.binary.i16_le" => Some(Self::BinaryI16Le),
            "std.binary.i16_be" => Some(Self::BinaryI16Be),
            "std.binary.u32_le" => Some(Self::BinaryU32Le),
            "std.binary.u32_be" => Some(Self::BinaryU32Be),
            "std.binary.i32_le" => Some(Self::BinaryI32Le),
            "std.binary.i32_be" => Some(Self::BinaryI32Be),
            "std.binary.pack_u16_le" => Some(Self::BinaryPackU16Le),
            "std.binary.pack_u16_be" => Some(Self::BinaryPackU16Be),
            "std.binary.pack_u32_le" => Some(Self::BinaryPackU32Le),
            "std.binary.pack_u32_be" => Some(Self::BinaryPackU32Be),
            "std.binary.write_u16_le" => Some(Self::BinaryWriteU16Le),
            "std.binary.write_u16_be" => Some(Self::BinaryWriteU16Be),
            "std.binary.write_u32_le" => Some(Self::BinaryWriteU32Le),
            "std.binary.write_u32_be" => Some(Self::BinaryWriteU32Be),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Bytes(Vec<u8>),
    Buffer(Rc<RefCell<Vec<u8>>>),
    List(Vec<Value>),
    Module(Rc<ModuleValue>),
    Void,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModuleValue {
    pub alias: String,
    pub members: HashMap<String, ModuleMember>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModuleMember {
    Value(Value),
    Function(Box<BytecodeFunction>),
    Native(NativeFn),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Str(s) => write!(f, "{}", s),
            Value::Bytes(bs) => write!(f, "{}", hex::encode(bs)),
            Value::Buffer(_) => write!(f, "<buffer>"),
            Value::List(vs) => {
                write!(f, "[")?;
                for (i, v) in vs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            Value::Module(m) => write!(f, "<module {}>", m.alias),
            Value::Void => write!(f, ""),
        }
    }
}

impl Value {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Bool(_) => "bool",
            Value::Str(_) => "str",
            Value::Bytes(_) => "bytes",
            Value::Buffer(_) => "buffer",
            Value::List(_) => "list",
            Value::Module(_) => "module",
            Value::Void => "void",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Const(usize),
    ConstI64(i64),
    FStr(Vec<FStrPart>),
    Add,
    Sub,
    Mul,
    Div,
    AddI64,
    SubI64,
    MulI64,
    DivI64,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    LtI64,
    LeI64,
    GtI64,
    GeI64,
    Neg,
    Not,
    MakeList(usize),
    Index,
    LoadLocal(usize),
    LoadLocalI64(usize),
    StoreLocal(usize),
    StoreLocalI64(usize),
    AddLocalI64 { dst: usize, src: usize },
    AddLocalConstI64 { slot: usize, value: i64 },
    LtLocalConstI64 { slot: usize, value: i64 },
    LeLocalConstI64 { slot: usize, value: i64 },
    GtLocalConstI64 { slot: usize, value: i64 },
    GeLocalConstI64 { slot: usize, value: i64 },
    Import(String, usize),
    Member(String),
    MemberCall(String, usize),
    ReadU32Le,
    BufferFind,
    BufferReplace,
    IterInit,
    IterNext { slot: usize, jump: usize },
    Jump(usize),
    JumpIfFalse(usize),
    Call(String, usize),
    Return,
    Pop,
    Halt,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub name: String,
    pub constants: Vec<Value>,
    pub code: Vec<Op>,
    pub local_names: Vec<String>,
    pub local_kinds: Vec<LocalKind>,
    pub local_i64_slots: Vec<Option<usize>>,
    pub i64_local_count: usize,
    pub functions: HashMap<String, BytecodeFunction>,
    pub public_values: Vec<(String, usize)>,
    pub public_functions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind {
    Value,
    I64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BytecodeFunction {
    pub name: String,
    pub params: Vec<String>,
    pub chunk: Chunk,
}

impl Chunk {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            constants: Vec::new(),
            code: Vec::new(),
            local_names: Vec::new(),
            local_kinds: Vec::new(),
            local_i64_slots: Vec::new(),
            i64_local_count: 0,
            functions: HashMap::new(),
            public_values: Vec::new(),
            public_functions: Vec::new(),
        }
    }

    pub fn add_const(&mut self, value: Value) -> usize {
        self.constants.push(value);
        self.constants.len() - 1
    }

    pub fn disassemble(&self) -> String {
        let mut out = String::new();
        self.write_disassembly(&mut out, 0);
        out
    }

    fn write_disassembly(&self, out: &mut String, indent: usize) {
        let pad = " ".repeat(indent);
        out.push_str(&format!("{}== {} ==\n", pad, self.name));
        for (i, op) in self.code.iter().enumerate() {
            out.push_str(&format!("{}{:04} {}\n", pad, i, format_op(op, self)));
        }
        for function in self.functions.values() {
            out.push('\n');
            function.chunk.write_disassembly(out, indent + 2);
        }
    }
}

pub fn format_op(op: &Op, chunk: &Chunk) -> String {
    match op {
        Op::Const(idx) => format!("CONST {}        ; {}", idx, chunk.constants[*idx]),
        Op::ConstI64(n) => format!("CONST_I64 {}", n),
        Op::FStr(_) => "FSTR".into(),
        Op::Add => "ADD".into(),
        Op::Sub => "SUB".into(),
        Op::Mul => "MUL".into(),
        Op::Div => "DIV".into(),
        Op::AddI64 => "ADD_I64".into(),
        Op::SubI64 => "SUB_I64".into(),
        Op::MulI64 => "MUL_I64".into(),
        Op::DivI64 => "DIV_I64".into(),
        Op::Eq => "EQ".into(),
        Op::Ne => "NE".into(),
        Op::Lt => "LT".into(),
        Op::Le => "LE".into(),
        Op::Gt => "GT".into(),
        Op::Ge => "GE".into(),
        Op::LtI64 => "LT_I64".into(),
        Op::LeI64 => "LE_I64".into(),
        Op::GtI64 => "GT_I64".into(),
        Op::GeI64 => "GE_I64".into(),
        Op::Neg => "NEG".into(),
        Op::Not => "NOT".into(),
        Op::MakeList(n) => format!("MAKE_LIST {}", n),
        Op::Index => "INDEX".into(),
        Op::LoadLocal(slot) => format!("LOAD_LOCAL {} ; {}", slot, local_name(chunk, *slot)),
        Op::LoadLocalI64(slot) => format!("LOAD_LOCAL_I64 {} ; {}", slot, local_name(chunk, *slot)),
        Op::StoreLocal(slot) => format!("STORE_LOCAL {} ; {}", slot, local_name(chunk, *slot)),
        Op::StoreLocalI64(slot) => {
            format!("STORE_LOCAL_I64 {} ; {}", slot, local_name(chunk, *slot))
        }
        Op::AddLocalI64 { dst, src } => format!(
            "ADD_LOCAL_I64 {} ; {} += {}",
            dst,
            local_name(chunk, *dst),
            local_name(chunk, *src)
        ),
        Op::AddLocalConstI64 { slot, value } => format!(
            "ADD_LOCAL_CONST_I64 {} {} ; {}",
            slot,
            value,
            local_name(chunk, *slot)
        ),
        Op::LtLocalConstI64 { slot, value } => format!(
            "LT_LOCAL_CONST_I64 {} {} ; {}",
            slot,
            value,
            local_name(chunk, *slot)
        ),
        Op::LeLocalConstI64 { slot, value } => format!(
            "LE_LOCAL_CONST_I64 {} {} ; {}",
            slot,
            value,
            local_name(chunk, *slot)
        ),
        Op::GtLocalConstI64 { slot, value } => format!(
            "GT_LOCAL_CONST_I64 {} {} ; {}",
            slot,
            value,
            local_name(chunk, *slot)
        ),
        Op::GeLocalConstI64 { slot, value } => format!(
            "GE_LOCAL_CONST_I64 {} {} ; {}",
            slot,
            value,
            local_name(chunk, *slot)
        ),
        Op::Import(path, slot) => format!("IMPORT {} -> {}", path, local_name(chunk, *slot)),
        Op::Member(name) => format!("MEMBER {}", name),
        Op::MemberCall(name, argc) => format!("MEMBER_CALL {} {}", name, argc),
        Op::ReadU32Le => "READ_U32_LE".into(),
        Op::BufferFind => "BUFFER_FIND".into(),
        Op::BufferReplace => "BUFFER_REPLACE".into(),
        Op::IterInit => "ITER_INIT".into(),
        Op::IterNext { slot, jump } => {
            format!("ITER_NEXT {} -> {}", local_name(chunk, *slot), jump)
        }
        Op::Jump(target) => format!("JUMP {}", target),
        Op::JumpIfFalse(target) => format!("JUMP_IF_FALSE {}", target),
        Op::Call(name, argc) => format!("CALL {} {}", name, argc),
        Op::Return => "RETURN".into(),
        Op::Pop => "POP".into(),
        Op::Halt => "HALT".into(),
    }
}

fn local_name(chunk: &Chunk, slot: usize) -> &str {
    chunk
        .local_names
        .get(slot)
        .map_or("<unknown>", String::as_str)
}
