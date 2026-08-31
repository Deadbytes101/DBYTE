use dbyte_ast::*;
use dbyte_bytecode::{BytecodeFunction, Chunk, LocalKind, NativeFn, Op, Value};

mod canonical;
pub use canonical::{
    lower_concrete_project_to_bytecode, lower_concrete_project_to_bytecode_at_level,
};

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
    local_nominal_types: HashMap<String, String>,
    drop_function_ids: HashMap<String, usize>,
    struct_defs: HashMap<String, Vec<String>>,
    enum_defs: HashMap<String, HashMap<String, usize>>,
    imports: HashMap<String, String>,
    return_type: ExprType,
    inlining_stack: Vec<usize>,
    inlining_counter: usize,
    current_function_name: String,
    active_cleanups: Vec<CleanupAction>,
    compiling_cleanup: bool,
    loops: Vec<LoopContext>,
}

struct LoopContext {
    continue_target: usize,
    cleanup_base: usize,
    break_jumps: Vec<usize>,
}

#[derive(Clone)]
enum CleanupAction {
    Defer(Vec<Stmt>),
    Drop {
        name: String,
        slot: usize,
        function_id: usize,
    },
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

fn nominal_drop_target(annotation: &TypeAnnotation) -> Option<String> {
    let TypeAnnotation::Borrow { pointee, .. } = annotation else {
        return None;
    };
    match pointee.as_ref() {
        TypeAnnotation::Named(name) | TypeAnnotation::Generic { base: name, .. } => {
            Some(name.clone())
        }
        _ => None,
    }
}

fn nominal_value_type(annotation: &TypeAnnotation, value: &Expr) -> Option<String> {
    match annotation {
        TypeAnnotation::Named(name) | TypeAnnotation::Generic { base: name, .. } => {
            Some(name.clone())
        }
        TypeAnnotation::Inferred => match value {
            Expr::Invoke { callee, .. } => match callee.as_ref() {
                Expr::Ident(name, _) => Some(name.clone()),
                Expr::GenericApply { target, .. } => match target.as_ref() {
                    Expr::Ident(name, _) => Some(name.clone()),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

impl FunctionCompiler {
    fn new(name: String) -> Self {
        Self::new_with_return(name, ExprType::Unknown)
    }

    fn new_with_return(name: String, return_type: ExprType) -> Self {
        let enum_defs = HashMap::from([
            (
                "Option".into(),
                HashMap::from([("None".into(), 0), ("Some".into(), 1)]),
            ),
            (
                "Result".into(),
                HashMap::from([("Ok".into(), 1), ("Err".into(), 1)]),
            ),
        ]);
        Self {
            chunk: Chunk::new(name.clone()),
            locals: HashMap::new(),
            local_types: HashMap::new(),
            function_return_types: HashMap::new(),
            function_param_types: HashMap::new(),
            local_nominal_types: HashMap::new(),
            drop_function_ids: HashMap::new(),
            struct_defs: HashMap::new(),
            enum_defs,
            imports: HashMap::new(),
            return_type,
            inlining_stack: Vec::new(),
            inlining_counter: 0,
            current_function_name: name,
            active_cleanups: Vec::new(),
            compiling_cleanup: false,
            loops: Vec::new(),
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
                kind,
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
                let id = self.reserve_function(name, params);
                if *kind == FunctionKind::Drop {
                    if let Some(target) = params
                        .first()
                        .and_then(|parameter| nominal_drop_target(&parameter.ty))
                    {
                        self.drop_function_ids.insert(target, id);
                    }
                }
            }
        }
    }

    fn reserve_types(&mut self, stmts: &[Stmt]) {
        for statement in stmts {
            match statement {
                Stmt::StructDef { name, fields, .. } => {
                    self.struct_defs.insert(
                        name.clone(),
                        fields.iter().map(|field| field.name.clone()).collect(),
                    );
                }
                Stmt::EnumDef { name, variants, .. } => {
                    self.enum_defs.insert(
                        name.clone(),
                        variants
                            .iter()
                            .map(|variant| (variant.name.clone(), variant.payload.len()))
                            .collect(),
                    );
                }
                _ => {}
            }
        }
    }

    fn compile_stmts(&mut self, stmts: &[Stmt]) -> Result<(), CompileError> {
        self.reserve_functions(stmts);
        self.reserve_types(stmts);
        let cleanup_base = self.active_cleanups.len();
        for stmt in stmts {
            if let Stmt::Defer { body, .. } = stmt {
                self.active_cleanups
                    .push(CleanupAction::Defer(body.clone()));
            } else {
                self.compile_stmt(stmt)?;
            }
        }
        if !self.compiling_cleanup {
            self.compile_cleanup_from(cleanup_base, None)?;
        }
        self.active_cleanups.truncate(cleanup_base);
        Ok(())
    }

    fn compile_cleanup_from(
        &mut self,
        base: usize,
        moved_local: Option<&str>,
    ) -> Result<(), CompileError> {
        let actions: Vec<_> = self.active_cleanups[base..].iter().rev().cloned().collect();
        let previous = self.compiling_cleanup;
        self.compiling_cleanup = true;
        let result = (|| {
            for action in actions {
                match action {
                    CleanupAction::Defer(body) => {
                        for statement in &body {
                            if matches!(statement, Stmt::Defer { .. }) {
                                return Err(CompileError {
                                    msg: "nested defer inside cleanup is not yet available in the hosted VM"
                                        .into(),
                                    span: statement.span(),
                                });
                            }
                            self.compile_stmt(statement)?;
                        }
                    }
                    CleanupAction::Drop {
                        name,
                        slot,
                        function_id,
                    } => {
                        if moved_local == Some(name.as_str()) {
                            continue;
                        }
                        self.emit(Op::LoadLocal(slot));
                        self.emit(Op::CallFnDiscard {
                            id: function_id,
                            argc: 1,
                        });
                    }
                }
            }
            Ok(())
        })();
        self.compiling_cleanup = previous;
        result
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
                if let Some(nominal) = nominal_value_type(ty, value) {
                    self.local_nominal_types.insert(name.clone(), nominal);
                }
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
                if let Some(nominal) = self.local_nominal_types.get(name) {
                    if let Some(function_id) = self.drop_function_ids.get(nominal).copied() {
                        self.active_cleanups.push(CleanupAction::Drop {
                            name: name.clone(),
                            slot,
                            function_id,
                        });
                    }
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
                let slot = self.existing_slot(name, *span)?;
                let drop_id = self
                    .active_cleanups
                    .iter()
                    .rev()
                    .find_map(|cleanup| match cleanup {
                        CleanupAction::Drop {
                            name: owner,
                            function_id,
                            ..
                        } if owner == name => Some(*function_id),
                        _ => None,
                    });
                self.compile_expr(value)?;
                if self.local_type(name) == ExprType::Int {
                    self.emit(Op::StoreLocalI64(slot));
                } else if let Some(function_id) = drop_id {
                    let temporary =
                        self.local_slot(&format!("__v121_assign_{}", self.inlining_counter));
                    self.inlining_counter += 1;
                    self.emit(Op::StoreLocal(temporary));
                    self.emit(Op::LoadLocal(slot));
                    self.emit(Op::CallFnDiscard {
                        id: function_id,
                        argc: 1,
                    });
                    self.emit(Op::LoadLocal(temporary));
                    self.emit(Op::StoreLocal(slot));
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
                child.drop_function_ids = self.drop_function_ids.clone();
                child.struct_defs = self.struct_defs.clone();
                child.enum_defs = self.enum_defs.clone();
                child.imports = self.imports.clone();
                child.inlining_counter = self.inlining_counter;

                for param in params {
                    let slot = child.local_slot(&param.name);
                    child.set_local_type(&param.name, expr_type_from_annotation(&param.ty));
                    if let Some(nominal) = nominal_value_type(&param.ty, &Expr::UnitLit(param.span))
                    {
                        child
                            .local_nominal_types
                            .insert(param.name.clone(), nominal.clone());
                        if let Some(function_id) = child.drop_function_ids.get(&nominal).copied() {
                            child.active_cleanups.push(CleanupAction::Drop {
                                name: param.name.clone(),
                                slot,
                                function_id,
                            });
                        }
                    }
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
                if !self.active_cleanups.is_empty() && !self.compiling_cleanup {
                    let moved_local = value.as_ref().and_then(|value| match value {
                        Expr::Ident(name, _) => Some(name.as_str()),
                        _ => None,
                    });
                    if let Some(value) = value {
                        self.compile_expr(value)?;
                        let temporary = format!("__v12_return_{}", self.inlining_counter);
                        self.inlining_counter += 1;
                        let slot = self.local_slot(&temporary);
                        self.set_local_type(&temporary, self.return_type);
                        if self.return_type == ExprType::Int {
                            self.emit(Op::StoreLocalI64(slot));
                        } else {
                            self.emit(Op::StoreLocal(slot));
                        }
                        self.compile_cleanup_from(0, moved_local)?;
                        if self.return_type == ExprType::Int {
                            self.emit(Op::LoadLocalI64(slot));
                            self.emit(Op::ReturnI64);
                        } else {
                            self.emit(Op::LoadLocal(slot));
                            self.emit(Op::Return);
                        }
                    } else {
                        self.compile_cleanup_from(0, None)?;
                        let index = self.add_const(Value::Void);
                        self.emit(Op::Const(index));
                        self.emit(Op::Return);
                    }
                    return Ok(());
                }
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
                self.loops.push(LoopContext {
                    continue_target: loop_start,
                    cleanup_base: self.active_cleanups.len(),
                    break_jumps: Vec::new(),
                });
                self.compile_stmts(body)?;
                let loop_context = self.loops.pop().expect("active while context");
                self.emit(Op::Jump(loop_start));
                let end = self.chunk.code.len();
                self.patch_jump(exit, end);
                for jump in loop_context.break_jumps {
                    self.patch_jump(jump, end);
                }
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
                self.loops.push(LoopContext {
                    continue_target: loop_start,
                    cleanup_base: self.active_cleanups.len(),
                    break_jumps: Vec::new(),
                });
                self.compile_stmts(body)?;
                let loop_context = self.loops.pop().expect("active for context");
                self.emit(Op::Jump(loop_start));
                let end = self.chunk.code.len();
                self.patch_jump(next, end);
                for jump in loop_context.break_jumps {
                    self.patch_jump(jump, end);
                }
            }
            Stmt::Import { path, alias, .. } => {
                let slot = self.local_slot(alias);
                self.imports.insert(alias.clone(), path.clone());
                self.set_local_type(alias, ExprType::Module);
                self.emit(Op::Import(path.clone(), slot));
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
                    return Err(CompileError {
                        msg: "hosted VM store currently requires a struct field lvalue".into(),
                        span: *span,
                    });
                };
                self.compile_expr(object)?;
                self.compile_expr(value)?;
                self.emit(Op::StoreField(property.clone()));
            }
            Stmt::StructDef { .. } | Stmt::EnumDef { .. } => {}
            Stmt::Match {
                value, arms, span, ..
            } => {
                self.compile_expr(value)?;
                let temporary_name = format!("__v12_match_{}", self.inlining_counter);
                self.inlining_counter += 1;
                let value_slot = self.local_slot(&temporary_name);
                self.emit(Op::StoreLocal(value_slot));
                let mut end_jumps = Vec::new();
                for arm in arms {
                    let next = match &arm.pattern {
                        Pattern::Wildcard(_) => None,
                        Pattern::Variant { path, bindings, .. } => {
                            let Some((enum_name, variant)) = path.rsplit_once('.') else {
                                return Err(CompileError {
                                    msg: "variant pattern must be qualified".into(),
                                    span: arm.span,
                                });
                            };
                            self.emit(Op::LoadLocal(value_slot));
                            self.emit(Op::IsVariant {
                                enum_name: enum_name.to_string(),
                                variant: variant.to_string(),
                            });
                            let jump = self.emit(Op::JumpIfFalse(usize::MAX));
                            for (index, binding) in bindings.iter().enumerate() {
                                self.emit(Op::EnumPayload {
                                    slot: value_slot,
                                    index,
                                });
                                let slot = self.local_slot(binding);
                                self.emit(Op::StoreLocal(slot));
                            }
                            Some(jump)
                        }
                    };
                    self.compile_stmts(&arm.body)?;
                    end_jumps.push(self.emit(Op::Jump(usize::MAX)));
                    if let Some(next) = next {
                        let target = self.chunk.code.len();
                        self.patch_jump(next, target);
                    }
                }
                if arms.is_empty() {
                    return Err(CompileError {
                        msg: "match requires at least one arm".into(),
                        span: *span,
                    });
                }
                let end = self.chunk.code.len();
                for jump in end_jumps {
                    self.patch_jump(jump, end);
                }
            }
            Stmt::Defer { .. } => {}
            Stmt::Break { span } => {
                let Some((cleanup_base, _)) = self
                    .loops
                    .last()
                    .map(|context| (context.cleanup_base, context.continue_target))
                else {
                    return Err(CompileError {
                        msg: "break outside loop".into(),
                        span: *span,
                    });
                };
                self.compile_cleanup_from(cleanup_base, None)?;
                let jump = self.emit(Op::Jump(usize::MAX));
                self.loops
                    .last_mut()
                    .expect("active loop context")
                    .break_jumps
                    .push(jump);
            }
            Stmt::Continue { span } => {
                let Some((cleanup_base, continue_target)) = self
                    .loops
                    .last()
                    .map(|context| (context.cleanup_base, context.continue_target))
                else {
                    return Err(CompileError {
                        msg: "continue outside loop".into(),
                        span: *span,
                    });
                };
                self.compile_cleanup_from(cleanup_base, None)?;
                self.emit(Op::Jump(continue_target));
            }
            Stmt::Const { span, .. } | Stmt::Static { span, .. } => {
                return Err(CompileError {
                    msg: "native systems statements are unavailable in the hosted VM compiler"
                        .into(),
                    span: *span,
                });
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
            Expr::UnitLit(_) => {
                let idx = self.add_const(Value::Void);
                self.emit(Op::Const(idx));
            }
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
            Expr::NullLit(span) | Expr::GenericApply { span, .. } => {
                return Err(CompileError {
                    msg: "v12 expression requires canonical HIR lowering".into(),
                    span: *span,
                });
            }
            Expr::Borrow { expr, .. } => self.compile_expr(expr)?,
            Expr::Drop { expr, .. } => {
                let drop_id = if let Expr::Ident(name, _) = expr.as_ref() {
                    let cleanup_index = self.active_cleanups.iter().rposition(|cleanup| {
                        matches!(cleanup, CleanupAction::Drop { name: owner, .. } if owner == name)
                    });
                    cleanup_index.and_then(|index| match self.active_cleanups.remove(index) {
                        CleanupAction::Drop { function_id, .. } => Some(function_id),
                        CleanupAction::Defer(_) => None,
                    })
                } else {
                    None
                };
                self.compile_expr(expr)?;
                if let Some(id) = drop_id {
                    self.emit(Op::CallFnDiscard { id, argc: 1 });
                } else {
                    self.emit(Op::Pop);
                }
                let idx = self.add_const(Value::Void);
                self.emit(Op::Const(idx));
            }
            Expr::Try { expr, .. } => {
                self.compile_expr(expr)?;
                if self.active_cleanups.is_empty() || self.compiling_cleanup {
                    self.emit(Op::TryResult);
                } else {
                    let temporary = format!("__v12_try_{}", self.inlining_counter);
                    self.inlining_counter += 1;
                    let slot = self.local_slot(&temporary);
                    self.emit(Op::StoreLocal(slot));
                    self.emit(Op::LoadLocal(slot));
                    self.emit(Op::IsVariant {
                        enum_name: "Result".into(),
                        variant: "Err".into(),
                    });
                    let ok = self.emit(Op::JumpIfFalse(usize::MAX));
                    self.compile_cleanup_from(0, None)?;
                    self.emit(Op::LoadLocal(slot));
                    self.emit(Op::Return);
                    let ok_target = self.chunk.code.len();
                    self.patch_jump(ok, ok_target);
                    self.emit(Op::EnumPayload { slot, index: 0 });
                }
            }
            Expr::Closure {
                captures,
                params,
                ret_ty,
                body,
                span,
            } => {
                if captures
                    .iter()
                    .any(|capture| capture.mode == CaptureMode::BorrowMut)
                {
                    return Err(CompileError {
                        msg: "borrow-mut closure requires reference-cell VM lowering".into(),
                        span: *span,
                    });
                }
                let closure_name = format!("__closure_{}", self.inlining_counter);
                self.inlining_counter += 1;
                let mut child = FunctionCompiler::new_with_return(
                    closure_name.clone(),
                    expr_type_from_annotation(ret_ty),
                );
                child.chunk.functions = self.chunk.functions.clone();
                child.chunk.function_ids = self.chunk.function_ids.clone();
                child.chunk.functions_by_id = self.chunk.functions_by_id.clone();
                child.function_return_types = self.function_return_types.clone();
                child.function_param_types = self.function_param_types.clone();
                child.drop_function_ids = self.drop_function_ids.clone();
                child.struct_defs = self.struct_defs.clone();
                child.enum_defs = self.enum_defs.clone();
                for capture in captures {
                    child.local_slot(&capture.name);
                    child.set_local_type(&capture.name, self.local_type(&capture.name));
                }
                for parameter in params {
                    child.local_slot(&parameter.name);
                    child.set_local_type(&parameter.name, expr_type_from_annotation(&parameter.ty));
                }
                child.compile_stmts(body)?;
                let void = child.add_const(Value::Void);
                child.emit(Op::Const(void));
                child.emit(Op::Return);
                for capture in captures {
                    let slot = self.existing_slot(&capture.name, capture.span)?;
                    if self.local_type(&capture.name) == ExprType::Int {
                        self.emit(Op::LoadLocalI64(slot));
                    } else {
                        self.emit(Op::LoadLocal(slot));
                    }
                }
                self.emit(Op::MakeClosure {
                    function: Box::new(BytecodeFunction {
                        name: closure_name,
                        params: captures
                            .iter()
                            .map(|capture| capture.name.clone())
                            .chain(params.iter().map(|parameter| parameter.name.clone()))
                            .collect(),
                        chunk: child.finish(),
                    }),
                    capture_count: captures.len(),
                });
            }
            Expr::Invoke {
                callee,
                args,
                named_args,
                span,
            } => {
                let name = match callee.as_ref() {
                    Expr::Ident(name, _) => Some(name.as_str()),
                    Expr::GenericApply { target, .. } => match target.as_ref() {
                        Expr::Ident(name, _) => Some(name.as_str()),
                        _ => None,
                    },
                    _ => None,
                };
                let Some(name) = name else {
                    if !named_args.is_empty() {
                        return Err(CompileError {
                            msg: "indirect calls do not accept named arguments".into(),
                            span: *span,
                        });
                    }
                    self.compile_expr(callee)?;
                    for argument in args {
                        self.compile_expr(argument)?;
                    }
                    self.emit(Op::CallValue(args.len()));
                    return Ok(());
                };
                if let Some(fields) = self.struct_defs.get(name).cloned() {
                    if !args.is_empty() || named_args.len() != fields.len() {
                        return Err(CompileError {
                            msg: format!("struct `{name}` requires every named field once"),
                            span: *span,
                        });
                    }
                    for field in &fields {
                        let argument = named_args
                            .iter()
                            .find(|argument| argument.name == *field)
                            .ok_or_else(|| CompileError {
                            msg: format!("struct `{name}` is missing field `{field}`"),
                            span: *span,
                        })?;
                        self.compile_expr(&argument.value)?;
                    }
                    self.emit(Op::MakeStruct {
                        name: name.to_string(),
                        fields,
                    });
                    return Ok(());
                }
                if !named_args.is_empty() {
                    return Err(CompileError {
                        msg: "function calls do not accept named arguments".into(),
                        span: *span,
                    });
                }
                for argument in args {
                    self.compile_expr(argument)?;
                }
                if let Some(id) = self.chunk.function_ids.get(name).copied() {
                    self.emit(Op::CallFn {
                        id,
                        argc: args.len(),
                    });
                } else {
                    self.emit(Op::Call(name.to_string(), args.len()));
                }
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
                    (true, BinOp::Rem) => Op::RemI64,
                    (true, BinOp::Lt) => Op::LtI64,
                    (true, BinOp::LtEq) => Op::LeI64,
                    (true, BinOp::Gt) => Op::GtI64,
                    (true, BinOp::GtEq) => Op::GeI64,
                    (_, BinOp::Add) => Op::Add,
                    (_, BinOp::Sub) => Op::Sub,
                    (_, BinOp::Mul) => Op::Mul,
                    (_, BinOp::Div) => Op::Div,
                    (_, BinOp::Rem) => Op::Rem,
                    (_, BinOp::EqEq) => Op::Eq,
                    (_, BinOp::NotEq) => Op::Ne,
                    (_, BinOp::Lt) => Op::Lt,
                    (_, BinOp::LtEq) => Op::Le,
                    (_, BinOp::Gt) => Op::Gt,
                    (_, BinOp::GtEq) => Op::Ge,
                    (_, BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr) => {
                        return Err(CompileError {
                            msg: format!(
                                "native systems operator `{op}` is unavailable in the hosted VM compiler"
                            ),
                            span: expr.span(),
                        });
                    }
                });
            }
            Expr::Unary { op, expr, .. } => {
                self.compile_expr(expr)?;
                self.emit(match op {
                    UnaryOp::Neg => Op::Neg,
                    UnaryOp::Not => Op::Not,
                    UnaryOp::BitNot | UnaryOp::AddressOf | UnaryOp::Deref => {
                        return Err(CompileError {
                            msg: "native systems unary operators are unavailable in the hosted VM compiler"
                                .into(),
                            span: expr.span(),
                        });
                    }
                });
            }
            Expr::Call { name, args, .. } => {
                if self.locals.contains_key(name) && !self.chunk.function_ids.contains_key(name) {
                    let slot = self.locals[name];
                    self.emit(Op::LoadLocal(slot));
                    for argument in args {
                        self.compile_expr(argument)?;
                    }
                    self.emit(Op::CallValue(args.len()));
                    return Ok(());
                }
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
                let enum_name = match object.as_ref() {
                    Expr::Ident(name, _) => Some(name.as_str()),
                    Expr::GenericApply { target, .. } => match target.as_ref() {
                        Expr::Ident(name, _) => Some(name.as_str()),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(enum_name) = enum_name {
                    if let Some(expected) = self
                        .enum_defs
                        .get(enum_name)
                        .and_then(|variants| variants.get(property))
                        .copied()
                    {
                        if args.len() != expected {
                            return Err(CompileError {
                                msg: format!(
                                    "variant `{enum_name}.{property}` expects {expected} payload values"
                                ),
                                span: expr.span(),
                            });
                        }
                        for argument in args {
                            self.compile_expr(argument)?;
                        }
                        self.emit(Op::MakeEnum {
                            enum_name: enum_name.to_string(),
                            variant: property.clone(),
                            payload_count: args.len(),
                        });
                        return Ok(());
                    }
                }
                if self.compile_intrinsic_member_call(object, property, args)? {
                    return Ok(());
                }
                self.compile_expr(object)?;
                for arg in args {
                    self.compile_expr(arg)?;
                }
                self.emit(Op::MemberCall(property.clone(), args.len()));
            }
            Expr::Cast { span, .. } => {
                return Err(CompileError {
                    msg: "native systems casts are unavailable in the hosted VM compiler".into(),
                    span: *span,
                });
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
                Op::Call(..)
                | Op::MemberCall(..)
                | Op::Import(..)
                | Op::MakeStruct { .. }
                | Op::MakeEnum { .. }
                | Op::IsVariant { .. }
                | Op::EnumPayload { .. }
                | Op::StoreField(..)
                | Op::MakeClosure { .. }
                | Op::CallValue(..)
                | Op::TryResult => {
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

        let mut params_read_only = vec![true; callee.params.len()];
        let mut param_usage_count = vec![0; callee.params.len()];
        for op in callee.chunk.code.iter() {
            match op {
                Op::StoreLocal(slot)
                | Op::StoreLocalI64(slot)
                | Op::StoreLocalI64Stack(slot)
                | Op::AddLocalI64 { dst: slot, .. }
                | Op::AddLocalConstI64 { slot, .. }
                | Op::IterNext { slot, .. }
                | Op::Import(_, slot)
                    if *slot < callee.params.len() =>
                {
                    params_read_only[*slot] = false;
                }
                Op::LoadLocal(slot) | Op::LoadLocalI64(slot) | Op::LoadLocalI64Stack(slot)
                    if *slot < callee.params.len() =>
                {
                    param_usage_count[*slot] += 1;
                }
                _ => {}
            }
        }

        // Check if we can use "Stack Pass-Through"
        // Condition: first N opcodes are LOAD_LOCAL_I64_STACK 0, 1, ..., N-1
        // and these are the only usages of those params.
        let mut can_pass_through = true;
        if callee.params.is_empty() || callee.chunk.code.len() < callee.params.len() {
            can_pass_through = false;
        } else {
            for i in 0..callee.params.len() {
                if !params_read_only[i] || param_usage_count[i] != 1 {
                    can_pass_through = false;
                    break;
                }
                match callee.chunk.code[i] {
                    Op::LoadLocalI64Stack(slot) if slot == i => {}
                    _ => {
                        can_pass_through = false;
                        break;
                    }
                }
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

        // Third, store arguments into parameter slots in reverse order (unless passing through)
        if !can_pass_through {
            for (slot, param_type) in param_slots.into_iter().rev() {
                if param_type == ExprType::Int {
                    self.emit(Op::StoreLocalI64Stack(slot));
                } else {
                    self.emit(Op::StoreLocal(slot));
                }
            }
        }

        let mut offset_map = HashMap::new();
        let instruction_offset = self.chunk.code.len();
        let start_index = if can_pass_through {
            callee.params.len()
        } else {
            0
        };

        for (i, mut op) in callee.chunk.code.clone().into_iter().enumerate() {
            if i < start_index {
                continue;
            }
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
                matches!(
                    op,
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem
                ) && self.can_compile_i64_stack_expr(left)
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
                    BinOp::Rem => Op::RemI64Stack,
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
                        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                            ExprType::Int
                        }
                        BinOp::EqEq
                        | BinOp::NotEq
                        | BinOp::Lt
                        | BinOp::Gt
                        | BinOp::LtEq
                        | BinOp::GtEq => ExprType::Bool,
                        BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                            ExprType::Unknown
                        }
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
            Expr::Invoke { callee, .. } => {
                let name = match callee.as_ref() {
                    Expr::Ident(name, _) => Some(name),
                    Expr::GenericApply { target, .. } => match target.as_ref() {
                        Expr::Ident(name, _) => Some(name),
                        _ => None,
                    },
                    _ => None,
                };
                name.and_then(|name| self.function_return_types.get(name).copied())
                    .unwrap_or(ExprType::Unknown)
            }
            Expr::Borrow { expr, .. } => self.expr_type(expr),
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
            Some("std.buffer") if property == "load" => ExprType::Buffer,
            Some("std.buffer") if property == "replace" || property == "save" => ExprType::Unknown,
            Some("std.fs") if property == "exists" => ExprType::Int,
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
            ("std.buffer", "load", 1) => {
                self.compile_expr(&args[0])?;
                self.emit(Op::BufferLoad);
                Ok(true)
            }
            ("std.buffer", "save", 2) => {
                self.compile_expr(&args[0])?;
                self.compile_expr(&args[1])?;
                self.emit(Op::BufferSave);
                Ok(true)
            }
            ("std.fs", "exists", 1) => {
                self.compile_expr(&args[0])?;
                self.emit(Op::CallNative(NativeFn::FsExists));
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

#[cfg(test)]
mod v12_tests {
    use super::*;
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;

    #[test]
    fn generic_direct_invocation_erases_to_an_ordinary_vm_call() {
        let source =
            "fn id[T](value: T) -> T:\n    return value\n\nlet answer: int = id[int](42)\n";
        let program = Parser::new(Lexer::new(source).tokenize().unwrap())
            .parse_program()
            .unwrap();
        let chunk = Compiler::new().compile_program(&program).unwrap();
        assert!(chunk
            .code
            .iter()
            .any(|operation| { matches!(operation, Op::CallFn { argc: 1, .. } | Op::Call(_, 1)) }));
    }
}
