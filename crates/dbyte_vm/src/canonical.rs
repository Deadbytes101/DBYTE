//! VM execution for canonical-ID bytecode emitted from Concrete HIR.

use crate::VmError;
use dbyte_ast::{BinOp, CaptureMode, MatchMode, Span, UnaryOp};
use dbyte_bytecode::*;
use dbyte_hir::{
    layout_of, struct_field_layout, CanonicalType, FieldId, FunctionInstanceKey, HirIntrinsic,
    IntegerType, LocalId, OwnershipAction, SymbolId, TypeDefKind, TypeId, VariantId,
};
use dbyte_interp::memory::{AllocationEvent, DeterministicMemory};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

type Slot = Rc<RefCell<Option<VmValue>>>;

#[derive(Debug, Clone)]
enum VmValue {
    Unit,
    Integer(i64),
    Float(f64),
    Bool(bool),
    String(Vec<u8>),
    Bytes(Vec<u8>),
    Buffer(Rc<RefCell<Vec<u8>>>),
    Null,
    List(Vec<Slot>),
    Struct(BTreeMap<FieldId, Slot>),
    Enum {
        variant: VariantId,
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
    Closure(Rc<VmClosure>),
    EarlyReturn(Box<VmValue>),
}

#[derive(Debug)]
struct VmClosure {
    captures: BTreeMap<LocalId, Slot>,
    parameters: Vec<dbyte_hir::HirLocal>,
    body: CanonicalBytecodeBlock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalVmExecution {
    pub output: String,
    pub allocator_events: Vec<AllocationEvent>,
    pub live_allocations: usize,
}

enum Flow {
    Normal,
    Return(VmValue),
    Break,
    Continue,
}
struct Frame {
    locals: BTreeMap<LocalId, Slot>,
}

struct CanonicalVm<'a> {
    project: &'a CanonicalBytecodeProject,
    globals: BTreeMap<SymbolId, Slot>,
    frames: Vec<Frame>,
    output: String,
    call_depth: usize,
    script_args: Vec<String>,
    memory: DeterministicMemory,
    memory_values: BTreeMap<u64, (TypeId, VmValue)>,
}

pub fn execute_canonical_bytecode(
    project: &CanonicalBytecodeProject,
) -> Result<CanonicalVmExecution, VmError> {
    execute_canonical_bytecode_with_args(project, Vec::new())
}

pub fn execute_canonical_bytecode_with_memory(
    project: &CanonicalBytecodeProject,
    memory: DeterministicMemory,
) -> Result<CanonicalVmExecution, VmError> {
    execute_canonical_bytecode_inner(project, Vec::new(), memory)
}

pub fn execute_canonical_bytecode_with_args(
    project: &CanonicalBytecodeProject,
    script_args: Vec<String>,
) -> Result<CanonicalVmExecution, VmError> {
    execute_canonical_bytecode_inner(project, script_args, DeterministicMemory::system())
}

fn execute_canonical_bytecode_inner(
    project: &CanonicalBytecodeProject,
    script_args: Vec<String>,
    memory: DeterministicMemory,
) -> Result<CanonicalVmExecution, VmError> {
    let mut vm = CanonicalVm {
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
    for global in &project.globals {
        vm.globals.insert(global.symbol, slot(None));
    }
    for global in &project.globals {
        if let Some(code) = &global.initializer {
            let value = vm.eval(code)?;
            vm.global(global.symbol, global.span)?
                .borrow_mut()
                .replace(value);
        }
    }
    match vm.block(&project.top_level)? {
        Flow::Normal => Ok(CanonicalVmExecution {
            output: vm.output,
            allocator_events: vm.memory.telemetry().to_vec(),
            live_allocations: vm.memory.live_allocations(),
        }),
        _ => Err(error(
            "control flow escaped top-level bytecode",
            Span::zero(),
        )),
    }
}

impl CanonicalVm<'_> {
    fn block(&mut self, block: &CanonicalBytecodeBlock) -> Result<Flow, VmError> {
        for statement in &block.statements {
            let flow = self.statement(statement)?;
            if !matches!(flow, Flow::Normal) {
                return Ok(flow);
            }
        }
        Ok(Flow::Normal)
    }

    fn statement(&mut self, statement: &CanonicalBytecodeStatement) -> Result<Flow, VmError> {
        match statement {
            CanonicalBytecodeStatement::Let { local, value, span } => {
                let value = self.eval(value)?;
                if let VmValue::EarlyReturn(value) = value {
                    return Ok(Flow::Return(*value));
                }
                self.ensure_local(*local, *span)?
                    .borrow_mut()
                    .replace(value);
                Ok(Flow::Normal)
            }
            CanonicalBytecodeStatement::Assign {
                place,
                value,
                cleanup_before_commit,
                ..
            } => {
                let value = self.eval(value)?;
                if let VmValue::EarlyReturn(value) = value {
                    return Ok(Flow::Return(*value));
                }
                let memory_destination = self.memory_place(place)?;
                if let Some(cleanup) = cleanup_before_commit {
                    self.cleanup(std::slice::from_ref(cleanup))?;
                }
                if let Some((address, ty)) = memory_destination {
                    self.write_memory_value(address, ty, value, place.span)?;
                } else {
                    self.place(place)?.borrow_mut().replace(value);
                }
                Ok(Flow::Normal)
            }
            CanonicalBytecodeStatement::Expression(code) => {
                if let VmValue::EarlyReturn(value) = self.eval(code)? {
                    return Ok(Flow::Return(*value));
                }
                Ok(Flow::Normal)
            }
            CanonicalBytecodeStatement::Return { value, cleanup, .. } => {
                let value = value
                    .as_ref()
                    .map(|value| self.eval(value))
                    .transpose()?
                    .unwrap_or(VmValue::Unit);
                if let VmValue::EarlyReturn(value) = value {
                    return Ok(Flow::Return(*value));
                }
                self.cleanup(cleanup)?;
                Ok(Flow::Return(value))
            }
            CanonicalBytecodeStatement::If {
                condition,
                then_block,
                else_block,
                span,
            } => {
                let condition = self.eval(condition)?;
                if let VmValue::EarlyReturn(value) = condition {
                    return Ok(Flow::Return(*value));
                }
                if condition.truthy(*span)? {
                    self.block(then_block)
                } else if let Some(block) = else_block {
                    self.block(block)
                } else {
                    Ok(Flow::Normal)
                }
            }
            CanonicalBytecodeStatement::While {
                condition, body, ..
            } => {
                loop {
                    let value = self.eval(condition)?;
                    if let VmValue::EarlyReturn(value) = value {
                        return Ok(Flow::Return(*value));
                    }
                    if !value.truthy(condition.span)? {
                        break;
                    }
                    match self.block(body)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        flow @ Flow::Return(_) => return Ok(flow),
                    }
                }
                Ok(Flow::Normal)
            }
            CanonicalBytecodeStatement::For {
                local,
                iterable,
                body,
                ..
            } => {
                let value = self.eval(iterable)?;
                if let VmValue::EarlyReturn(value) = value {
                    return Ok(Flow::Return(*value));
                }
                let items = match value {
                    VmValue::List(items) => items
                        .into_iter()
                        .map(|item| {
                            item.borrow()
                                .as_ref()
                                .cloned()
                                .ok_or_else(|| error("for item was moved", local.span))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    VmValue::String(bytes) => String::from_utf8(bytes)
                        .map_err(|_| error("hosted str is not UTF-8", local.span))?
                        .chars()
                        .map(|value| VmValue::String(value.to_string().into_bytes()))
                        .collect(),
                    _ => return Err(error("value is not iterable", local.span)),
                };
                let destination = self.ensure_local(local.id, local.span)?;
                let previous = destination.borrow_mut().take();
                for item in items {
                    destination.borrow_mut().replace(item);
                    match self.block(body)? {
                        Flow::Normal | Flow::Continue => {}
                        Flow::Break => break,
                        flow @ Flow::Return(_) => {
                            *destination.borrow_mut() = previous;
                            return Ok(flow);
                        }
                    }
                }
                *destination.borrow_mut() = previous;
                Ok(Flow::Normal)
            }
            CanonicalBytecodeStatement::Match {
                mode,
                value,
                arms,
                span,
            } => {
                let value = self.eval(value)?;
                if let VmValue::EarlyReturn(value) = value {
                    return Ok(Flow::Return(*value));
                }
                let VmValue::Enum { variant, payload } = value else {
                    return Err(error("match value is not enum", *span));
                };
                let arm = arms
                    .iter()
                    .find(|arm| arm.variant.is_none_or(|candidate| candidate == variant))
                    .ok_or_else(|| error("exhaustive match has no arm", *span))?;
                let mut restore = Vec::new();
                for (binding, source) in arm.bindings.iter().zip(payload) {
                    let destination = self.ensure_local(binding.id, binding.span)?;
                    let value = match mode {
                        MatchMode::Move => source
                            .borrow_mut()
                            .take()
                            .ok_or_else(|| error("match payload moved", binding.span))?,
                        MatchMode::Borrow | MatchMode::BorrowMut => VmValue::Reference(source),
                    };
                    restore.push((destination.clone(), destination.borrow_mut().replace(value)));
                }
                let result = self.block(&arm.body);
                for (slot, prior) in restore.into_iter().rev() {
                    *slot.borrow_mut() = prior;
                }
                result
            }
            CanonicalBytecodeStatement::RegisterDefer => Ok(Flow::Normal),
            CanonicalBytecodeStatement::Break { cleanup, .. } => {
                self.cleanup(cleanup)?;
                Ok(Flow::Break)
            }
            CanonicalBytecodeStatement::Continue { cleanup, .. } => {
                self.cleanup(cleanup)?;
                Ok(Flow::Continue)
            }
        }
    }

    fn cleanup(&mut self, blocks: &[CanonicalBytecodeBlock]) -> Result<(), VmError> {
        for block in blocks {
            if !matches!(self.block(block)?, Flow::Normal) {
                return Err(error("cleanup attempted control flow", Span::zero()));
            }
        }
        Ok(())
    }

    fn eval(&mut self, code: &CanonicalExprCode) -> Result<VmValue, VmError> {
        let mut stack = Vec::new();
        for op in &code.ops {
            match op {
                CanonicalOp::PushUnit => stack.push(VmValue::Unit),
                CanonicalOp::PushInteger(value) => stack.push(VmValue::Integer(*value)),
                CanonicalOp::PushLayout { target, alignment } => {
                    let layout = layout_of(&self.project.semantic, *target)
                        .map_err(|e| error(&e.to_string(), code.span))?;
                    stack.push(VmValue::Integer(i64::from(if *alignment {
                        layout.align
                    } else {
                        layout.size
                    })));
                }
                CanonicalOp::PushFloat(value) => stack.push(VmValue::Float(*value)),
                CanonicalOp::PushBool(value) => stack.push(VmValue::Bool(*value)),
                CanonicalOp::PushString(value) => stack.push(VmValue::String(value.clone())),
                CanonicalOp::PushBytes(value) => stack.push(VmValue::Bytes(value.clone())),
                CanonicalOp::PushNull => stack.push(VmValue::Null),
                CanonicalOp::LoadLocal { local, action } => {
                    stack.push(read(&self.local(*local, code.span)?, *action, code.span)?)
                }
                CanonicalOp::LoadGlobal { symbol, action } => {
                    stack.push(read(&self.global(*symbol, code.span)?, *action, code.span)?)
                }
                CanonicalOp::PushFunction(key) => stack.push(VmValue::Function(key.clone())),
                CanonicalOp::MakeList(count) => {
                    let values = pop_many(&mut stack, *count, code.span)?;
                    stack.push(VmValue::List(
                        values.into_iter().map(|value| slot(Some(value))).collect(),
                    ));
                }
                CanonicalOp::Binary(op) => {
                    let right = pop(&mut stack, code.span)?;
                    let left = pop(&mut stack, code.span)?;
                    stack.push(binary(left, op, right, code.span)?);
                }
                CanonicalOp::Unary(op) => {
                    let value = pop(&mut stack, code.span)?;
                    stack.push(unary(op, value, code.span)?);
                }
                CanonicalOp::Cast(target) => {
                    let value = pop(&mut stack, code.span)?;
                    stack.push(cast(
                        value,
                        self.project.semantic.type_of(*target),
                        code.span,
                    )?);
                }
                CanonicalOp::Length => {
                    let value = pop(&mut stack, code.span)?;
                    stack.push(length(value, code.span)?);
                }
                CanonicalOp::CallDirect {
                    callee,
                    argument_count,
                    ..
                } => {
                    let args = pop_many(&mut stack, *argument_count, code.span)?;
                    stack.push(self.call(callee, args, code.span)?);
                }
                CanonicalOp::CallIndirect { argument_count, .. } => {
                    let args = pop_many(&mut stack, *argument_count, code.span)?;
                    let callee = pop(&mut stack, code.span)?;
                    let value = match callee {
                        VmValue::Function(key) => self.call(&key, args, code.span)?,
                        VmValue::Closure(closure) => {
                            self.call_closure(&closure, args, code.span)?
                        }
                        _ => return Err(error("indirect callee is not callable", code.span)),
                    };
                    stack.push(value);
                }
                CanonicalOp::Intrinsic {
                    intrinsic,
                    argument_count,
                    result,
                } if intrinsic.is_runtime_host_memory() => {
                    let args = pop_many(&mut stack, *argument_count, code.span)?;
                    stack.push(self.runtime_memory(*intrinsic, &args, *result, code.span)?);
                }
                CanonicalOp::Intrinsic {
                    intrinsic,
                    argument_count,
                    result,
                } if intrinsic.is_runtime_view() => {
                    let args = pop_many(&mut stack, *argument_count, code.span)?;
                    stack.push(self.runtime_view(*intrinsic, &args, *result, code.span)?);
                }
                CanonicalOp::Intrinsic { .. } => {
                    return Err(error("target intrinsic in hosted VM bytecode", code.span))
                }
                CanonicalOp::HostCall {
                    capability,
                    argument_count,
                } => {
                    let args = pop_many(&mut stack, *argument_count, code.span)?;
                    match capability {
                        dbyte_hir::HostCapabilityId::Print => {
                            for (index, value) in args.iter().enumerate() {
                                if index != 0 {
                                    self.output.push(' ');
                                }
                                self.output.push_str(&value.display());
                            }
                            self.output.push('\n');
                            stack.push(VmValue::Unit);
                        }
                        dbyte_hir::HostCapabilityId::EnvArgs => {
                            stack.push(VmValue::List(
                                self.script_args
                                    .iter()
                                    .map(|argument| {
                                        slot(Some(VmValue::String(argument.as_bytes().to_vec())))
                                    })
                                    .collect(),
                            ));
                        }
                        capability => {
                            let args = args
                                .into_iter()
                                .map(to_host_value)
                                .collect::<Result<Vec<_>, _>>()?;
                            let value =
                                dbyte_interp::execute_resolved_host_capability(*capability, &args)
                                    .map_err(|message| error(&message, code.span))?;
                            stack.push(from_host_value(value)?);
                        }
                    }
                }
                CanonicalOp::Format { parts, value_count } => {
                    let values = pop_many(&mut stack, *value_count, code.span)?;
                    let mut values = values.into_iter();
                    let mut bytes = Vec::new();
                    for part in parts {
                        match part {
                            CanonicalFormatPart::Literal(value) => bytes.extend_from_slice(value),
                            CanonicalFormatPart::Value => bytes.extend_from_slice(
                                values
                                    .next()
                                    .ok_or_else(|| error("format stack underflow", code.span))?
                                    .display()
                                    .as_bytes(),
                            ),
                        }
                    }
                    stack.push(VmValue::String(bytes));
                }
                CanonicalOp::MakeStruct { fields, .. } => {
                    let values = pop_many(&mut stack, fields.len(), code.span)?;
                    stack.push(VmValue::Struct(
                        fields
                            .iter()
                            .copied()
                            .zip(values)
                            .map(|(field, value)| (field, slot(Some(value))))
                            .collect(),
                    ));
                }
                CanonicalOp::MakeEnum {
                    variant,
                    payload_count,
                    ..
                } => {
                    let values = pop_many(&mut stack, *payload_count, code.span)?;
                    stack.push(VmValue::Enum {
                        variant: *variant,
                        payload: values.into_iter().map(|value| slot(Some(value))).collect(),
                    });
                }
                CanonicalOp::LoadPlace { place, action } => {
                    if let Some((address, ty)) = self.memory_place(place)? {
                        stack.push(self.read_memory_value(address, ty, *action, code.span)?);
                    } else {
                        stack.push(read(&self.place(place)?, *action, code.span)?)
                    }
                }
                CanonicalOp::BorrowPlace { place, .. } => match self.memory_place(place)? {
                    Some((address, ty)) => stack.push(VmValue::MemoryPointer {
                        address,
                        pointee: ty,
                    }),
                    None => stack.push(VmValue::Reference(self.place(place)?)),
                },
                CanonicalOp::PropagateResult {
                    ok_variant,
                    err_variant,
                    cleanup,
                } => {
                    let value = pop(&mut stack, code.span)?;
                    let VmValue::Enum { variant, payload } = value else {
                        return Err(error("? operand is not Result", code.span));
                    };
                    if variant == *ok_variant {
                        stack.push(
                            payload
                                .first()
                                .map(|value| {
                                    value
                                        .borrow_mut()
                                        .take()
                                        .ok_or_else(|| error("Ok payload moved", code.span))
                                })
                                .transpose()?
                                .unwrap_or(VmValue::Unit),
                        );
                    } else if variant == *err_variant {
                        self.cleanup(cleanup)?;
                        stack.push(VmValue::EarlyReturn(Box::new(VmValue::Enum {
                            variant,
                            payload,
                        })));
                    } else {
                        return Err(error("invalid Result tag", code.span));
                    }
                }
                CanonicalOp::DropPlace { place, cleanup } => {
                    self.block(cleanup)?;
                    if let Some((address, _)) = self.memory_place(place)? {
                        self.memory_values.remove(&address);
                    } else {
                        self.place(place)?.borrow_mut().take();
                    }
                    stack.push(VmValue::Unit);
                }
                CanonicalOp::MakeClosure {
                    captures,
                    parameters,
                    body,
                    ..
                } => {
                    let mut captured = BTreeMap::new();
                    for capture in captures {
                        let source = self.local(capture.local, capture.span)?;
                        let stored = match capture.mode {
                            CaptureMode::Move => slot(Some(
                                source
                                    .borrow_mut()
                                    .take()
                                    .ok_or_else(|| error("capture moved", capture.span))?,
                            )),
                            CaptureMode::Borrow | CaptureMode::BorrowMut => source,
                        };
                        captured.insert(capture.local, stored);
                    }
                    stack.push(VmValue::Closure(Rc::new(VmClosure {
                        captures: captured,
                        parameters: parameters.clone(),
                        body: body.clone(),
                    })));
                }
            }
        }
        if stack.len() != 1 {
            return Err(error("canonical expression stack imbalance", code.span));
        }
        pop(&mut stack, code.span)
    }

    fn runtime_memory(
        &mut self,
        intrinsic: HirIntrinsic,
        values: &[VmValue],
        result_ty: TypeId,
        span: Span,
    ) -> Result<VmValue, VmError> {
        let integer = |value: &VmValue| match value {
            VmValue::Integer(value) if *value >= 0 => Ok(*value as u64),
            VmValue::MemoryPointer { address, .. } => Ok(*address),
            VmValue::Null => Ok(0),
            _ => Err(error(
                "runtime memory argument is not an address or usize",
                span,
            )),
        };
        match intrinsic {
            HirIntrinsic::RuntimeMemoryAlloc => {
                let size = integer(&values[0])?;
                let align = integer(&values[1])?;
                let pointee = match self.project.semantic.type_of(result_ty) {
                    CanonicalType::Pointer { pointee, .. } => *pointee,
                    _ => return Err(error("runtime allocation result is not a pointer", span)),
                };
                Ok(self
                    .memory
                    .alloc(size, align)
                    .map_or(VmValue::Null, |address| VmValue::MemoryPointer {
                        address,
                        pointee,
                    }))
            }
            HirIntrinsic::RuntimeMemoryResize => {
                let address = integer(&values[0])?;
                let old_size = integer(&values[1])?;
                let new_size = integer(&values[2])?;
                let align = integer(&values[3])?;
                let pointee = match self.project.semantic.type_of(result_ty) {
                    CanonicalType::Pointer { pointee, .. } => *pointee,
                    _ => return Err(error("runtime resize result is not a pointer", span)),
                };
                match self.memory.resize(address, old_size, new_size, align) {
                    Ok(replacement) => {
                        self.relocate_memory_values(address, old_size, replacement, new_size);
                        Ok(VmValue::MemoryPointer {
                            address: replacement,
                            pointee,
                        })
                    }
                    Err(_) => Ok(VmValue::Null),
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
                Ok(VmValue::Bool(released))
            }
            HirIntrinsic::RuntimeMemoryRead8 => {
                let address = integer(&values[0])?;
                let offset = integer(&values[1])?;
                let bytes = self.memory.read(address, offset, 1).map_err(|failure| {
                    error(&format!("runtime memory read failed: {failure:?}"), span)
                })?;
                Ok(VmValue::Integer(i64::from(bytes[0])))
            }
            HirIntrinsic::RuntimeMemoryWrite8 => {
                let address = integer(&values[0])?;
                let offset = integer(&values[1])?;
                let byte = integer(&values[2])?;
                let byte = u8::try_from(byte)
                    .map_err(|_| error("runtime memory byte is out of range", span))?;
                self.memory
                    .write(address, offset, &[byte])
                    .map_err(|failure| {
                        error(&format!("runtime memory write failed: {failure:?}"), span)
                    })?;
                Ok(VmValue::Unit)
            }
            _ => Err(error("unsupported runtime memory capability", span)),
        }
    }

    fn runtime_view(
        &self,
        intrinsic: HirIntrinsic,
        values: &[VmValue],
        result_ty: TypeId,
        span: Span,
    ) -> Result<VmValue, VmError> {
        let VmValue::MemoryPointer { address, .. } = values[0] else {
            return Err(error("runtime view data is not a memory pointer", span));
        };
        let length = values[1].integer(span)?;
        let length =
            u64::try_from(length).map_err(|_| error("runtime view length is negative", span))?;
        let (element, mutable, string) = match self.project.semantic.type_of(result_ty) {
            CanonicalType::Slice { element, mutable } => (*element, *mutable, false),
            CanonicalType::StringView => {
                let element = match intrinsic {
                    HirIntrinsic::RuntimeStringViewFromRaw => self
                        .project
                        .semantic
                        .types
                        .iter()
                        .position(|ty| *ty == CanonicalType::Integer(IntegerType::U8))
                        .map(|index| TypeId(index as u32))
                        .ok_or_else(|| error("canonical u8 type is missing", span))?,
                    _ => return Err(error("StringView uses the wrong intrinsic", span)),
                };
                (element, false, true)
            }
            _ => return Err(error("runtime view result has the wrong type", span)),
        };
        Ok(VmValue::View {
            address,
            length,
            element,
            mutable,
            string,
        })
    }

    fn place(&mut self, place: &CanonicalPlace) -> Result<Slot, VmError> {
        let mut current = match &place.base {
            CanonicalPlaceBase::Local(local) => self.local(*local, place.span)?,
            CanonicalPlaceBase::Global(symbol) => self.global(*symbol, place.span)?,
            CanonicalPlaceBase::Pointer(code) => match self.eval(code)? {
                VmValue::Reference(slot) => dereference_slot(slot),
                _ => return Err(error("pointer place is invalid", place.span)),
            },
        };
        for projection in &place.projections {
            current = match projection {
                CanonicalProjection::Field { field, .. } => {
                    let mut value = current.borrow_mut();
                    if value.is_none() {
                        *value = Some(VmValue::Struct(BTreeMap::new()));
                    }
                    match value.as_mut() {
                        Some(VmValue::Struct(fields)) => {
                            fields.entry(*field).or_insert_with(|| slot(None)).clone()
                        }
                        _ => return Err(error("field base is not struct", place.span)),
                    }
                }
                CanonicalProjection::Payload { variant, index, .. } => {
                    match current.borrow().as_ref() {
                        Some(VmValue::Enum {
                            variant: active,
                            payload,
                        }) if active == variant => payload
                            .get(*index as usize)
                            .cloned()
                            .ok_or_else(|| error("payload missing", place.span))?,
                        Some(VmValue::Enum { .. }) => {
                            return Err(error("inactive payload", place.span))
                        }
                        _ => return Err(error("payload base is not enum", place.span)),
                    }
                }
                CanonicalProjection::Index { index, .. } => {
                    let raw_offset = self.eval(index)?.integer(index.span)?;
                    let length = match current.borrow().as_ref() {
                        Some(VmValue::List(values)) => values.len(),
                        Some(VmValue::String(bytes) | VmValue::Bytes(bytes)) => bytes.len(),
                        _ => 0,
                    };
                    let normalized = if raw_offset < 0 {
                        (length as i64).checked_add(raw_offset)
                    } else {
                        Some(raw_offset)
                    };
                    let offset = normalized
                        .and_then(|value| usize::try_from(value).ok())
                        .filter(|value| *value < length)
                        .ok_or_else(|| error("index out of range", index.span))?;
                    match current.borrow().as_ref() {
                        Some(VmValue::List(values)) => values
                            .get(offset)
                            .cloned()
                            .ok_or_else(|| error("index out of range", place.span))?,
                        Some(VmValue::String(bytes) | VmValue::Bytes(bytes)) => {
                            let byte = bytes
                                .get(offset)
                                .copied()
                                .ok_or_else(|| error("index out of range", place.span))?;
                            slot(Some(VmValue::Integer(i64::from(byte))))
                        }
                        _ => return Err(error("index base is not list", place.span)),
                    }
                }
            };
        }
        Ok(current)
    }

    fn memory_place(&mut self, place: &CanonicalPlace) -> Result<Option<(u64, TypeId)>, VmError> {
        let base = match &place.base {
            CanonicalPlaceBase::Pointer(code) => self.eval(code)?,
            CanonicalPlaceBase::Local(local) => self
                .local(*local, place.span)?
                .borrow()
                .as_ref()
                .cloned()
                .unwrap_or(VmValue::Unit),
            CanonicalPlaceBase::Global(symbol) => self
                .global(*symbol, place.span)?
                .borrow()
                .as_ref()
                .cloned()
                .unwrap_or(VmValue::Unit),
        };
        let (mut address, mut pointee, view_length) = match base {
            VmValue::MemoryPointer { address, pointee } => (address, pointee, None),
            VmValue::View {
                address,
                length,
                element,
                ..
            } => (address, element, Some(length)),
            _ => return Ok(None),
        };
        for projection in &place.projections {
            match projection {
                CanonicalProjection::Field { ty, offset, .. }
                | CanonicalProjection::Payload { ty, offset, .. } => {
                    address = address
                        .checked_add(u64::from(*offset))
                        .ok_or_else(|| error("runtime memory address overflow", place.span))?;
                    pointee = *ty;
                }
                CanonicalProjection::Index {
                    index,
                    element,
                    element_size,
                } => {
                    let index_span = index.span;
                    let index = self.eval(index)?.integer(index_span)?;
                    let index = u64::try_from(index)
                        .map_err(|_| error("negative runtime memory index", index_span))?;
                    if view_length.is_some_and(|length| index >= length) {
                        return Err(error("view index out of bounds", index_span));
                    }
                    let offset = index
                        .checked_mul(u64::from(*element_size))
                        .ok_or_else(|| error("runtime memory index overflow", place.span))?;
                    address = address
                        .checked_add(offset)
                        .ok_or_else(|| error("runtime memory address overflow", place.span))?;
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
    ) -> Result<VmValue, VmError> {
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
                self.memory_values
                    .get(&address)
                    .expect("checked aggregate memory value")
                    .clone()
            };
            if stored_ty != ty {
                return Err(error(
                    &format!(
                        "runtime memory aggregate type mismatch: stored {:?}, requested {:?}",
                        stored_ty, ty
                    ),
                    span,
                ));
            }
            return Ok(value);
        }
        let canonical = self.project.semantic.type_of(ty);
        let size = layout_of(&self.project.semantic, ty)
            .map_err(|failure| error(&failure.to_string(), span))?
            .size;
        let bytes = self
            .memory
            .read_address(address, u64::from(size))
            .map_err(|failure| error(&format!("runtime memory read failed: {failure:?}"), span))?;
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
                Ok(VmValue::Integer(value))
            }
            CanonicalType::Bool => Ok(VmValue::Bool(raw != 0)),
            CanonicalType::Pointer { pointee, .. } => Ok(if raw == 0 {
                VmValue::Null
            } else {
                VmValue::MemoryPointer {
                    address: raw,
                    pointee: *pointee,
                }
            }),
            CanonicalType::Struct { symbol, .. } => {
                let definition = self
                    .project
                    .semantic
                    .definitions
                    .iter()
                    .find(|definition| definition.symbol == *symbol)
                    .ok_or_else(|| error("runtime struct definition is missing", span))?;
                let TypeDefKind::Struct { fields, .. } = &definition.kind else {
                    unreachable!()
                };
                let fields = fields.clone();
                let mut values = BTreeMap::new();
                for field in fields {
                    let (field_ty, offset) =
                        struct_field_layout(&self.project.semantic, ty, field.id)
                            .map_err(|failure| error(&failure.to_string(), span))?;
                    let value = self.read_memory_value(
                        address + u64::from(offset),
                        field_ty,
                        action,
                        span,
                    )?;
                    values.insert(field.id, slot(Some(value)));
                }
                Ok(VmValue::Struct(values))
            }
            _ => Err(error("runtime memory value is not representable", span)),
        }
    }

    fn write_memory_value(
        &mut self,
        address: u64,
        ty: TypeId,
        value: VmValue,
        span: Span,
    ) -> Result<(), VmError> {
        let size = layout_of(&self.project.semantic, ty)
            .map_err(|failure| error(&failure.to_string(), span))?
            .size as usize;
        let raw = match value {
            VmValue::Integer(value) => value as u64,
            VmValue::Bool(value) => u64::from(value),
            VmValue::MemoryPointer { address, .. } => address,
            VmValue::Null => 0,
            VmValue::Struct(fields) => {
                if !matches!(
                    self.project.semantic.type_of(ty),
                    CanonicalType::Struct { .. }
                ) {
                    return Err(error("runtime aggregate store type mismatch", span));
                }
                self.remove_memory_values(address, size as u64);
                for (field, value) in fields {
                    let (field_ty, offset) = struct_field_layout(&self.project.semantic, ty, field)
                        .map_err(|failure| error(&failure.to_string(), span))?;
                    let value = value
                        .borrow_mut()
                        .take()
                        .ok_or_else(|| error("runtime struct field was moved", span))?;
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
            .map_err(|failure| error(&format!("runtime memory write failed: {failure:?}"), span))
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

    fn call(
        &mut self,
        key: &FunctionInstanceKey,
        args: Vec<VmValue>,
        span: Span,
    ) -> Result<VmValue, VmError> {
        let function = self
            .project
            .functions
            .iter()
            .find(|function| function.key == *key)
            .cloned()
            .ok_or_else(|| error("function instance missing", span))?;
        if self.call_depth >= 16 {
            return Err(error("maximum call depth exceeded", span));
        }
        if function.declaration_only || args.len() != function.parameters.len() {
            return Err(error("invalid canonical function entry", span));
        }
        let mut locals = BTreeMap::new();
        for local in &function.locals {
            locals.insert(local.id, slot(None));
        }
        for (parameter, value) in function.parameters.iter().zip(args) {
            locals
                .entry(parameter.id)
                .or_insert_with(|| slot(None))
                .borrow_mut()
                .replace(value);
        }
        self.frames.push(Frame { locals });
        self.call_depth += 1;
        let result = self.block(&function.body);
        self.call_depth -= 1;
        self.frames.pop();
        match result? {
            Flow::Normal => Ok(VmValue::Unit),
            Flow::Return(value) => Ok(value),
            _ => Err(error("loop control escaped function", span)),
        }
    }

    fn call_closure(
        &mut self,
        closure: &VmClosure,
        args: Vec<VmValue>,
        span: Span,
    ) -> Result<VmValue, VmError> {
        if args.len() != closure.parameters.len() {
            return Err(error("closure arity mismatch", span));
        }
        let mut locals = closure.captures.clone();
        for (parameter, value) in closure.parameters.iter().zip(args) {
            locals.insert(parameter.id, slot(Some(value)));
        }
        self.frames.push(Frame { locals });
        let result = self.block(&closure.body);
        self.frames.pop();
        match result? {
            Flow::Normal => Ok(VmValue::Unit),
            Flow::Return(value) => Ok(value),
            _ => Err(error("loop control escaped closure", span)),
        }
    }

    fn local(&self, id: LocalId, span: Span) -> Result<Slot, VmError> {
        self.frames
            .last()
            .and_then(|frame| frame.locals.get(&id))
            .cloned()
            .ok_or_else(|| error("local slot missing", span))
    }
    fn ensure_local(&mut self, id: LocalId, span: Span) -> Result<Slot, VmError> {
        let frame = self
            .frames
            .last_mut()
            .ok_or_else(|| error("execution frame missing", span))?;
        Ok(frame.locals.entry(id).or_insert_with(|| slot(None)).clone())
    }
    fn global(&self, id: SymbolId, span: Span) -> Result<Slot, VmError> {
        self.globals
            .get(&id)
            .cloned()
            .ok_or_else(|| error("global slot missing", span))
    }
}

impl VmValue {
    fn display(&self) -> String {
        match self {
            Self::Unit => String::new(),
            Self::Integer(v) => v.to_string(),
            Self::Float(v) => v.to_string(),
            Self::Bool(v) => v.to_string(),
            Self::String(v) => String::from_utf8_lossy(v).into_owned(),
            Self::Bytes(v) => hex::encode(v),
            Self::Buffer(_) => "<buffer>".into(),
            Self::Null => "null".into(),
            Self::List(_) => "<list>".into(),
            Self::Struct(_) => "<struct>".into(),
            Self::Enum { .. } => "<enum>".into(),
            Self::Function(_) => "<fn>".into(),
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
            Self::EarlyReturn(v) => v.display(),
        }
    }
    fn truthy(&self, span: Span) -> Result<bool, VmError> {
        match self {
            Self::Bool(v) => Ok(*v),
            Self::Integer(v) => Ok(*v != 0),
            _ => Err(error("condition is not bool", span)),
        }
    }
    fn integer(&self, span: Span) -> Result<i64, VmError> {
        match self {
            Self::Integer(v) => Ok(*v),
            Self::MemoryPointer { address, .. } => {
                i64::try_from(*address).map_err(|_| error("pointer does not fit integer", span))
            }
            _ => Err(error("expected integer", span)),
        }
    }
}

fn slot(value: Option<VmValue>) -> Slot {
    Rc::new(RefCell::new(value))
}
fn dereference_slot(mut current: Slot) -> Slot {
    loop {
        let next = match current.borrow().as_ref() {
            Some(VmValue::Reference(next)) => Some(next.clone()),
            _ => None,
        };
        match next {
            Some(next) => current = next,
            None => return current,
        }
    }
}
fn read(source: &Slot, action: OwnershipAction, span: Span) -> Result<VmValue, VmError> {
    match action {
        OwnershipAction::Move => source
            .borrow_mut()
            .take()
            .ok_or_else(|| error("use after move", span)),
        OwnershipAction::Copy => source
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| error("use after move", span)),
        OwnershipAction::Borrow | OwnershipAction::BorrowMut => {
            Ok(VmValue::Reference(source.clone()))
        }
    }
}
fn pop(stack: &mut Vec<VmValue>, span: Span) -> Result<VmValue, VmError> {
    stack
        .pop()
        .ok_or_else(|| error("canonical stack underflow", span))
}
fn pop_many(stack: &mut Vec<VmValue>, count: usize, span: Span) -> Result<Vec<VmValue>, VmError> {
    if stack.len() < count {
        return Err(error("canonical stack underflow", span));
    }
    Ok(stack.split_off(stack.len() - count))
}
fn length(value: VmValue, span: Span) -> Result<VmValue, VmError> {
    match value {
        VmValue::String(v) | VmValue::Bytes(v) => Ok(VmValue::Integer(v.len() as i64)),
        VmValue::Buffer(v) => Ok(VmValue::Integer(v.borrow().len() as i64)),
        VmValue::List(v) => Ok(VmValue::Integer(v.len() as i64)),
        VmValue::View { length, .. } => Ok(VmValue::Integer(length as i64)),
        VmValue::Reference(value) => {
            let value = value
                .borrow()
                .as_ref()
                .cloned()
                .ok_or_else(|| error("len operand moved", span))?;
            length(value, span)
        }
        _ => Err(error("len operand invalid", span)),
    }
}
fn unary(op: &UnaryOp, value: VmValue, span: Span) -> Result<VmValue, VmError> {
    match (op, value) {
        (UnaryOp::Neg, VmValue::Integer(v)) => v
            .checked_neg()
            .map(VmValue::Integer)
            .ok_or_else(|| error("integer overflow", span)),
        (UnaryOp::Neg, VmValue::Float(v)) => Ok(VmValue::Float(-v)),
        (UnaryOp::Not, v) => Ok(VmValue::Bool(!v.truthy(span)?)),
        (UnaryOp::BitNot, VmValue::Integer(v)) => Ok(VmValue::Integer(!v)),
        (UnaryOp::Deref, VmValue::Reference(v)) => v
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| error("dead dereference", span)),
        (UnaryOp::AddressOf, VmValue::Reference(v)) => Ok(VmValue::Reference(v)),
        _ => Err(error("invalid unary operand", span)),
    }
}
fn binary(left: VmValue, op: &BinOp, right: VmValue, span: Span) -> Result<VmValue, VmError> {
    use BinOp::*;
    match (left, right) {
        (VmValue::MemoryPointer { address, .. }, VmValue::Null)
        | (VmValue::Null, VmValue::MemoryPointer { address, .. })
            if matches!(op, EqEq | NotEq) =>
        {
            Ok(VmValue::Bool(if *op == EqEq {
                address == 0
            } else {
                address != 0
            }))
        }
        (
            VmValue::MemoryPointer { address: left, .. },
            VmValue::MemoryPointer { address: right, .. },
        ) if matches!(op, EqEq | NotEq) => Ok(VmValue::Bool(if *op == EqEq {
            left == right
        } else {
            left != right
        })),
        (VmValue::Null, VmValue::Null) if matches!(op, EqEq | NotEq) => {
            Ok(VmValue::Bool(*op == EqEq))
        }
        (VmValue::Integer(a), VmValue::Integer(b)) => match op {
            Add => a.checked_add(b).map(VmValue::Integer),
            Sub => a.checked_sub(b).map(VmValue::Integer),
            Mul => a.checked_mul(b).map(VmValue::Integer),
            Div => a.checked_div(b).map(VmValue::Integer),
            Rem => a.checked_rem(b).map(VmValue::Integer),
            BitAnd => return Ok(VmValue::Integer(a & b)),
            BitOr => return Ok(VmValue::Integer(a | b)),
            BitXor => return Ok(VmValue::Integer(a ^ b)),
            Shl => a.checked_shl(b as u32).map(VmValue::Integer),
            Shr => a.checked_shr(b as u32).map(VmValue::Integer),
            EqEq => return Ok(VmValue::Bool(a == b)),
            NotEq => return Ok(VmValue::Bool(a != b)),
            Lt => return Ok(VmValue::Bool(a < b)),
            LtEq => return Ok(VmValue::Bool(a <= b)),
            Gt => return Ok(VmValue::Bool(a > b)),
            GtEq => return Ok(VmValue::Bool(a >= b)),
        }
        .ok_or_else(|| {
            error(
                if matches!(op, Div | Rem) && b == 0 {
                    "division by zero"
                } else if matches!(op, Shl | Shr) {
                    "invalid shift"
                } else {
                    "integer overflow"
                },
                span,
            )
        }),
        (VmValue::String(mut a), VmValue::String(b)) => match op {
            Add => {
                a.extend_from_slice(&b);
                Ok(VmValue::String(a))
            }
            EqEq => Ok(VmValue::Bool(a == b)),
            NotEq => Ok(VmValue::Bool(a != b)),
            _ => Err(error("invalid string operation", span)),
        },
        (VmValue::Bool(a), VmValue::Bool(b)) if matches!(op, EqEq | NotEq) => {
            Ok(VmValue::Bool(if *op == EqEq { a == b } else { a != b }))
        }
        _ => Err(error("binary operand mismatch", span)),
    }
}
fn cast(value: VmValue, target: &CanonicalType, span: Span) -> Result<VmValue, VmError> {
    match (value, target) {
        (VmValue::Integer(v), CanonicalType::Integer(_)) => Ok(VmValue::Integer(v)),
        (VmValue::Integer(v), CanonicalType::Bool) => Ok(VmValue::Bool(v != 0)),
        (VmValue::Bool(v), CanonicalType::Integer(_)) => Ok(VmValue::Integer(i64::from(v))),
        (VmValue::Integer(v), CanonicalType::Pointer { pointee, .. }) if v >= 0 => Ok(if v == 0 {
            VmValue::Null
        } else {
            VmValue::MemoryPointer {
                address: v as u64,
                pointee: *pointee,
            }
        }),
        (VmValue::Null, CanonicalType::Pointer { .. }) => Ok(VmValue::Null),
        (VmValue::MemoryPointer { address, .. }, CanonicalType::Integer(_)) => {
            Ok(VmValue::Integer(address as i64))
        }
        (VmValue::Reference(_), CanonicalType::Integer(_)) => Ok(VmValue::Integer(1)),
        (VmValue::MemoryPointer { address, .. }, CanonicalType::Pointer { pointee, .. }) => {
            Ok(VmValue::MemoryPointer {
                address,
                pointee: *pointee,
            })
        }
        _ => Err(error("unsupported cast", span)),
    }
}
fn error(message: &str, span: Span) -> VmError {
    VmError {
        msg: message.into(),
        span,
    }
}

fn to_host_value(value: VmValue) -> Result<dbyte_interp::Value, VmError> {
    Ok(match value {
        VmValue::Unit => dbyte_interp::Value::Void,
        VmValue::Integer(value) => dbyte_interp::Value::Int(value),
        VmValue::Float(value) => dbyte_interp::Value::Float(value),
        VmValue::Bool(value) => dbyte_interp::Value::Bool(value),
        VmValue::String(value) => dbyte_interp::Value::Str(
            String::from_utf8(value)
                .map_err(|_| error("host string is not UTF-8", Span::zero()))?,
        ),
        VmValue::Bytes(value) => dbyte_interp::Value::Bytes(value),
        VmValue::Buffer(value) => dbyte_interp::Value::Buffer(value),
        VmValue::Null => dbyte_interp::Value::Null,
        VmValue::List(values) => dbyte_interp::Value::List(
            values
                .into_iter()
                .map(|value| {
                    value
                        .borrow()
                        .as_ref()
                        .cloned()
                        .ok_or_else(|| error("host list item moved", Span::zero()))
                        .and_then(to_host_value)
                })
                .collect::<Result<_, _>>()?,
        ),
        VmValue::Reference(value) => value
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| error("host borrow is dead", Span::zero()))
            .and_then(to_host_value)?,
        _ => {
            return Err(error(
                "host capability argument is unsupported",
                Span::zero(),
            ))
        }
    })
}

fn from_host_value(value: dbyte_interp::Value) -> Result<VmValue, VmError> {
    Ok(match value {
        dbyte_interp::Value::Void => VmValue::Unit,
        dbyte_interp::Value::Int(value) => VmValue::Integer(value),
        dbyte_interp::Value::Float(value) => VmValue::Float(value),
        dbyte_interp::Value::Bool(value) => VmValue::Bool(value),
        dbyte_interp::Value::Str(value) => VmValue::String(value.into_bytes()),
        dbyte_interp::Value::Bytes(value) => VmValue::Bytes(value),
        dbyte_interp::Value::Buffer(value) => VmValue::Buffer(value),
        dbyte_interp::Value::Null => VmValue::Null,
        dbyte_interp::Value::List(values) => VmValue::List(
            values
                .into_iter()
                .map(from_host_value)
                .map(|value| value.map(|value| slot(Some(value))))
                .collect::<Result<_, _>>()?,
        ),
        _ => return Err(error("host capability result is unsupported", Span::zero())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbyte_compiler::lower_concrete_project_to_bytecode;
    use dbyte_hir::{
        analyze_project, load_source_project_from_program, monomorphize_project, ModuleId, Profile,
        ProjectInput, ProjectModuleInput, TargetDataLayout,
    };
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;

    fn bytecode(source: &str) -> CanonicalBytecodeProject {
        let program = Parser::new(Lexer::new(source).tokenize().unwrap())
            .parse_program()
            .unwrap();
        let analyzed = analyze_project(
            &ProjectInput {
                root: ModuleId(0),
                modules: vec![ProjectModuleInput {
                    id: ModuleId(0),
                    name: "root".into(),
                    program,
                    imports: Vec::new(),
                }],
            },
            Profile::Vm,
            TargetDataLayout::I686,
        )
        .unwrap();
        let concrete = monomorphize_project(&analyzed).unwrap();
        lower_concrete_project_to_bytecode(&concrete).unwrap()
    }

    #[test]
    fn canonical_vm_executes_ids_cleanup_and_result_propagation() {
        let source = "struct Resource:\n    value: int\n\ndrop fn destroy(value: borrow mut Resource):\n    print((*value).value)\n\nfn attempt(input: Result[int, int]) -> Result[int, int]:\n    defer []:\n        print(\"CLEAN\")\n    let value = input?\n    return Result[int, int].Ok(value)\n\nlet resource = Resource(value = 1)\nresource = Resource(value = 2)\ndrop(resource)\nlet result = attempt(Result[int, int].Err(7))\nmatch result:\n    case Result.Ok(value):\n        print(value)\n    case Result.Err(error):\n        print(error)\n";
        let execution = execute_canonical_bytecode(&bytecode(source)).unwrap();
        assert_eq!(execution.output, "1\n2\nCLEAN\n7\n");
    }

    #[test]
    fn canonical_vm_executes_generic_closure_format_and_for() {
        let source = "fn id[T](value: T) -> T:\n    return value\n\nlet total = 0\nfor value in [1, 2, 3]:\n    total = total + value\nprint(\"TOTAL {total}\")\nlet factor: int = 2\nlet add = fn [borrow factor](value: int) -> int:\n    return value + factor\nlet answer = add(id[int](40))\nprint(\"VALUE {answer}\")\n";
        let execution = execute_canonical_bytecode(&bytecode(source)).unwrap();
        assert_eq!(execution.output, "TOTAL 6\nVALUE 42\n");
    }

    #[test]
    fn hosted_and_vm_execute_the_same_concrete_hir_authority() {
        let source = "struct Resource:\n    value: int\n\ndrop fn destroy(value: borrow mut Resource):\n    print((*value).value)\n\nfn id[T](value: T) -> T:\n    return value\n\nlet resource = Resource(value = 1)\nresource = Resource(value = 2)\ndrop(resource)\nprint(id[int](40) + 2)\n";
        let program = Parser::new(Lexer::new(source).tokenize().unwrap())
            .parse_program()
            .unwrap();
        let analyzed = analyze_project(
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
        let concrete = monomorphize_project(&analyzed).unwrap();
        let hosted = dbyte_interp::execute_concrete_project(&concrete).unwrap();
        let bytecode = lower_concrete_project_to_bytecode(&concrete).unwrap();
        let vm = execute_canonical_bytecode(&bytecode).unwrap();
        assert_eq!(hosted.output, "1\n2\n42\n");
        assert_eq!(vm.output, hosted.output);
    }

    #[test]
    fn hosted_and_vm_share_runtime_memory_events_from_concrete_hir() {
        let source = format!(
            "{}\n{}",
            "fn system_alloc_raw(size: usize, alignment: usize) -> ptr[u8]:\n    return __dbyte_runtime.memory_alloc(size, alignment)\n\nfn system_resize_raw(pointer: ptr[u8], old_size: usize, new_size: usize, alignment: usize) -> ptr[u8]:\n    return __dbyte_runtime.memory_resize(pointer, old_size, new_size, alignment)\n\nfn system_free_raw(pointer: ptr[u8], size: usize, alignment: usize) -> bool:\n    return __dbyte_runtime.memory_free(pointer, size, alignment)\n\nfn system_read8_raw(pointer: ptr[u8], offset: usize) -> u8:\n    return __dbyte_runtime.memory_read8(pointer, offset)\n\nfn system_write8_raw(pointer: ptr[u8], offset: usize, value: u8) -> unit:\n    __dbyte_runtime.memory_write8(pointer, offset, value)\n    return unit\n",
            "struct OwnedAllocation:\n    pointer: ptr[u8]\n    size: usize\n\ndrop fn destroy_owned(value: borrow mut OwnedAllocation):\n    system_free_raw((*value).pointer, (*value).size, 4 as usize)\n    return unit\n\nstruct OwnedPair:\n    left: OwnedAllocation\n    right: OwnedAllocation\n\nlet pointer = system_alloc_raw(4 as usize, 4 as usize)\nlet typed: ptr[u32] = pointer as usize as ptr[u32]\ntyped[0 as usize] = 42 as u32\nprint(typed[0 as usize])\nprint(system_read8_raw(pointer, 0 as usize))\nlet replacement = system_resize_raw(pointer, 4 as usize, 8 as usize, 4 as usize)\nprint(system_read8_raw(replacement, 0 as usize))\nprint(system_free_raw(replacement, 8 as usize, 4 as usize))\nlet stable = system_alloc_raw(4 as usize, 4 as usize)\nsystem_write8_raw(stable, 0 as usize, 7 as u8)\nlet failed = system_resize_raw(stable, 4 as usize, 16 as usize, 4 as usize)\nprint(failed)\nprint(system_read8_raw(stable, 0 as usize))\nprint(system_free_raw(stable, 4 as usize, 4 as usize))\nlet owned_pair = OwnedPair(left = OwnedAllocation(pointer = system_alloc_raw(4 as usize, 4 as usize), size = 4 as usize), right = OwnedAllocation(pointer = system_alloc_raw(4 as usize, 4 as usize), size = 4 as usize))\n"
        );
        let program = Parser::new(Lexer::new(&source).tokenize().unwrap())
            .parse_program()
            .unwrap();
        let analyzed = analyze_project(
            &ProjectInput {
                root: ModuleId(0),
                modules: vec![ProjectModuleInput {
                    id: ModuleId(0),
                    name: "runtime_parity".into(),
                    program,
                    imports: Vec::new(),
                }],
            },
            Profile::Hosted,
            TargetDataLayout::I686,
        )
        .unwrap();
        let concrete = monomorphize_project(&analyzed).unwrap();
        let mut hosted_memory = DeterministicMemory::system();
        hosted_memory.set_budget(Some(12));
        let hosted =
            dbyte_interp::execute_concrete_project_with_memory(&concrete, hosted_memory).unwrap();
        let bytecode = lower_concrete_project_to_bytecode(&concrete).unwrap();
        let mut vm_memory = DeterministicMemory::system();
        vm_memory.set_budget(Some(12));
        let vm = execute_canonical_bytecode_with_memory(&bytecode, vm_memory).unwrap();
        assert_eq!(hosted.output, "42\n42\n42\ntrue\nnull\n7\ntrue\n");
        assert_eq!(vm.output, hosted.output);
        assert_eq!(vm.allocator_events, hosted.allocator_events);
        assert_eq!(hosted.live_allocations, 0);
        assert_eq!(vm.live_allocations, 0);
        assert_eq!(hosted.allocator_events.len(), 11);
        let left_address = match hosted.allocator_events[7] {
            AllocationEvent::Alloc { address, .. } => address,
            ref event => panic!("expected left owned allocation, found {event:?}"),
        };
        let right_address = match hosted.allocator_events[8] {
            AllocationEvent::Alloc { address, .. } => address,
            ref event => panic!("expected right owned allocation, found {event:?}"),
        };
        assert!(matches!(
            hosted.allocator_events[9],
            AllocationEvent::Free { address, .. } if address == right_address
        ));
        assert!(matches!(
            hosted.allocator_events[10],
            AllocationEvent::Free { address, .. } if address == left_address
        ));
    }

    #[test]
    fn versioned_array_runtime_executes_through_system_allocator_on_hosted_and_vm() {
        let source = include_str!("../../../runtime/v12.1.0/hosted_parity.dby");
        let program = Parser::new(Lexer::new(source).tokenize().unwrap())
            .parse_program()
            .unwrap();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../runtime/v12.1.0/runtime_parity.dby");
        let input = load_source_project_from_program(&root, program).unwrap();
        let analyzed = analyze_project(&input, Profile::Hosted, TargetDataLayout::I686).unwrap();
        let freestanding = analyze_project(&input, Profile::I686None, TargetDataLayout::I686)
            .expect_err("freestanding constructors must not select SystemAllocator");
        assert!(freestanding.iter().any(|diagnostic| {
            diagnostic.code == "E_PROFILE_CAPABILITY"
                && diagnostic.message.contains("__dbyte_runtime")
        }));
        let concrete = monomorphize_project(&analyzed).unwrap();

        let mut hosted_memory = DeterministicMemory::system();
        hosted_memory.set_budget(Some(256));
        let hosted =
            dbyte_interp::execute_concrete_project_with_memory(&concrete, hosted_memory).unwrap();
        let bytecode = lower_concrete_project_to_bytecode(&concrete).unwrap();
        let mut vm_memory = DeterministicMemory::system();
        vm_memory.set_budget(Some(256));
        let vm = execute_canonical_bytecode_with_memory(&bytecode, vm_memory).unwrap();

        assert_eq!(
            hosted.output,
            "40\n2\n2\n3\n2\n2\n2\n105\n2\n2\n3\nINVALID\n1\n"
        );
        assert_eq!(vm.output, hosted.output);
        assert_eq!(vm.allocator_events, hosted.allocator_events);
        assert_eq!(hosted.live_allocations, 0);
        assert_eq!(hosted.allocator_events.len(), 22);
        assert_eq!(
            hosted
                .allocator_events
                .iter()
                .filter(|event| matches!(event, AllocationEvent::Alloc { .. }))
                .count(),
            11
        );
        assert_eq!(
            hosted
                .allocator_events
                .iter()
                .filter(|event| matches!(event, AllocationEvent::Free { .. }))
                .count(),
            11
        );
        let nested_string = match hosted.allocator_events[18] {
            AllocationEvent::Alloc { address, .. } => address,
            ref event => panic!("expected nested String allocation, found {event:?}"),
        };
        let nested_array = match hosted.allocator_events[19] {
            AllocationEvent::Alloc { address, .. } => address,
            ref event => panic!("expected nested Array backing allocation, found {event:?}"),
        };
        assert!(matches!(
            hosted.allocator_events[20],
            AllocationEvent::Free { address, .. } if address == nested_string
        ));
        assert!(matches!(
            hosted.allocator_events[21],
            AllocationEvent::Free { address, .. } if address == nested_array
        ));
    }
}
