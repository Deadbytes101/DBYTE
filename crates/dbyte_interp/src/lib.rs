use dbyte_ast::*;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Void,
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(n)   => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Bool(b)  => write!(f, "{}", b),
            Value::Str(s)   => write!(f, "{}", s),
            Value::Void     => write!(f, ""),
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
}

impl Interpreter {
    pub fn new() -> Self {
        Self {
            env: vec![HashMap::new()],
            fns: HashMap::new(),
        }
    }

    fn push_scope(&mut self) { self.env.push(HashMap::new()); }
    fn pop_scope(&mut self)  { self.env.pop(); }

    fn get(&self, name: &str) -> Option<Value> {
        for scope in self.env.iter().rev() {
            if let Some(v) = scope.get(name) { return Some(v.clone()); }
        }
        None
    }

    fn set(&mut self, name: &str, val: Value) {
        for scope in self.env.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), val);
                return;
            }
        }
        self.env.last_mut().unwrap().insert(name.to_string(), val);
    }

    fn define(&mut self, name: &str, val: Value) {
        self.env.last_mut().unwrap().insert(name.to_string(), val);
    }

    fn eval_expr(&mut self, expr: &Expr) -> Result<Value, RuntimeError> {
        match expr {
            Expr::IntLit(n, _)   => Ok(Value::Int(*n)),
            Expr::FloatLit(n, _) => Ok(Value::Float(*n)),
            Expr::BoolLit(b, _)  => Ok(Value::Bool(*b)),
            Expr::StrLit(s, _)   => Ok(Value::Str(s.clone())),

            Expr::Ident(name, span) => {
                self.get(name).ok_or_else(|| RuntimeError {
                    msg: format!("undefined variable `{}`", name),
                    span: *span,
                })
            }

            Expr::Binary { left, op, right, span } => {
                let lv = self.eval_expr(left)?;
                let rv = self.eval_expr(right)?;
                match (lv, rv) {
                    (Value::Int(a), Value::Int(b)) => match op {
                        BinOp::Add  => Ok(Value::Int(a + b)),
                        BinOp::Sub  => Ok(Value::Int(a - b)),
                        BinOp::Mul  => Ok(Value::Int(a * b)),
                        BinOp::Div  => {
                            if b == 0 { return Err(RuntimeError { msg: "division by zero".into(), span: *span }); }
                            Ok(Value::Int(a / b))
                        }
                        BinOp::EqEq  => Ok(Value::Bool(a == b)),
                        BinOp::NotEq => Ok(Value::Bool(a != b)),
                        BinOp::Lt    => Ok(Value::Bool(a < b)),
                        BinOp::LtEq  => Ok(Value::Bool(a <= b)),
                        BinOp::Gt    => Ok(Value::Bool(a > b)),
                        BinOp::GtEq  => Ok(Value::Bool(a >= b)),
                    },
                    (Value::Float(a), Value::Float(b)) => match op {
                        BinOp::Add  => Ok(Value::Float(a + b)),
                        BinOp::Sub  => Ok(Value::Float(a - b)),
                        BinOp::Mul  => Ok(Value::Float(a * b)),
                        BinOp::Div  => Ok(Value::Float(a / b)),
                        BinOp::EqEq  => Ok(Value::Bool(a == b)),
                        BinOp::NotEq => Ok(Value::Bool(a != b)),
                        BinOp::Lt    => Ok(Value::Bool(a < b)),
                        BinOp::LtEq  => Ok(Value::Bool(a <= b)),
                        BinOp::Gt    => Ok(Value::Bool(a > b)),
                        BinOp::GtEq  => Ok(Value::Bool(a >= b)),
                    },
                    (Value::Str(a), Value::Str(b)) => match op {
                        BinOp::Add   => Ok(Value::Str(a + &b)),
                        BinOp::EqEq  => Ok(Value::Bool(a == b)),
                        BinOp::NotEq => Ok(Value::Bool(a != b)),
                        _ => Err(RuntimeError { msg: "unsupported str operation".into(), span: *span }),
                    },
                    (Value::Bool(a), Value::Bool(b)) => match op {
                        BinOp::EqEq  => Ok(Value::Bool(a == b)),
                        BinOp::NotEq => Ok(Value::Bool(a != b)),
                        _ => Err(RuntimeError { msg: "unsupported bool operation".into(), span: *span }),
                    },
                    _ => Err(RuntimeError { msg: "type mismatch in binary expression".into(), span: *span }),
                }
            }

            Expr::Unary { op, expr, span } => {
                let v = self.eval_expr(expr)?;
                match op {
                    UnaryOp::Neg => match v {
                        Value::Int(n)   => Ok(Value::Int(-n)),
                        Value::Float(n) => Ok(Value::Float(-n)),
                        _ => Err(RuntimeError { msg: "unary `-` requires numeric type".into(), span: *span }),
                    },
                    UnaryOp::Not => match v {
                        Value::Bool(b) => Ok(Value::Bool(!b)),
                        _ => Err(RuntimeError { msg: "unary `!` requires bool".into(), span: *span }),
                    },
                }
            }

            Expr::Call { name, args, span } => {
                if name == "print" {
                    let parts: Result<Vec<_>, _> = args.iter().map(|a| self.eval_expr(a)).collect();
                    let vals = parts?;
                    let strs: Vec<String> = vals.iter().map(|v| format!("{}", v)).collect();
                    println!("{}", strs.join(" "));
                    return Ok(Value::Void);
                }

                let (params, body) = match self.fns.get(name).cloned() {
                    Some(f) => f,
                    None => return Err(RuntimeError {
                        msg: format!("undefined function `{}`", name),
                        span: *span,
                    }),
                };

                if args.len() != params.len() {
                    return Err(RuntimeError {
                        msg: format!("function `{}` expects {} args, got {}", name, params.len(), args.len()),
                        span: *span,
                    });
                }

                let arg_vals: Result<Vec<_>, _> = args.iter().map(|a| self.eval_expr(a)).collect();
                let arg_vals = arg_vals?;

                self.push_scope();
                for (p, v) in params.iter().zip(arg_vals.into_iter()) {
                    self.define(&p.name, v);
                }

                let result = self.exec_stmts_returning(&body);
                self.pop_scope();

                match result {
                    Ok(v) => Ok(v.unwrap_or(Value::Void)),
                    Err(Signal::Return(v)) => Ok(v),
                    Err(Signal::Error(e))  => Err(e),
                }
            }
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
                if self.get(name).is_none() {
                    return Err(Signal::Error(RuntimeError {
                        msg: format!("undefined variable `{}`", name),
                        span: *span,
                    }));
                }
                self.set(name, v);
            }

            Stmt::FnDef { name, params, body, .. } => {
                self.fns.insert(name.clone(), (params.clone(), body.clone()));
            }

            Stmt::Return { value, .. } => {
                let v = match value {
                    Some(e) => self.eval_expr(e).map_err(Signal::Error)?,
                    None    => Value::Void,
                };
                return Err(Signal::Return(v));
            }

            Stmt::If { cond, then_body, else_body, span } => {
                let cv = self.eval_expr(cond).map_err(Signal::Error)?;
                match cv {
                    Value::Bool(true) => {
                        self.push_scope();
                        let r = self.exec_stmts(then_body);
                        self.pop_scope();
                        r?;
                    }
                    Value::Bool(false) => {
                        if let Some(eb) = else_body {
                            self.push_scope();
                            let r = self.exec_stmts(eb);
                            self.pop_scope();
                            r?;
                        }
                    }
                    _ => return Err(Signal::Error(RuntimeError {
                        msg: "condition must be bool".into(),
                        span: *span,
                    })),
                }
            }

            Stmt::Expr(e) => {
                self.eval_expr(e).map_err(Signal::Error)?;
            }
        }
        Ok(())
    }

    fn exec_stmts(&mut self, stmts: &[Stmt]) -> Result<(), Signal> {
        for s in stmts { self.exec_stmt(s)?; }
        Ok(())
    }

    fn exec_stmts_returning(&mut self, stmts: &[Stmt]) -> Result<Option<Value>, Signal> {
        for s in stmts { self.exec_stmt(s)?; }
        Ok(None)
    }

    pub fn run(&mut self, program: &Program) -> Result<(), RuntimeError> {
        match self.exec_stmts(&program.stmts) {
            Ok(_) | Err(Signal::Return(_)) => Ok(()),
            Err(Signal::Error(e)) => Err(e),
        }
    }
}
