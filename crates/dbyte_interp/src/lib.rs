use dbyte_ast::*;
use dbyte_lexer::Lexer;
use dbyte_module::{resolve_import, ImportTarget, ModuleState};
use dbyte_parser::Parser;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

type NativeFn = fn(&[Value]) -> Result<Value, String>;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
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
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Str(s) => write!(f, "{}", s),
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

pub struct Interpreter {
    env: Vec<HashMap<String, Value>>,
    fns: HashMap<String, (Vec<Param>, Vec<Stmt>)>,
    current_file: Option<PathBuf>,
    module_cache: HashMap<String, ModuleState<ModuleValue>>,
    in_function: usize,
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
            in_function: 0,
        }
    }

    pub fn with_entry_path(path: impl Into<PathBuf>) -> Self {
        let mut interp = Self::new();
        interp.current_file = Some(path.into());
        interp
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
                msg: format!("ImportError: {}", e),
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
                return Err(RuntimeError {
                    msg: "ImportError: circular import detected".into(),
                    span,
                });
            }
            None => {}
        }

        self.module_cache.insert(key.clone(), ModuleState::Loading);
        let loaded = match target {
            ImportTarget::Std(name) => Self::load_std_module(&name, alias, span),
            ImportTarget::File(path) => self.load_file_module(&path, alias, span),
        };

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
            }
            "std.env" => {
                members.insert("args".into(), ModuleMember::Native(native_env_args));
            }
            _ => {
                return Err(RuntimeError {
                    msg: format!("ImportError: unknown std module `{}`", name),
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
        self.in_function = 0;

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
                let i = match ival {
                    Value::Int(n) => n,
                    _ => {
                        return Err(RuntimeError {
                            msg: "list index must be int".into(),
                            span: *span,
                        })
                    }
                };
                match tval {
                    Value::List(vs) => {
                        let idx = if i < 0 { vs.len() as i64 + i } else { i };
                        if idx < 0 || idx as usize >= vs.len() {
                            return Err(RuntimeError {
                                msg: "index out of range".into(),
                                span: *span,
                            });
                        }
                        Ok(vs[idx as usize].clone())
                    }
                    _ => Err(RuntimeError {
                        msg: "value is not indexable".into(),
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
                        BinOp::Add => Ok(Value::Int(a + b)),
                        BinOp::Sub => Ok(Value::Int(a - b)),
                        BinOp::Mul => Ok(Value::Int(a * b)),
                        BinOp::Div => {
                            if b == 0 {
                                return Err(RuntimeError {
                                    msg: "division by zero".into(),
                                    span: *span,
                                });
                            }
                            Ok(Value::Int(a / b))
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
                        Value::Int(n) => Ok(Value::Int(-n)),
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
                    println!("{}", strs.join(" "));
                    return Ok(Value::Void);
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
                ModuleMember::Function(_, _) | ModuleMember::Native(_) => Err(RuntimeError {
                    msg: format!("module member `{}` is callable", property),
                    span: *span,
                }),
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

        let arg_vals: Result<Vec<_>, _> = args.iter().map(|a| self.eval_expr(a)).collect();
        let arg_vals = arg_vals?;
        self.push_scope();
        self.in_function += 1;
        for (p, v) in params.iter().zip(arg_vals) {
            self.define(&p.name, v);
        }
        let result = self.exec_stmts(body);
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
            other
        )),
        None => Err(format!("missing argument {}", idx + 1)),
    }
}

fn expect_str(args: &[Value], idx: usize) -> Result<&str, String> {
    match args.get(idx) {
        Some(Value::Str(s)) => Ok(s),
        Some(other) => Err(format!(
            "expected str argument {}, found {}",
            idx + 1,
            other
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

fn native_env_args(args: &[Value]) -> Result<Value, String> {
    if !args.is_empty() {
        return Err(format!(
            "function `env.args` expects 0 args, got {}",
            args.len()
        ));
    }
    Ok(Value::List(std::env::args().map(Value::Str).collect()))
}
