use dbyte_ast::*;
use dbyte_lexer::Lexer;
use dbyte_module::{
    resolve_import, stdlib_exports, ImportTarget, ModuleError, ModuleState, StdlibExport,
};
use dbyte_parser::Parser;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
    Int,
    Float,
    Bool,
    Str,
    Bytes,
    Buffer,
    List(Box<ResolvedType>),
    Module(ModuleType),
    Void,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModuleType {
    pub alias: String,
    pub members: HashMap<String, ModuleMemberType>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModuleMemberType {
    Value(ResolvedType),
    Function(Vec<ResolvedType>, ResolvedType),
}

impl std::fmt::Display for ResolvedType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolvedType::Int => write!(f, "int"),
            ResolvedType::Float => write!(f, "float"),
            ResolvedType::Bool => write!(f, "bool"),
            ResolvedType::Str => write!(f, "str"),
            ResolvedType::Bytes => write!(f, "bytes"),
            ResolvedType::Buffer => write!(f, "buffer"),
            ResolvedType::List(inner) => write!(f, "list[{}]", inner),
            ResolvedType::Module(m) => write!(f, "module '{}'", m.alias),
            ResolvedType::Void => write!(f, "void"),
        }
    }
}

fn ann_to_resolved(ann: &TypeAnnotation) -> Option<ResolvedType> {
    match ann {
        TypeAnnotation::Int => Some(ResolvedType::Int),
        TypeAnnotation::Float => Some(ResolvedType::Float),
        TypeAnnotation::Bool => Some(ResolvedType::Bool),
        TypeAnnotation::Str => Some(ResolvedType::Str),
        TypeAnnotation::Bytes => Some(ResolvedType::Bytes),
        TypeAnnotation::Buffer => Some(ResolvedType::Buffer),
        TypeAnnotation::List(inner) => {
            ann_to_resolved(inner).map(|t| ResolvedType::List(Box::new(t)))
        }
        TypeAnnotation::Inferred => None,
    }
}

pub struct TypeChecker {
    env: Vec<HashMap<String, ResolvedType>>,
    fn_sigs: HashMap<String, (Vec<ResolvedType>, ResolvedType)>,
    current_file: Option<PathBuf>,
    module_cache: HashMap<String, ModuleState<ModuleType>>,
    loading_stack: Vec<String>,
    in_function: usize,
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            env: vec![HashMap::new()],
            fn_sigs: HashMap::new(),
            current_file: None,
            module_cache: HashMap::new(),
            loading_stack: Vec::new(),
            in_function: 0,
        }
    }

    pub fn with_entry_path(path: impl Into<PathBuf>) -> Self {
        let mut checker = Self::new();
        checker.current_file = Some(path.into());
        checker
    }

    fn push_scope(&mut self) {
        self.env.push(HashMap::new());
    }
    fn pop_scope(&mut self) {
        self.env.pop();
    }

    fn lookup(&self, name: &str) -> Option<&ResolvedType> {
        for scope in self.env.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty);
            }
        }
        None
    }

    fn define(&mut self, name: &str, ty: ResolvedType) {
        self.env.last_mut().unwrap().insert(name.to_string(), ty);
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
    ) -> Result<ModuleType, TypeError> {
        let target = resolve_import(path, self.current_file.as_deref()).map_err(|e| TypeError {
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
                return Err(TypeError {
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
            ImportTarget::Std(name) => self.load_std_module(&name, alias, span),
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

    fn load_std_module(
        &self,
        name: &str,
        alias: &str,
        span: Span,
    ) -> Result<ModuleType, TypeError> {
        let exports = stdlib_exports(name).ok_or_else(|| TypeError {
            msg: format!("ImportError: standard module not found: {}", name),
            span,
        })?;
        let mut members = HashMap::new();
        for (name, export) in exports {
            match export {
                StdlibExport::Function { params, ret } => {
                    let params = params
                        .iter()
                        .map(|p| ann_to_resolved(p).unwrap_or(ResolvedType::Void))
                        .collect();
                    let ret = ann_to_resolved(&ret).unwrap_or(ResolvedType::Void);
                    members.insert(name, ModuleMemberType::Function(params, ret));
                }
            }
        }
        Ok(ModuleType {
            alias: alias.to_string(),
            members,
        })
    }

    fn parse_module_file(path: &Path, span: Span) -> Result<Program, TypeError> {
        let src = std::fs::read_to_string(path).map_err(|e| TypeError {
            msg: format!("ImportError: cannot read `{}`: {}", path.display(), e),
            span,
        })?;
        let tokens = Lexer::new(&src).tokenize().map_err(|e| TypeError {
            msg: format!("ImportError: {}", e.msg),
            span: e.span,
        })?;
        Parser::new(tokens).parse_program().map_err(|e| TypeError {
            msg: format!("ImportError: {}", e.msg),
            span: e.span,
        })
    }

    fn load_file_module(
        &mut self,
        path: &Path,
        alias: &str,
        span: Span,
    ) -> Result<ModuleType, TypeError> {
        let program = Self::parse_module_file(path, span)?;

        let saved_env = std::mem::replace(&mut self.env, vec![HashMap::new()]);
        let saved_fns = std::mem::take(&mut self.fn_sigs);
        let saved_file = self.current_file.replace(path.to_path_buf());
        let saved_in_function = self.in_function;
        self.in_function = 0;

        let checked = self.check_stmts(&program.stmts);
        let mut members = HashMap::new();
        if checked.is_ok() {
            for stmt in &program.stmts {
                match stmt {
                    Stmt::Let {
                        is_pub: true, name, ..
                    } => {
                        if let Some(ty) = self.lookup(name).cloned() {
                            members.insert(name.clone(), ModuleMemberType::Value(ty));
                        }
                    }
                    Stmt::FnDef {
                        is_pub: true, name, ..
                    } => {
                        if let Some((params, ret)) = self.fn_sigs.get(name).cloned() {
                            members.insert(name.clone(), ModuleMemberType::Function(params, ret));
                        }
                    }
                    _ => {}
                }
            }
        }

        self.env = saved_env;
        self.fn_sigs = saved_fns;
        self.current_file = saved_file;
        self.in_function = saved_in_function;
        checked?;

        Ok(ModuleType {
            alias: alias.to_string(),
            members,
        })
    }

    fn check_module_member(
        module: &ModuleType,
        property: &str,
        span: Span,
    ) -> Result<ModuleMemberType, TypeError> {
        module
            .members
            .get(property)
            .cloned()
            .ok_or_else(|| TypeError {
                msg: format!(
                    "module '{}' has no public member '{}'",
                    module.alias, property
                ),
                span,
            })
    }

    fn check_expr(&mut self, expr: &Expr) -> Result<ResolvedType, TypeError> {
        match expr {
            Expr::IntLit(..) => Ok(ResolvedType::Int),
            Expr::FloatLit(..) => Ok(ResolvedType::Float),
            Expr::BoolLit(..) => Ok(ResolvedType::Bool),
            Expr::StrLit(..) => Ok(ResolvedType::Str),
            Expr::BytesLit(..) => Ok(ResolvedType::Bytes),

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

            Expr::Ident(name, span) => self.lookup(name).cloned().ok_or_else(|| TypeError {
                msg: format!("undefined variable `{}`", name),
                span: *span,
            }),

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

            Expr::Index {
                target,
                index,
                span,
            } => {
                let idx_ty = self.check_expr(index)?;
                if idx_ty != ResolvedType::Int {
                    return Err(TypeError {
                        msg: format!("index must be int, found {}", idx_ty),
                        span: *span,
                    });
                }
                match self.check_expr(target)? {
                    ResolvedType::List(inner) => Ok(*inner),
                    ResolvedType::Str => Ok(ResolvedType::Str),
                    ResolvedType::Bytes => Ok(ResolvedType::Int),
                    other => Err(TypeError {
                        msg: format!("cannot index {}", other),
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
                    BinOp::EqEq
                    | BinOp::NotEq
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::LtEq
                    | BinOp::GtEq => Ok(ResolvedType::Bool),
                }
            }

            Expr::Unary { op, expr, span } => {
                let ty = self.check_expr(expr)?;
                match op {
                    UnaryOp::Neg => match &ty {
                        ResolvedType::Int | ResolvedType::Float => Ok(ty),
                        _ => Err(TypeError {
                            msg: format!("unary `-` not supported for `{}`", ty),
                            span: *span,
                        }),
                    },
                    UnaryOp::Not => match &ty {
                        ResolvedType::Bool => Ok(ResolvedType::Bool),
                        _ => Err(TypeError {
                            msg: format!("unary `!` expects `bool`, found `{}`", ty),
                            span: *span,
                        }),
                    },
                }
            }

            Expr::Call { name, args, span } => {
                if name == "print" {
                    for arg in args {
                        self.check_expr(arg)?;
                    }
                    return Ok(ResolvedType::Void);
                }
                if name == "len" {
                    if args.len() != 1 {
                        return Err(TypeError {
                            msg: "len() expects 1 argument".into(),
                            span: *span,
                        });
                    }
                    let ty = self.check_expr(&args[0])?;
                    return match ty {
                        ResolvedType::Str | ResolvedType::List(_) | ResolvedType::Bytes => {
                            Ok(ResolvedType::Int)
                        }
                        _ => Err(TypeError {
                            msg: format!("len() expects str, list, or bytes, found `{}`", ty),
                            span: *span,
                        }),
                    };
                }
                match self.fn_sigs.get(name).cloned() {
                    Some((param_tys, ret_ty)) => {
                        self.check_call_args(name, args, &param_tys, *span)?;
                        Ok(ret_ty)
                    }
                    None => Err(TypeError {
                        msg: format!("undefined function `{}`", name),
                        span: *span,
                    }),
                }
            }

            Expr::Member {
                object,
                property,
                span,
            } => match self.check_expr(object)? {
                ResolvedType::Module(module) => {
                    match Self::check_module_member(&module, property, *span)? {
                        ModuleMemberType::Value(ty) => Ok(ty),
                        ModuleMemberType::Function(_, _) => Err(TypeError {
                            msg: format!(
                                "module member '{}.{}' is a function",
                                module.alias, property
                            ),
                            span: *span,
                        }),
                    }
                }
                other => Err(TypeError {
                    msg: format!("member access not supported for `{}`", other),
                    span: *span,
                }),
            },

            Expr::MemberCall {
                object,
                property,
                args,
                span,
            } => match self.check_expr(object)? {
                ResolvedType::Module(module) => {
                    match Self::check_module_member(&module, property, *span)? {
                        ModuleMemberType::Function(param_tys, ret_ty) => {
                            self.check_call_args(
                                &format!("{}.{}", module.alias, property),
                                args,
                                &param_tys,
                                *span,
                            )?;
                            Ok(ret_ty)
                        }
                        ModuleMemberType::Value(_) => Err(TypeError {
                            msg: format!(
                                "module member '{}.{}' is not callable",
                                module.alias, property
                            ),
                            span: *span,
                        }),
                    }
                }
                other => Err(TypeError {
                    msg: format!("member call not supported for `{}`", other),
                    span: *span,
                }),
            },
        }
    }

    fn check_call_args(
        &mut self,
        name: &str,
        args: &[Expr],
        param_tys: &[ResolvedType],
        span: Span,
    ) -> Result<(), TypeError> {
        if args.len() != param_tys.len() {
            return Err(TypeError {
                msg: format!(
                    "function `{}` expects {} args, got {}",
                    name,
                    param_tys.len(),
                    args.len()
                ),
                span,
            });
        }
        for (i, (arg, expected)) in args.iter().zip(param_tys.iter()).enumerate() {
            let got = self.check_expr(arg)?;
            if got != *expected {
                return Err(TypeError {
                    msg: format!("expected {}, found {} (argument {})", expected, got, i + 1),
                    span: arg.span(),
                });
            }
        }
        Ok(())
    }

    fn check_stmts(&mut self, stmts: &[Stmt]) -> Result<(), TypeError> {
        for s in stmts {
            self.check_stmt(s)?;
        }
        Ok(())
    }

    fn check_stmt(&mut self, stmt: &Stmt) -> Result<(), TypeError> {
        match stmt {
            Stmt::Let {
                name,
                ty,
                value,
                span,
                ..
            } => {
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
                            msg: format!(
                                "cannot assign `{}` to variable of type `{}`",
                                inferred, existing
                            ),
                            span: *span,
                        });
                    }
                    None => {
                        return Err(TypeError {
                            msg: format!("undefined variable `{}`", name),
                            span: *span,
                        });
                    }
                    _ => {}
                }
            }

            Stmt::FnDef {
                name,
                params,
                ret_ty,
                body,
                ..
            } => {
                let param_tys: Vec<ResolvedType> = params
                    .iter()
                    .map(|p| ann_to_resolved(&p.ty).unwrap_or(ResolvedType::Int))
                    .collect();
                let ret = ann_to_resolved(ret_ty).unwrap_or(ResolvedType::Void);
                self.fn_sigs.insert(name.clone(), (param_tys.clone(), ret));
                self.push_scope();
                self.in_function += 1;
                for (p, ty) in params.iter().zip(param_tys.iter()) {
                    self.define(&p.name, ty.clone());
                }
                let checked = self.check_stmts(body);
                self.in_function -= 1;
                self.pop_scope();
                checked?;
            }

            Stmt::Return { value, span } => {
                if self.in_function == 0 {
                    return Err(TypeError {
                        msg: "return outside function".into(),
                        span: *span,
                    });
                }
                if let Some(v) = value {
                    self.check_expr(v)?;
                }
            }

            Stmt::If {
                cond,
                then_body,
                else_body,
                span,
            } => {
                let ct = self.check_expr(cond)?;
                if ct != ResolvedType::Bool {
                    return Err(TypeError {
                        msg: format!("condition must be `bool`, found `{}`", ct),
                        span: *span,
                    });
                }
                self.push_scope();
                self.check_stmts(then_body)?;
                self.pop_scope();
                if let Some(eb) = else_body {
                    self.push_scope();
                    self.check_stmts(eb)?;
                    self.pop_scope();
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
                self.push_scope();
                self.check_stmts(body)?;
                self.pop_scope();
            }

            Stmt::For {
                var,
                iterable,
                body,
                span,
            } => {
                let iter_ty = self.check_expr(iterable)?;
                let elem_ty = match iter_ty {
                    ResolvedType::List(inner) => *inner,
                    ResolvedType::Str => ResolvedType::Str,
                    other => {
                        return Err(TypeError {
                            msg: format!("`{}` is not iterable", other),
                            span: *span,
                        })
                    }
                };
                self.push_scope();
                self.define(var, elem_ty);
                self.check_stmts(body)?;
                self.pop_scope();
            }

            Stmt::Import { path, alias, span } => {
                if self.env.last().unwrap().contains_key(alias) {
                    return Err(TypeError {
                        msg: format!("ImportError: duplicate import alias: {}", alias),
                        span: *span,
                    });
                }
                let module = self.load_module(path, alias, *span)?;
                self.define(alias, ResolvedType::Module(module));
            }

            Stmt::Expr(e) => {
                self.check_expr(e)?;
            }
        }
        Ok(())
    }

    pub fn check_program(&mut self, program: &Program) -> Result<(), TypeError> {
        self.check_stmts(&program.stmts)
    }
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
