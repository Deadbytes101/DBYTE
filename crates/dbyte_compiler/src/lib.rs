use dbyte_ast::*;
use dbyte_bytecode::{BytecodeFunction, Chunk, Op, Value};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug)]
pub struct CompileError {
    pub msg: String,
    pub span: Span,
}

pub struct Compiler {
    current_file: Option<PathBuf>,
}

impl Compiler {
    pub fn new() -> Self {
        Self { current_file: None }
    }

    pub fn with_entry_path(path: impl Into<PathBuf>) -> Self {
        Self {
            current_file: Some(path.into()),
        }
    }

    pub fn compile_program(&self, program: &Program) -> Result<Chunk, CompileError> {
        let name = self
            .current_file
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("main")
            .to_string();
        let mut fc = FunctionCompiler::new(name);
        fc.compile_stmts(&program.stmts)?;
        fc.emit(Op::Halt);
        Ok(fc.finish())
    }
}

impl Default for Compiler {
    fn default() -> Self {
        Self::new()
    }
}

struct FunctionCompiler {
    chunk: Chunk,
    locals: HashMap<String, usize>,
}

impl FunctionCompiler {
    fn new(name: String) -> Self {
        Self {
            chunk: Chunk::new(name),
            locals: HashMap::new(),
        }
    }

    fn finish(self) -> Chunk {
        self.chunk
    }

    fn emit(&mut self, op: Op) -> usize {
        self.chunk.code.push(op);
        self.chunk.code.len() - 1
    }

    fn patch_jump(&mut self, at: usize, target: usize) {
        match &mut self.chunk.code[at] {
            Op::Jump(slot) | Op::JumpIfFalse(slot) => *slot = target,
            Op::IterNext { jump, .. } => *jump = target,
            _ => {}
        }
    }

    fn local_slot(&mut self, name: &str) -> usize {
        if let Some(slot) = self.locals.get(name) {
            return *slot;
        }
        let slot = self.chunk.local_names.len();
        self.locals.insert(name.to_string(), slot);
        self.chunk.local_names.push(name.to_string());
        slot
    }

    fn existing_slot(&self, name: &str, span: Span) -> Result<usize, CompileError> {
        self.locals.get(name).copied().ok_or_else(|| CompileError {
            msg: format!("undefined variable `{}`", name),
            span,
        })
    }

    fn add_const(&mut self, value: Value) -> usize {
        self.chunk.add_const(value)
    }

    fn compile_stmts(&mut self, stmts: &[Stmt]) -> Result<(), CompileError> {
        for stmt in stmts {
            self.compile_stmt(stmt)?;
        }
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), CompileError> {
        match stmt {
            Stmt::Let {
                is_pub,
                name,
                value,
                ..
            } => {
                self.compile_expr(value)?;
                let slot = self.local_slot(name);
                self.emit(Op::StoreLocal(slot));
                if *is_pub {
                    self.chunk.public_values.push((name.clone(), slot));
                }
            }
            Stmt::Assign { name, value, span } => {
                self.compile_expr(value)?;
                let slot = self.existing_slot(name, *span)?;
                self.emit(Op::StoreLocal(slot));
            }
            Stmt::FnDef {
                is_pub,
                name,
                params,
                body,
                ..
            } => {
                let mut child = FunctionCompiler::new(name.clone());
                for param in params {
                    child.local_slot(&param.name);
                }
                child.compile_stmts(body)?;
                let void_idx = child.add_const(Value::Void);
                child.emit(Op::Const(void_idx));
                child.emit(Op::Return);
                let function = BytecodeFunction {
                    name: name.clone(),
                    params: params.iter().map(|p| p.name.clone()).collect(),
                    chunk: child.finish(),
                };
                self.chunk.functions.insert(name.clone(), function);
                if *is_pub {
                    self.chunk.public_functions.push(name.clone());
                }
            }
            Stmt::Return { value, .. } => {
                if let Some(value) = value {
                    self.compile_expr(value)?;
                } else {
                    let idx = self.add_const(Value::Void);
                    self.emit(Op::Const(idx));
                }
                self.emit(Op::Return);
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                self.compile_expr(cond)?;
                let jf = self.emit(Op::JumpIfFalse(usize::MAX));
                self.compile_stmts(then_body)?;
                let jend = self.emit(Op::Jump(usize::MAX));
                let else_start = self.chunk.code.len();
                self.patch_jump(jf, else_start);
                if let Some(else_body) = else_body {
                    self.compile_stmts(else_body)?;
                }
                let end = self.chunk.code.len();
                self.patch_jump(jend, end);
            }
            Stmt::While { cond, body, .. } => {
                let loop_start = self.chunk.code.len();
                self.compile_expr(cond)?;
                let exit = self.emit(Op::JumpIfFalse(usize::MAX));
                self.compile_stmts(body)?;
                self.emit(Op::Jump(loop_start));
                let end = self.chunk.code.len();
                self.patch_jump(exit, end);
            }
            Stmt::For {
                var,
                iterable,
                body,
                ..
            } => {
                self.compile_expr(iterable)?;
                self.emit(Op::IterInit);
                let slot = self.local_slot(var);
                let loop_start = self.chunk.code.len();
                let next = self.emit(Op::IterNext {
                    slot,
                    jump: usize::MAX,
                });
                self.compile_stmts(body)?;
                self.emit(Op::Jump(loop_start));
                let end = self.chunk.code.len();
                self.patch_jump(next, end);
            }
            Stmt::Import { path, alias, .. } => {
                let slot = self.local_slot(alias);
                self.emit(Op::Import(path.clone(), slot));
            }
            Stmt::Expr(expr) => {
                self.compile_expr(expr)?;
                self.emit(Op::Pop);
            }
        }
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), CompileError> {
        match expr {
            Expr::IntLit(n, _) => {
                let idx = self.add_const(Value::Int(*n));
                self.emit(Op::Const(idx));
            }
            Expr::FloatLit(n, _) => {
                let idx = self.add_const(Value::Float(*n));
                self.emit(Op::Const(idx));
            }
            Expr::BoolLit(b, _) => {
                let idx = self.add_const(Value::Bool(*b));
                self.emit(Op::Const(idx));
            }
            Expr::StrLit(s, _) => {
                let idx = self.add_const(Value::Str(s.clone()));
                self.emit(Op::Const(idx));
            }
            Expr::FStr(parts, _) => {
                self.emit(Op::FStr(parts.clone()));
            }
            Expr::Ident(name, span) => {
                let slot = self.existing_slot(name, *span)?;
                self.emit(Op::LoadLocal(slot));
            }
            Expr::List(elems, _) => {
                for elem in elems {
                    self.compile_expr(elem)?;
                }
                self.emit(Op::MakeList(elems.len()));
            }
            Expr::Binary {
                left, op, right, ..
            } => {
                self.compile_expr(left)?;
                self.compile_expr(right)?;
                self.emit(match op {
                    BinOp::Add => Op::Add,
                    BinOp::Sub => Op::Sub,
                    BinOp::Mul => Op::Mul,
                    BinOp::Div => Op::Div,
                    BinOp::EqEq => Op::Eq,
                    BinOp::NotEq => Op::Ne,
                    BinOp::Lt => Op::Lt,
                    BinOp::LtEq => Op::Le,
                    BinOp::Gt => Op::Gt,
                    BinOp::GtEq => Op::Ge,
                });
            }
            Expr::Unary { op, expr, .. } => {
                self.compile_expr(expr)?;
                self.emit(match op {
                    UnaryOp::Neg => Op::Neg,
                    UnaryOp::Not => Op::Not,
                });
            }
            Expr::Call { name, args, .. } => {
                for arg in args {
                    self.compile_expr(arg)?;
                }
                self.emit(Op::Call(name.clone(), args.len()));
            }
            Expr::Index { target, index, .. } => {
                self.compile_expr(target)?;
                self.compile_expr(index)?;
                self.emit(Op::Index);
            }
            Expr::Member {
                object, property, ..
            } => {
                self.compile_expr(object)?;
                self.emit(Op::Member(property.clone()));
            }
            Expr::MemberCall {
                object,
                property,
                args,
                ..
            } => {
                self.compile_expr(object)?;
                for arg in args {
                    self.compile_expr(arg)?;
                }
                self.emit(Op::MemberCall(property.clone(), args.len()));
            }
        }
        Ok(())
    }
}
