use dbyte_ast::*;
use dbyte_bytecode::{BytecodeFunction, Chunk, LocalKind, Op, Value};
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
    local_types: HashMap<String, ExprType>,
    function_return_types: HashMap<String, ExprType>,
    function_param_types: HashMap<String, Vec<ExprType>>,
    imports: HashMap<String, String>,
    return_type: ExprType,
    inlining_stack: Vec<usize>,
    inlining_counter: usize,
    current_function_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExprType {
    Int,
    Bool,
    Str,
    Bytes,
    Buffer,
    Module,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineReturn {
    ValueStack,
    I64Stack,
    I64Local(usize),
    Discard,
}

impl FunctionCompiler {
    fn new(name: String) -> Self {
        Self::new_with_return(name, ExprType::Unknown)
    }

    fn new_with_return(name: String, return_type: ExprType) -> Self {
        Self {
            chunk: Chunk::new(name.clone()),
            locals: HashMap::new(),
            local_types: HashMap::new(),
            function_return_types: HashMap::new(),
            function_param_types: HashMap::new(),
            imports: HashMap::new(),
            return_type,
            inlining_stack: Vec::new(),
            inlining_counter: 0,
            current_function_name: name,
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
            Op::JumpIfNotLtLocalConstI64 { target: slot, .. }
            | Op::JumpIfNotLeLocalConstI64 { target: slot, .. }
            | Op::JumpIfNotGtLocalConstI64 { target: slot, .. }
            | Op::JumpIfNotGeLocalConstI64 { target: slot, .. } => *slot = target,
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
        self.chunk.local_kinds.push(LocalKind::Value);
        self.chunk.local_i64_slots.push(None);
        slot
    }

    fn set_local_type(&mut self, name: &str, ty: ExprType) {
        if ty != ExprType::Unknown {
            self.local_types.insert(name.to_string(), ty);
            if let Some(slot) = self.locals.get(name).copied() {
                if self.chunk.local_kinds.len() <= slot {
                    self.chunk.local_kinds.resize(slot + 1, LocalKind::Value);
                }
                if self.chunk.local_i64_slots.len() <= slot {
                    self.chunk.local_i64_slots.resize(slot + 1, None);
                }
                match ty {
                    ExprType::Int => {
                        if self.chunk.local_kinds[slot] != LocalKind::I64 {
                            self.chunk.local_i64_slots[slot] = Some(self.chunk.i64_local_count);
                            self.chunk.i64_local_count += 1;
                        }
                        self.chunk.local_kinds[slot] = LocalKind::I64;
                    }
                    _ => {
                        self.chunk.local_kinds[slot] = LocalKind::Value;
                        self.chunk.local_i64_slots[slot] = None;
                    }
                }
            }
        }
    }

    fn local_type(&self, name: &str) -> ExprType {
        self.local_types
            .get(name)
            .copied()
            .unwrap_or(ExprType::Unknown)
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

    fn insert_function(&mut self, function: BytecodeFunction) -> usize {
        let id = match self.chunk.function_ids.get(&function.name).copied() {
            Some(id) => {
                self.chunk.functions_by_id[id] = function.clone();
                id
            }
            None => {
                let id = self.chunk.functions_by_id.len();
                self.chunk.function_ids.insert(function.name.clone(), id);
                self.chunk.functions_by_id.push(function.clone());
                id
            }
        };
        self.chunk.functions.insert(function.name.clone(), function);
        id
    }

    fn reserve_function(&mut self, name: &str, params: &[Param]) -> usize {
        if let Some(id) = self.chunk.function_ids.get(name).copied() {
            return id;
        }
        let id = self.chunk.functions_by_id.len();
        self.chunk.function_ids.insert(name.to_string(), id);
        self.chunk.functions_by_id.push(BytecodeFunction {
            name: name.to_string(),
            params: params.iter().map(|p| p.name.clone()).collect(),
            chunk: Chunk::new(name),
        });
        id
    }

    fn reserve_functions(&mut self, stmts: &[Stmt]) {
        for stmt in stmts {
            if let Stmt::FnDef {
                name,
                params,
                ret_ty,
                ..
            } = stmt
            {
                self.function_return_types
                    .insert(name.clone(), expr_type_from_annotation(ret_ty));
                self.function_param_types.insert(
                    name.clone(),
                    params
                        .iter()
                        .map(|param| expr_type_from_annotation(&param.ty))
                        .collect(),
                );
                self.reserve_function(name, params);
            }
        }
    }

    fn compile_stmts(&mut self, stmts: &[Stmt]) -> Result<(), CompileError> {
        self.reserve_functions(stmts);
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
                ty,
                value,
                ..
            } => {
                let value_type = self.declared_or_expr_type(ty, value);
                if value_type == ExprType::Int
                    && self.can_compile_i64_call_to_local_fast_path(value)
                {
                    let slot = self.local_slot(name);
                    self.set_local_type(name, value_type);
                    self.compile_i64_call_to_local_fast_path(value, slot)?;
                    if *is_pub {
                        self.chunk.public_values.push((name.clone(), slot));
                    }
                    return Ok(());
                }
                if value_type == ExprType::Int
                    && self.expr_has_i64_stack_call(value)
                    && self.can_compile_i64_stack_expr(value)
                {
                    self.compile_i64_stack_expr(value)?;
                    let slot = self.local_slot(name);
                    self.set_local_type(name, value_type);
                    self.emit(Op::StoreLocalI64Stack(slot));
                    if *is_pub {
                        self.chunk.public_values.push((name.clone(), slot));
                    }
                    return Ok(());
                }
                self.compile_expr(value)?;
                let slot = self.local_slot(name);
                self.set_local_type(name, value_type);
                if value_type == ExprType::Int {
                    self.emit(Op::StoreLocalI64(slot));
                } else {
                    self.emit(Op::StoreLocal(slot));
                }
                if *is_pub {
                    self.chunk.public_values.push((name.clone(), slot));
                }
            }
            Stmt::Assign { name, value, span } => {
                if self.compile_i64_assign_fast_path(name, value, *span)? {
                    return Ok(());
                }
                if self.compile_i64_stack_assign_fast_path(name, value, *span)? {
                    return Ok(());
                }
                self.compile_expr(value)?;
                let slot = self.existing_slot(name, *span)?;
                if self.local_type(name) == ExprType::Int {
                    self.emit(Op::StoreLocalI64(slot));
                } else {
                    self.emit(Op::StoreLocal(slot));
                }
            }
            Stmt::FnDef {
                is_pub,
                name,
                params,
                ret_ty,
                body,
                ..
            } => {
                let mut child = FunctionCompiler::new_with_return(
                    name.clone(),
                    expr_type_from_annotation(ret_ty),
                );
                child.chunk.functions = self.chunk.functions.clone();
                child.chunk.function_ids = self.chunk.function_ids.clone();
                child.chunk.functions_by_id = self.chunk.functions_by_id.clone();
                child.function_return_types = self.function_return_types.clone();
                child.function_param_types = self.function_param_types.clone();
                child.inlining_counter = self.inlining_counter;
                for param in params {
                    child.local_slot(&param.name);
                    child.set_local_type(&param.name, expr_type_from_annotation(&param.ty));
                }
                child.compile_stmts(body)?;
                let void_idx = child.add_const(Value::Void);
                child.emit(Op::Const(void_idx));
                child.emit(Op::Return);
                self.inlining_counter = child.inlining_counter;
                let function = BytecodeFunction {
                    name: name.clone(),
                    params: params.iter().map(|p| p.name.clone()).collect(),
                    chunk: child.finish(),
                };
                self.insert_function(function);
                if *is_pub {
                    self.chunk.public_functions.push(name.clone());
                }
            }
            Stmt::Return { value, .. } => {
                if let Some(value) = value {
                    if self.return_type == ExprType::Int && self.can_compile_i64_stack_expr(value) {
                        self.compile_i64_stack_expr(value)?;
                        self.emit(Op::ReturnI64ToI64Stack);
                        return Ok(());
                    }
                    self.compile_expr(value)?;
                } else {
                    let idx = self.add_const(Value::Void);
                    self.emit(Op::Const(idx));
                }
                if self.return_type == ExprType::Int && value.is_some() {
                    self.emit(Op::ReturnI64);
                } else {
                    self.emit(Op::Return);
                }
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                let jf = if let Some(jump) = self.compile_i64_compare_jump_fast_path(cond)? {
                    jump
                } else {
                    if !self.compile_i64_compare_fast_path(cond)? {
                        self.compile_expr(cond)?;
                    }
                    self.emit(Op::JumpIfFalse(usize::MAX))
                };
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
                let exit = if let Some(jump) = self.compile_i64_compare_jump_fast_path(cond)? {
                    jump
                } else {
                    if !self.compile_i64_compare_fast_path(cond)? {
                        self.compile_expr(cond)?;
                    }
                    self.emit(Op::JumpIfFalse(usize::MAX))
                };
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
                self.imports.insert(alias.clone(), path.clone());
                self.set_local_type(alias, ExprType::Module);
                self.emit(Op::Import(path.clone(), slot));
            }
            Stmt::Expr(expr) => {
                if self.compile_discarded_call_fast_path(expr)? {
                    return Ok(());
                }
                self.compile_expr(expr)?;
                self.emit(Op::Pop);
            }
        }
        Ok(())
    }

    fn compile_discarded_call_fast_path(&mut self, expr: &Expr) -> Result<bool, CompileError> {
        let Expr::Call { name, args, .. } = expr else {
            return Ok(false);
        };
        let Some(id) = self.chunk.function_ids.get(name).copied() else {
            return Ok(false);
        };
        if self.try_inline_function(name, args, InlineReturn::Discard)? {
            return Ok(true);
        }

        let is_i64_func = self.function_return_types.get(name).copied() == Some(ExprType::Int);

        if is_i64_func {
            for arg in args {
                if !self.can_compile_i64_stack_expr(arg) {
                    // Fallback to generic call
                    for arg in args {
                        self.compile_expr(arg)?;
                    }
                    self.emit(Op::Call(name.clone(), args.len()));
                    self.emit(Op::Pop);
                    return Ok(true);
                }
            }
            for arg in args {
                self.compile_i64_stack_expr(arg)?;
            }
            self.emit(Op::CallFnI64Discard {
                id,
                argc: args.len(),
            });
        } else {
            for arg in args {
                self.compile_expr(arg)?;
            }
            self.emit(Op::CallFnDiscard {
                id,
                argc: args.len(),
            });
        }
        Ok(true)
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), CompileError> {
        match expr {
            Expr::IntLit(n, _) => {
                self.emit(Op::ConstI64(*n));
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
            Expr::BytesLit(b, _) => {
                let idx = self.add_const(Value::Bytes(b.clone()));
                self.emit(Op::Const(idx));
            }
            Expr::FStr(parts, _) => {
                self.emit(Op::FStr(parts.clone()));
            }
            Expr::Ident(name, span) => {
                let slot = self.existing_slot(name, *span)?;
                if self.local_type(name) == ExprType::Int {
                    self.emit(Op::LoadLocalI64(slot));
                } else {
                    self.emit(Op::LoadLocal(slot));
                }
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
                let left_is_int = self.expr_type(left) == ExprType::Int;
                let right_is_int = self.expr_type(right) == ExprType::Int;
                self.compile_expr(left)?;
                self.compile_expr(right)?;
                let typed = left_is_int && right_is_int;
                self.emit(match (typed, op) {
                    (true, BinOp::Add) => Op::AddI64,
                    (true, BinOp::Sub) => Op::SubI64,
                    (true, BinOp::Mul) => Op::MulI64,
                    (true, BinOp::Div) => Op::DivI64,
                    (true, BinOp::Lt) => Op::LtI64,
                    (true, BinOp::LtEq) => Op::LeI64,
                    (true, BinOp::Gt) => Op::GtI64,
                    (true, BinOp::GtEq) => Op::GeI64,
                    (_, BinOp::Add) => Op::Add,
                    (_, BinOp::Sub) => Op::Sub,
                    (_, BinOp::Mul) => Op::Mul,
                    (_, BinOp::Div) => Op::Div,
                    (_, BinOp::EqEq) => Op::Eq,
                    (_, BinOp::NotEq) => Op::Ne,
                    (_, BinOp::Lt) => Op::Lt,
                    (_, BinOp::LtEq) => Op::Le,
                    (_, BinOp::Gt) => Op::Gt,
                    (_, BinOp::GtEq) => Op::Ge,
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
                if self.try_inline_function(name, args, InlineReturn::ValueStack)? {
                    return Ok(());
                }
                for arg in args {
                    self.compile_expr(arg)?;
                }
                if let Some(id) = self.chunk.function_ids.get(name).copied() {
                    self.emit(Op::CallFn {
                        id,
                        argc: args.len(),
                    });
                } else {
                    self.emit(Op::Call(name.clone(), args.len()));
                }
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
                if self.compile_intrinsic_member_call(object, property, args)? {
                    return Ok(());
                }
                self.compile_expr(object)?;
                for arg in args {
                    self.compile_expr(arg)?;
                }
                self.emit(Op::MemberCall(property.clone(), args.len()));
            }
        }
        Ok(())
    }

    fn compile_i64_assign_fast_path(
        &mut self,
        name: &str,
        value: &Expr,
        span: Span,
    ) -> Result<bool, CompileError> {
        if self.local_type(name) != ExprType::Int {
            return Ok(false);
        }
        let dst = self.existing_slot(name, span)?;
        if self.can_compile_i64_call_to_local_fast_path(value) {
            self.compile_i64_call_to_local_fast_path(value, dst)?;
            return Ok(true);
        }
        let Expr::Binary {
            left, op, right, ..
        } = value
        else {
            return Ok(false);
        };
        let Expr::Ident(left_name, _) = &**left else {
            return Ok(false);
        };
        if left_name != name {
            return Ok(false);
        }
        match (&**right, op) {
            (Expr::Ident(src_name, src_span), BinOp::Add)
                if self.local_type(src_name) == ExprType::Int =>
            {
                let src = self.existing_slot(src_name, *src_span)?;
                self.emit(Op::AddLocalI64 { dst, src });
                Ok(true)
            }
            (Expr::IntLit(n, _), BinOp::Add) => {
                self.emit(Op::AddLocalConstI64 {
                    slot: dst,
                    value: *n,
                });
                Ok(true)
            }
            (Expr::IntLit(n, _), BinOp::Sub) => {
                self.emit(Op::AddLocalConstI64 {
                    slot: dst,
                    value: -*n,
                });
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn try_inline_function(
        &mut self,
        name: &str,
        args: &[Expr],
        return_target: InlineReturn,
    ) -> Result<bool, CompileError> {
        if name == self.current_function_name {
            return Ok(false);
        }
        let Some(id) = self.chunk.function_ids.get(name).copied() else {
            return Ok(false);
        };
        if self.inlining_stack.contains(&id) {
            return Ok(false);
        }
        let callee = self.chunk.functions_by_id[id].clone();
        if args.len() != callee.params.len() {
            return Ok(false);
        }
        self.inlining_stack.push(id);

        if callee.chunk.code.len() > 30 {
            self.inlining_stack.pop();
            return Ok(false);
        }

        // Conservative guards: Only inline simple arithmetic/logic functions.
        // Reject if it contains complex operations or loops.
        for op in callee.chunk.code.iter() {
            match op {
                Op::Call(..) | Op::MemberCall(..) | Op::Import(..) => {
                    self.inlining_stack.pop();
                    return Ok(false);
                }
                Op::IterInit | Op::IterNext { .. } => {
                    // Loops are too complex for initial inlining stabilization
                    self.inlining_stack.pop();
                    return Ok(false);
                }
                Op::CallFn { .. }
                | Op::CallFnI64ToI64Stack { .. }
                | Op::CallFnI64ToLocal { .. }
                | Op::CallFnDiscard { .. }
                | Op::CallFnI64Discard { .. } => {
                    // Reject nested function calls to keep stack management simple
                    self.inlining_stack.pop();
                    return Ok(false);
                }
                _ => {}
            }
        }

        let mut constant_map = HashMap::new();
        for (i, constant) in callee.chunk.constants.iter().enumerate() {
            let new_idx = self.add_const(constant.clone());
            constant_map.insert(i, new_idx);
        }

        let inline_id = self.inlining_counter;
        self.inlining_counter += 1;

        // Check if we can inline all arguments before emitting anything
        for (i, arg_expr) in args.iter().enumerate() {
            let param_type = self
                .function_param_types
                .get(name)
                .and_then(|tys| tys.get(i))
                .copied()
                .unwrap_or(ExprType::Unknown);

            if param_type == ExprType::Int && !self.can_compile_i64_stack_expr(arg_expr) {
                self.inlining_stack.pop();
                return Ok(false);
            }
        }

        // Now compile all arguments to the stack first
        for (i, arg_expr) in args.iter().enumerate() {
            let param_type = self
                .function_param_types
                .get(name)
                .and_then(|tys| tys.get(i))
                .copied()
                .unwrap_or(ExprType::Unknown);

            if param_type == ExprType::Int {
                self.compile_i64_stack_expr(arg_expr)?;
            } else {
                self.compile_expr(arg_expr)?;
            }
        }

        let local_base = self.chunk.local_names.len();
        let mut param_slots = Vec::new();

        // First, allocate all parameter slots in forward order
        for (i, param_name) in callee.params.iter().enumerate() {
            let param_type = self
                .function_param_types
                .get(name)
                .and_then(|tys| tys.get(i))
                .copied()
                .unwrap_or(ExprType::Unknown);

            let full_name = format!("$inline_{}_{}_{}", inline_id, name, param_name);
            let slot = self.local_slot(&full_name);
            self.set_local_type(&full_name, param_type);
            param_slots.push((slot, param_type));
        }

        // Second, allocate any other locals in the callee
        for i in callee.params.len()..callee.chunk.local_names.len() {
            let callee_local_name = &callee.chunk.local_names[i];
            let full_name = format!("$inline_{}_{}_{}", inline_id, name, callee_local_name);
            let _slot = self.local_slot(&full_name);
            let kind = callee.chunk.local_kinds[i];
            let ty = if kind == LocalKind::I64 {
                ExprType::Int
            } else {
                ExprType::Unknown
            };
            self.set_local_type(&full_name, ty);
        }

        // Third, store arguments into parameter slots in reverse order
        for (slot, param_type) in param_slots.into_iter().rev() {
            if param_type == ExprType::Int {
                self.emit(Op::StoreLocalI64Stack(slot));
            } else {
                self.emit(Op::StoreLocal(slot));
            }
        }

        let mut offset_map = HashMap::new();
        let instruction_offset = self.chunk.code.len();
        for (i, mut op) in callee.chunk.code.clone().into_iter().enumerate() {
            offset_map.insert(i, self.chunk.code.len());
            match &mut op {
                Op::LoadLocal(slot)
                | Op::StoreLocal(slot)
                | Op::LoadLocalI64(slot)
                | Op::StoreLocalI64(slot)
                | Op::LoadLocalI64Stack(slot)
                | Op::StoreLocalI64Stack(slot)
                | Op::AddLocalConstI64 { slot, .. }
                | Op::LtLocalConstI64 { slot, .. }
                | Op::LeLocalConstI64 { slot, .. }
                | Op::GtLocalConstI64 { slot, .. }
                | Op::GeLocalConstI64 { slot, .. }
                | Op::JumpIfNotLtLocalConstI64 { slot, .. }
                | Op::JumpIfNotLeLocalConstI64 { slot, .. }
                | Op::JumpIfNotGtLocalConstI64 { slot, .. }
                | Op::JumpIfNotGeLocalConstI64 { slot, .. }
                | Op::IterNext { slot, .. }
                | Op::Import(_, slot) => {
                    *slot += local_base;
                    if let Op::AddLocalI64 { src: s, .. } = &mut op {
                        *s += local_base;
                    }
                }
                Op::AddLocalI64 { dst, src } => {
                    *dst += local_base;
                    *src += local_base;
                }
                Op::CallFnI64ToLocal { dst, .. } => {
                    *dst += local_base;
                }
                Op::Jump(_) | Op::JumpIfFalse(_) => {
                    // Jumps will be patched in a second pass
                }
                Op::Return => {
                    if !matches!(
                        return_target,
                        InlineReturn::ValueStack | InlineReturn::Discard
                    ) {
                        // Type mismatch in inline return
                    }
                    self.emit(Op::Jump(usize::MAX));
                    continue;
                }
                Op::ReturnI64 | Op::ReturnI64ToI64Stack => {
                    match return_target {
                        InlineReturn::I64Stack => {
                            self.emit(Op::Jump(usize::MAX));
                        }
                        InlineReturn::ValueStack => {
                            self.emit(Op::I64ToStack);
                            self.emit(Op::Jump(usize::MAX));
                        }
                        InlineReturn::I64Local(dst) => {
                            self.emit(Op::StoreLocalI64Stack(dst));
                            self.emit(Op::Jump(usize::MAX));
                        }
                        InlineReturn::Discard => {
                            self.emit(Op::PopI64Stack);
                            self.emit(Op::Jump(usize::MAX));
                        }
                    }
                    continue;
                }
                Op::Const(idx) => {
                    *idx = constant_map[idx];
                }
                Op::FStr(parts) => {
                    for part in parts {
                        if let dbyte_ast::FStrPart::Interp(name) = part {
                            *name = format!("$inline_{}_{}_{}", inline_id, callee.name, name);
                        }
                    }
                }
                _ => {}
            }
            if let Op::JumpIfNotLtLocalConstI64 { .. }
            | Op::JumpIfNotLeLocalConstI64 { .. }
            | Op::JumpIfNotGtLocalConstI64 { .. }
            | Op::JumpIfNotGeLocalConstI64 { .. } = &mut op
            {
                // Jumps will be patched in a second pass
            }
            if let Op::AddLocalI64 { .. } = &mut op {
                // Already handled in match
            }
            self.emit(op);
        }

        let end_of_inline = self.chunk.code.len();
        offset_map.insert(callee.chunk.code.len(), end_of_inline);

        for i in instruction_offset..end_of_inline {
            match &mut self.chunk.code[i] {
                Op::Jump(target) | Op::JumpIfFalse(target) => {
                    if *target == usize::MAX {
                        *target = end_of_inline;
                    } else if let Some(&new_target) = offset_map.get(target) {
                        *target = new_target;
                    }
                }
                Op::JumpIfNotLtLocalConstI64 { target, .. }
                | Op::JumpIfNotLeLocalConstI64 { target, .. }
                | Op::JumpIfNotGtLocalConstI64 { target, .. }
                | Op::JumpIfNotGeLocalConstI64 { target, .. } => {
                    if let Some(&new_target) = offset_map.get(target) {
                        *target = new_target;
                    }
                }
                Op::IterNext { jump, .. } => {
                    if let Some(&new_target) = offset_map.get(jump) {
                        *jump = new_target;
                    }
                }
                _ => {}
            }
        }

        self.inlining_stack.pop();
        Ok(true)
    }

    fn can_compile_i64_stack_expr(&self, expr: &Expr) -> bool {
        match expr {
            Expr::IntLit(_, _) => true,
            Expr::Ident(name, _) => self.local_type(name) == ExprType::Int,
            Expr::Binary {
                left, op, right, ..
            } => {
                matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div)
                    && self.can_compile_i64_stack_expr(left)
                    && self.can_compile_i64_stack_expr(right)
            }
            Expr::Call { name, args, .. } => self.can_compile_i64_stack_call(name, args),
            _ => false,
        }
    }

    fn expr_has_i64_stack_call(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Binary { left, right, .. } => {
                self.expr_has_i64_stack_call(left) || self.expr_has_i64_stack_call(right)
            }
            Expr::Call { name, args, .. } => {
                self.can_compile_i64_stack_call(name, args)
                    || args.iter().any(|arg| self.expr_has_i64_stack_call(arg))
            }
            _ => false,
        }
    }

    fn can_compile_i64_stack_call(&self, name: &str, args: &[Expr]) -> bool {
        self.chunk.function_ids.contains_key(name)
            && self.function_return_types.get(name).copied() == Some(ExprType::Int)
            && self.function_param_types.get(name).is_some_and(|params| {
                params.len() == args.len() && params.iter().all(|param| *param == ExprType::Int)
            })
            && args.iter().all(|arg| self.can_compile_i64_stack_expr(arg))
    }

    fn compile_i64_stack_expr(&mut self, expr: &Expr) -> Result<(), CompileError> {
        match expr {
            Expr::IntLit(value, _) => {
                self.emit(Op::ConstI64Stack(*value));
            }
            Expr::Ident(name, span) => {
                let slot = self.existing_slot(name, *span)?;
                self.emit(Op::LoadLocalI64Stack(slot));
            }
            Expr::Binary {
                left, op, right, ..
            } => {
                self.compile_i64_stack_expr(left)?;
                self.compile_i64_stack_expr(right)?;
                self.emit(match op {
                    BinOp::Add => Op::AddI64Stack,
                    BinOp::Sub => Op::SubI64Stack,
                    BinOp::Mul => Op::MulI64Stack,
                    BinOp::Div => Op::DivI64Stack,
                    _ => {
                        return Err(CompileError {
                            msg: "unsupported i64 stack expression".into(),
                            span: expr.span(),
                        })
                    }
                });
            }
            Expr::Call { name, args, .. } => {
                if self.try_inline_function(name, args, InlineReturn::I64Stack)? {
                    return Ok(());
                }
                for arg in args {
                    self.compile_i64_stack_expr(arg)?;
                }
                let id =
                    self.chunk
                        .function_ids
                        .get(name)
                        .copied()
                        .ok_or_else(|| CompileError {
                            msg: format!("undefined function `{}`", name),
                            span: expr.span(),
                        })?;
                self.emit(Op::CallFnI64ToI64Stack {
                    id,
                    argc: args.len(),
                });
            }
            _ => {
                return Err(CompileError {
                    msg: "unsupported i64 stack expression".into(),
                    span: expr.span(),
                })
            }
        }
        Ok(())
    }

    fn can_compile_i64_call_to_local_fast_path(&self, value: &Expr) -> bool {
        let Expr::Call { name, args, .. } = value else {
            return false;
        };
        self.chunk.function_ids.contains_key(name)
            && self.function_return_types.get(name).copied() == Some(ExprType::Int)
            && args.iter().all(Self::is_simple_call_arg)
    }

    fn compile_i64_call_to_local_fast_path(
        &mut self,
        value: &Expr,
        dst: usize,
    ) -> Result<(), CompileError> {
        let Expr::Call { name, args, .. } = value else {
            return Ok(());
        };
        if self.try_inline_function(name, args, InlineReturn::I64Local(dst))? {
            return Ok(());
        }
        for arg in args {
            self.compile_i64_stack_expr(arg)?;
        }
        let id = self
            .chunk
            .function_ids
            .get(name)
            .copied()
            .ok_or_else(|| CompileError {
                msg: format!("undefined function `{}`", name),
                span: value.span(),
            })?;
        self.emit(Op::CallFnI64ToLocal {
            id,
            argc: args.len(),
            dst,
        });
        Ok(())
    }

    fn is_simple_call_arg(expr: &Expr) -> bool {
        matches!(
            expr,
            Expr::IntLit(_, _)
                | Expr::FloatLit(_, _)
                | Expr::BoolLit(_, _)
                | Expr::StrLit(_, _)
                | Expr::BytesLit(_, _)
                | Expr::FStr(_, _)
                | Expr::Ident(_, _)
        )
    }

    fn compile_i64_compare_jump_fast_path(
        &mut self,
        expr: &Expr,
    ) -> Result<Option<usize>, CompileError> {
        let Expr::Binary {
            left, op, right, ..
        } = expr
        else {
            return Ok(None);
        };
        let Expr::Ident(name, span) = &**left else {
            return Ok(None);
        };
        if self.local_type(name) != ExprType::Int {
            return Ok(None);
        }
        let Expr::IntLit(value, _) = &**right else {
            return Ok(None);
        };
        let slot = self.existing_slot(name, *span)?;
        let op = match op {
            BinOp::Lt => Op::JumpIfNotLtLocalConstI64 {
                slot,
                value: *value,
                target: usize::MAX,
            },
            BinOp::LtEq => Op::JumpIfNotLeLocalConstI64 {
                slot,
                value: *value,
                target: usize::MAX,
            },
            BinOp::Gt => Op::JumpIfNotGtLocalConstI64 {
                slot,
                value: *value,
                target: usize::MAX,
            },
            BinOp::GtEq => Op::JumpIfNotGeLocalConstI64 {
                slot,
                value: *value,
                target: usize::MAX,
            },
            _ => return Ok(None),
        };
        Ok(Some(self.emit(op)))
    }

    fn compile_i64_compare_fast_path(&mut self, expr: &Expr) -> Result<bool, CompileError> {
        let Expr::Binary {
            left, op, right, ..
        } = expr
        else {
            return Ok(false);
        };
        let Expr::Ident(name, span) = &**left else {
            return Ok(false);
        };
        if self.local_type(name) != ExprType::Int {
            return Ok(false);
        }
        let Expr::IntLit(value, _) = &**right else {
            return Ok(false);
        };
        let slot = self.existing_slot(name, *span)?;
        let op = match op {
            BinOp::Lt => Op::LtLocalConstI64 {
                slot,
                value: *value,
            },
            BinOp::LtEq => Op::LeLocalConstI64 {
                slot,
                value: *value,
            },
            BinOp::Gt => Op::GtLocalConstI64 {
                slot,
                value: *value,
            },
            BinOp::GtEq => Op::GeLocalConstI64 {
                slot,
                value: *value,
            },
            _ => return Ok(false),
        };
        self.emit(op);
        Ok(true)
    }

    fn declared_or_expr_type(&self, ty: &TypeAnnotation, value: &Expr) -> ExprType {
        let declared = expr_type_from_annotation(ty);
        if declared != ExprType::Unknown {
            declared
        } else {
            self.expr_type(value)
        }
    }

    fn expr_type(&self, expr: &Expr) -> ExprType {
        match expr {
            Expr::IntLit(_, _) => ExprType::Int,
            Expr::BoolLit(_, _) => ExprType::Bool,
            Expr::StrLit(_, _) | Expr::FStr(_, _) => ExprType::Str,
            Expr::BytesLit(_, _) => ExprType::Bytes,
            Expr::Ident(name, _) => self.local_type(name),
            Expr::Binary {
                left, op, right, ..
            } => {
                let left = self.expr_type(left);
                let right = self.expr_type(right);
                if left == ExprType::Int && right == ExprType::Int {
                    match op {
                        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => ExprType::Int,
                        BinOp::EqEq
                        | BinOp::NotEq
                        | BinOp::Lt
                        | BinOp::Gt
                        | BinOp::LtEq
                        | BinOp::GtEq => ExprType::Bool,
                    }
                } else {
                    ExprType::Unknown
                }
            }
            Expr::Unary { expr, .. } => self.expr_type(expr),
            Expr::Call { name, .. } if name == "len" => ExprType::Int,
            Expr::Call { name, .. } => self
                .function_return_types
                .get(name)
                .copied()
                .unwrap_or(ExprType::Unknown),
            Expr::MemberCall {
                object, property, ..
            } => self.intrinsic_return_type(object, property),
            _ => ExprType::Unknown,
        }
    }

    fn intrinsic_return_type(&self, object: &Expr, property: &str) -> ExprType {
        match self.std_import_path(object) {
            Some("std.binary") if property == "u32_le" => ExprType::Int,
            Some("std.buffer") if property == "find" => ExprType::Int,
            Some("std.buffer") if property == "replace" => ExprType::Unknown,
            _ => ExprType::Unknown,
        }
    }

    fn compile_intrinsic_member_call(
        &mut self,
        object: &Expr,
        property: &str,
        args: &[Expr],
    ) -> Result<bool, CompileError> {
        let Some(path) = self.std_import_path(object) else {
            return Ok(false);
        };
        match (path, property, args.len()) {
            ("std.binary", "u32_le", 2) => {
                self.compile_expr(&args[0])?;
                self.compile_expr(&args[1])?;
                self.emit(Op::ReadU32Le);
                Ok(true)
            }
            ("std.buffer", "find", 2) => {
                self.compile_expr(&args[0])?;
                self.compile_expr(&args[1])?;
                self.emit(Op::BufferFind);
                Ok(true)
            }
            ("std.buffer", "replace", 3) => {
                self.compile_expr(&args[0])?;
                self.compile_expr(&args[1])?;
                self.compile_expr(&args[2])?;
                self.emit(Op::BufferReplace);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn compile_i64_stack_assign_fast_path(
        &mut self,
        name: &str,
        value: &Expr,
        span: Span,
    ) -> Result<bool, CompileError> {
        if self.local_type(name) != ExprType::Int
            || !self.expr_has_i64_stack_call(value)
            || !self.can_compile_i64_stack_expr(value)
        {
            return Ok(false);
        }
        let dst = self.existing_slot(name, span)?;
        self.compile_i64_stack_expr(value)?;
        self.emit(Op::StoreLocalI64Stack(dst));
        Ok(true)
    }

    fn std_import_path(&self, expr: &Expr) -> Option<&str> {
        let Expr::Ident(alias, _) = expr else {
            return None;
        };
        self.imports.get(alias).map(String::as_str)
    }
}

fn expr_type_from_annotation(ty: &TypeAnnotation) -> ExprType {
    match ty {
        TypeAnnotation::Int => ExprType::Int,
        TypeAnnotation::Bool => ExprType::Bool,
        TypeAnnotation::Str => ExprType::Str,
        TypeAnnotation::Bytes => ExprType::Bytes,
        TypeAnnotation::Buffer => ExprType::Buffer,
        _ => ExprType::Unknown,
    }
}
