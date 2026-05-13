use byteorder::{ByteOrder, BE, LE};
use dbyte_ast::*;
use dbyte_lexer::Lexer;
use dbyte_module::{resolve_import, ImportTarget, ModuleError, ModuleState};
use dbyte_parser::Parser;
use memchr::memmem;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

type NativeFn = fn(&[Value]) -> Result<Value, String>;
// Keep this below the host thread stack ceiling so recursion fails as a DByte
// RuntimeError instead of aborting the Rust process.
const MAX_CALL_DEPTH: usize = 32;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Bytes(Vec<u8>),
    Buffer(Rc<RefCell<Vec<u8>>>),
    List(Vec<Value>),
    Module(ModuleValue),
    Void,
}

#[derive(Debug, Clone)]
pub struct ModuleValue {
    pub alias: String,
    pub members: HashMap<String, ModuleMember>,
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
    fns: HashMap<String, (Vec<Param>, Vec<Stmt>)>,
    current_file: Option<PathBuf>,
    module_cache: HashMap<String, ModuleState<ModuleValue>>,
    loading_stack: Vec<String>,
    in_function: usize,
    call_depth: usize,
    output: OutputSink,
    script_args: Vec<String>,
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Interpreter {
    pub fn new() -> Self {
        Self {
            env: vec![HashMap::new()],
            fns: HashMap::new(),
            current_file: None,
            module_cache: HashMap::new(),
            loading_stack: Vec::new(),
            in_function: 0,
            call_depth: 0,
            output: OutputSink::Stdout,
            script_args: Vec::new(),
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
    }
    fn pop_scope(&mut self) {
        self.env.pop();
    }

    fn get(&self, name: &str) -> Option<Value> {
        for scope in self.env.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v.clone());
            }
        }
        None
    }

    fn set(&mut self, name: &str, val: Value) -> bool {
        for scope in self.env.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), val);
                return true;
            }
        }
        false
    }

    fn define(&mut self, name: &str, val: Value) {
        self.env.last_mut().unwrap().insert(name.to_string(), val);
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
        let saved_fns = std::mem::take(&mut self.fns);
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

        self.env = saved_env;
        self.fns = saved_fns;
        self.current_file = saved_file;
        self.in_function = saved_in_function;
        self.call_depth = saved_call_depth;

        match executed {
            Ok(()) => Ok(ModuleValue {
                alias: alias.to_string(),
                members,
            }),
            Err(Signal::Error(e)) => Err(e),
            Err(Signal::Return(_)) => Err(RuntimeError {
                msg: "return outside function".into(),
                span,
            }),
        }
    }

    fn eval_member(
        &mut self,
        object: &Expr,
        property: &str,
        span: Span,
    ) -> Result<ModuleMember, RuntimeError> {
        match self.eval_expr(object)? {
            Value::Module(module) => {
                module
                    .members
                    .get(property)
                    .cloned()
                    .ok_or_else(|| RuntimeError {
                        msg: format!(
                            "module '{}' has no public member '{}'",
                            module.alias, property
                        ),
                        span,
                    })
            }
            other => Err(RuntimeError {
                msg: format!("member access not supported for `{}`", other),
                span,
            }),
        }
    }

    fn eval_expr(&mut self, expr: &Expr) -> Result<Value, RuntimeError> {
        match expr {
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

            Expr::Ident(name, span) => self.get(name).ok_or_else(|| RuntimeError {
                msg: format!("undefined variable `{}`", name),
                span: *span,
            }),

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
                        BinOp::EqEq => Ok(Value::Bool(a == b)),
                        BinOp::NotEq => Ok(Value::Bool(a != b)),
                        BinOp::Lt => Ok(Value::Bool(a < b)),
                        BinOp::LtEq => Ok(Value::Bool(a <= b)),
                        BinOp::Gt => Ok(Value::Bool(a > b)),
                        BinOp::GtEq => Ok(Value::Bool(a >= b)),
                    },
                    (Value::Float(a), Value::Float(b)) => match op {
                        BinOp::Add => Ok(Value::Float(a + b)),
                        BinOp::Sub => Ok(Value::Float(a - b)),
                        BinOp::Mul => Ok(Value::Float(a * b)),
                        BinOp::Div => Ok(Value::Float(a / b)),
                        BinOp::EqEq => Ok(Value::Bool(a == b)),
                        BinOp::NotEq => Ok(Value::Bool(a != b)),
                        BinOp::Lt => Ok(Value::Bool(a < b)),
                        BinOp::LtEq => Ok(Value::Bool(a <= b)),
                        BinOp::Gt => Ok(Value::Bool(a > b)),
                        BinOp::GtEq => Ok(Value::Bool(a >= b)),
                    },
                    (Value::Str(a), Value::Str(b)) => match op {
                        BinOp::Add => Ok(Value::Str(a + &b)),
                        BinOp::EqEq => Ok(Value::Bool(a == b)),
                        BinOp::NotEq => Ok(Value::Bool(a != b)),
                        _ => Err(RuntimeError {
                            msg: "unsupported str operation".into(),
                            span: *span,
                        }),
                    },
                    (Value::Bool(a), Value::Bool(b)) => match op {
                        BinOp::EqEq => Ok(Value::Bool(a == b)),
                        BinOp::NotEq => Ok(Value::Bool(a != b)),
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

                let (params, body) = match self.fns.get(name).cloned() {
                    Some(f) => f,
                    None => {
                        return Err(RuntimeError {
                            msg: format!("undefined function `{}`", name),
                            span: *span,
                        })
                    }
                };
                self.call_user_function(name, &params, &body, args, *span)
            }

            Expr::Member {
                object,
                property,
                span,
            } => match self.eval_member(object, property, *span)? {
                ModuleMember::Value(value) => Ok(value),
                ModuleMember::Function(_, _) | ModuleMember::Native(_) | ModuleMember::EnvArgs => {
                    Err(RuntimeError {
                        msg: format!("module member `{}` is callable", property),
                        span: *span,
                    })
                }
            },

            Expr::MemberCall {
                object,
                property,
                args,
                span,
            } => match self.eval_member(object, property, *span)? {
                ModuleMember::Function(params, body) => {
                    self.call_user_function(property, &params, &body, args, *span)
                }
                ModuleMember::Native(f) => {
                    let vals: Result<Vec<_>, _> = args.iter().map(|a| self.eval_expr(a)).collect();
                    f(&vals?).map_err(|msg| RuntimeError { msg, span: *span })
                }
                ModuleMember::EnvArgs => {
                    if !args.is_empty() {
                        return Err(RuntimeError {
                            msg: format!("function `env.args` expects 0 args, got {}", args.len()),
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
            },
        }
    }

    fn call_user_function(
        &mut self,
        name: &str,
        params: &[Param],
        body: &[Stmt],
        args: &[Expr],
        span: Span,
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

        let arg_vals: Result<Vec<_>, _> = args.iter().map(|a| self.eval_expr(a)).collect();
        let arg_vals = arg_vals?;
        self.push_scope();
        self.in_function += 1;
        self.call_depth += 1;
        for (p, v) in params.iter().zip(arg_vals) {
            self.define(&p.name, v);
        }
        let result = self.exec_stmts(body);
        self.call_depth -= 1;
        self.in_function -= 1;
        self.pop_scope();
        match result {
            Ok(_) => Ok(Value::Void),
            Err(Signal::Return(v)) => Ok(v),
            Err(Signal::Error(e)) => Err(e),
        }
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> Result<(), Signal> {
        match stmt {
            Stmt::Let { name, value, .. } => {
                let v = self.eval_expr(value).map_err(Signal::Error)?;
                self.define(name, v);
            }

            Stmt::Assign { name, value, span } => {
                let v = self.eval_expr(value).map_err(Signal::Error)?;
                if !self.set(name, v) {
                    return Err(Signal::Error(RuntimeError {
                        msg: format!("undefined variable `{}`", name),
                        span: *span,
                    }));
                }
            }

            Stmt::FnDef {
                name, params, body, ..
            } => {
                self.fns
                    .insert(name.clone(), (params.clone(), body.clone()));
            }

            Stmt::Return { value, span } => {
                if self.in_function == 0 {
                    return Err(Signal::Error(RuntimeError {
                        msg: "return outside function".into(),
                        span: *span,
                    }));
                }
                let v = match value {
                    Some(e) => self.eval_expr(e).map_err(Signal::Error)?,
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
                let cv = self.eval_expr(cond).map_err(Signal::Error)?;
                match cv {
                    Value::Bool(true) => {
                        self.push_scope();
                        self.exec_stmts(then_body)?;
                        self.pop_scope();
                    }
                    Value::Bool(false) => {
                        if let Some(eb) = else_body {
                            self.push_scope();
                            self.exec_stmts(eb)?;
                            self.pop_scope();
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
                let cv = self.eval_expr(cond).map_err(Signal::Error)?;
                match cv {
                    Value::Bool(true) => {
                        self.push_scope();
                        let r = self.exec_stmts(body);
                        self.pop_scope();
                        r?;
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
                let iter_val = self.eval_expr(iterable).map_err(Signal::Error)?;
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
                    self.pop_scope();
                    r?;
                }
            }

            Stmt::Import { path, alias, span } => {
                let module = self
                    .load_module(path, alias, *span)
                    .map_err(Signal::Error)?;
                self.define(alias, Value::Module(module));
            }

            Stmt::Expr(e) => {
                self.eval_expr(e).map_err(Signal::Error)?;
            }
        }
        Ok(())
    }

    fn exec_stmts(&mut self, stmts: &[Stmt]) -> Result<(), Signal> {
        for s in stmts {
            self.exec_stmt(s)?;
        }
        Ok(())
    }

    pub fn run(&mut self, program: &Program) -> Result<(), RuntimeError> {
        match self.exec_stmts(&program.stmts) {
            Ok(_) => Ok(()),
            Err(Signal::Return(_)) => Err(RuntimeError {
                msg: "return outside function".into(),
                span: Span::zero(),
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
