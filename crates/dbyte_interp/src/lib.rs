use byteorder::{ByteOrder, BE, LE};
use dbyte_ast::*;
use dbyte_lexer::Lexer;
use dbyte_module::{resolve_import, ImportTarget, ModuleError, ModuleState};
use dbyte_parser::Parser;
use memchr::memmem;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

mod concrete;
pub mod memory;
pub use concrete::{
    execute_concrete_project, execute_concrete_project_incremental,
    execute_concrete_project_with_args, execute_concrete_project_with_memory, ConcreteExecution,
    ConcreteIncrementalExecution, ConcreteSession, ConcreteValue,
};

/// Top-level bindings and functions captured when loading a local file module.
/// Used when invoking `module.pub_fn(...)` so the body sees imports, module `let`s,
/// and sibling `fn` definitions from that file.
#[derive(Debug, Clone)]
pub struct ModuleLexical {
    pub env: HashMap<String, Value>,
    pub fns: HashMap<String, (Vec<Param>, Vec<Stmt>)>,
}

type NativeFn = fn(&[Value]) -> Result<Value, String>;

#[doc(hidden)]
pub fn execute_resolved_host_capability(
    capability: dbyte_hir::HostCapabilityId,
    arguments: &[Value],
) -> Result<Value, String> {
    use dbyte_hir::HostCapabilityId as C;
    let function: NativeFn = match capability {
        C::MathAbs => native_math_abs,
        C::MathMin => native_math_min,
        C::MathMax => native_math_max,
        C::FsReadText => native_fs_read_text,
        C::FsWriteText => native_fs_write_text,
        C::FsReadBytes => native_fs_read_bytes,
        C::FsWriteBytes => native_fs_write_bytes,
        C::FsExists => native_fs_exists,
        C::FsMkdir => native_fs_mkdir,
        C::FsRemove => native_fs_remove,
        C::EncodingHexEncode => native_encoding_hex_encode,
        C::EncodingHexDecode => native_encoding_hex_decode,
        C::HashSha256 => native_hash_sha256,
        C::ProcessRun => native_process_run,
        C::BufferNew => native_buffer_new,
        C::BufferFromBytes => native_buffer_from_bytes,
        C::BufferToBytes => native_buffer_to_bytes,
        C::BufferLen => native_buffer_len,
        C::BufferGet => native_buffer_get,
        C::BufferSet => native_buffer_set,
        C::BufferSlice => native_buffer_slice,
        C::BufferLoad => native_buffer_load,
        C::BufferSave => native_buffer_save,
        C::BufferFind => native_buffer_find,
        C::BufferReplace => native_buffer_replace,
        C::BinaryU8 => native_binary_u8,
        C::BinaryI8 => native_binary_i8,
        C::BinaryU16Le => native_binary_u16_le,
        C::BinaryU16Be => native_binary_u16_be,
        C::BinaryI16Le => native_binary_i16_le,
        C::BinaryI16Be => native_binary_i16_be,
        C::BinaryU32Le => native_binary_u32_le,
        C::BinaryU32Be => native_binary_u32_be,
        C::BinaryI32Le => native_binary_i32_le,
        C::BinaryI32Be => native_binary_i32_be,
        C::BinaryPackU16Le => native_binary_pack_u16_le,
        C::BinaryPackU16Be => native_binary_pack_u16_be,
        C::BinaryPackU32Le => native_binary_pack_u32_le,
        C::BinaryPackU32Be => native_binary_pack_u32_be,
        C::BinaryWriteU16Le => native_binary_write_u16_le,
        C::BinaryWriteU16Be => native_binary_write_u16_be,
        C::BinaryWriteU32Le => native_binary_write_u32_le,
        C::BinaryWriteU32Be => native_binary_write_u32_be,
        C::Print | C::EnvArgs => return Err("capability requires executor context".into()),
    };
    function(arguments)
}
// Keep this below the host thread stack ceiling so recursion fails as a DByte
// RuntimeError instead of aborting the Rust process.
// V12 aggregate/closure values make each tree-interpreter Rust frame larger.
// Keep the language guard below the host thread-stack ceiling so hostile
// recursion still returns a deterministic DByte error instead of aborting.
const MAX_CALL_DEPTH: usize = 16;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Bytes(Vec<u8>),
    Buffer(Rc<RefCell<Vec<u8>>>),
    List(Vec<Value>),
    Struct(Rc<RefCell<StructValue>>),
    Enum(EnumValue),
    Function(String),
    Closure(Rc<ClosureValue>),
    StructConstructor(String),
    EnumType(String),
    VariantConstructor { enum_name: String, variant: String },
    EarlyReturn(Box<Value>),
    Null,
    Module(ModuleValue),
    Void,
}

#[derive(Debug, Clone)]
pub struct StructValue {
    pub type_name: String,
    pub fields: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct EnumValue {
    pub enum_name: String,
    pub variant: String,
    pub payload: Vec<Value>,
}

#[derive(Debug, Clone)]
pub struct ClosureValue {
    pub captures: HashMap<String, Value>,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
pub struct ModuleValue {
    pub alias: String,
    pub members: HashMap<String, ModuleMember>,
    /// Present for file-backed modules; absent for `std.*` modules.
    pub lexical: Option<Rc<ModuleLexical>>,
}

#[derive(Debug, Clone)]
pub enum ModuleMember {
    Value(Value),
    Function(Vec<Param>, Vec<Stmt>),
    Native(NativeFn),
    EnvArgs,
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
            Value::Struct(value) => write!(f, "<{}>", value.borrow().type_name),
            Value::Enum(value) => write!(f, "{}.{}", value.enum_name, value.variant),
            Value::Function(name) => write!(f, "<fn {name}>"),
            Value::Closure(_) => write!(f, "<closure>"),
            Value::StructConstructor(name) => write!(f, "<struct {name}>"),
            Value::EnumType(name) => write!(f, "<enum {name}>"),
            Value::VariantConstructor { enum_name, variant } => {
                write!(f, "<variant {enum_name}.{variant}>")
            }
            Value::EarlyReturn(_) => write!(f, "<early return>"),
            Value::Null => write!(f, "null"),
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
            Value::Struct(_) => "struct",
            Value::Enum(_) => "enum",
            Value::Function(_) => "function",
            Value::Closure(_) => "closure",
            Value::StructConstructor(_) => "struct constructor",
            Value::EnumType(_) => "enum type",
            Value::VariantConstructor { .. } => "enum variant constructor",
            Value::EarlyReturn(_) => "early return",
            Value::Null => "null",
            Value::Module(_) => "module",
            Value::Void => "void",
        }
    }
}

#[derive(Debug)]
pub struct RuntimeError {
    pub msg: String,
    pub span: Span,
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RuntimeError at {}: {}", self.span, self.msg)
    }
}

#[derive(Debug)]
enum Signal {
    Return(Value),
    Break(Span),
    Continue(Span),
    Error(RuntimeError),
}

#[derive(Clone)]
enum OutputSink {
    Stdout,
    Capture(Rc<RefCell<String>>),
}

#[derive(Clone)]
pub struct Interpreter {
    env: Vec<HashMap<String, Value>>,
    scope_order: Vec<Vec<String>>,
    fns: HashMap<String, (Vec<Param>, Vec<Stmt>)>,
    structs: HashMap<String, Vec<StructField>>,
    enums: HashMap<String, Vec<EnumVariant>>,
    /// Runtime bodies for canonical `drop fn` declarations, keyed by the
    /// nominal type they own. These are invoked explicitly by DByte cleanup;
    /// Rust `Drop`/`Rc` lifetime is never observable language behavior.
    drop_fns: HashMap<String, (Vec<Param>, Vec<Stmt>)>,
    destroyed_structs: HashSet<usize>,
    current_file: Option<PathBuf>,
    module_cache: HashMap<String, ModuleState<ModuleValue>>,
    loading_stack: Vec<String>,
    in_function: usize,
    call_depth: usize,
    output: OutputSink,
    script_args: Vec<String>,
    /// While executing a user function entered from `module.member(...)`, function
    /// name resolution checks this table before the host script's `fns`.
    active_module_lexical: Option<Rc<ModuleLexical>>,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Interpreter {
    pub fn new() -> Self {
        let mut enums = HashMap::new();
        enums.insert(
            "Option".into(),
            vec![
                EnumVariant {
                    name: "None".into(),
                    payload: Vec::new(),
                    span: Span::zero(),
                },
                EnumVariant {
                    name: "Some".into(),
                    payload: vec![TypeAnnotation::Inferred],
                    span: Span::zero(),
                },
            ],
        );
        enums.insert(
            "Result".into(),
            vec![
                EnumVariant {
                    name: "Ok".into(),
                    payload: vec![TypeAnnotation::Inferred],
                    span: Span::zero(),
                },
                EnumVariant {
                    name: "Err".into(),
                    payload: vec![TypeAnnotation::Inferred],
                    span: Span::zero(),
                },
            ],
        );
        Self {
            env: vec![HashMap::new()],
            scope_order: vec![Vec::new()],
            fns: HashMap::new(),
            structs: HashMap::new(),
            enums,
            drop_fns: HashMap::new(),
            destroyed_structs: HashSet::new(),
            current_file: None,
            module_cache: HashMap::new(),
            loading_stack: Vec::new(),
            in_function: 0,
            call_depth: 0,
            output: OutputSink::Stdout,
            script_args: Vec::new(),
            active_module_lexical: None,
        }
    }

    pub fn with_entry_path(path: impl Into<PathBuf>) -> Self {
        let mut interp = Self::new();
        interp.current_file = Some(path.into());
        interp
    }

    pub fn set_entry_path(&mut self, path: impl Into<PathBuf>) {
        self.current_file = Some(path.into());
    }

    pub fn set_script_args(&mut self, args: Vec<String>) {
        self.script_args = args;
    }

    pub fn script_args(&self) -> &[String] {
        &self.script_args
    }

    pub fn with_captured_output(path: impl Into<PathBuf>) -> Self {
        let mut interp = Self::with_entry_path(path);
        interp.capture_output();
        interp
    }

    pub fn capture_output(&mut self) {
        self.output = OutputSink::Capture(Rc::new(RefCell::new(String::new())));
    }

    pub fn clear_captured_output(&mut self) {
        if let OutputSink::Capture(output) = &self.output {
            output.borrow_mut().clear();
        }
    }

    pub fn take_captured_output(&mut self) -> String {
        match &self.output {
            OutputSink::Capture(output) => std::mem::take(&mut *output.borrow_mut()),
            OutputSink::Stdout => String::new(),
        }
    }

    fn emit_line(&mut self, line: &str) {
        match &self.output {
            OutputSink::Stdout => println!("{}", line),
            OutputSink::Capture(output) => {
                let mut output = output.borrow_mut();
                output.push_str(line);
                output.push('\n');
            }
        }
    }

    fn push_scope(&mut self) {
        self.env.push(HashMap::new());
        self.scope_order.push(Vec::new());
    }
    fn pop_scope_without_drop(&mut self) {
        self.env.pop();
        self.scope_order.pop();
    }

    fn pop_scope(&mut self, span: Span) -> Result<(), RuntimeError> {
        let mut scope = self.env.pop().unwrap_or_default();
        let order = self.scope_order.pop().unwrap_or_default();
        for name in order.into_iter().rev() {
            if let Some(value) = scope.remove(&name) {
                self.destroy_value(value, span)?;
            }
        }
        Ok(())
    }

    fn get(&self, name: &str) -> Option<Value> {
        for scope in self.env.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v.clone());
            }
        }
        None
    }

    fn replace(&mut self, name: &str, val: Value) -> Option<Value> {
        for scope in self.env.iter_mut().rev() {
            if let Some(slot) = scope.get_mut(name) {
                return Some(std::mem::replace(slot, val));
            }
        }
        None
    }

    fn define(&mut self, name: &str, val: Value) {
        let scope = self.env.last_mut().unwrap();
        if !scope.contains_key(name) {
            self.scope_order
                .last_mut()
                .expect("scope order")
                .push(name.to_string());
        }
        scope.insert(name.to_string(), val);
    }

    fn take(&mut self, name: &str) -> Option<Value> {
        self.env
            .iter_mut()
            .rev()
            .find_map(|scope| scope.remove(name))
    }

    fn module_key(target: &ImportTarget) -> String {
        match target {
            ImportTarget::File(path) => path.to_string_lossy().to_string(),
            ImportTarget::Std(name) => name.clone(),
        }
    }

    fn load_module(
        &mut self,
        path: &str,
        alias: &str,
        span: Span,
    ) -> Result<ModuleValue, RuntimeError> {
        let target =
            resolve_import(path, self.current_file.as_deref()).map_err(|e| RuntimeError {
                msg: format!("ImportError: {}", format_module_error(&e)),
                span,
            })?;
        let key = Self::module_key(&target);

        match self.module_cache.get(&key) {
            Some(ModuleState::Loaded(module)) => {
                let mut module = module.clone();
                module.alias = alias.to_string();
                return Ok(module);
            }
            Some(ModuleState::Loading) => {
                let mut chain = self.loading_stack.clone();
                chain.push(key.clone());
                return Err(RuntimeError {
                    msg: format!(
                        "ImportError: circular import detected: {}",
                        chain.join(" -> ")
                    ),
                    span,
                });
            }
            None => {}
        }

        self.module_cache.insert(key.clone(), ModuleState::Loading);
        self.loading_stack.push(key.clone());
        let loaded = match target {
            ImportTarget::Std(name) => Self::load_std_module(&name, alias, span),
            ImportTarget::File(path) => self.load_file_module(&path, alias, span),
        };
        self.loading_stack.pop();

        match loaded {
            Ok(module) => {
                self.module_cache
                    .insert(key, ModuleState::Loaded(module.clone()));
                Ok(module)
            }
            Err(e) => {
                self.module_cache.remove(&key);
                Err(e)
            }
        }
    }

    fn load_std_module(name: &str, alias: &str, span: Span) -> Result<ModuleValue, RuntimeError> {
        let mut members = HashMap::new();
        match name {
            "std.math" => {
                members.insert("abs".into(), ModuleMember::Native(native_math_abs));
                members.insert("min".into(), ModuleMember::Native(native_math_min));
                members.insert("max".into(), ModuleMember::Native(native_math_max));
            }
            "std.fs" => {
                members.insert(
                    "read_text".into(),
                    ModuleMember::Native(native_fs_read_text),
                );
                members.insert(
                    "write_text".into(),
                    ModuleMember::Native(native_fs_write_text),
                );
                members.insert(
                    "read_bytes".into(),
                    ModuleMember::Native(native_fs_read_bytes),
                );
                members.insert(
                    "write_bytes".into(),
                    ModuleMember::Native(native_fs_write_bytes),
                );
                members.insert("exists".into(), ModuleMember::Native(native_fs_exists));
                members.insert("mkdir".into(), ModuleMember::Native(native_fs_mkdir));
                members.insert("remove".into(), ModuleMember::Native(native_fs_remove));
            }

            "std.encoding" => {
                members.insert(
                    "hex_encode".into(),
                    ModuleMember::Native(native_encoding_hex_encode),
                );
                members.insert(
                    "hex_decode".into(),
                    ModuleMember::Native(native_encoding_hex_decode),
                );
            }
            "std.hash" => {
                members.insert("sha256".into(), ModuleMember::Native(native_hash_sha256));
            }
            "std.env" => {
                members.insert("args".into(), ModuleMember::EnvArgs);
            }
            "std.process" => {
                members.insert("run".into(), ModuleMember::Native(native_process_run));
            }
            "std.buffer" => {
                members.insert("new".into(), ModuleMember::Native(native_buffer_new));
                members.insert(
                    "from_bytes".into(),
                    ModuleMember::Native(native_buffer_from_bytes),
                );
                members.insert(
                    "to_bytes".into(),
                    ModuleMember::Native(native_buffer_to_bytes),
                );
                members.insert("len".into(), ModuleMember::Native(native_buffer_len));
                members.insert("get".into(), ModuleMember::Native(native_buffer_get));
                members.insert("set".into(), ModuleMember::Native(native_buffer_set));
                members.insert("slice".into(), ModuleMember::Native(native_buffer_slice));
                members.insert("load".into(), ModuleMember::Native(native_buffer_load));
                members.insert("save".into(), ModuleMember::Native(native_buffer_save));
                members.insert("find".into(), ModuleMember::Native(native_buffer_find));
                members.insert(
                    "replace".into(),
                    ModuleMember::Native(native_buffer_replace),
                );
            }
            "std.binary" => {
                members.insert("u8".into(), ModuleMember::Native(native_binary_u8));
                members.insert("i8".into(), ModuleMember::Native(native_binary_i8));
                members.insert("u16_le".into(), ModuleMember::Native(native_binary_u16_le));
                members.insert("u16_be".into(), ModuleMember::Native(native_binary_u16_be));
                members.insert("i16_le".into(), ModuleMember::Native(native_binary_i16_le));
                members.insert("i16_be".into(), ModuleMember::Native(native_binary_i16_be));
                members.insert("u32_le".into(), ModuleMember::Native(native_binary_u32_le));
                members.insert("u32_be".into(), ModuleMember::Native(native_binary_u32_be));
                members.insert("i32_le".into(), ModuleMember::Native(native_binary_i32_le));
                members.insert("i32_be".into(), ModuleMember::Native(native_binary_i32_be));
                members.insert(
                    "pack_u16_le".into(),
                    ModuleMember::Native(native_binary_pack_u16_le),
                );
                members.insert(
                    "pack_u16_be".into(),
                    ModuleMember::Native(native_binary_pack_u16_be),
                );
                members.insert(
                    "pack_u32_le".into(),
                    ModuleMember::Native(native_binary_pack_u32_le),
                );
                members.insert(
                    "pack_u32_be".into(),
                    ModuleMember::Native(native_binary_pack_u32_be),
                );
                members.insert(
                    "write_u16_le".into(),
                    ModuleMember::Native(native_binary_write_u16_le),
                );
                members.insert(
                    "write_u16_be".into(),
                    ModuleMember::Native(native_binary_write_u16_be),
                );
                members.insert(
                    "write_u32_le".into(),
                    ModuleMember::Native(native_binary_write_u32_le),
                );
                members.insert(
                    "write_u32_be".into(),
                    ModuleMember::Native(native_binary_write_u32_be),
                );
            }
            _ => {
                return Err(RuntimeError {
                    msg: format!("ImportError: standard module not found: {}", name),
                    span,
                });
            }
        }
        Ok(ModuleValue {
            alias: alias.to_string(),
            members,
            lexical: None,
        })
    }

    fn parse_module_file(path: &Path, span: Span) -> Result<Program, RuntimeError> {
        let src = std::fs::read_to_string(path).map_err(|e| RuntimeError {
            msg: format!("ImportError: cannot read `{}`: {}", path.display(), e),
            span,
        })?;
        let tokens = Lexer::new(&src).tokenize().map_err(|e| RuntimeError {
            msg: format!("ImportError: {}", e.msg),
            span: e.span,
        })?;
        Parser::new(tokens)
            .parse_program()
            .map_err(|e| RuntimeError {
                msg: format!("ImportError: {}", e.msg),
                span: e.span,
            })
    }

    fn load_file_module(
        &mut self,
        path: &Path,
        alias: &str,
        span: Span,
    ) -> Result<ModuleValue, RuntimeError> {
        let program = Self::parse_module_file(path, span)?;

        let saved_env = std::mem::replace(&mut self.env, vec![HashMap::new()]);
        let saved_scope_order = std::mem::replace(&mut self.scope_order, vec![Vec::new()]);
        let saved_fns = std::mem::take(&mut self.fns);
        let saved_structs = std::mem::take(&mut self.structs);
        let saved_enums = std::mem::take(&mut self.enums);
        let saved_drop_fns = std::mem::take(&mut self.drop_fns);
        let saved_file = self.current_file.replace(path.to_path_buf());
        let saved_in_function = self.in_function;
        let saved_call_depth = self.call_depth;
        self.in_function = 0;
        self.call_depth = 0;

        let executed = self.exec_stmts(&program.stmts);
        let mut members = HashMap::new();
        if executed.is_ok() {
            for stmt in &program.stmts {
                match stmt {
                    Stmt::Let {
                        is_pub: true, name, ..
                    } => {
                        if let Some(value) = self.get(name) {
                            members.insert(name.clone(), ModuleMember::Value(value));
                        }
                    }
                    Stmt::FnDef {
                        is_pub: true, name, ..
                    } => {
                        if let Some((params, body)) = self.fns.get(name).cloned() {
                            members.insert(name.clone(), ModuleMember::Function(params, body));
                        }
                    }
                    _ => {}
                }
            }
        }

        let lexical_capture = if executed.is_ok() {
            let env_snapshot = self.env.last().cloned().unwrap_or_default();
            let fns_snapshot = self.fns.clone();
            Some(Rc::new(ModuleLexical {
                env: env_snapshot,
                fns: fns_snapshot,
            }))
        } else {
            None
        };

        self.env = saved_env;
        self.scope_order = saved_scope_order;
        self.fns = saved_fns;
        self.structs = saved_structs;
        self.enums = saved_enums;
        self.drop_fns = saved_drop_fns;
        self.current_file = saved_file;
        self.in_function = saved_in_function;
        self.call_depth = saved_call_depth;

        match executed {
            Ok(()) => Ok(ModuleValue {
                alias: alias.to_string(),
                members,
                lexical: lexical_capture,
            }),
            Err(Signal::Error(e)) => Err(e),
            Err(Signal::Return(_)) => Err(RuntimeError {
                msg: "return outside function".into(),
                span,
            }),
            Err(Signal::Break(span)) => Err(RuntimeError {
                msg: "break outside loop".into(),
                span,
            }),
            Err(Signal::Continue(span)) => Err(RuntimeError {
                msg: "continue outside loop".into(),
                span,
            }),
        }
    }

    fn eval_expr(&mut self, expr: &Expr) -> Result<Value, RuntimeError> {
        match expr {
            Expr::UnitLit(_) => Ok(Value::Void),
            Expr::IntLit(n, _) => Ok(Value::Int(*n)),
            Expr::FloatLit(n, _) => Ok(Value::Float(*n)),
            Expr::BoolLit(b, _) => Ok(Value::Bool(*b)),
            Expr::StrLit(s, _) => Ok(Value::Str(s.clone())),
            Expr::BytesLit(b, _) => Ok(Value::Bytes(b.clone())),
            Expr::FStr(parts, span) => {
                let mut result = String::new();
                for part in parts {
                    match part {
                        FStrPart::Literal(s) => result.push_str(s),
                        FStrPart::Interp(name) => {
                            let val = self.get(name).ok_or_else(|| RuntimeError {
                                msg: format!(
                                    "undefined variable `{}` in string interpolation",
                                    name
                                ),
                                span: *span,
                            })?;
                            result.push_str(&format!("{}", val));
                        }
                    }
                }
                Ok(Value::Str(result))
            }
            Expr::NullLit(_) => Ok(Value::Null),
            Expr::GenericApply { target, .. } => self.eval_expr(target),
            Expr::Borrow { expr, .. } => self.eval_expr(expr),
            Expr::Drop { expr, span } => {
                let value = self.eval_owned_expr(expr)?;
                self.destroy_value(value, *span)?;
                Ok(Value::Void)
            }
            Expr::Try { expr, span } => match self.eval_expr(expr)? {
                Value::Enum(value)
                    if value.enum_name == "Result"
                        && value.variant == "Ok"
                        && value.payload.len() == 1 =>
                {
                    Ok(value.payload[0].clone())
                }
                Value::Enum(value)
                    if value.enum_name == "Result"
                        && value.variant == "Err"
                        && value.payload.len() == 1 =>
                {
                    Ok(Value::EarlyReturn(Box::new(Value::Enum(value))))
                }
                _ => Err(RuntimeError {
                    msg: "operator `?` requires canonical Result with one payload value".into(),
                    span: *span,
                }),
            },
            Expr::Closure {
                captures,
                params,
                body,
                span,
                ..
            } => {
                let mut captured = HashMap::new();
                for capture in captures {
                    let value = self.get(&capture.name).ok_or_else(|| RuntimeError {
                        msg: format!("undefined closure capture `{}`", capture.name),
                        span: capture.span,
                    })?;
                    captured.insert(capture.name.clone(), value);
                }
                if captures
                    .iter()
                    .any(|capture| capture.mode == CaptureMode::BorrowMut)
                {
                    return Err(RuntimeError {
                        msg: "borrow-mut closure execution requires canonical reference cells"
                            .into(),
                        span: *span,
                    });
                }
                Ok(Value::Closure(Rc::new(ClosureValue {
                    captures: captured,
                    params: params.clone(),
                    body: body.clone(),
                })))
            }
            Expr::Invoke {
                callee,
                args,
                named_args,
                span,
            } => {
                let callee = self.eval_expr(callee)?;
                self.invoke_value(callee, args, named_args, *span)
            }

            Expr::Ident(name, span) => {
                if let Some(value) = self.get(name) {
                    Ok(value)
                } else if self.fns.contains_key(name)
                    || self
                        .active_module_lexical
                        .as_ref()
                        .is_some_and(|lexical| lexical.fns.contains_key(name))
                {
                    Ok(Value::Function(name.clone()))
                } else if self.structs.contains_key(name) {
                    Ok(Value::StructConstructor(name.clone()))
                } else if self.enums.contains_key(name) {
                    Ok(Value::EnumType(name.clone()))
                } else {
                    Err(RuntimeError {
                        msg: format!("undefined variable `{}`", name),
                        span: *span,
                    })
                }
            }

            Expr::List(elems, _) => {
                let vals: Result<Vec<_>, _> = elems.iter().map(|e| self.eval_expr(e)).collect();
                Ok(Value::List(vals?))
            }

            Expr::Index {
                target,
                index,
                span,
            } => {
                let tval = self.eval_expr(target)?;
                let ival = self.eval_expr(index)?;
                match (tval, ival) {
                    (Value::List(vs), Value::Int(idx)) => {
                        let normalized = if idx < 0 { vs.len() as i64 + idx } else { idx };
                        if normalized < 0 || normalized as usize >= vs.len() {
                            return Err(RuntimeError {
                                msg: format!(
                                    "index out of range: list length is {}, but index is {}",
                                    vs.len(),
                                    idx
                                ),
                                span: *span,
                            });
                        }
                        Ok(vs[normalized as usize].clone())
                    }
                    (Value::Bytes(bs), Value::Int(idx)) => {
                        let normalized = if idx < 0 { bs.len() as i64 + idx } else { idx };
                        if normalized < 0 || normalized as usize >= bs.len() {
                            return Err(RuntimeError {
                                msg: format!(
                                    "index out of range: bytes length is {}, but index is {}",
                                    bs.len(),
                                    idx
                                ),
                                span: *span,
                            });
                        }
                        Ok(Value::Int(bs[normalized as usize] as i64))
                    }
                    (_, Value::Int(_)) => Err(RuntimeError {
                        msg: "value is not indexable".into(),
                        span: *span,
                    }),
                    _ => Err(RuntimeError {
                        msg: "index must be int".into(),
                        span: *span,
                    }),
                }
            }

            Expr::Binary {
                left,
                op,
                right,
                span,
            } => {
                let lv = self.eval_expr(left)?;
                let rv = self.eval_expr(right)?;
                match (lv, rv) {
                    (Value::Int(a), Value::Int(b)) => match op {
                        BinOp::Add => checked_int(a.checked_add(b), *span),
                        BinOp::Sub => checked_int(a.checked_sub(b), *span),
                        BinOp::Mul => checked_int(a.checked_mul(b), *span),
                        BinOp::Div => {
                            if b == 0 {
                                return Err(RuntimeError {
                                    msg: "division by zero".into(),
                                    span: *span,
                                });
                            }
                            checked_int(a.checked_div(b), *span)
                        }
                        BinOp::Rem => {
                            if b == 0 {
                                return Err(RuntimeError {
                                    msg: "remainder by zero".into(),
                                    span: *span,
                                });
                            }
                            checked_int(a.checked_rem(b), *span)
                        }
                        BinOp::EqEq => Ok(Value::Bool(a == b)),
                        BinOp::NotEq => Ok(Value::Bool(a != b)),
                        BinOp::Lt => Ok(Value::Bool(a < b)),
                        BinOp::LtEq => Ok(Value::Bool(a <= b)),
                        BinOp::Gt => Ok(Value::Bool(a > b)),
                        BinOp::GtEq => Ok(Value::Bool(a >= b)),
                        BinOp::BitAnd
                        | BinOp::BitOr
                        | BinOp::BitXor
                        | BinOp::Shl
                        | BinOp::Shr => Err(RuntimeError {
                            msg: format!(
                                "native systems operator `{op}` is unavailable in the hosted interpreter"
                            ),
                            span: *span,
                        }),
                    },
                    (Value::Float(a), Value::Float(b)) => match op {
                        BinOp::Add => Ok(Value::Float(a + b)),
                        BinOp::Sub => Ok(Value::Float(a - b)),
                        BinOp::Mul => Ok(Value::Float(a * b)),
                        BinOp::Div => Ok(Value::Float(a / b)),
                        BinOp::Rem => Err(RuntimeError {
                            msg: "operator '%' requires integer operands".into(),
                            span: *span,
                        }),
                        BinOp::EqEq => Ok(Value::Bool(a == b)),
                        BinOp::NotEq => Ok(Value::Bool(a != b)),
                        BinOp::Lt => Ok(Value::Bool(a < b)),
                        BinOp::LtEq => Ok(Value::Bool(a <= b)),
                        BinOp::Gt => Ok(Value::Bool(a > b)),
                        BinOp::GtEq => Ok(Value::Bool(a >= b)),
                        BinOp::BitAnd
                        | BinOp::BitOr
                        | BinOp::BitXor
                        | BinOp::Shl
                        | BinOp::Shr => Err(RuntimeError {
                            msg: format!(
                                "native systems operator `{op}` is unavailable in the hosted interpreter"
                            ),
                            span: *span,
                        }),
                    },
                    (Value::Str(a), Value::Str(b)) => match op {
                        BinOp::Add => Ok(Value::Str(a + &b)),
                        BinOp::EqEq => Ok(Value::Bool(a == b)),
                        BinOp::NotEq => Ok(Value::Bool(a != b)),
                        BinOp::Rem => Err(RuntimeError {
                            msg: "operator '%' requires integer operands".into(),
                            span: *span,
                        }),
                        _ => Err(RuntimeError {
                            msg: "unsupported str operation".into(),
                            span: *span,
                        }),
                    },
                    (Value::Bool(a), Value::Bool(b)) => match op {
                        BinOp::EqEq => Ok(Value::Bool(a == b)),
                        BinOp::NotEq => Ok(Value::Bool(a != b)),
                        BinOp::Rem => Err(RuntimeError {
                            msg: "operator '%' requires integer operands".into(),
                            span: *span,
                        }),
                        _ => Err(RuntimeError {
                            msg: "unsupported bool operation".into(),
                            span: *span,
                        }),
                    },
                    _ => Err(RuntimeError {
                        msg: "type mismatch in binary expression".into(),
                        span: *span,
                    }),
                }
            }

            Expr::Unary { op, expr, span } => {
                let v = self.eval_expr(expr)?;
                match op {
                    UnaryOp::Neg => match v {
                        Value::Int(n) => checked_int(n.checked_neg(), *span),
                        Value::Float(n) => Ok(Value::Float(-n)),
                        _ => Err(RuntimeError {
                            msg: "unary `-` requires numeric".into(),
                            span: *span,
                        }),
                    },
                    UnaryOp::Not => match v {
                        Value::Bool(b) => Ok(Value::Bool(!b)),
                        _ => Err(RuntimeError {
                            msg: "unary `!` requires bool".into(),
                            span: *span,
                        }),
                    },
                    // Canonical borrows are represented by the same reference cell in the
                    // hosted model. Dereference therefore projects that value without
                    // inventing a host pointer or a second ownership decision.
                    UnaryOp::Deref => Ok(v),
                    UnaryOp::BitNot | UnaryOp::AddressOf => Err(RuntimeError {
                        msg: "native systems unary operators are unavailable in the hosted interpreter"
                            .into(),
                        span: *span,
                    }),
                }
            }

            Expr::Call { name, args, span } => {
                if name == "print" {
                    let vals: Result<Vec<_>, _> = args.iter().map(|a| self.eval_expr(a)).collect();
                    let strs: Vec<String> = vals?.iter().map(|v| format!("{}", v)).collect();
                    self.emit_line(&strs.join(" "));
                    return Ok(Value::Void);
                }
                if name == "len" {
                    if args.len() != 1 {
                        return Err(RuntimeError {
                            msg: "len() expects 1 argument".into(),
                            span: *span,
                        });
                    }
                    let val = self.eval_expr(&args[0])?;
                    let length = match val {
                        Value::Str(s) => s.len(),
                        Value::List(l) => l.len(),
                        Value::Bytes(b) => b.len(),
                        Value::Buffer(b) => b.borrow().len(),
                        _ => {
                            return Err(RuntimeError {
                                msg: "len() expects str, list, bytes, or buffer".into(),
                                span: *span,
                            })
                        }
                    };
                    return Ok(Value::Int(length as i64));
                }

                if let Some(value) = self.get(name) {
                    if matches!(
                        value,
                        Value::Function(_)
                            | Value::Closure(_)
                            | Value::StructConstructor(_)
                            | Value::VariantConstructor { .. }
                    ) {
                        return self.invoke_value(value, args, &[], *span);
                    }
                }

                let (params, body) = if let Some(ref lex) = self.active_module_lexical {
                    lex.fns
                        .get(name)
                        .cloned()
                        .or_else(|| self.fns.get(name).cloned())
                } else {
                    self.fns.get(name).cloned()
                }
                .ok_or_else(|| RuntimeError {
                    msg: format!("undefined function `{}`", name),
                    span: *span,
                })?;
                self.call_user_function(name, &params, &body, args, *span, None)
            }

            Expr::Member {
                object,
                property,
                span,
            } => {
                match self.eval_expr(object)? {
                    Value::Struct(value) => value
                        .borrow()
                        .fields
                        .get(property)
                        .cloned()
                        .ok_or_else(|| RuntimeError {
                            msg: format!("struct has no field `{property}`"),
                            span: *span,
                        }),
                    Value::EnumType(enum_name) => {
                        let variants = self.enums.get(&enum_name).ok_or_else(|| RuntimeError {
                            msg: format!("unknown enum `{enum_name}`"),
                            span: *span,
                        })?;
                        if variants.iter().any(|variant| variant.name == *property) {
                            Ok(Value::VariantConstructor {
                                enum_name,
                                variant: property.clone(),
                            })
                        } else {
                            Err(RuntimeError {
                                msg: format!("enum has no variant `{property}`"),
                                span: *span,
                            })
                        }
                    }
                    Value::Module(module) => match module.members.get(property).cloned() {
                        Some(ModuleMember::Value(value)) => Ok(value),
                        Some(
                            ModuleMember::Function(_, _)
                            | ModuleMember::Native(_)
                            | ModuleMember::EnvArgs,
                        ) => Err(RuntimeError {
                            msg: format!("module member `{property}` is callable"),
                            span: *span,
                        }),
                        None => Err(RuntimeError {
                            msg: format!(
                                "module '{}' has no public member '{}'",
                                module.alias, property
                            ),
                            span: *span,
                        }),
                    },
                    other => Err(RuntimeError {
                        msg: format!("member access not supported for `{}`", other.kind_name()),
                        span: *span,
                    }),
                }
            }

            Expr::MemberCall {
                object,
                property,
                args,
                span,
            } => match self.eval_expr(object)? {
                Value::EnumType(enum_name) => self.invoke_value(
                    Value::VariantConstructor {
                        enum_name,
                        variant: property.clone(),
                    },
                    args,
                    &[],
                    *span,
                ),
                Value::Module(module) => {
                    let module_entry = module.lexical.clone();
                    let member =
                        module
                            .members
                            .get(property)
                            .cloned()
                            .ok_or_else(|| RuntimeError {
                                msg: format!(
                                    "module '{}' has no public member '{}'",
                                    module.alias, property
                                ),
                                span: *span,
                            })?;
                    match member {
                        ModuleMember::Function(params, body) => self.call_user_function(
                            property,
                            &params,
                            &body,
                            args,
                            *span,
                            module_entry,
                        ),
                        ModuleMember::Native(f) => {
                            let vals: Result<Vec<_>, _> =
                                args.iter().map(|a| self.eval_expr(a)).collect();
                            f(&vals?).map_err(|msg| RuntimeError { msg, span: *span })
                        }
                        ModuleMember::EnvArgs => {
                            if !args.is_empty() {
                                return Err(RuntimeError {
                                    msg: format!(
                                        "function `env.args` expects 0 args, got {}",
                                        args.len()
                                    ),
                                    span: *span,
                                });
                            }
                            Ok(Value::List(
                                self.script_args.iter().cloned().map(Value::Str).collect(),
                            ))
                        }
                        ModuleMember::Value(_) => Err(RuntimeError {
                            msg: format!("module member `{}` is not callable", property),
                            span: *span,
                        }),
                    }
                }
                _ => Err(RuntimeError {
                    msg: "member access requires a module value".into(),
                    span: *span,
                }),
            },
            Expr::Cast { span, .. } => Err(RuntimeError {
                msg: "native systems casts are unavailable in the hosted interpreter".into(),
                span: *span,
            }),
            Expr::Bitcast { span, .. } => Err(RuntimeError {
                msg: "exact bitcasts require canonical hosted execution".into(),
                span: *span,
            }),
        }
    }

    fn call_user_function(
        &mut self,
        name: &str,
        params: &[Param],
        body: &[Stmt],
        args: &[Expr],
        span: Span,
        module_entry: Option<Rc<ModuleLexical>>,
    ) -> Result<Value, RuntimeError> {
        if args.len() != params.len() {
            return Err(RuntimeError {
                msg: format!(
                    "function `{}` expects {} args, got {}",
                    name,
                    params.len(),
                    args.len()
                ),
                span,
            });
        }
        if self.call_depth >= MAX_CALL_DEPTH {
            return Err(RuntimeError {
                msg: "maximum call depth exceeded".into(),
                span,
            });
        }

        let saved_active = if module_entry.is_some() {
            self.active_module_lexical.take()
        } else {
            None
        };
        if let Some(ref lex) = module_entry {
            self.active_module_lexical = Some(Rc::clone(lex));
            self.push_scope();
            for (k, v) in &lex.env {
                self.define(k, v.clone());
            }
        }

        let arg_vals: Result<Vec<_>, _> = args.iter().map(|a| self.eval_owned_expr(a)).collect();
        let result = match arg_vals {
            Ok(values) => self.call_user_function_values(name, params, body, values, span),
            Err(error) => Err(error),
        };
        if module_entry.is_some() {
            self.pop_scope_without_drop();
            self.active_module_lexical = saved_active;
        }
        result
    }

    fn call_user_function_values(
        &mut self,
        name: &str,
        params: &[Param],
        body: &[Stmt],
        arg_vals: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        if arg_vals.len() != params.len() {
            return Err(RuntimeError {
                msg: format!(
                    "function `{}` expects {} args, got {}",
                    name,
                    params.len(),
                    arg_vals.len()
                ),
                span,
            });
        }
        self.push_scope();
        self.in_function += 1;
        self.call_depth += 1;
        for (p, v) in params.iter().zip(arg_vals) {
            self.define(&p.name, v);
        }
        let result = self.exec_stmts(body);
        self.call_depth -= 1;
        self.in_function -= 1;
        let cleanup = self.pop_scope(span);

        cleanup?;

        match result {
            Ok(_) => Ok(Value::Void),
            Err(Signal::Return(v)) => Ok(v),
            Err(Signal::Break(span)) => Err(RuntimeError {
                msg: "break outside loop".into(),
                span,
            }),
            Err(Signal::Continue(span)) => Err(RuntimeError {
                msg: "continue outside loop".into(),
                span,
            }),
            Err(Signal::Error(e)) => Err(e),
        }
    }

    fn destroy_value(&mut self, value: Value, span: Span) -> Result<(), RuntimeError> {
        if !self.value_needs_drop(&value) {
            return Ok(());
        }
        match value {
            Value::Struct(value) => {
                let identity = Rc::as_ptr(&value) as usize;
                if !self.destroyed_structs.insert(identity) {
                    return Ok(());
                }
                let type_name = value.borrow().type_name.clone();
                if let Some((params, body)) = self.drop_fns.get(&type_name).cloned() {
                    self.call_user_function_values(
                        "<drop>",
                        &params,
                        &body,
                        vec![Value::Struct(Rc::clone(&value))],
                        span,
                    )?;
                }
                let fields = self.structs.get(&type_name).cloned().unwrap_or_default();
                for field in fields.into_iter().rev() {
                    if let Some(field_value) = value.borrow_mut().fields.remove(&field.name) {
                        self.destroy_value(field_value, span)?;
                    }
                }
                Ok(())
            }
            Value::Enum(mut value) => {
                if let Some((params, body)) = self.drop_fns.get(&value.enum_name).cloned() {
                    self.call_user_function_values(
                        "<drop>",
                        &params,
                        &body,
                        vec![Value::Enum(value.clone())],
                        span,
                    )?;
                }
                for payload in value.payload.drain(..).rev() {
                    self.destroy_value(payload, span)?;
                }
                Ok(())
            }
            Value::List(values) => {
                for element in values.into_iter().rev() {
                    self.destroy_value(element, span)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn value_needs_drop(&self, value: &Value) -> bool {
        match value {
            Value::Struct(value) => {
                let value = value.borrow();
                self.drop_fns.contains_key(&value.type_name)
                    || value
                        .fields
                        .values()
                        .any(|field| self.value_needs_drop(field))
            }
            Value::Enum(value) => {
                self.drop_fns.contains_key(&value.enum_name)
                    || value
                        .payload
                        .iter()
                        .any(|payload| self.value_needs_drop(payload))
            }
            Value::List(values) => values.iter().any(|value| self.value_needs_drop(value)),
            _ => false,
        }
    }

    fn eval_owned_expr(&mut self, expression: &Expr) -> Result<Value, RuntimeError> {
        if let Expr::Ident(name, span) = expression {
            if let Some(value) = self.get(name) {
                if self.value_needs_drop(&value) {
                    return self.take(name).ok_or_else(|| RuntimeError {
                        msg: format!("owned value `{name}` was already moved"),
                        span: *span,
                    });
                }
            }
        }
        self.eval_expr(expression)
    }

    fn invoke_value(
        &mut self,
        callee: Value,
        args: &[Expr],
        named_args: &[NamedArgument],
        span: Span,
    ) -> Result<Value, RuntimeError> {
        match callee {
            Value::Function(name) => {
                if !named_args.is_empty() {
                    return Err(RuntimeError {
                        msg: "function calls do not accept named arguments".into(),
                        span,
                    });
                }
                let (params, body) = if let Some(ref lexical) = self.active_module_lexical {
                    lexical
                        .fns
                        .get(&name)
                        .cloned()
                        .or_else(|| self.fns.get(&name).cloned())
                } else {
                    self.fns.get(&name).cloned()
                }
                .ok_or_else(|| RuntimeError {
                    msg: format!("undefined function `{name}`"),
                    span,
                })?;
                self.call_user_function(&name, &params, &body, args, span, None)
            }
            Value::Closure(closure) => {
                if !named_args.is_empty() || args.len() != closure.params.len() {
                    return Err(RuntimeError {
                        msg: format!(
                            "closure expects {} positional arguments, found {}",
                            closure.params.len(),
                            args.len()
                        ),
                        span,
                    });
                }
                let values: Result<Vec<_>, _> = args
                    .iter()
                    .map(|argument| self.eval_owned_expr(argument))
                    .collect();
                let values = values?;
                if self.call_depth >= MAX_CALL_DEPTH {
                    return Err(RuntimeError {
                        msg: "maximum call depth exceeded".into(),
                        span,
                    });
                }
                self.push_scope();
                for (name, value) in &closure.captures {
                    self.define(name, value.clone());
                }
                for (parameter, value) in closure.params.iter().zip(values) {
                    self.define(&parameter.name, value);
                }
                self.in_function += 1;
                self.call_depth += 1;
                let result = self.exec_stmts(&closure.body);
                self.call_depth -= 1;
                self.in_function -= 1;
                self.pop_scope(span)?;
                match result {
                    Ok(()) => Ok(Value::Void),
                    Err(Signal::Return(value)) => Ok(value),
                    Err(Signal::Break(span)) => Err(RuntimeError {
                        msg: "break outside loop".into(),
                        span,
                    }),
                    Err(Signal::Continue(span)) => Err(RuntimeError {
                        msg: "continue outside loop".into(),
                        span,
                    }),
                    Err(Signal::Error(error)) => Err(error),
                }
            }
            Value::StructConstructor(name) => {
                if !args.is_empty() {
                    return Err(RuntimeError {
                        msg: "struct construction requires named fields".into(),
                        span,
                    });
                }
                let definitions = self
                    .structs
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| RuntimeError {
                        msg: format!("unknown struct `{name}`"),
                        span,
                    })?;
                let mut fields = BTreeMap::new();
                for argument in named_args {
                    if fields.contains_key(&argument.name) {
                        return Err(RuntimeError {
                            msg: format!("duplicate field `{}`", argument.name),
                            span: argument.span,
                        });
                    }
                    if !definitions.iter().any(|field| field.name == argument.name) {
                        return Err(RuntimeError {
                            msg: format!("unknown field `{}`", argument.name),
                            span: argument.span,
                        });
                    }
                    fields.insert(
                        argument.name.clone(),
                        self.eval_owned_expr(&argument.value)?,
                    );
                }
                if let Some(missing) = definitions
                    .iter()
                    .find(|field| !fields.contains_key(&field.name))
                {
                    return Err(RuntimeError {
                        msg: format!("missing field `{}`", missing.name),
                        span,
                    });
                }
                Ok(Value::Struct(Rc::new(RefCell::new(StructValue {
                    type_name: name,
                    fields,
                }))))
            }
            Value::VariantConstructor { enum_name, variant } => {
                if !named_args.is_empty() {
                    return Err(RuntimeError {
                        msg: "enum tuple payload does not accept named arguments".into(),
                        span,
                    });
                }
                let expected = self
                    .enums
                    .get(&enum_name)
                    .and_then(|variants| variants.iter().find(|item| item.name == variant))
                    .map(|item| item.payload.len())
                    .ok_or_else(|| RuntimeError {
                        msg: format!("unknown variant `{enum_name}.{variant}`"),
                        span,
                    })?;
                if args.len() != expected {
                    return Err(RuntimeError {
                        msg: format!(
                            "variant `{enum_name}.{variant}` expects {expected} payload values, found {}",
                            args.len()
                        ),
                        span,
                    });
                }
                let payload: Result<Vec<_>, _> = args
                    .iter()
                    .map(|argument| self.eval_owned_expr(argument))
                    .collect();
                Ok(Value::Enum(EnumValue {
                    enum_name,
                    variant,
                    payload: payload?,
                }))
            }
            other => Err(RuntimeError {
                msg: format!("{} is not callable", other.kind_name()),
                span,
            }),
        }
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> Result<(), Signal> {
        match stmt {
            Stmt::Let { name, value, .. } => {
                let v = Self::complete_value(self.eval_owned_expr(value).map_err(Signal::Error)?)?;
                self.define(name, v);
            }

            Stmt::Assign { name, value, span } => {
                let v = Self::complete_value(self.eval_owned_expr(value).map_err(Signal::Error)?)?;
                let Some(previous) = self.replace(name, v) else {
                    return Err(Signal::Error(RuntimeError {
                        msg: format!("undefined variable `{}`", name),
                        span: *span,
                    }));
                };
                self.destroy_value(previous, *span).map_err(Signal::Error)?;
            }

            Stmt::FnDef {
                kind,
                name,
                params,
                body,
                ..
            } => {
                if *kind == FunctionKind::Drop {
                    let target = params
                        .first()
                        .and_then(|parameter| match &parameter.ty {
                            TypeAnnotation::Borrow { pointee, .. } => match pointee.as_ref() {
                                TypeAnnotation::Named(name)
                                | TypeAnnotation::Generic { base: name, .. } => Some(name.clone()),
                                _ => None,
                            },
                            _ => None,
                        })
                        .ok_or_else(|| {
                            Signal::Error(RuntimeError {
                                msg: "drop function requires one nominal borrow parameter".into(),
                                span: stmt.span(),
                            })
                        })?;
                    self.drop_fns.insert(target, (params.clone(), body.clone()));
                } else {
                    self.fns
                        .insert(name.clone(), (params.clone(), body.clone()));
                }
            }

            Stmt::StructDef { name, fields, .. } => {
                self.structs.insert(name.clone(), fields.clone());
            }

            Stmt::EnumDef { name, variants, .. } => {
                self.enums.insert(name.clone(), variants.clone());
            }

            Stmt::Store {
                target,
                value,
                span,
            } => {
                let Expr::Member {
                    object, property, ..
                } = target
                else {
                    return Err(Signal::Error(RuntimeError {
                        msg: "hosted v12 store currently requires a struct field lvalue".into(),
                        span: *span,
                    }));
                };
                let object = self.eval_expr(object).map_err(Signal::Error)?;
                let new_value =
                    Self::complete_value(self.eval_owned_expr(value).map_err(Signal::Error)?)?;
                let Value::Struct(object) = object else {
                    return Err(Signal::Error(RuntimeError {
                        msg: "field store requires a struct value".into(),
                        span: *span,
                    }));
                };
                let previous = {
                    let mut object = object.borrow_mut();
                    let Some(field) = object.fields.get_mut(property) else {
                        return Err(Signal::Error(RuntimeError {
                            msg: format!("struct has no field `{property}`"),
                            span: *span,
                        }));
                    };
                    std::mem::replace(field, new_value)
                };
                self.destroy_value(previous, *span).map_err(Signal::Error)?;
            }

            Stmt::Return { value, span } => {
                if self.in_function == 0 {
                    return Err(Signal::Error(RuntimeError {
                        msg: "return outside function".into(),
                        span: *span,
                    }));
                }
                let v = match value {
                    Some(e) => {
                        Self::complete_value(self.eval_owned_expr(e).map_err(Signal::Error)?)?
                    }
                    None => Value::Void,
                };
                return Err(Signal::Return(v));
            }

            Stmt::If {
                cond,
                then_body,
                else_body,
                span,
            } => {
                let cv = Self::complete_value(self.eval_expr(cond).map_err(Signal::Error)?)?;
                match cv {
                    Value::Bool(true) => {
                        self.push_scope();
                        let result = self.exec_stmts(then_body);
                        self.pop_scope(*span).map_err(Signal::Error)?;
                        result?;
                    }
                    Value::Bool(false) => {
                        if let Some(eb) = else_body {
                            self.push_scope();
                            let result = self.exec_stmts(eb);
                            self.pop_scope(*span).map_err(Signal::Error)?;
                            result?;
                        }
                    }
                    _ => {
                        return Err(Signal::Error(RuntimeError {
                            msg: "condition must be bool".into(),
                            span: *span,
                        }))
                    }
                }
            }

            Stmt::While { cond, body, span } => loop {
                let cv = Self::complete_value(self.eval_expr(cond).map_err(Signal::Error)?)?;
                match cv {
                    Value::Bool(true) => {
                        self.push_scope();
                        let r = self.exec_stmts(body);
                        self.pop_scope(*span).map_err(Signal::Error)?;
                        match r {
                            Ok(()) | Err(Signal::Continue(_)) => {}
                            Err(Signal::Break(_)) => break,
                            Err(other) => return Err(other),
                        }
                    }
                    Value::Bool(false) => break,
                    _ => {
                        return Err(Signal::Error(RuntimeError {
                            msg: "while condition must be bool".into(),
                            span: *span,
                        }));
                    }
                }
            },

            Stmt::For {
                var,
                iterable,
                body,
                span,
            } => {
                let iter_val =
                    Self::complete_value(self.eval_expr(iterable).map_err(Signal::Error)?)?;
                let items = match iter_val {
                    Value::List(vs) => vs,
                    Value::Str(s) => s.chars().map(|c| Value::Str(c.to_string())).collect(),
                    _ => {
                        return Err(Signal::Error(RuntimeError {
                            msg: "value is not iterable".into(),
                            span: *span,
                        }))
                    }
                };
                for item in items {
                    self.push_scope();
                    self.define(var, item);
                    let r = self.exec_stmts(body);
                    self.pop_scope(*span).map_err(Signal::Error)?;
                    match r {
                        Ok(()) | Err(Signal::Continue(_)) => {}
                        Err(Signal::Break(_)) => break,
                        Err(other) => return Err(other),
                    }
                }
            }

            Stmt::Match {
                value, arms, span, ..
            } => {
                let value = Self::complete_value(self.eval_expr(value).map_err(Signal::Error)?)?;
                let selected = arms.iter().find_map(|arm| match (&arm.pattern, &value) {
                    (Pattern::Wildcard(_), _) => Some((arm, Vec::new())),
                    (Pattern::Variant { path, bindings, .. }, Value::Enum(value))
                        if path.rsplit_once('.').is_some_and(|(owner, variant)| {
                            owner == value.enum_name && variant == value.variant
                        }) =>
                    {
                        Some((arm, bindings.clone()))
                    }
                    _ => None,
                });
                let Some((arm, bindings)) = selected else {
                    return Err(Signal::Error(RuntimeError {
                        msg: "non-exhaustive match reached at runtime".into(),
                        span: *span,
                    }));
                };
                self.push_scope();
                if let Value::Enum(value) = &value {
                    for (binding, payload) in bindings.iter().zip(&value.payload) {
                        self.define(binding, payload.clone());
                    }
                }
                let result = self.exec_stmts(&arm.body);
                self.pop_scope(*span).map_err(Signal::Error)?;
                result?;
            }

            Stmt::Break { span } => return Err(Signal::Break(*span)),
            Stmt::Continue { span } => return Err(Signal::Continue(*span)),

            Stmt::Import { path, alias, span } => {
                let module = self
                    .load_module(path, alias, *span)
                    .map_err(Signal::Error)?;
                self.define(alias, Value::Module(module));
            }
            Stmt::Defer { span, .. } => {
                return Err(Signal::Error(RuntimeError {
                    msg: "internal defer scheduling error".into(),
                    span: *span,
                }));
            }
            Stmt::Const { span, .. } | Stmt::Static { span, .. } => {
                return Err(Signal::Error(RuntimeError {
                    msg: "native systems statements are unavailable in the hosted interpreter"
                        .into(),
                    span: *span,
                }));
            }

            Stmt::Expr(e) => {
                Self::complete_value(self.eval_expr(e).map_err(Signal::Error)?)?;
            }
        }
        Ok(())
    }

    fn complete_value(value: Value) -> Result<Value, Signal> {
        match value {
            Value::EarlyReturn(value) => Err(Signal::Return(*value)),
            value => Ok(value),
        }
    }

    fn exec_stmts(&mut self, stmts: &[Stmt]) -> Result<(), Signal> {
        let mut defers: Vec<&[Stmt]> = Vec::new();
        let mut outcome = Ok(());
        for statement in stmts {
            if let Stmt::Defer { body, .. } = statement {
                defers.push(body);
                continue;
            }
            if let Err(signal) = self.exec_stmt(statement) {
                outcome = Err(signal);
                break;
            }
        }
        for body in defers.into_iter().rev() {
            if let Err(signal) = self.exec_stmts(body) {
                if outcome.is_ok() {
                    outcome = Err(signal);
                }
            }
        }
        outcome
    }

    pub fn run(&mut self, program: &Program) -> Result<(), RuntimeError> {
        match self.exec_stmts(&program.stmts) {
            Ok(_) => Ok(()),
            Err(Signal::Return(_)) => Err(RuntimeError {
                msg: "return outside function".into(),
                span: Span::zero(),
            }),
            Err(Signal::Break(span)) => Err(RuntimeError {
                msg: "break outside loop".into(),
                span,
            }),
            Err(Signal::Continue(span)) => Err(RuntimeError {
                msg: "continue outside loop".into(),
                span,
            }),
            Err(Signal::Error(e)) => Err(e),
        }
    }
}

fn expect_int(args: &[Value], idx: usize) -> Result<i64, String> {
    match args.get(idx) {
        Some(Value::Int(n)) => Ok(*n),
        Some(other) => Err(format!(
            "expected int argument {}, found {}",
            idx + 1,
            other.kind_name()
        )),
        None => Err(format!("missing argument {}", idx + 1)),
    }
}

fn checked_int(value: Option<i64>, span: Span) -> Result<Value, RuntimeError> {
    value.map(Value::Int).ok_or_else(|| RuntimeError {
        msg: "integer overflow".into(),
        span,
    })
}

fn expect_str(args: &[Value], idx: usize) -> Result<&str, String> {
    match args.get(idx) {
        Some(Value::Str(s)) => Ok(s),
        Some(other) => Err(format!(
            "expected str argument {}, found {}",
            idx + 1,
            other.kind_name()
        )),
        None => Err(format!("missing argument {}", idx + 1)),
    }
}

fn expect_bytes(args: &[Value], idx: usize) -> Result<&[u8], String> {
    match args.get(idx) {
        Some(Value::Bytes(bs)) => Ok(bs),
        Some(other) => Err(format!(
            "expected bytes argument {}, found {}",
            idx + 1,
            other.kind_name()
        )),
        None => Err(format!("missing argument {}", idx + 1)),
    }
}

fn expect_str_list(args: &[Value], idx: usize) -> Result<Vec<String>, String> {
    match args.get(idx) {
        Some(Value::List(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::Str(s) => out.push(s.clone()),
                    other => {
                        return Err(format!(
                            "expected list[str] argument {}, found list item {}",
                            idx + 1,
                            other.kind_name()
                        ))
                    }
                }
            }
            Ok(out)
        }
        Some(other) => Err(format!(
            "expected list[str] argument {}, found {}",
            idx + 1,
            other.kind_name()
        )),
        None => Err(format!("missing argument {}", idx + 1)),
    }
}

fn expect_buffer(args: &[Value], idx: usize) -> Result<Rc<RefCell<Vec<u8>>>, String> {
    match args.get(idx) {
        Some(Value::Buffer(b)) => Ok(b.clone()),
        Some(other) => Err(format!(
            "expected buffer argument {}, found {}",
            idx + 1,
            other.kind_name()
        )),
        None => Err(format!("missing argument {}", idx + 1)),
    }
}

fn native_math_abs(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Int(expect_int(args, 0)?.abs()))
}

fn native_math_min(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Int(expect_int(args, 0)?.min(expect_int(args, 1)?)))
}

fn native_math_max(args: &[Value]) -> Result<Value, String> {
    Ok(Value::Int(expect_int(args, 0)?.max(expect_int(args, 1)?)))
}

fn native_fs_exists(args: &[Value]) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("exists expects 1 argument".into());
    }
    let path = match &args[0] {
        Value::Str(s) => s,
        _ => return Err("exists expects a string path".into()),
    };
    Ok(Value::Int(if Path::new(path).exists() { 1 } else { 0 }))
}

fn native_fs_read_text(args: &[Value]) -> Result<Value, String> {
    let path = expect_str(args, 0)?;
    std::fs::read_to_string(path)
        .map(Value::Str)
        .map_err(|e| format!("fs.read_text failed for `{}`: {}", path, e))
}

fn native_fs_write_text(args: &[Value]) -> Result<Value, String> {
    let path = expect_str(args, 0)?;
    let text = expect_str(args, 1)?;
    std::fs::write(path, text)
        .map(|_| Value::Void)
        .map_err(|e| format!("fs.write_text failed for `{}`: {}", path, e))
}

fn native_fs_read_bytes(args: &[Value]) -> Result<Value, String> {
    let path = expect_str(args, 0)?;
    std::fs::read(path)
        .map(Value::Bytes)
        .map_err(|e| format!("fs.read_bytes failed for `{}`: {}", path, e))
}

fn native_fs_write_bytes(args: &[Value]) -> Result<Value, String> {
    let path = expect_str(args, 0)?;
    let bytes = expect_bytes(args, 1)?;
    std::fs::write(path, bytes)
        .map(|_| Value::Void)
        .map_err(|e| format!("fs.write_bytes failed for `{}`: {}", path, e))
}

fn native_fs_mkdir(args: &[Value]) -> Result<Value, String> {
    let path = expect_str(args, 0)?;
    std::fs::create_dir_all(path)
        .map(|_| Value::Void)
        .map_err(|e| format!("fs.mkdir failed for `{}`: {}", path, e))
}

fn native_fs_remove(args: &[Value]) -> Result<Value, String> {
    let path = expect_str(args, 0)?;
    let p = Path::new(path);
    if p.is_dir() {
        std::fs::remove_dir_all(p)
            .map(|_| Value::Void)
            .map_err(|e| format!("fs.remove failed for `{}`: {}", path, e))
    } else {
        std::fs::remove_file(p)
            .map(|_| Value::Void)
            .map_err(|e| format!("fs.remove failed for `{}`: {}", path, e))
    }
}

fn native_process_run(args: &[Value]) -> Result<Value, String> {
    let exe = expect_str(args, 0)?;
    let argv = expect_str_list(args, 1)?;
    let output = std::process::Command::new(exe)
        .args(&argv)
        .output()
        .map_err(|e| format!("process.run failed for `{}`: {}", exe, e))?;
    let stream = if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    Ok(Value::Str(
        String::from_utf8_lossy(stream).trim_end().to_string(),
    ))
}

fn native_encoding_hex_encode(args: &[Value]) -> Result<Value, String> {
    let bytes = expect_bytes(args, 0)?;
    Ok(Value::Str(hex::encode(bytes)))
}

fn native_encoding_hex_decode(args: &[Value]) -> Result<Value, String> {
    let s = expect_str(args, 0)?;
    hex::decode(s)
        .map(Value::Bytes)
        .map_err(|e| format!("hex_decode failed: {}", e))
}

fn native_hash_sha256(args: &[Value]) -> Result<Value, String> {
    use sha2::{Digest, Sha256};
    let bytes = expect_bytes(args, 0)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let result = hasher.finalize();
    Ok(Value::Bytes(result.to_vec()))
}

fn checked_offset(_name: &str, offset: i64) -> Result<usize, String> {
    if offset < 0 {
        return Err("offset must be non-negative".into());
    }
    Ok(offset as usize)
}

fn require_len(_name: &str, data: &[u8], offset: usize, width: usize) -> Result<(), String> {
    if offset.checked_add(width).is_none_or(|end| end > data.len()) {
        return Err(format!(
            "read out of range: need {} bytes at offset {}, but length is {}",
            width,
            offset,
            data.len()
        ));
    }
    Ok(())
}

fn native_binary_read(name: &str, args: &[Value]) -> Result<Value, String> {
    let bs = expect_bytes(args, 0)?;
    let offset = checked_offset(name, expect_int(args, 1)?)?;

    match name {
        "u8" => {
            require_len(name, bs, offset, 1)?;
            Ok(Value::Int(bs[offset] as i64))
        }
        "i8" => {
            require_len(name, bs, offset, 1)?;
            Ok(Value::Int(bs[offset] as i8 as i64))
        }
        "u16_le" => {
            require_len(name, bs, offset, 2)?;
            Ok(Value::Int(LE::read_u16(&bs[offset..]) as i64))
        }
        "u16_be" => {
            require_len(name, bs, offset, 2)?;
            Ok(Value::Int(BE::read_u16(&bs[offset..]) as i64))
        }
        "i16_le" => {
            require_len(name, bs, offset, 2)?;
            Ok(Value::Int(LE::read_i16(&bs[offset..]) as i64))
        }
        "i16_be" => {
            require_len(name, bs, offset, 2)?;
            Ok(Value::Int(BE::read_i16(&bs[offset..]) as i64))
        }
        "u32_le" => {
            require_len(name, bs, offset, 4)?;
            Ok(Value::Int(LE::read_u32(&bs[offset..]) as i64))
        }
        "u32_be" => {
            require_len(name, bs, offset, 4)?;
            Ok(Value::Int(BE::read_u32(&bs[offset..]) as i64))
        }
        "i32_le" => {
            require_len(name, bs, offset, 4)?;
            Ok(Value::Int(LE::read_i32(&bs[offset..]) as i64))
        }
        "i32_be" => {
            require_len(name, bs, offset, 4)?;
            Ok(Value::Int(BE::read_i32(&bs[offset..]) as i64))
        }
        _ => unreachable!(),
    }
}

fn native_binary_pack(name: &str, args: &[Value]) -> Result<Value, String> {
    let val = expect_int(args, 0)?;

    match name {
        "pack_u16_le" => {
            if !(0..=65535).contains(&val) {
                return Err(format!(
                    "std.binary.pack_u16_le failed: value {} out of u16 range",
                    val
                ));
            }
            let mut buf = [0u8; 2];
            LE::write_u16(&mut buf, val as u16);
            Ok(Value::Bytes(buf.to_vec()))
        }
        "pack_u16_be" => {
            if !(0..=65535).contains(&val) {
                return Err(format!(
                    "std.binary.pack_u16_be failed: value {} out of u16 range",
                    val
                ));
            }
            let mut buf = [0u8; 2];
            BE::write_u16(&mut buf, val as u16);
            Ok(Value::Bytes(buf.to_vec()))
        }
        "pack_u32_le" => {
            if !(0..=4294967295).contains(&val) {
                return Err(format!(
                    "std.binary.pack_u32_le failed: value {} out of u32 range",
                    val
                ));
            }
            let mut buf = [0u8; 4];
            LE::write_u32(&mut buf, val as u32);
            Ok(Value::Bytes(buf.to_vec()))
        }
        "pack_u32_be" => {
            if !(0..=4294967295).contains(&val) {
                return Err(format!(
                    "std.binary.pack_u32_be failed: value {} out of u32 range",
                    val
                ));
            }
            let mut buf = [0u8; 4];
            BE::write_u32(&mut buf, val as u32);
            Ok(Value::Bytes(buf.to_vec()))
        }
        _ => unreachable!(),
    }
}

fn native_binary_u8(args: &[Value]) -> Result<Value, String> {
    native_binary_read("u8", args)
}
fn native_binary_i8(args: &[Value]) -> Result<Value, String> {
    native_binary_read("i8", args)
}
fn native_binary_u16_le(args: &[Value]) -> Result<Value, String> {
    native_binary_read("u16_le", args)
}
fn native_binary_u16_be(args: &[Value]) -> Result<Value, String> {
    native_binary_read("u16_be", args)
}
fn native_binary_i16_le(args: &[Value]) -> Result<Value, String> {
    native_binary_read("i16_le", args)
}
fn native_binary_i16_be(args: &[Value]) -> Result<Value, String> {
    native_binary_read("i16_be", args)
}
fn native_binary_u32_le(args: &[Value]) -> Result<Value, String> {
    native_binary_read("u32_le", args)
}
fn native_binary_u32_be(args: &[Value]) -> Result<Value, String> {
    native_binary_read("u32_be", args)
}
fn native_binary_i32_le(args: &[Value]) -> Result<Value, String> {
    native_binary_read("i32_le", args)
}
fn native_binary_i32_be(args: &[Value]) -> Result<Value, String> {
    native_binary_read("i32_be", args)
}
fn native_binary_pack_u16_le(args: &[Value]) -> Result<Value, String> {
    native_binary_pack("pack_u16_le", args)
}
fn native_binary_pack_u16_be(args: &[Value]) -> Result<Value, String> {
    native_binary_pack("pack_u16_be", args)
}
fn native_binary_pack_u32_le(args: &[Value]) -> Result<Value, String> {
    native_binary_pack("pack_u32_le", args)
}
fn native_binary_pack_u32_be(args: &[Value]) -> Result<Value, String> {
    native_binary_pack("pack_u32_be", args)
}

fn native_binary_write_u16_le(args: &[Value]) -> Result<Value, String> {
    native_binary_write("write_u16_le", args)
}
fn native_binary_write_u16_be(args: &[Value]) -> Result<Value, String> {
    native_binary_write("write_u16_be", args)
}
fn native_binary_write_u32_le(args: &[Value]) -> Result<Value, String> {
    native_binary_write("write_u32_le", args)
}
fn native_binary_write_u32_be(args: &[Value]) -> Result<Value, String> {
    native_binary_write("write_u32_be", args)
}

fn native_buffer_new(args: &[Value]) -> Result<Value, String> {
    let size = expect_int(args, 0)?;
    if size < 0 {
        return Err("buffer size must be non-negative".into());
    }
    Ok(Value::Buffer(Rc::new(RefCell::new(vec![
        0u8;
        size as usize
    ]))))
}

fn native_buffer_from_bytes(args: &[Value]) -> Result<Value, String> {
    let bs = expect_bytes(args, 0)?;
    Ok(Value::Buffer(Rc::new(RefCell::new(bs.to_vec()))))
}

fn native_buffer_to_bytes(args: &[Value]) -> Result<Value, String> {
    let b = expect_buffer(args, 0)?;
    let val = Value::Bytes(b.borrow().clone());
    Ok(val)
}

fn native_buffer_len(args: &[Value]) -> Result<Value, String> {
    let b = expect_buffer(args, 0)?;
    let len = b.borrow().len() as i64;
    Ok(Value::Int(len))
}

fn native_buffer_get(args: &[Value]) -> Result<Value, String> {
    let b = expect_buffer(args, 0)?;
    let offset = expect_int(args, 1)?;
    if offset < 0 {
        return Err("offset must be non-negative".into());
    }
    let buf = b.borrow();
    if offset as usize >= buf.len() {
        return Err(format!(
            "buffer get out of range: offset {}, but length is {}",
            offset,
            buf.len()
        ));
    }
    Ok(Value::Int(buf[offset as usize] as i64))
}

fn native_buffer_set(args: &[Value]) -> Result<Value, String> {
    let b = expect_buffer(args, 0)?;
    let offset = expect_int(args, 1)?;
    let val = expect_int(args, 2)?;
    if offset < 0 {
        return Err("offset must be non-negative".into());
    }
    if !(0..=255).contains(&val) {
        return Err(format!("buffer set value out of range: {}", val));
    }
    let mut buf = b.borrow_mut();
    if offset as usize >= buf.len() {
        return Err(format!(
            "buffer set out of range: offset {}, but length is {}",
            offset,
            buf.len()
        ));
    }
    buf[offset as usize] = val as u8;
    Ok(Value::Void)
}

fn native_buffer_slice(args: &[Value]) -> Result<Value, String> {
    let b = expect_buffer(args, 0)?;
    let offset = expect_int(args, 1)?;
    let length = expect_int(args, 2)?;
    if offset < 0 {
        return Err("offset must be non-negative".into());
    }
    if length < 0 {
        return Err("length must be non-negative".into());
    }
    let buf = b.borrow();
    if offset
        .checked_add(length)
        .is_none_or(|end| end as usize > buf.len())
    {
        return Err(format!(
            "buffer slice out of range: need {} bytes at offset {}, but length is {}",
            length,
            offset,
            buf.len()
        ));
    }
    let start = offset as usize;
    let end = start + length as usize;
    Ok(Value::Bytes(buf[start..end].to_vec()))
}

fn native_buffer_load(args: &[Value]) -> Result<Value, String> {
    let path = expect_str(args, 0)?;
    let data =
        std::fs::read(path).map_err(|e| format!("buffer.load failed for `{}`: {}", path, e))?;
    Ok(Value::Buffer(Rc::new(RefCell::new(data))))
}

fn native_buffer_save(args: &[Value]) -> Result<Value, String> {
    let path = expect_str(args, 0)?;
    let b = expect_buffer(args, 1)?;
    std::fs::write(path, &*b.borrow())
        .map_err(|e| format!("buffer.save failed for `{}`: {}", path, e))?;
    Ok(Value::Void)
}

fn native_buffer_find(args: &[Value]) -> Result<Value, String> {
    let b = expect_buffer(args, 0)?;
    let pattern = expect_bytes(args, 1)?;
    if pattern.is_empty() {
        return Err("buffer.find: pattern cannot be empty".into());
    }
    let buf = b.borrow();
    let pos = match pattern.len() {
        1 => memchr::memchr(pattern[0], &buf)
            .map(|p| p as i64)
            .unwrap_or(-1),
        _ => memmem::find(&buf, pattern).map(|p| p as i64).unwrap_or(-1),
    };
    Ok(Value::Int(pos))
}

fn native_buffer_replace(args: &[Value]) -> Result<Value, String> {
    let b = expect_buffer(args, 0)?;
    let offset = expect_int(args, 1)?;
    let data = expect_bytes(args, 2)?;
    if offset < 0 {
        return Err("offset must be non-negative".into());
    }
    let mut buf = b.borrow_mut();
    let start = offset as usize;
    let end = start + data.len();
    if end > buf.len() {
        return Err(format!(
            "buffer.replace out of range: need {} bytes at offset {}, but length is {}",
            data.len(),
            offset,
            buf.len()
        ));
    }
    buf[start..end].copy_from_slice(data);
    Ok(Value::Void)
}

fn native_binary_write(name: &str, args: &[Value]) -> Result<Value, String> {
    let b = expect_buffer(args, 0)?;
    let offset = expect_int(args, 1)?;
    let val = expect_int(args, 2)?;
    if offset < 0 {
        return Err("offset must be non-negative".into());
    }
    let mut buf = b.borrow_mut();

    match name {
        "write_u16_le" => {
            if !(0..=65535).contains(&val) {
                return Err(format!("value {} out of u16 range", val));
            }
            if offset as usize + 2 > buf.len() {
                return Err(format!(
                    "write out of range: need 2 bytes at offset {}, but length is {}",
                    offset,
                    buf.len()
                ));
            }
            LE::write_u16(&mut buf[offset as usize..], val as u16);
        }
        "write_u16_be" => {
            if !(0..=65535).contains(&val) {
                return Err(format!("value {} out of u16 range", val));
            }
            if offset as usize + 2 > buf.len() {
                return Err(format!(
                    "write out of range: need 2 bytes at offset {}, but length is {}",
                    offset,
                    buf.len()
                ));
            }
            BE::write_u16(&mut buf[offset as usize..], val as u16);
        }
        "write_u32_le" => {
            if !(0..=4294967295).contains(&val) {
                return Err(format!("value {} out of u32 range", val));
            }
            if offset as usize + 4 > buf.len() {
                return Err(format!(
                    "write out of range: need 4 bytes at offset {}, but length is {}",
                    offset,
                    buf.len()
                ));
            }
            LE::write_u32(&mut buf[offset as usize..], val as u32);
        }
        "write_u32_be" => {
            if !(0..=4294967295).contains(&val) {
                return Err(format!("value {} out of u32 range", val));
            }
            if offset as usize + 4 > buf.len() {
                return Err(format!(
                    "write out of range: need 4 bytes at offset {}, but length is {}",
                    offset,
                    buf.len()
                ));
            }
            BE::write_u32(&mut buf[offset as usize..], val as u32);
        }
        _ => unreachable!(),
    }
    Ok(Value::Void)
}

fn format_module_error(error: &ModuleError) -> String {
    match error {
        ModuleError::LocalImportWithoutSource { requested } => {
            format!("local import requires a source file path: {}", requested)
        }
        ModuleError::LocalModuleNotFound { requested, .. } => {
            format!("local module not found: {}", requested)
        }
    }
}

#[cfg(test)]
mod remainder_tests {
    use super::*;
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;

    fn parse(source: &str) -> dbyte_ast::Program {
        Parser::new(Lexer::new(source).tokenize().unwrap())
            .parse_program()
            .unwrap()
    }

    #[test]
    fn signed_remainder_matches_truncation_and_zero_is_an_error() {
        let mut interpreter = Interpreter::new();
        interpreter.capture_output();
        interpreter.run(&parse("print(-17 % 5)\n")).unwrap();
        assert_eq!(interpreter.take_captured_output(), "-2\n");

        let error = Interpreter::new()
            .run(&parse("let value: int = 1 % 0\n"))
            .unwrap_err();
        assert_eq!(error.msg, "remainder by zero");
    }

    #[test]
    fn hosted_v12_executes_erased_generics_aggregates_match_closures_and_defer() {
        let source = "struct Pair[A, B]:\n    first: A\n    second: B\n\nfn id[T](value: T) -> T:\n    return value\n\nlet pair = Pair[i32, u8](first = id[i32](40), second = 2)\npair.first = pair.first + pair.second\nlet result = Result[i32, u8].Ok(pair.first)\nmatch result:\n    case Result.Ok(value):\n        print(value)\n    case Result.Err(error):\n        print(error)\nlet factor = 2\nlet add = fn [borrow factor](value: i32) -> i32:\n    return value + factor\nprint(add(40))\ndefer []:\n    print(9)\n";
        let mut interpreter = Interpreter::new();
        interpreter.capture_output();
        interpreter.run(&parse(source)).unwrap();
        assert_eq!(interpreter.take_captured_output(), "42\n42\n9\n");
    }

    #[test]
    fn hosted_defer_runs_on_return_and_loop_control() {
        let source = "fn work() -> i32:\n    defer []:\n        print(3)\n    return 7\n\nlet count = 0\nwhile count < 2:\n    defer []:\n        print(count)\n    count = count + 1\n    continue\nprint(work())\n";
        let mut interpreter = Interpreter::new();
        interpreter.capture_output();
        interpreter.run(&parse(source)).unwrap();
        assert_eq!(interpreter.take_captured_output(), "1\n2\n3\n7\n");
    }

    #[test]
    fn hosted_result_question_mark_returns_err_and_runs_defer() {
        let source = "fn unwrap(input: Result[i32, u8]) -> Result[i32, u8]:\n    defer []:\n        print(5)\n    let value = input?\n    return Result[i32, u8].Ok(value + 1)\n\nlet failed = unwrap(Result[i32, u8].Err(7))\nmatch failed:\n    case Result.Ok(value):\n        print(value)\n    case Result.Err(error):\n        print(error)\n";
        let mut interpreter = Interpreter::new();
        interpreter.capture_output();
        interpreter.run(&parse(source)).unwrap();
        assert_eq!(interpreter.take_captured_output(), "5\n7\n");
    }

    #[test]
    fn explicit_drop_executes_custom_then_structural_cleanup() {
        let source = "struct Child:\n    value: int\n\ndrop fn destroy_child(value: borrow mut Child):\n    print((*value).value)\n\nstruct Parent:\n    first: Child\n    second: Child\n\ndrop fn destroy_parent(value: borrow mut Parent):\n    print(3)\n\nlet resource = Parent(first = Child(value = 1), second = Child(value = 2))\ndrop(resource)\n";
        let mut interpreter = Interpreter::new();
        interpreter.capture_output();
        interpreter.run(&parse(source)).unwrap();
        assert_eq!(interpreter.take_captured_output(), "3\n2\n1\n");
    }

    #[test]
    fn automatic_drop_runs_in_reverse_activation_order_before_return() {
        let source = "struct Resource:\n    value: int\n\ndrop fn destroy_resource(value: borrow mut Resource):\n    print((*value).value)\n\nfn work() -> int:\n    let first = Resource(value = 1)\n    let second = Resource(value = 2)\n    return 7\n\nprint(work())\n";
        let mut interpreter = Interpreter::new();
        interpreter.capture_output();
        interpreter.run(&parse(source)).unwrap();
        assert_eq!(interpreter.take_captured_output(), "2\n1\n7\n");
    }

    #[test]
    fn reassignment_drops_old_value_after_rhs_succeeds() {
        let source = "struct Resource:\n    value: int\n\ndrop fn destroy_resource(value: borrow mut Resource):\n    print((*value).value)\n\nfn work() -> int:\n    let resource = Resource(value = 1)\n    resource = Resource(value = 2)\n    return 0\n\nprint(work())\n";
        let mut interpreter = Interpreter::new();
        interpreter.capture_output();
        interpreter.run(&parse(source)).unwrap();
        assert_eq!(interpreter.take_captured_output(), "1\n2\n0\n");
    }

    #[test]
    fn hosted_v12_parity_corpus_has_exact_tree_output() {
        let source = "struct Pair[A, B]:\n    first: A\n    second: B\n\nfn id[T](value: T) -> T:\n    return value\n\nfn attempt(input: Result[int, int]) -> Result[int, int]:\n    defer []:\n        print(\"CLEAN\")\n    let value = input?\n    return Result[int, int].Ok(value)\n\nlet pair = Pair[int, int](first = id[int](40), second = 2)\nprint(pair.first + pair.second)\nlet factor: int = 2\nlet add = fn [borrow factor](value: int) -> int:\n    return value + factor\nprint(add(40))\nlet result = attempt(Result[int, int].Err(7))\nmatch result:\n    case Result.Ok(value):\n        print(value)\n    case Result.Err(error):\n        print(error)\n";
        let mut interpreter = Interpreter::new();
        interpreter.capture_output();
        interpreter.run(&parse(source)).unwrap();
        assert_eq!(interpreter.take_captured_output(), "42\n42\nCLEAN\n7\n");
    }
}
