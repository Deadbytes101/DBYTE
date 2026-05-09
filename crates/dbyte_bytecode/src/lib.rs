use dbyte_ast::FStrPart;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Module(ModuleValue),
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
    Function(BytecodeFunction),
    Native(String),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Str(s) => write!(f, "{}", s),
            Value::Bytes(bs) => write!(f, "{}", hex::encode(bs)),
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
            Value::List(_) => "list",
            Value::Module(_) => "module",
            Value::Void => "void",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Const(usize),
    FStr(Vec<FStrPart>),
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Neg,
    Not,
    MakeList(usize),
    Index,
    LoadLocal(usize),
    StoreLocal(usize),
    Import(String, usize),
    Member(String),
    MemberCall(String, usize),
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
    pub functions: HashMap<String, BytecodeFunction>,
    pub public_values: Vec<(String, usize)>,
    pub public_functions: Vec<String>,
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
        Op::FStr(_) => "FSTR".into(),
        Op::Add => "ADD".into(),
        Op::Sub => "SUB".into(),
        Op::Mul => "MUL".into(),
        Op::Div => "DIV".into(),
        Op::Eq => "EQ".into(),
        Op::Ne => "NE".into(),
        Op::Lt => "LT".into(),
        Op::Le => "LE".into(),
        Op::Gt => "GT".into(),
        Op::Ge => "GE".into(),
        Op::Neg => "NEG".into(),
        Op::Not => "NOT".into(),
        Op::MakeList(n) => format!("MAKE_LIST {}", n),
        Op::Index => "INDEX".into(),
        Op::LoadLocal(slot) => format!("LOAD_LOCAL {} ; {}", slot, local_name(chunk, *slot)),
        Op::StoreLocal(slot) => format!("STORE_LOCAL {} ; {}", slot, local_name(chunk, *slot)),
        Op::Import(path, slot) => format!("IMPORT {} -> {}", path, local_name(chunk, *slot)),
        Op::Member(name) => format!("MEMBER {}", name),
        Op::MemberCall(name, argc) => format!("MEMBER_CALL {} {}", name, argc),
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
