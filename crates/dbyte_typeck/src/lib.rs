use dbyte_ast::*;
use std::collections::HashMap;

#[derive(Debug)]
pub struct TypeError {
    pub msg: String,
    pub span: Span,
}

impl std::fmt::Display for TypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TypeError at {}: {}", self.span, self.msg)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResolvedType {
    Int, Float, Bool, Str,
    List(Box<ResolvedType>),
    Void,
}

impl std::fmt::Display for ResolvedType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolvedType::Int         => write!(f, "int"),
            ResolvedType::Float       => write!(f, "float"),
            ResolvedType::Bool        => write!(f, "bool"),
            ResolvedType::Str         => write!(f, "str"),
            ResolvedType::List(inner) => write!(f, "list[{}]", inner),
            ResolvedType::Void        => write!(f, "void"),
        }
    }
}

fn ann_to_resolved(ann: &TypeAnnotation) -> Option<ResolvedType> {
    match ann {
        TypeAnnotation::Int          => Some(ResolvedType::Int),
        TypeAnnotation::Float        => Some(ResolvedType::Float),
        TypeAnnotation::Bool         => Some(ResolvedType::Bool),
        TypeAnnotation::Str          => Some(ResolvedType::Str),
        TypeAnnotation::List(inner)  => ann_to_resolved(inner).map(|t| ResolvedType::List(Box::new(t))),
        TypeAnnotation::Inferred     => None,
    }
}

pub struct TypeChecker {
    env: Vec<HashMap<String, ResolvedType>>,
    fn_sigs: HashMap<String, (Vec<ResolvedType>, ResolvedType)>,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self { env: vec![HashMap::new()], fn_sigs: HashMap::new() }
    }

    fn push_scope(&mut self) { self.env.push(HashMap::new()); }
    fn pop_scope(&mut self)  { self.env.pop(); }

    fn lookup(&self, name: &str) -> Option<&ResolvedType> {
        for scope in self.env.iter().rev() {
            if let Some(ty) = scope.get(name) { return Some(ty); }
        }
        None
    }

    fn define(&mut self, name: &str, ty: ResolvedType) {
        self.env.last_mut().unwrap().insert(name.to_string(), ty);
    }

    fn check_expr(&self, expr: &Expr) -> Result<ResolvedType, TypeError> {
        match expr {
            Expr::IntLit(..)   => Ok(ResolvedType::Int),
            Expr::FloatLit(..) => Ok(ResolvedType::Float),
            Expr::BoolLit(..)  => Ok(ResolvedType::Bool),
            Expr::StrLit(..)   => Ok(ResolvedType::Str),

            Expr::FStr(parts, span) => {
                for part in parts {
                    if let FStrPart::Interp(name) = part {
                        self.lookup(name).ok_or_else(|| TypeError {
                            msg: format!("undefined variable `{}` in string interpolation", name),
                            span: *span,
                        })?;
                    }
                }
                Ok(ResolvedType::Str)
            }

            Expr::Ident(name, span) => {
                self.lookup(name).cloned().ok_or_else(|| TypeError {
                    msg: format!("undefined variable `{}`", name),
                    span: *span,
                })
            }

            Expr::List(elems, _span) => {
                if elems.is_empty() {
                    return Ok(ResolvedType::List(Box::new(ResolvedType::Int)));
                }
                let first_ty = self.check_expr(&elems[0])?;
                for elem in elems.iter().skip(1) {
                    let ty = self.check_expr(elem)?;
                    if ty != first_ty {
                        return Err(TypeError {
                            msg: format!("list element expected `{}`, found `{}`", first_ty, ty),
                            span: elem.span(),
                        });
                    }
                }
                Ok(ResolvedType::List(Box::new(first_ty)))
            }

            Expr::Index { target, index, span } => {
                let idx_ty = self.check_expr(index)?;
                if idx_ty != ResolvedType::Int {
                    return Err(TypeError {
                        msg: format!("list index must be `int`, found `{}`", idx_ty),
                        span: *span,
                    });
                }
                match self.check_expr(target)? {
                    ResolvedType::List(inner) => Ok(*inner),
                    ResolvedType::Str => Ok(ResolvedType::Str),
                    other => Err(TypeError {
                        msg: format!("cannot index into `{}`", other),
                        span: *span,
                    }),
                }
            }

            Expr::Binary { left, op, right, span } => {
                let lt = self.check_expr(left)?;
                let rt = self.check_expr(right)?;
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                        if lt != rt {
                            return Err(TypeError {
                                msg: format!("type mismatch: `{}` {} `{}`", lt, op, rt),
                                span: *span,
                            });
                        }
                        match &lt {
                            ResolvedType::Int | ResolvedType::Float => Ok(lt),
                            ResolvedType::Str if matches!(op, BinOp::Add) => Ok(lt),
                            _ => Err(TypeError {
                                msg: format!("operator `{}` not supported for `{}`", op, lt),
                                span: *span,
                            }),
                        }
                    }
                    BinOp::EqEq | BinOp::NotEq | BinOp::Lt | BinOp::Gt
                    | BinOp::LtEq | BinOp::GtEq => Ok(ResolvedType::Bool),
                }
            }

            Expr::Unary { op, expr, span } => {
                let ty = self.check_expr(expr)?;
                match op {
                    UnaryOp::Neg => match &ty {
                        ResolvedType::Int | ResolvedType::Float => Ok(ty),
                        _ => Err(TypeError { msg: format!("unary `-` not supported for `{}`", ty), span: *span }),
                    },
                    UnaryOp::Not => match &ty {
                        ResolvedType::Bool => Ok(ResolvedType::Bool),
                        _ => Err(TypeError { msg: format!("unary `!` expects `bool`, found `{}`", ty), span: *span }),
                    },
                }
            }

            Expr::Call { name, args, span } => {
                if name == "print" {
                    for arg in args { self.check_expr(arg)?; }
                    return Ok(ResolvedType::Void);
                }
                match self.fn_sigs.get(name) {
                    Some((param_tys, ret_ty)) => {
                        if args.len() != param_tys.len() {
                            return Err(TypeError {
                                msg: format!("function `{}` expects {} args, got {}", name, param_tys.len(), args.len()),
                                span: *span,
                            });
                        }
                        for (arg, expected) in args.iter().zip(param_tys.iter()) {
                            let got = self.check_expr(arg)?;
                            if got != *expected {
                                return Err(TypeError {
                                    msg: format!("expected `{}`, found `{}`", expected, got),
                                    span: arg.span(),
                                });
                            }
                        }
                        Ok(ret_ty.clone())
                    }
                    None => Err(TypeError { msg: format!("undefined function `{}`", name), span: *span }),
                }
            }
        }
    }

    fn check_stmts(&mut self, stmts: &[Stmt]) -> Result<(), TypeError> {
        for s in stmts { self.check_stmt(s)?; }
        Ok(())
    }

    fn check_stmt(&mut self, stmt: &Stmt) -> Result<(), TypeError> {
        match stmt {
            Stmt::Let { name, ty, value, span } => {
                let inferred = self.check_expr(value)?;
                if let Some(ann) = ann_to_resolved(ty) {
                    if ann != inferred {
                        return Err(TypeError {
                            msg: format!("expected `{}`, found `{}`", ann, inferred),
                            span: *span,
                        });
                    }
                }
                self.define(name, inferred);
            }

            Stmt::Assign { name, value, span } => {
                let inferred = self.check_expr(value)?;
                match self.lookup(name).cloned() {
                    Some(existing) if existing != inferred => {
                        return Err(TypeError {
                            msg: format!("cannot assign `{}` to variable of type `{}`", inferred, existing),
                            span: *span,
                        });
                    }
                    None => { self.define(name, inferred); }
                    _ => {}
                }
            }

            Stmt::FnDef { name, params, ret_ty, body, .. } => {
                let param_tys: Vec<ResolvedType> = params.iter()
                    .map(|p| ann_to_resolved(&p.ty).unwrap_or(ResolvedType::Int))
                    .collect();
                let ret = ann_to_resolved(ret_ty).unwrap_or(ResolvedType::Void);
                self.fn_sigs.insert(name.clone(), (param_tys.clone(), ret));
                self.push_scope();
                for (p, ty) in params.iter().zip(param_tys.iter()) {
                    self.define(&p.name, ty.clone());
                }
                self.check_stmts(body)?;
                self.pop_scope();
            }

            Stmt::Return { value, .. } => {
                if let Some(v) = value { self.check_expr(v)?; }
            }

            Stmt::If { cond, then_body, else_body, span } => {
                let ct = self.check_expr(cond)?;
                if ct != ResolvedType::Bool {
                    return Err(TypeError {
                        msg: format!("condition must be `bool`, found `{}`", ct),
                        span: *span,
                    });
                }
                self.push_scope(); self.check_stmts(then_body)?; self.pop_scope();
                if let Some(eb) = else_body {
                    self.push_scope(); self.check_stmts(eb)?; self.pop_scope();
                }
            }

            Stmt::While { cond, body, span } => {
                let ct = self.check_expr(cond)?;
                if ct != ResolvedType::Bool {
                    return Err(TypeError {
                        msg: format!("while condition must be `bool`, found `{}`", ct),
                        span: *span,
                    });
                }
                self.push_scope(); self.check_stmts(body)?; self.pop_scope();
            }

            Stmt::For { var, iterable, body, span } => {
                let iter_ty = self.check_expr(iterable)?;
                let elem_ty = match iter_ty {
                    ResolvedType::List(inner) => *inner,
                    ResolvedType::Str => ResolvedType::Str,
                    other => return Err(TypeError {
                        msg: format!("`{}` is not iterable", other),
                        span: *span,
                    }),
                };
                self.push_scope();
                self.define(var, elem_ty);
                self.check_stmts(body)?;
                self.pop_scope();
            }

            Stmt::Expr(e) => { self.check_expr(e)?; }
        }
        Ok(())
    }

    pub fn check_program(&mut self, program: &Program) -> Result<(), TypeError> {
        self.check_stmts(&program.stmts)
    }
}
