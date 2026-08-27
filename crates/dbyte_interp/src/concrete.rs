//! Hosted execution of fully resolved, monomorphized HIR.
//!
//! This executor deliberately has no access to source names or AST nodes. All
//! semantic choices are consumed through canonical IDs and explicit cleanup
//! edges produced by `dbyte_hir`.

use crate::memory::{AllocationEvent, DeterministicMemory};
use crate::RuntimeError;
use dbyte_ast::{BinOp, CaptureMode, MatchMode, Span, UnaryOp};
use dbyte_hir::{
    enum_variant_layout, layout_of, struct_field_layout, CanonicalType, ConcreteHirFunction,
    ConcreteHirProject, FieldId, FunctionInstanceKey, HirBlock, HirExpr, HirExprKind,
    HirFormatPart, HirIntrinsic, HirMatchArm, HirPlace, HirPlaceBase, HirProjection, HirStatement,
    HostCapabilityId, IntegerType, LocalId, OwnershipAction, SymbolId, TypeDefKind, TypeId,
    VariantId,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

type Slot = Rc<RefCell<Option<ConcreteValue>>>;

#[derive(Debug, Clone)]
pub enum ConcreteValue {
    Unit,
    Integer(i64),
    Float(f64),
    Bool(bool),
    String(Vec<u8>),
    Bytes(Vec<u8>),
    Buffer(Rc<RefCell<Vec<u8>>>),
    Null,
    List(Vec<Slot>),
    Struct {
        symbol: SymbolId,
        fields: BTreeMap<FieldId, Slot>,
    },
    Enum {
        symbol: SymbolId,
        variant: VariantId,
        discriminant: u32,
        payload: Vec<Slot>,
    },
    Function(FunctionInstanceKey),
    MemoryPointer {
        address: u64,
        pointee: TypeId,
    },
    View {
        address: u64,
        length: u64,
        element: TypeId,
        mutable: bool,
        string: bool,
    },
    Reference(Slot),
    Closure(Rc<ConcreteClosure>),
    #[doc(hidden)]
    EarlyReturn(Box<ConcreteValue>),
}

#[derive(Debug)]
pub struct ConcreteClosure {
    captures: BTreeMap<LocalId, Slot>,
    parameters: Vec<dbyte_hir::HirLocal>,
    body: HirBlock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConcreteExecution {
    pub output: String,
    pub allocator_events: Vec<AllocationEvent>,
    pub live_allocations: usize,
}

#[derive(Debug)]
pub struct ConcreteSession {
    globals: BTreeMap<SymbolId, Slot>,
    top_level_locals: BTreeMap<LocalId, Slot>,
    initialized_globals: BTreeSet<SymbolId>,
    memory: DeterministicMemory,
    memory_values: BTreeMap<u64, (TypeId, ConcreteValue)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConcreteIncrementalExecution {
    pub output: String,
    pub next_cursor: usize,
}

enum Flow {
    Normal,
    Return(ConcreteValue),
    Break,
    Continue,
}

struct Frame {
    locals: BTreeMap<LocalId, Slot>,
}

struct Executor<'a> {
    project: &'a ConcreteHirProject,
    globals: BTreeMap<SymbolId, Slot>,
    frames: Vec<Frame>,
    output: String,
    call_depth: usize,
    script_args: Vec<String>,
    memory: DeterministicMemory,
    memory_values: BTreeMap<u64, (TypeId, ConcreteValue)>,
}

pub fn execute_concrete_project(
    project: &ConcreteHirProject,
) -> Result<ConcreteExecution, RuntimeError> {
    execute_concrete_project_with_args(project, Vec::new())
}

pub fn execute_concrete_project_with_memory(
    project: &ConcreteHirProject,
    memory: DeterministicMemory,
) -> Result<ConcreteExecution, RuntimeError> {
    execute_concrete_project_inner(project, Vec::new(), memory)
}

pub fn execute_concrete_project_with_args(
    project: &ConcreteHirProject,
    script_args: Vec<String>,
) -> Result<ConcreteExecution, RuntimeError> {
    execute_concrete_project_inner(project, script_args, DeterministicMemory::system())
}

fn execute_concrete_project_inner(
    project: &ConcreteHirProject,
    script_args: Vec<String>,
    memory: DeterministicMemory,
) -> Result<ConcreteExecution, RuntimeError> {
    let mut executor = Executor {
        project,
        globals: BTreeMap::new(),
        frames: vec![Frame {
            locals: BTreeMap::new(),
        }],
        output: String::new(),
        call_depth: 0,
        script_args,
        memory,
        memory_values: BTreeMap::new(),
    };
    executor.initialize_globals()?;
    match executor.execute_hir_block(&project.executable.top_level)? {
        Flow::Normal => Ok(ConcreteExecution {
            output: executor.output,
            allocator_events: executor.memory.telemetry().to_vec(),
            live_allocations: executor.memory.live_allocations(),
        }),
        Flow::Return(_) => Err(runtime_error("return outside function", Span::zero())),
        Flow::Break => Err(runtime_error("break outside loop", Span::zero())),
        Flow::Continue => Err(runtime_error("continue outside loop", Span::zero())),
    }
}

/// Transactionally execute only top-level statements after `cursor`.
pub fn execute_concrete_project_incremental(
    session: &mut ConcreteSession,
    project: &ConcreteHirProject,
    cursor: usize,
    script_args: Vec<String>,
) -> Result<ConcreteIncrementalExecution, RuntimeError> {
    if cursor > project.executable.top_level.statements.len() {
        return Err(runtime_error(
            "canonical execution cursor is outside the project",
            Span::zero(),
        ));
    }
    let mut candidate = session.deep_clone();
    let mut executor = Executor {
        project,
        globals: std::mem::take(&mut candidate.globals),
        frames: vec![Frame {
            locals: std::mem::take(&mut candidate.top_level_locals),
        }],
        output: String::new(),
        call_depth: 0,
        script_args,
        memory: candidate.memory.clone(),
        memory_values: candidate.memory_values.clone(),
    };
    for global in &project.executable.globals {
        executor
            .globals
            .entry(global.symbol)
            .or_insert_with(|| slot(None));
        if !candidate.initialized_globals.contains(&global.symbol) {
            if let Some(initializer) = &global.initializer {
                let value = executor.execute_hir_expr(initializer)?;
                executor
                    .globals
                    .get(&global.symbol)
                    .expect("inserted canonical global")
                    .borrow_mut()
                    .replace(value);
            }
            candidate.initialized_globals.insert(global.symbol);
        }
    }
    let delta = HirBlock {
        statements: project.executable.top_level.statements[cursor..].to_vec(),
    };
    if !matches!(executor.execute_hir_block(&delta)?, Flow::Normal) {
        return Err(runtime_error(
            "control flow escaped incremental top level",
            Span::zero(),
        ));
    }
    let output = std::mem::take(&mut executor.output);
    candidate.globals = std::mem::take(&mut executor.globals);
    candidate.top_level_locals = std::mem::take(&mut executor.frames[0].locals);
    candidate.memory = executor.memory;
    candidate.memory_values = executor.memory_values;
    *session = candidate;
    Ok(ConcreteIncrementalExecution {
        output,
        next_cursor: project.executable.top_level.statements.len(),
    })
}

impl ConcreteSession {
    fn deep_clone(&self) -> Self {
        let mut slots = BTreeMap::new();
        Self {
            globals: self
                .globals
                .iter()
                .map(|(id, value)| (*id, clone_slot(value, &mut slots)))
                .collect(),
            top_level_locals: self
                .top_level_locals
                .iter()
                .map(|(id, value)| (*id, clone_slot(value, &mut slots)))
                .collect(),
            initialized_globals: self.initialized_globals.clone(),
            memory: self.memory.clone(),
            memory_values: self
                .memory_values
                .iter()
                .map(|(address, (ty, value))| (*address, (*ty, clone_value(value, &mut slots))))
                .collect(),
        }
    }
}

impl Default for ConcreteSession {
    fn default() -> Self {
        Self {
            globals: BTreeMap::new(),
            top_level_locals: BTreeMap::new(),
            initialized_globals: BTreeSet::new(),
            memory: DeterministicMemory::system(),
            memory_values: BTreeMap::new(),
        }
    }
}

fn clone_slot(source: &Slot, slots: &mut BTreeMap<usize, Slot>) -> Slot {
    let key = Rc::as_ptr(source) as usize;
    if let Some(existing) = slots.get(&key) {
        return existing.clone();
    }
    let destination = slot(None);
    slots.insert(key, destination.clone());
    let value = source
        .borrow()
        .as_ref()
        .map(|value| clone_value(value, slots));
    *destination.borrow_mut() = value;
    destination
}

fn clone_value(value: &ConcreteValue, slots: &mut BTreeMap<usize, Slot>) -> ConcreteValue {
    match value {
        ConcreteValue::Unit => ConcreteValue::Unit,
        ConcreteValue::Integer(value) => ConcreteValue::Integer(*value),
        ConcreteValue::Float(value) => ConcreteValue::Float(*value),
        ConcreteValue::Bool(value) => ConcreteValue::Bool(*value),
        ConcreteValue::String(value) => ConcreteValue::String(value.clone()),
        ConcreteValue::Bytes(value) => ConcreteValue::Bytes(value.clone()),
        ConcreteValue::Buffer(value) => {
            ConcreteValue::Buffer(Rc::new(RefCell::new(value.borrow().clone())))
        }
        ConcreteValue::Null => ConcreteValue::Null,
        ConcreteValue::List(values) => ConcreteValue::List(
            values
                .iter()
                .map(|value| clone_slot(value, slots))
                .collect(),
        ),
        ConcreteValue::Struct { symbol, fields } => ConcreteValue::Struct {
            symbol: *symbol,
            fields: fields
                .iter()
                .map(|(field, value)| (*field, clone_slot(value, slots)))
                .collect(),
        },
        ConcreteValue::Enum {
            symbol,
            variant,
            discriminant,
            payload,
        } => ConcreteValue::Enum {
            symbol: *symbol,
            variant: *variant,
            discriminant: *discriminant,
            payload: payload
                .iter()
                .map(|value| clone_slot(value, slots))
                .collect(),
        },
        ConcreteValue::Function(function) => ConcreteValue::Function(function.clone()),
        ConcreteValue::MemoryPointer { address, pointee } => ConcreteValue::MemoryPointer {
            address: *address,
            pointee: *pointee,
        },
        ConcreteValue::View {
            address,
            length,
            element,
            mutable,
            string,
        } => ConcreteValue::View {
            address: *address,
            length: *length,
            element: *element,
            mutable: *mutable,
            string: *string,
        },
        ConcreteValue::Reference(value) => ConcreteValue::Reference(clone_slot(value, slots)),
        ConcreteValue::Closure(closure) => ConcreteValue::Closure(Rc::new(ConcreteClosure {
            captures: closure
                .captures
                .iter()
                .map(|(local, value)| (*local, clone_slot(value, slots)))
                .collect(),
            parameters: closure.parameters.clone(),
            body: closure.body.clone(),
        })),
        ConcreteValue::EarlyReturn(value) => {
            ConcreteValue::EarlyReturn(Box::new(clone_value(value, slots)))
        }
    }
}

impl Executor<'_> {
    fn initialize_globals(&mut self) -> Result<(), RuntimeError> {
        for global in &self.project.executable.globals {
            self.globals.insert(global.symbol, slot(None));
        }
        for global in &self.project.executable.globals {
            if let Some(initializer) = &global.initializer {
                let value = self.execute_hir_expr(initializer)?;
                let destination = self.globals.get(&global.symbol).cloned().ok_or_else(|| {
                    runtime_error("canonical global slot is missing", global.span)
                })?;
                *destination.borrow_mut() = Some(value);
            }
        }
        Ok(())
    }

    fn execute_concrete_function(
        &mut self,
        function: &ConcreteHirFunction,
        arguments: Vec<ConcreteValue>,
        span: Span,
    ) -> Result<ConcreteValue, RuntimeError> {
        if function.declaration_only {
            return Err(runtime_error(
                "hosted execution cannot enter an external declaration",
                span,
            ));
        }
        if arguments.len() != function.parameters.len() {
            return Err(runtime_error("canonical call arity mismatch", span));
        }
        if self.call_depth >= 16 {
            return Err(runtime_error("maximum call depth exceeded", span));
        }
        let mut locals = BTreeMap::new();
        for local in &function.locals {
            locals.insert(local.id, slot(None));
        }
        for (parameter, value) in function.parameters.iter().zip(arguments) {
            locals
                .entry(parameter.id)
                .or_insert_with(|| slot(None))
                .borrow_mut()
                .replace(value);
        }
        self.frames.push(Frame { locals });
        self.call_depth += 1;
        let result = self.execute_hir_block(&function.body);
        self.call_depth -= 1;
        self.frames.pop();
        match result? {
            Flow::Normal => Ok(ConcreteValue::Unit),
            Flow::Return(value) => Ok(value),
            Flow::Break => Err(runtime_error("break escaped a function", span)),
            Flow::Continue => Err(runtime_error("continue escaped a function", span)),
        }
    }

    fn execute_hir_block(&mut self, block: &HirBlock) -> Result<Flow, RuntimeError> {
        for statement in &block.statements {
            let flow = self.execute_hir_stmt(statement)?;
            if !matches!(flow, Flow::Normal) {
                return Ok(flow);
            }
        }
        Ok(Flow::Normal)
    }

    fn execute_hir_stmt(&mut self, statement: &HirStatement) -> Result<Flow, RuntimeError> {
        match statement {
            HirStatement::Let { local, value, span } => {
                let value = self.execute_hir_expr(value)?;
                if let ConcreteValue::EarlyReturn(value) = value {
                    return Ok(Flow::Return(*value));
                }
                let destination = self.ensure_local_slot(*local, *span)?;
                destination.borrow_mut().replace(value);
                Ok(Flow::Normal)
            }
            HirStatement::Assign {
                place,
                value,
                cleanup_before_commit,
                ..
            } => {
                // Canonical transactional order: RHS first, then cleanup of the
                // old value, then one commit into the resolved place.
                let replacement = self.execute_hir_expr(value)?;
                if let ConcreteValue::EarlyReturn(value) = replacement {
                    return Ok(Flow::Return(*value));
                }
                let memory_destination = self.memory_place(place)?;
                if let Some(cleanup) = cleanup_before_commit {
                    if !matches!(self.execute_hir_block(cleanup)?, Flow::Normal) {
                        return Err(runtime_error(
                            "canonical reassignment cleanup escaped",
                            place.span,
                        ));
                    }
                }
                if let Some((address, ty)) = memory_destination {
                    self.write_memory_value(address, ty, replacement, place.span)?;
                } else {
                    self.execute_hir_place(place)?
                        .borrow_mut()
                        .replace(replacement);
                }
                Ok(Flow::Normal)
            }
            HirStatement::Expression(expression) => {
                if let ConcreteValue::EarlyReturn(value) = self.execute_hir_expr(expression)? {
                    return Ok(Flow::Return(*value));
                }
                Ok(Flow::Normal)
            }
            HirStatement::Return { value, cleanup, .. } => {
                // Result storage is materialized before cleanup begins.
                let result = value
                    .as_ref()
                    .map(|value| self.execute_hir_expr(value))
                    .transpose()?
                    .unwrap_or(ConcreteValue::Unit);
                if let ConcreteValue::EarlyReturn(result) = result {
                    return Ok(Flow::Return(*result));
                }
                self.execute_cleanup_blocks(cleanup)?;
                Ok(Flow::Return(result))
            }
            HirStatement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                let condition_value = self.execute_hir_expr(condition)?;
                if let ConcreteValue::EarlyReturn(value) = condition_value {
                    return Ok(Flow::Return(*value));
                }
                if condition_value.truthy(condition.span)? {
                    self.execute_hir_block(then_block)
                } else if let Some(else_block) = else_block {
                    self.execute_hir_block(else_block)
                } else {
                    Ok(Flow::Normal)
                }
            }
            HirStatement::While {
                condition, body, ..
            } => {
                loop {
                    let condition_value = self.execute_hir_expr(condition)?;
                    if let ConcreteValue::EarlyReturn(value) = condition_value {
                        return Ok(Flow::Return(*value));
                    }
                    if !condition_value.truthy(condition.span)? {
                        break;
                    }
                    match self.execute_hir_block(body)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        returned @ Flow::Return(_) => return Ok(returned),
                    }
                }
                Ok(Flow::Normal)
            }
            HirStatement::For {
                local,
                iterable,
                body,
                ..
            } => {
                let iterable = self.execute_hir_expr(iterable)?;
                if let ConcreteValue::EarlyReturn(value) = iterable {
                    return Ok(Flow::Return(*value));
                }
                let items = match iterable {
                    ConcreteValue::List(items) => items
                        .into_iter()
                        .map(|item| {
                            item.borrow()
                                .as_ref()
                                .cloned()
                                .ok_or_else(|| runtime_error("for item was moved", local.span))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    ConcreteValue::String(bytes) => String::from_utf8(bytes)
                        .map_err(|_| runtime_error("hosted str is not UTF-8", local.span))?
                        .chars()
                        .map(|character| ConcreteValue::String(character.to_string().into_bytes()))
                        .collect(),
                    ConcreteValue::Reference(source) => match source.borrow().as_ref() {
                        Some(ConcreteValue::List(items)) => items
                            .iter()
                            .map(|item| {
                                item.borrow()
                                    .as_ref()
                                    .cloned()
                                    .ok_or_else(|| runtime_error("for item was moved", local.span))
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                        _ => return Err(runtime_error("value is not iterable", local.span)),
                    },
                    _ => return Err(runtime_error("value is not iterable", local.span)),
                };
                let destination = self.ensure_local_slot(local.id, local.span)?;
                let previous = destination.borrow_mut().take();
                for item in items {
                    destination.borrow_mut().replace(item);
                    match self.execute_hir_block(body)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        returned @ Flow::Return(_) => {
                            *destination.borrow_mut() = previous;
                            return Ok(returned);
                        }
                    }
                }
                *destination.borrow_mut() = previous;
                Ok(Flow::Normal)
            }
            HirStatement::Match {
                mode, value, arms, ..
            } => self.execute_match(*mode, value, arms),
            HirStatement::Defer { .. } => {
                // Registration has no runtime action. Every execution edge
                // already carries the exact cleanup blocks it must run.
                Ok(Flow::Normal)
            }
            HirStatement::Break { cleanup, .. } => {
                self.execute_cleanup_blocks(cleanup)?;
                Ok(Flow::Break)
            }
            HirStatement::Continue { cleanup, .. } => {
                self.execute_cleanup_blocks(cleanup)?;
                Ok(Flow::Continue)
            }
        }
    }

    fn execute_match(
        &mut self,
        mode: MatchMode,
        value: &HirExpr,
        arms: &[HirMatchArm],
    ) -> Result<Flow, RuntimeError> {
        let value = self.execute_hir_expr(value)?;
        if let ConcreteValue::EarlyReturn(value) = value {
            return Ok(Flow::Return(*value));
        }
        let ConcreteValue::Enum {
            variant, payload, ..
        } = value
        else {
            return Err(runtime_error(
                "canonical match value is not an enum",
                value_span(value),
            ));
        };
        let arm = arms
            .iter()
            .find(|arm| {
                arm.variant
                    .is_none_or(|candidate| candidate.variant == variant)
            })
            .ok_or_else(|| runtime_error("exhaustive canonical match has no arm", Span::zero()))?;
        let mut restore = Vec::new();
        for (binding, source) in arm.bindings.iter().zip(payload) {
            let destination = self.ensure_local_slot(binding.id, binding.span)?;
            let bound = match mode {
                MatchMode::Move => source.borrow_mut().take().ok_or_else(|| {
                    runtime_error("match payload was already moved", binding.span)
                })?,
                MatchMode::Borrow | MatchMode::BorrowMut => ConcreteValue::Reference(source),
            };
            restore.push((destination.clone(), destination.borrow_mut().replace(bound)));
        }
        let result = self.execute_hir_block(&arm.body);
        for (destination, previous) in restore.into_iter().rev() {
            *destination.borrow_mut() = previous;
        }
        result
    }

    fn execute_cleanup_blocks(&mut self, cleanup: &[HirBlock]) -> Result<(), RuntimeError> {
        for block in cleanup {
            if !matches!(self.execute_hir_block(block)?, Flow::Normal) {
                return Err(runtime_error(
                    "canonical cleanup block attempted control flow",
                    Span::zero(),
                ));
            }
        }
        Ok(())
    }

    fn execute_hir_expr(&mut self, expression: &HirExpr) -> Result<ConcreteValue, RuntimeError> {
        match &expression.kind {
            HirExprKind::Unit => Ok(ConcreteValue::Unit),
            HirExprKind::Integer(value) => Ok(ConcreteValue::Integer(*value)),
            HirExprKind::LayoutOf { target, alignment } => {
                let layout = layout_of(&self.project.executable.semantic, *target)
                    .map_err(|error| runtime_error(&error.to_string(), expression.span))?;
                Ok(ConcreteValue::Integer(i64::from(if *alignment {
                    layout.align
                } else {
                    layout.size
                })))
            }
            HirExprKind::Length { value } => match self.execute_hir_expr(value)? {
                ConcreteValue::String(bytes) | ConcreteValue::Bytes(bytes) => {
                    Ok(ConcreteValue::Integer(bytes.len() as i64))
                }
                ConcreteValue::List(values) => Ok(ConcreteValue::Integer(values.len() as i64)),
                ConcreteValue::Buffer(bytes) => {
                    Ok(ConcreteValue::Integer(bytes.borrow().len() as i64))
                }
                ConcreteValue::View { length, .. } => Ok(ConcreteValue::Integer(length as i64)),
                ConcreteValue::Reference(slot) => match slot.borrow().as_ref() {
                    Some(ConcreteValue::String(bytes)) | Some(ConcreteValue::Bytes(bytes)) => {
                        Ok(ConcreteValue::Integer(bytes.len() as i64))
                    }
                    Some(ConcreteValue::List(values)) => {
                        Ok(ConcreteValue::Integer(values.len() as i64))
                    }
                    Some(ConcreteValue::Buffer(bytes)) => {
                        Ok(ConcreteValue::Integer(bytes.borrow().len() as i64))
                    }
                    _ => Err(runtime_error(
                        "len operand is not a bounded value",
                        expression.span,
                    )),
                },
                _ => Err(runtime_error(
                    "len operand is not a bounded value",
                    expression.span,
                )),
            },
            HirExprKind::Float(value) => Ok(ConcreteValue::Float(*value)),
            HirExprKind::Bool(value) => Ok(ConcreteValue::Bool(*value)),
            HirExprKind::String(value) => Ok(ConcreteValue::String(value.clone())),
            HirExprKind::Bytes(value) => Ok(ConcreteValue::Bytes(value.clone())),
            HirExprKind::Null => Ok(ConcreteValue::Null),
            HirExprKind::Local(local) => {
                let source = self.local_slot(*local, expression.span)?;
                read_slot(&source, expression.ownership, expression.span)
            }
            HirExprKind::Global(symbol) => {
                let source = self.global_slot(*symbol, expression.span)?;
                read_slot(&source, expression.ownership, expression.span)
            }
            HirExprKind::Constant { value, .. } => self.execute_hir_expr(value),
            HirExprKind::Function(key) => Ok(ConcreteValue::Function(key.clone())),
            HirExprKind::List(values) => values
                .iter()
                .map(|value| self.execute_hir_expr(value).map(|value| slot(Some(value))))
                .collect::<Result<Vec<_>, _>>()
                .map(ConcreteValue::List),
            HirExprKind::Binary { left, op, right } => {
                let left = self.execute_hir_expr(left)?;
                let right = self.execute_hir_expr(right)?;
                binary(left, op, right, expression.span)
            }
            HirExprKind::Unary { op, value } => {
                let value = self.execute_hir_expr(value)?;
                unary(op, value, expression.span)
            }
            HirExprKind::Cast { value, target } => {
                let value = self.execute_hir_expr(value)?;
                cast(
                    value,
                    self.project.executable.semantic.type_of(*target),
                    expression.span,
                )
            }
            HirExprKind::DirectCall {
                callee, arguments, ..
            } => {
                let arguments = self.arguments(arguments)?;
                self.call(callee, arguments, expression.span)
            }
            HirExprKind::IndirectCall {
                callee, arguments, ..
            } => {
                let callee = self.execute_hir_expr(callee)?;
                let arguments = self.arguments(arguments)?;
                match callee {
                    ConcreteValue::Function(key) => self.call(&key, arguments, expression.span),
                    ConcreteValue::Closure(closure) => {
                        self.call_closure(&closure, arguments, expression.span)
                    }
                    _ => Err(runtime_error(
                        "indirect callee is not callable",
                        expression.span,
                    )),
                }
            }
            HirExprKind::IntrinsicCall {
                intrinsic,
                arguments,
            } if intrinsic.is_runtime_host_memory() => {
                self.execute_runtime_memory(*intrinsic, arguments, expression.ty, expression.span)
            }
            HirExprKind::IntrinsicCall {
                intrinsic,
                arguments,
            } if intrinsic.is_runtime_view() => {
                self.execute_runtime_view(*intrinsic, arguments, expression.ty, expression.span)
            }
            HirExprKind::IntrinsicCall { .. }
            | HirExprKind::InterruptAddress { .. }
            | HirExprKind::TaskAddress { .. } => Err(runtime_error(
                "target intrinsic is unavailable in hosted Concrete HIR execution",
                expression.span,
            )),
            HirExprKind::HostCall {
                capability,
                arguments,
            } => self.execute_host_call(*capability, arguments, expression.span),
            HirExprKind::FormatString(parts) => {
                let mut bytes = Vec::new();
                for part in parts {
                    match part {
                        HirFormatPart::Literal(literal) => bytes.extend_from_slice(literal),
                        HirFormatPart::Value(value) => bytes
                            .extend_from_slice(self.execute_hir_expr(value)?.display().as_bytes()),
                    }
                }
                Ok(ConcreteValue::String(bytes))
            }
            HirExprKind::Struct { symbol, fields } => {
                let mut result = BTreeMap::new();
                for (field, value) in fields {
                    result.insert(*field, slot(Some(self.execute_hir_expr(value)?)));
                }
                Ok(ConcreteValue::Struct {
                    symbol: *symbol,
                    fields: result,
                })
            }
            HirExprKind::Enum {
                symbol,
                variant,
                discriminant,
                payload,
            } => Ok(ConcreteValue::Enum {
                symbol: *symbol,
                variant: *variant,
                discriminant: *discriminant,
                payload: payload
                    .iter()
                    .map(|value| self.execute_hir_expr(value).map(|value| slot(Some(value))))
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            HirExprKind::Place(place) => {
                if let Some((address, ty)) = self.memory_place(place)? {
                    self.read_memory_value(address, ty, expression.ownership, expression.span)
                } else {
                    let source = self.execute_hir_place(place)?;
                    read_slot(&source, expression.ownership, expression.span)
                }
            }
            HirExprKind::Borrow { place, .. } => match self.memory_place(place)? {
                Some((address, ty)) => Ok(ConcreteValue::MemoryPointer {
                    address,
                    pointee: ty,
                }),
                None => Ok(ConcreteValue::Reference(self.execute_hir_place(place)?)),
            },
            HirExprKind::Try {
                value,
                ok_variant,
                err_variant,
                cleanup,
            } => {
                let result = self.execute_hir_expr(value)?;
                let ConcreteValue::Enum {
                    variant, payload, ..
                } = result
                else {
                    return Err(runtime_error("? operand is not Result", expression.span));
                };
                if variant == *ok_variant {
                    payload
                        .first()
                        .map(|value| {
                            value.borrow_mut().take().ok_or_else(|| {
                                runtime_error("Result.Ok payload was moved", expression.span)
                            })
                        })
                        .transpose()
                        .map(|value| value.unwrap_or(ConcreteValue::Unit))
                } else if variant == *err_variant {
                    self.execute_cleanup_blocks(cleanup)?;
                    // The HIR has selected the exact Err identity and return
                    // type; preserve that aggregate without reconstruction.
                    Ok(ConcreteValue::EarlyReturn(Box::new(ConcreteValue::Enum {
                        symbol: match self.project.executable.semantic.type_of(value.ty) {
                            CanonicalType::Enum { symbol, .. } => *symbol,
                            _ => {
                                return Err(runtime_error(
                                    "Result type is not enum",
                                    expression.span,
                                ))
                            }
                        },
                        variant,
                        discriminant: self.variant_discriminant(
                            value.ty,
                            variant,
                            expression.span,
                        )?,
                        payload,
                    })))
                } else {
                    Err(runtime_error(
                        "Result contains an invalid tag",
                        expression.span,
                    ))
                }
            }
            HirExprKind::Drop { place, cleanup } => {
                self.execute_hir_block(cleanup)?;
                if let Some((address, _)) = self.memory_place(place)? {
                    self.memory_values.remove(&address);
                } else {
                    self.execute_hir_place(place)?.borrow_mut().take();
                }
                Ok(ConcreteValue::Unit)
            }
            HirExprKind::Closure {
                captures,
                parameters,
                body,
                ..
            } => {
                let mut captured = BTreeMap::new();
                for capture in captures {
                    let source = self.local_slot(capture.local, capture.span)?;
                    let stored = match capture.mode {
                        CaptureMode::Move => {
                            slot(Some(source.borrow_mut().take().ok_or_else(|| {
                                runtime_error("closure capture uses moved value", capture.span)
                            })?))
                        }
                        CaptureMode::Borrow | CaptureMode::BorrowMut => source,
                    };
                    captured.insert(capture.local, stored);
                }
                Ok(ConcreteValue::Closure(Rc::new(ConcreteClosure {
                    captures: captured,
                    parameters: parameters.clone(),
                    body: body.clone(),
                })))
            }
        }
    }

    fn execute_runtime_memory(
        &mut self,
        intrinsic: HirIntrinsic,
        arguments: &[HirExpr],
        result_ty: TypeId,
        span: Span,
    ) -> Result<ConcreteValue, RuntimeError> {
        let values = self.arguments(arguments)?;
        let integer = |value: &ConcreteValue| match value {
            ConcreteValue::Integer(value) if *value >= 0 => Ok(*value as u64),
            ConcreteValue::MemoryPointer { address, .. } => Ok(*address),
            ConcreteValue::Null => Ok(0),
            _ => Err(runtime_error(
                "runtime memory argument is not an address or usize",
                span,
            )),
        };
        match intrinsic {
            HirIntrinsic::RuntimeMemoryAlloc => {
                let size = integer(&values[0])?;
                let align = integer(&values[1])?;
                let pointee = match self.project.executable.semantic.type_of(result_ty) {
                    CanonicalType::Pointer { pointee, .. } => *pointee,
                    _ => {
                        return Err(runtime_error(
                            "runtime allocation result is not a pointer",
                            span,
                        ))
                    }
                };
                Ok(self
                    .memory
                    .alloc(size, align)
                    .map_or(ConcreteValue::Null, |address| {
                        ConcreteValue::MemoryPointer { address, pointee }
                    }))
            }
            HirIntrinsic::RuntimeMemoryResize => {
                let address = integer(&values[0])?;
                let old_size = integer(&values[1])?;
                let new_size = integer(&values[2])?;
                let align = integer(&values[3])?;
                let pointee = match self.project.executable.semantic.type_of(result_ty) {
                    CanonicalType::Pointer { pointee, .. } => *pointee,
                    _ => {
                        return Err(runtime_error(
                            "runtime resize result is not a pointer",
                            span,
                        ))
                    }
                };
                match self.memory.resize(address, old_size, new_size, align) {
                    Ok(replacement) => {
                        self.relocate_memory_values(address, old_size, replacement, new_size);
                        Ok(ConcreteValue::MemoryPointer {
                            address: replacement,
                            pointee,
                        })
                    }
                    Err(_) => Ok(ConcreteValue::Null),
                }
            }
            HirIntrinsic::RuntimeMemoryFree => {
                let address = integer(&values[0])?;
                let size = integer(&values[1])?;
                let align = integer(&values[2])?;
                let released = self.memory.free(address, size, align).is_ok();
                if released {
                    self.remove_memory_values(address, size);
                }
                Ok(ConcreteValue::Bool(released))
            }
            HirIntrinsic::RuntimeMemoryRead8 => {
                let address = integer(&values[0])?;
                let offset = integer(&values[1])?;
                let bytes = self.memory.read(address, offset, 1).map_err(|error| {
                    runtime_error(&format!("runtime memory read failed: {error:?}"), span)
                })?;
                Ok(ConcreteValue::Integer(i64::from(bytes[0])))
            }
            HirIntrinsic::RuntimeMemoryWrite8 => {
                let address = integer(&values[0])?;
                let offset = integer(&values[1])?;
                let byte = integer(&values[2])?;
                let byte = u8::try_from(byte)
                    .map_err(|_| runtime_error("runtime memory byte is out of range", span))?;
                self.memory
                    .write(address, offset, &[byte])
                    .map_err(|error| {
                        runtime_error(&format!("runtime memory write failed: {error:?}"), span)
                    })?;
                Ok(ConcreteValue::Unit)
            }
            _ => Err(runtime_error("unsupported runtime memory capability", span)),
        }
    }

    fn execute_runtime_view(
        &mut self,
        intrinsic: HirIntrinsic,
        arguments: &[HirExpr],
        result_ty: TypeId,
        span: Span,
    ) -> Result<ConcreteValue, RuntimeError> {
        let values = self.arguments(arguments)?;
        let ConcreteValue::MemoryPointer { address, .. } = values[0] else {
            return Err(runtime_error(
                "runtime view data is not a memory pointer",
                span,
            ));
        };
        let length = values[1].integer(span)?;
        let length = u64::try_from(length)
            .map_err(|_| runtime_error("runtime view length is negative", span))?;
        let (element, mutable, string) = match self.project.executable.semantic.type_of(result_ty) {
            CanonicalType::Slice { element, mutable } => (*element, *mutable, false),
            CanonicalType::StringView => {
                let element = match self.project.executable.semantic.type_of(arguments[0].ty) {
                    CanonicalType::Pointer { pointee, .. } => *pointee,
                    _ => return Err(runtime_error("StringView data is not ptr[u8]", span)),
                };
                (element, false, true)
            }
            _ => {
                return Err(runtime_error(
                    "runtime view result has the wrong type",
                    span,
                ))
            }
        };
        debug_assert!(intrinsic.is_runtime_view());
        Ok(ConcreteValue::View {
            address,
            length,
            element,
            mutable,
            string,
        })
    }

    fn execute_hir_place(&mut self, place: &HirPlace) -> Result<Slot, RuntimeError> {
        let mut current = match &place.base {
            HirPlaceBase::Local(local) => self.local_slot(*local, place.span)?,
            HirPlaceBase::Global(symbol) => self.global_slot(*symbol, place.span)?,
            HirPlaceBase::Pointer(pointer) => match self.execute_hir_expr(pointer)? {
                ConcreteValue::Reference(slot) => dereference_slot(slot),
                _ => {
                    return Err(runtime_error(
                        "canonical pointer place is invalid",
                        place.span,
                    ))
                }
            },
        };
        for projection in &place.projections {
            current = match projection {
                HirProjection::Field { field, .. } => {
                    let mut value = current.borrow_mut();
                    if value.is_none() {
                        *value = Some(ConcreteValue::Struct {
                            symbol: SymbolId(u32::MAX),
                            fields: BTreeMap::new(),
                        });
                    }
                    match value.as_mut() {
                        Some(ConcreteValue::Struct { fields, .. }) => {
                            fields.entry(*field).or_insert_with(|| slot(None)).clone()
                        }
                        _ => return Err(runtime_error("field place is not a struct", place.span)),
                    }
                }
                HirProjection::Payload { variant, index, .. } => match current.borrow().as_ref() {
                    Some(ConcreteValue::Enum {
                        variant: active,
                        payload,
                        ..
                    }) if active == variant => {
                        payload.get(*index as usize).cloned().ok_or_else(|| {
                            runtime_error("canonical payload slot is missing", place.span)
                        })?
                    }
                    Some(ConcreteValue::Enum { .. }) => {
                        return Err(runtime_error("inactive enum payload access", place.span))
                    }
                    _ => return Err(runtime_error("payload place is not an enum", place.span)),
                },
                HirProjection::Index { index, .. } => {
                    let index_span = index.span;
                    let raw_index = self.execute_hir_expr(index)?.integer(index_span)?;
                    let length = match current.borrow().as_ref() {
                        Some(ConcreteValue::List(values)) => values.len(),
                        Some(ConcreteValue::String(bytes) | ConcreteValue::Bytes(bytes)) => {
                            bytes.len()
                        }
                        _ => 0,
                    };
                    let normalized = if raw_index < 0 {
                        (length as i64).checked_add(raw_index)
                    } else {
                        Some(raw_index)
                    };
                    let index = normalized
                        .and_then(|value| usize::try_from(value).ok())
                        .filter(|value| *value < length)
                        .ok_or_else(|| runtime_error("index out of range", index_span))?;
                    match current.borrow().as_ref() {
                        Some(ConcreteValue::List(values)) => values
                            .get(index)
                            .cloned()
                            .ok_or_else(|| runtime_error("index out of range", place.span))?,
                        Some(ConcreteValue::String(bytes) | ConcreteValue::Bytes(bytes)) => {
                            let byte = bytes
                                .get(index)
                                .copied()
                                .ok_or_else(|| runtime_error("index out of range", place.span))?;
                            slot(Some(ConcreteValue::Integer(i64::from(byte))))
                        }
                        _ => return Err(runtime_error("indexed place is not a list", place.span)),
                    }
                }
            };
        }
        Ok(current)
    }

    fn memory_place(&mut self, place: &HirPlace) -> Result<Option<(u64, TypeId)>, RuntimeError> {
        let base = match &place.base {
            HirPlaceBase::Pointer(pointer) => self.execute_hir_expr(pointer)?,
            HirPlaceBase::Local(local) => self
                .local_slot(*local, place.span)?
                .borrow()
                .as_ref()
                .cloned()
                .unwrap_or(ConcreteValue::Unit),
            HirPlaceBase::Global(symbol) => self
                .global_slot(*symbol, place.span)?
                .borrow()
                .as_ref()
                .cloned()
                .unwrap_or(ConcreteValue::Unit),
        };
        let (mut address, mut pointee, view_length) = match base {
            ConcreteValue::MemoryPointer { address, pointee } => (address, pointee, None),
            ConcreteValue::View {
                address,
                length,
                element,
                ..
            } => (address, element, Some(length)),
            _ => return Ok(None),
        };
        for projection in &place.projections {
            match projection {
                HirProjection::Field { ty, offset, .. }
                | HirProjection::Payload { ty, offset, .. } => {
                    address = address.checked_add(u64::from(*offset)).ok_or_else(|| {
                        runtime_error("runtime memory address overflow", place.span)
                    })?;
                    pointee = *ty;
                }
                HirProjection::Index {
                    index,
                    element,
                    element_size,
                } => {
                    let index_span = index.span;
                    let index = self.execute_hir_expr(index)?.integer(index_span)?;
                    let index = u64::try_from(index)
                        .map_err(|_| runtime_error("negative runtime memory index", index_span))?;
                    if view_length.is_some_and(|length| index >= length) {
                        return Err(runtime_error("view index out of bounds", index_span));
                    }
                    let offset = index.checked_mul(u64::from(*element_size)).ok_or_else(|| {
                        runtime_error("runtime memory index overflow", index_span)
                    })?;
                    address = address.checked_add(offset).ok_or_else(|| {
                        runtime_error("runtime memory address overflow", index_span)
                    })?;
                    pointee = *element;
                }
            }
        }
        Ok(Some((address, pointee)))
    }

    fn read_memory_value(
        &mut self,
        address: u64,
        ty: TypeId,
        action: OwnershipAction,
        span: Span,
    ) -> Result<ConcreteValue, RuntimeError> {
        if self
            .memory_values
            .get(&address)
            .is_some_and(|(stored_ty, _)| *stored_ty == ty)
        {
            let (stored_ty, value) = if matches!(action, OwnershipAction::Move) {
                self.memory_values
                    .remove(&address)
                    .expect("checked aggregate memory value")
            } else {
                let (stored_ty, value) = self
                    .memory_values
                    .get(&address)
                    .expect("checked aggregate memory value");
                let mut slots = BTreeMap::new();
                (*stored_ty, clone_value(value, &mut slots))
            };
            if stored_ty != ty {
                return Err(runtime_error(
                    &format!(
                        "runtime memory aggregate type mismatch: stored {:?}, requested {:?}",
                        stored_ty, ty
                    ),
                    span,
                ));
            }
            return Ok(value);
        }
        let canonical = self.project.executable.semantic.type_of(ty);
        let size = layout_of(&self.project.executable.semantic, ty)
            .map_err(|failure| runtime_error(&failure.to_string(), span))?
            .size;
        let bytes = self
            .memory
            .read_address(address, u64::from(size))
            .map_err(|failure| {
                runtime_error(&format!("runtime memory read failed: {failure:?}"), span)
            })?;
        let mut raw = 0_u64;
        for (index, byte) in bytes.iter().take(8).enumerate() {
            raw |= u64::from(*byte) << (index * 8);
        }
        match canonical {
            CanonicalType::Integer(integer) => {
                let signed = matches!(
                    integer,
                    IntegerType::I8 | IntegerType::I16 | IntegerType::I32 | IntegerType::LegacyInt
                );
                let bits = size * 8;
                let value = if signed && bits < 64 {
                    ((raw << (64 - bits)) as i64) >> (64 - bits)
                } else {
                    raw as i64
                };
                Ok(ConcreteValue::Integer(value))
            }
            CanonicalType::Bool => Ok(ConcreteValue::Bool(raw != 0)),
            CanonicalType::Pointer { pointee, .. } => Ok(if raw == 0 {
                ConcreteValue::Null
            } else {
                ConcreteValue::MemoryPointer {
                    address: raw,
                    pointee: *pointee,
                }
            }),
            CanonicalType::Struct { symbol, .. } => {
                let definition = self
                    .project
                    .executable
                    .semantic
                    .definitions
                    .iter()
                    .find(|definition| definition.symbol == *symbol)
                    .ok_or_else(|| runtime_error("runtime struct definition is missing", span))?;
                let TypeDefKind::Struct { fields, .. } = &definition.kind else {
                    unreachable!()
                };
                let fields = fields.clone();
                let mut values = BTreeMap::new();
                for field in fields {
                    let (field_ty, offset) =
                        struct_field_layout(&self.project.executable.semantic, ty, field.id)
                            .map_err(|failure| runtime_error(&failure.to_string(), span))?;
                    let value = self.read_memory_value(
                        address + u64::from(offset),
                        field_ty,
                        action,
                        span,
                    )?;
                    values.insert(field.id, slot(Some(value)));
                }
                Ok(ConcreteValue::Struct {
                    symbol: *symbol,
                    fields: values,
                })
            }
            CanonicalType::Enum { symbol, .. } => {
                let discriminant = raw as u32;
                let definition = self
                    .project
                    .executable
                    .semantic
                    .definitions
                    .iter()
                    .find(|definition| definition.symbol == *symbol)
                    .ok_or_else(|| runtime_error("runtime enum definition is missing", span))?;
                let TypeDefKind::Enum { variants } = &definition.kind else {
                    unreachable!()
                };
                let variant = variants
                    .iter()
                    .find(|variant| variant.discriminant == discriminant)
                    .ok_or_else(|| runtime_error("runtime enum tag is invalid", span))?;
                let variant_id = variant.id;
                let (_, _, payload_layout) =
                    enum_variant_layout(&self.project.executable.semantic, ty, variant_id)
                        .map_err(|failure| runtime_error(&failure.to_string(), span))?;
                let mut payload = Vec::new();
                for (payload_ty, offset) in payload_layout {
                    payload.push(slot(Some(self.read_memory_value(
                        address + u64::from(offset),
                        payload_ty,
                        action,
                        span,
                    )?)));
                }
                Ok(ConcreteValue::Enum {
                    symbol: *symbol,
                    variant: variant_id,
                    discriminant,
                    payload,
                })
            }
            _ => Err(runtime_error(
                "runtime memory value is not representable",
                span,
            )),
        }
    }

    fn write_memory_value(
        &mut self,
        address: u64,
        ty: TypeId,
        value: ConcreteValue,
        span: Span,
    ) -> Result<(), RuntimeError> {
        let size = layout_of(&self.project.executable.semantic, ty)
            .map_err(|failure| runtime_error(&failure.to_string(), span))?
            .size as usize;
        let raw = match value {
            ConcreteValue::Integer(value) => value as u64,
            ConcreteValue::Bool(value) => u64::from(value),
            ConcreteValue::MemoryPointer { address, .. } => address,
            ConcreteValue::Null => 0,
            ConcreteValue::Struct { symbol, fields } => {
                let CanonicalType::Struct {
                    symbol: expected, ..
                } = self.project.executable.semantic.type_of(ty)
                else {
                    return Err(runtime_error("runtime aggregate store type mismatch", span));
                };
                if symbol != *expected {
                    return Err(runtime_error("runtime struct symbol mismatch", span));
                }
                self.remove_memory_values(address, size as u64);
                for (field, value) in fields {
                    let (field_ty, offset) =
                        struct_field_layout(&self.project.executable.semantic, ty, field)
                            .map_err(|failure| runtime_error(&failure.to_string(), span))?;
                    let value = value
                        .borrow_mut()
                        .take()
                        .ok_or_else(|| runtime_error("runtime struct field was moved", span))?;
                    self.write_memory_value(address + u64::from(offset), field_ty, value, span)?;
                }
                return Ok(());
            }
            value => {
                self.remove_memory_values(address, size as u64);
                self.memory_values.insert(address, (ty, value));
                return Ok(());
            }
        };
        let bytes = raw.to_le_bytes();
        self.memory
            .write_address(address, &bytes[..size])
            .map_err(|failure| {
                runtime_error(&format!("runtime memory write failed: {failure:?}"), span)
            })
    }

    fn remove_memory_values(&mut self, address: u64, size: u64) {
        let end = address.saturating_add(size);
        self.memory_values
            .retain(|stored, _| *stored < address || *stored >= end);
    }

    fn relocate_memory_values(
        &mut self,
        old_address: u64,
        old_size: u64,
        new_address: u64,
        new_size: u64,
    ) {
        let old_end = old_address.saturating_add(old_size);
        let mut moved = Vec::new();
        self.memory_values.retain(|stored, value| {
            if *stored >= old_address && *stored < old_end {
                let offset = *stored - old_address;
                if offset < new_size {
                    moved.push((new_address.saturating_add(offset), value.clone()));
                }
                false
            } else {
                true
            }
        });
        self.memory_values.extend(moved);
    }

    fn arguments(&mut self, expressions: &[HirExpr]) -> Result<Vec<ConcreteValue>, RuntimeError> {
        expressions
            .iter()
            .map(|argument| self.execute_hir_expr(argument))
            .collect()
    }

    fn call(
        &mut self,
        key: &FunctionInstanceKey,
        arguments: Vec<ConcreteValue>,
        span: Span,
    ) -> Result<ConcreteValue, RuntimeError> {
        let function = self
            .project
            .executable
            .functions
            .iter()
            .find(|function| function.key == *key)
            .cloned()
            .ok_or_else(|| runtime_error("canonical function instance is missing", span))?;
        self.execute_concrete_function(&function, arguments, span)
    }

    fn call_closure(
        &mut self,
        closure: &ConcreteClosure,
        arguments: Vec<ConcreteValue>,
        span: Span,
    ) -> Result<ConcreteValue, RuntimeError> {
        if arguments.len() != closure.parameters.len() {
            return Err(runtime_error("closure call arity mismatch", span));
        }
        let mut locals = closure.captures.clone();
        for (parameter, value) in closure.parameters.iter().zip(arguments) {
            locals.insert(parameter.id, slot(Some(value)));
        }
        self.frames.push(Frame { locals });
        let result = self.execute_hir_block(&closure.body);
        self.frames.pop();
        match result? {
            Flow::Normal => Ok(ConcreteValue::Unit),
            Flow::Return(value) => Ok(value),
            Flow::Break | Flow::Continue => {
                Err(runtime_error("loop control escaped a closure", span))
            }
        }
    }

    fn execute_host_call(
        &mut self,
        capability: HostCapabilityId,
        arguments: &[HirExpr],
        span: Span,
    ) -> Result<ConcreteValue, RuntimeError> {
        match capability {
            HostCapabilityId::Print => {
                let values = self.arguments(arguments)?;
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        self.output.push(' ');
                    }
                    self.output.push_str(&value.display());
                }
                self.output.push('\n');
                Ok(ConcreteValue::Unit)
            }
            HostCapabilityId::EnvArgs => Ok(ConcreteValue::List(
                self.script_args
                    .iter()
                    .map(|argument| slot(Some(ConcreteValue::String(argument.as_bytes().to_vec()))))
                    .collect(),
            )),
            capability => {
                let function: fn(&[crate::Value]) -> Result<crate::Value, String> = match capability
                {
                    HostCapabilityId::MathAbs => super::native_math_abs,
                    HostCapabilityId::MathMin => super::native_math_min,
                    HostCapabilityId::MathMax => super::native_math_max,
                    HostCapabilityId::FsReadText => super::native_fs_read_text,
                    HostCapabilityId::FsWriteText => super::native_fs_write_text,
                    HostCapabilityId::FsReadBytes => super::native_fs_read_bytes,
                    HostCapabilityId::FsWriteBytes => super::native_fs_write_bytes,
                    HostCapabilityId::FsExists => super::native_fs_exists,
                    HostCapabilityId::FsMkdir => super::native_fs_mkdir,
                    HostCapabilityId::FsRemove => super::native_fs_remove,
                    HostCapabilityId::EncodingHexEncode => super::native_encoding_hex_encode,
                    HostCapabilityId::EncodingHexDecode => super::native_encoding_hex_decode,
                    HostCapabilityId::HashSha256 => super::native_hash_sha256,
                    HostCapabilityId::ProcessRun => super::native_process_run,
                    HostCapabilityId::BufferNew => super::native_buffer_new,
                    HostCapabilityId::BufferFromBytes => super::native_buffer_from_bytes,
                    HostCapabilityId::BufferToBytes => super::native_buffer_to_bytes,
                    HostCapabilityId::BufferLen => super::native_buffer_len,
                    HostCapabilityId::BufferGet => super::native_buffer_get,
                    HostCapabilityId::BufferSet => super::native_buffer_set,
                    HostCapabilityId::BufferSlice => super::native_buffer_slice,
                    HostCapabilityId::BufferLoad => super::native_buffer_load,
                    HostCapabilityId::BufferSave => super::native_buffer_save,
                    HostCapabilityId::BufferFind => super::native_buffer_find,
                    HostCapabilityId::BufferReplace => super::native_buffer_replace,
                    HostCapabilityId::BinaryU8 => super::native_binary_u8,
                    HostCapabilityId::BinaryI8 => super::native_binary_i8,
                    HostCapabilityId::BinaryU16Le => super::native_binary_u16_le,
                    HostCapabilityId::BinaryU16Be => super::native_binary_u16_be,
                    HostCapabilityId::BinaryI16Le => super::native_binary_i16_le,
                    HostCapabilityId::BinaryI16Be => super::native_binary_i16_be,
                    HostCapabilityId::BinaryU32Le => super::native_binary_u32_le,
                    HostCapabilityId::BinaryU32Be => super::native_binary_u32_be,
                    HostCapabilityId::BinaryI32Le => super::native_binary_i32_le,
                    HostCapabilityId::BinaryI32Be => super::native_binary_i32_be,
                    HostCapabilityId::BinaryPackU16Le => super::native_binary_pack_u16_le,
                    HostCapabilityId::BinaryPackU16Be => super::native_binary_pack_u16_be,
                    HostCapabilityId::BinaryPackU32Le => super::native_binary_pack_u32_le,
                    HostCapabilityId::BinaryPackU32Be => super::native_binary_pack_u32_be,
                    HostCapabilityId::BinaryWriteU16Le => super::native_binary_write_u16_le,
                    HostCapabilityId::BinaryWriteU16Be => super::native_binary_write_u16_be,
                    HostCapabilityId::BinaryWriteU32Le => super::native_binary_write_u32_le,
                    HostCapabilityId::BinaryWriteU32Be => super::native_binary_write_u32_be,
                    HostCapabilityId::Print | HostCapabilityId::EnvArgs => unreachable!(),
                };
                let arguments = self.arguments(arguments)?;
                let arguments = arguments
                    .into_iter()
                    .map(to_legacy_value)
                    .collect::<Result<Vec<_>, _>>()?;
                function(&arguments)
                    .map_err(|message| runtime_error(&message, span))
                    .and_then(from_legacy_value)
            }
        }
        .map_err(|mut error: RuntimeError| {
            if error.span == Span::zero() {
                error.span = span;
            }
            error
        })
    }

    fn local_slot(&self, local: LocalId, span: Span) -> Result<Slot, RuntimeError> {
        self.frames
            .last()
            .and_then(|frame| frame.locals.get(&local))
            .cloned()
            .ok_or_else(|| runtime_error("canonical local slot is missing", span))
    }

    fn ensure_local_slot(&mut self, local: LocalId, span: Span) -> Result<Slot, RuntimeError> {
        let frame = self
            .frames
            .last_mut()
            .ok_or_else(|| runtime_error("canonical execution frame is missing", span))?;
        Ok(frame
            .locals
            .entry(local)
            .or_insert_with(|| slot(None))
            .clone())
    }

    fn global_slot(&self, symbol: SymbolId, span: Span) -> Result<Slot, RuntimeError> {
        self.globals
            .get(&symbol)
            .cloned()
            .ok_or_else(|| runtime_error("canonical global slot is missing", span))
    }

    fn variant_discriminant(
        &self,
        ty: TypeId,
        variant: VariantId,
        span: Span,
    ) -> Result<u32, RuntimeError> {
        let CanonicalType::Enum { symbol, .. } = self.project.executable.semantic.type_of(ty)
        else {
            return Err(runtime_error("variant owner is not enum", span));
        };
        self.project
            .executable
            .semantic
            .definitions
            .iter()
            .find(|definition| definition.symbol == *symbol)
            .and_then(|definition| match &definition.kind {
                dbyte_hir::TypeDefKind::Enum { variants } => variants
                    .iter()
                    .find(|candidate| candidate.id == variant)
                    .map(|candidate| candidate.discriminant),
                _ => None,
            })
            .ok_or_else(|| runtime_error("canonical variant is missing", span))
    }
}

impl ConcreteValue {
    fn display(&self) -> String {
        match self {
            Self::Unit => String::new(),
            Self::Integer(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::Bool(value) => value.to_string(),
            Self::String(value) => String::from_utf8_lossy(value).into_owned(),
            Self::Bytes(value) => hex::encode(value),
            Self::Buffer(_) => "<buffer>".into(),
            Self::Null => "null".into(),
            Self::List(values) => {
                let values = values
                    .iter()
                    .map(|value| {
                        value
                            .borrow()
                            .as_ref()
                            .map_or_else(|| "<moved>".into(), ConcreteValue::display)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[{values}]")
            }
            Self::Struct { symbol, .. } => format!("<struct s{}>", symbol.0),
            Self::Enum {
                symbol, variant, ..
            } => format!("<enum s{} v{}>", symbol.0, variant.0),
            Self::Function(key) => format!("<fn s{}>", key.symbol.0),
            Self::MemoryPointer { address, .. } => format!("0x{address:08x}"),
            Self::View {
                length,
                mutable,
                string,
                ..
            } => {
                if *string {
                    format!("<StringView {length} bytes>")
                } else if *mutable {
                    format!("<mut_slice {length} elements>")
                } else {
                    format!("<slice {length} elements>")
                }
            }
            Self::Reference(_) => "<borrow>".into(),
            Self::Closure(_) => "<closure>".into(),
            Self::EarlyReturn(value) => value.display(),
        }
    }

    fn truthy(&self, span: Span) -> Result<bool, RuntimeError> {
        match self {
            Self::Bool(value) => Ok(*value),
            Self::Integer(value) => Ok(*value != 0),
            _ => Err(runtime_error("condition is not bool", span)),
        }
    }

    fn integer(&self, span: Span) -> Result<i64, RuntimeError> {
        match self {
            Self::Integer(value) => Ok(*value),
            Self::MemoryPointer { address, .. } => i64::try_from(*address)
                .map_err(|_| runtime_error("pointer does not fit hosted integer", span)),
            _ => Err(runtime_error("expected integer value", span)),
        }
    }
}

fn slot(value: Option<ConcreteValue>) -> Slot {
    Rc::new(RefCell::new(value))
}

fn dereference_slot(mut current: Slot) -> Slot {
    loop {
        let next = match current.borrow().as_ref() {
            Some(ConcreteValue::Reference(next)) => Some(next.clone()),
            _ => None,
        };
        match next {
            Some(next) => current = next,
            None => return current,
        }
    }
}

fn read_slot(
    source: &Slot,
    action: OwnershipAction,
    span: Span,
) -> Result<ConcreteValue, RuntimeError> {
    match action {
        OwnershipAction::Move => source
            .borrow_mut()
            .take()
            .ok_or_else(|| runtime_error("use after move", span)),
        OwnershipAction::Copy => source
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| runtime_error("use after move", span)),
        OwnershipAction::Borrow | OwnershipAction::BorrowMut => {
            Ok(ConcreteValue::Reference(source.clone()))
        }
    }
}

fn unary(op: &UnaryOp, value: ConcreteValue, span: Span) -> Result<ConcreteValue, RuntimeError> {
    match (op, value) {
        (UnaryOp::Neg, ConcreteValue::Integer(value)) => value
            .checked_neg()
            .map(ConcreteValue::Integer)
            .ok_or_else(|| runtime_error("integer overflow", span)),
        (UnaryOp::Neg, ConcreteValue::Float(value)) => Ok(ConcreteValue::Float(-value)),
        (UnaryOp::Not, value) => Ok(ConcreteValue::Bool(!value.truthy(span)?)),
        (UnaryOp::BitNot, ConcreteValue::Integer(value)) => Ok(ConcreteValue::Integer(!value)),
        (UnaryOp::Deref, ConcreteValue::Reference(value)) => value
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| runtime_error("dereference of dead value", span)),
        (UnaryOp::AddressOf, ConcreteValue::Reference(value)) => {
            Ok(ConcreteValue::Reference(value))
        }
        _ => Err(runtime_error("invalid canonical unary operand", span)),
    }
}

fn binary(
    left: ConcreteValue,
    op: &BinOp,
    right: ConcreteValue,
    span: Span,
) -> Result<ConcreteValue, RuntimeError> {
    use BinOp::*;
    match (left, right) {
        (ConcreteValue::MemoryPointer { address, .. }, ConcreteValue::Null)
        | (ConcreteValue::Null, ConcreteValue::MemoryPointer { address, .. })
            if matches!(op, EqEq | NotEq) =>
        {
            Ok(ConcreteValue::Bool(if *op == EqEq {
                address == 0
            } else {
                address != 0
            }))
        }
        (
            ConcreteValue::MemoryPointer { address: left, .. },
            ConcreteValue::MemoryPointer { address: right, .. },
        ) if matches!(op, EqEq | NotEq) => Ok(ConcreteValue::Bool(if *op == EqEq {
            left == right
        } else {
            left != right
        })),
        (ConcreteValue::Null, ConcreteValue::Null) if matches!(op, EqEq | NotEq) => {
            Ok(ConcreteValue::Bool(*op == EqEq))
        }
        (ConcreteValue::Integer(left), ConcreteValue::Integer(right)) => {
            let integer = |value| Ok(ConcreteValue::Integer(value));
            match op {
                Add => left.checked_add(right).map(ConcreteValue::Integer),
                Sub => left.checked_sub(right).map(ConcreteValue::Integer),
                Mul => left.checked_mul(right).map(ConcreteValue::Integer),
                Div => left.checked_div(right).map(ConcreteValue::Integer),
                Rem => left.checked_rem(right).map(ConcreteValue::Integer),
                BitAnd => return integer(left & right),
                BitOr => return integer(left | right),
                BitXor => return integer(left ^ right),
                Shl => left.checked_shl(right as u32).map(ConcreteValue::Integer),
                Shr => left.checked_shr(right as u32).map(ConcreteValue::Integer),
                EqEq => return Ok(ConcreteValue::Bool(left == right)),
                NotEq => return Ok(ConcreteValue::Bool(left != right)),
                Lt => return Ok(ConcreteValue::Bool(left < right)),
                LtEq => return Ok(ConcreteValue::Bool(left <= right)),
                Gt => return Ok(ConcreteValue::Bool(left > right)),
                GtEq => return Ok(ConcreteValue::Bool(left >= right)),
            }
            .ok_or_else(|| {
                runtime_error(
                    if matches!(op, Div | Rem) && right == 0 {
                        "division by zero"
                    } else if matches!(op, Shl | Shr) {
                        "invalid shift"
                    } else {
                        "integer overflow"
                    },
                    span,
                )
            })
        }
        (ConcreteValue::Float(left), ConcreteValue::Float(right)) => match op {
            Add => Ok(ConcreteValue::Float(left + right)),
            Sub => Ok(ConcreteValue::Float(left - right)),
            Mul => Ok(ConcreteValue::Float(left * right)),
            Div if right != 0.0 => Ok(ConcreteValue::Float(left / right)),
            EqEq => Ok(ConcreteValue::Bool(left == right)),
            NotEq => Ok(ConcreteValue::Bool(left != right)),
            Lt => Ok(ConcreteValue::Bool(left < right)),
            LtEq => Ok(ConcreteValue::Bool(left <= right)),
            Gt => Ok(ConcreteValue::Bool(left > right)),
            GtEq => Ok(ConcreteValue::Bool(left >= right)),
            _ => Err(runtime_error("invalid float operation", span)),
        },
        (ConcreteValue::String(mut left), ConcreteValue::String(right)) => match op {
            Add => {
                left.extend_from_slice(&right);
                Ok(ConcreteValue::String(left))
            }
            EqEq => Ok(ConcreteValue::Bool(left == right)),
            NotEq => Ok(ConcreteValue::Bool(left != right)),
            _ => Err(runtime_error("invalid string operation", span)),
        },
        (ConcreteValue::Bool(left), ConcreteValue::Bool(right)) => match op {
            EqEq => Ok(ConcreteValue::Bool(left == right)),
            NotEq => Ok(ConcreteValue::Bool(left != right)),
            _ => Err(runtime_error("invalid bool operation", span)),
        },
        _ => Err(runtime_error("binary operand type mismatch", span)),
    }
}

fn cast(
    value: ConcreteValue,
    target: &CanonicalType,
    span: Span,
) -> Result<ConcreteValue, RuntimeError> {
    match (value, target) {
        (ConcreteValue::Integer(value), CanonicalType::Integer(_)) => {
            Ok(ConcreteValue::Integer(value))
        }
        (ConcreteValue::Integer(value), CanonicalType::Bool) => Ok(ConcreteValue::Bool(value != 0)),
        (ConcreteValue::Bool(value), CanonicalType::Integer(_)) => {
            Ok(ConcreteValue::Integer(i64::from(value)))
        }
        (ConcreteValue::Integer(value), CanonicalType::Pointer { pointee, .. }) if value >= 0 => {
            Ok(if value == 0 {
                ConcreteValue::Null
            } else {
                ConcreteValue::MemoryPointer {
                    address: value as u64,
                    pointee: *pointee,
                }
            })
        }
        (ConcreteValue::Null, CanonicalType::Pointer { .. }) => Ok(ConcreteValue::Null),
        (ConcreteValue::MemoryPointer { address, .. }, CanonicalType::Integer(_)) => {
            Ok(ConcreteValue::Integer(address as i64))
        }
        (ConcreteValue::Reference(_), CanonicalType::Integer(_)) => {
            // Hosted object references are non-null canonical capabilities;
            // their host address is never exposed as language state.
            Ok(ConcreteValue::Integer(1))
        }
        (ConcreteValue::MemoryPointer { address, .. }, CanonicalType::Pointer { pointee, .. }) => {
            Ok(ConcreteValue::MemoryPointer {
                address,
                pointee: *pointee,
            })
        }
        (value, _) => Err(runtime_error(
            &format!("unsupported canonical cast from {}", value.display()),
            span,
        )),
    }
}

fn runtime_error(message: &str, span: Span) -> RuntimeError {
    RuntimeError {
        msg: message.into(),
        span,
    }
}

fn value_span(_: ConcreteValue) -> Span {
    Span::zero()
}

fn to_legacy_value(value: ConcreteValue) -> Result<crate::Value, RuntimeError> {
    Ok(match value {
        ConcreteValue::Unit => crate::Value::Void,
        ConcreteValue::Integer(value) => crate::Value::Int(value),
        ConcreteValue::Float(value) => crate::Value::Float(value),
        ConcreteValue::Bool(value) => crate::Value::Bool(value),
        ConcreteValue::String(value) => crate::Value::Str(
            String::from_utf8(value)
                .map_err(|_| runtime_error("host string is not UTF-8", Span::zero()))?,
        ),
        ConcreteValue::Bytes(value) => crate::Value::Bytes(value),
        ConcreteValue::Buffer(value) => crate::Value::Buffer(value),
        ConcreteValue::Null => crate::Value::Null,
        ConcreteValue::List(values) => crate::Value::List(
            values
                .into_iter()
                .map(|value| {
                    value
                        .borrow()
                        .as_ref()
                        .cloned()
                        .ok_or_else(|| runtime_error("host list element was moved", Span::zero()))
                        .and_then(to_legacy_value)
                })
                .collect::<Result<_, _>>()?,
        ),
        ConcreteValue::Reference(value) => value
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| runtime_error("host borrow points to dead value", Span::zero()))
            .and_then(to_legacy_value)?,
        _ => {
            return Err(runtime_error(
                "host capability received unsupported aggregate",
                Span::zero(),
            ))
        }
    })
}

fn from_legacy_value(value: crate::Value) -> Result<ConcreteValue, RuntimeError> {
    Ok(match value {
        crate::Value::Void => ConcreteValue::Unit,
        crate::Value::Int(value) => ConcreteValue::Integer(value),
        crate::Value::Float(value) => ConcreteValue::Float(value),
        crate::Value::Bool(value) => ConcreteValue::Bool(value),
        crate::Value::Str(value) => ConcreteValue::String(value.into_bytes()),
        crate::Value::Bytes(value) => ConcreteValue::Bytes(value),
        crate::Value::Buffer(value) => ConcreteValue::Buffer(value),
        crate::Value::Null => ConcreteValue::Null,
        crate::Value::List(values) => ConcreteValue::List(
            values
                .into_iter()
                .map(from_legacy_value)
                .map(|value| value.map(|value| slot(Some(value))))
                .collect::<Result<_, _>>()?,
        ),
        _ => {
            return Err(runtime_error(
                "host capability returned unsupported aggregate",
                Span::zero(),
            ))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbyte_hir::{
        analyze_project, monomorphize_project, ModuleId, Profile, ProjectInput, ProjectModuleInput,
        TargetDataLayout,
    };
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;

    fn concrete(source: &str) -> ConcreteHirProject {
        let program = Parser::new(Lexer::new(source).tokenize().unwrap())
            .parse_program()
            .unwrap();
        let canonical = analyze_project(
            &ProjectInput {
                root: ModuleId(0),
                modules: vec![ProjectModuleInput {
                    id: ModuleId(0),
                    name: "root".into(),
                    program,
                    imports: Vec::new(),
                }],
            },
            Profile::Hosted,
            TargetDataLayout::I686,
        )
        .unwrap();
        monomorphize_project(&canonical).unwrap()
    }

    #[test]
    fn hosted_execution_uses_resolved_host_capability_and_format_parts() {
        let source = "fn id[T](value: T) -> T:\n    return value\n\nlet answer = id[int](40) + 2\nprint(\"VALUE {answer}\")\n";
        let project = concrete(source);
        let expression = project
            .executable
            .top_level
            .statements
            .iter()
            .find_map(|statement| match statement {
                HirStatement::Expression(HirExpr {
                    kind:
                        HirExprKind::HostCall {
                            capability: HostCapabilityId::Print,
                            arguments,
                        },
                    ..
                }) => arguments.first(),
                _ => None,
            })
            .expect("print must be a resolved host capability");
        assert!(matches!(expression.kind, HirExprKind::FormatString(..)));

        let execution = execute_concrete_project(&project).unwrap();
        assert_eq!(execution.output, "VALUE 42\n");
    }

    #[test]
    fn concrete_reassignment_runs_old_cleanup_after_rhs() {
        let source = "struct Resource:\n    value: int\n\ndrop fn destroy_resource(value: borrow mut Resource):\n    print((*value).value)\n\nlet resource = Resource(value = 1)\nresource = Resource(value = 2)\ndrop(resource)\n";
        let execution = execute_concrete_project(&concrete(source)).unwrap();
        assert_eq!(execution.output, "1\n2\n");
    }

    #[test]
    fn concrete_result_propagation_runs_cleanup_once() {
        let source = "fn attempt(input: Result[int, int]) -> Result[int, int]:\n    defer []:\n        print(\"CLEAN\")\n    let value = input?\n    return Result[int, int].Ok(value)\n\nlet result = attempt(Result[int, int].Err(7))\nmatch result:\n    case Result.Ok(value):\n        print(value)\n    case Result.Err(error):\n        print(error)\n";
        let execution = execute_concrete_project(&concrete(source)).unwrap();
        assert_eq!(execution.output, "CLEAN\n7\n");
    }

    #[test]
    fn concrete_for_uses_resolved_loop_local() {
        let source =
            "let total = 0\nfor value in [1, 2, 3]:\n    total = total + value\nprint(total)\n";
        let project = concrete(source);
        assert!(project
            .executable
            .top_level
            .statements
            .iter()
            .any(|statement| matches!(statement, HirStatement::For { .. })));
        let execution = execute_concrete_project(&project).unwrap();
        assert_eq!(execution.output, "6\n");
    }

    #[test]
    fn concrete_hosted_v12_parity_smoke() {
        let source = "struct Pair[A, B]:\n    first: A\n    second: B\n\nfn id[T](value: T) -> T:\n    return value\n\nfn attempt(input: Result[int, int]) -> Result[int, int]:\n    defer []:\n        print(\"CLEAN\")\n    let value = input?\n    return Result[int, int].Ok(value)\n\nlet pair = Pair[int, int](first = id[int](40), second = 2)\nprint(pair.first + pair.second)\nlet factor: int = 2\nlet add = fn [borrow factor](value: int) -> int:\n    return value + factor\nprint(add(40))\nlet result = attempt(Result[int, int].Err(7))\nmatch result:\n    case Result.Ok(value):\n        print(value)\n    case Result.Err(error):\n        print(error)\n";
        let execution = execute_concrete_project(&concrete(source)).unwrap();
        assert_eq!(execution.output, "42\n42\nCLEAN\n7\n");
    }

    #[test]
    fn concrete_standard_module_call_has_a_resolved_capability_id() {
        let source = "import std.math as math\nprint(math.abs(-42))\n";
        let project = concrete(source);
        let execution = execute_concrete_project(&project).unwrap();
        assert_eq!(execution.output, "42\n");
        assert!(matches!(
            project.executable.top_level.statements[0],
            HirStatement::Expression(HirExpr {
                kind: HirExprKind::HostCall { ref arguments, .. },
                ..
            }) if matches!(
                arguments[0].kind,
                HirExprKind::HostCall {
                    capability: HostCapabilityId::MathAbs,
                    ..
                }
            )
        ));
    }

    #[test]
    fn incremental_session_executes_delta_and_rolls_back_runtime_failure() {
        let mut session = ConcreteSession::default();
        let first = concrete("let value = 40\nprint(value)\n");
        let first_run =
            execute_concrete_project_incremental(&mut session, &first, 0, Vec::new()).unwrap();
        assert_eq!(first_run.output, "40\n");

        let failing = concrete("let value = 40\nprint(value)\nvalue = 99\nprint(1 / 0)\n");
        assert!(execute_concrete_project_incremental(
            &mut session,
            &failing,
            first_run.next_cursor,
            Vec::new()
        )
        .is_err());

        let recovered = concrete("let value = 40\nprint(value)\nprint(value + 2)\n");
        let recovered_run = execute_concrete_project_incremental(
            &mut session,
            &recovered,
            first_run.next_cursor,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(recovered_run.output, "42\n");
    }
}
