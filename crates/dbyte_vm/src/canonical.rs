//! Non-recursive VM execution for flat canonical bytecode emitted from Concrete HIR.

use crate::VmError;
use dbyte_ast::{BinOp, CaptureMode, MatchMode, Span, UnaryOp};
use dbyte_bytecode::*;
use dbyte_hir::{
    integer_semantics::{is_signed, IntegerBinaryOp, IntegerError, IntegerValue},
    layout_of, struct_field_layout, CanonicalType, FieldId, FunctionInstanceId, HirIntrinsic,
    IntegerType, LocalId, OwnershipAction, SymbolId, TypeDefKind, TypeId, VariantId,
};
use dbyte_interp::memory::{AllocationEvent, DeterministicMemory};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

type Slot = Rc<RefCell<Option<VmValue>>>;

#[derive(Debug, Clone)]
enum ResolvedPlace {
    Local { frame: usize, local: usize },
    Global(usize),
    Cell(Slot),
    Memory { address: u64, ty: TypeId },
}

#[derive(Debug, Clone)]
enum VmValue {
    Unit,
    Integer(i64),
    Float(f64),
    F32Bits(u32),
    F64Bits(u64),
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
    Function(FunctionInstanceId),
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
    Place(ResolvedPlace),
}

#[derive(Debug)]
struct VmClosure {
    closure: dbyte_hir::ClosureId,
    captures: Vec<(LocalId, Slot)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalVmExecution {
    pub output: String,
    pub allocator_events: Vec<AllocationEvent>,
    pub live_allocations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CanonicalVmMetrics {
    pub total_opcodes: u64,
    pub per_opcode: BTreeMap<&'static str, u64>,
    pub function_calls: u64,
    pub allocations: u64,
    pub bytecode_ops: usize,
    pub peak_frames: usize,
    pub peak_value_stack: usize,
    pub peak_live_allocations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfiledCanonicalVmExecution {
    pub execution: CanonicalVmExecution,
    pub metrics: CanonicalVmMetrics,
}

enum LocalSlot {
    Inline(Option<VmValue>),
    Cell(Slot),
}

#[derive(Clone, Copy)]
enum CodeUnit {
    Entry,
    Function(FunctionInstanceId),
    Closure(dbyte_hir::ClosureId),
}

struct IteratorState {
    local: LocalId,
    items: Vec<VmValue>,
    next: usize,
    previous: Option<VmValue>,
}

struct Frame {
    code: CodeUnit,
    ip: usize,
    locals: Vec<LocalSlot>,
    stack_base: usize,
    iterators: Vec<IteratorState>,
    match_restore: Vec<Vec<(LocalId, Option<VmValue>)>>,
}

struct CanonicalVm<'a, const PROFILE: bool> {
    project: &'a CanonicalBytecodeProject,
    globals: Vec<Option<Slot>>,
    frames: Vec<Frame>,
    frame_pool: Vec<Vec<LocalSlot>>,
    output: String,
    script_args: Vec<String>,
    memory: DeterministicMemory,
    memory_values: BTreeMap<u64, (TypeId, VmValue)>,
    value_stack: Vec<VmValue>,
    metrics: CanonicalVmMetrics,
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
    execute_canonical_bytecode_inner_profiled::<false>(project, script_args, memory)
        .map(|result| result.execution)
}

pub fn execute_canonical_bytecode_profiled(
    project: &CanonicalBytecodeProject,
) -> Result<ProfiledCanonicalVmExecution, VmError> {
    execute_canonical_bytecode_inner_profiled::<true>(
        project,
        Vec::new(),
        DeterministicMemory::system(),
    )
}

fn execute_canonical_bytecode_inner_profiled<const PROFILE: bool>(
    project: &CanonicalBytecodeProject,
    script_args: Vec<String>,
    memory: DeterministicMemory,
) -> Result<ProfiledCanonicalVmExecution, VmError> {
    let mut vm = CanonicalVm::<PROFILE> {
        project,
        globals: Vec::new(),
        frames: Vec::new(),
        frame_pool: Vec::new(),
        output: String::new(),
        script_args,
        memory,
        memory_values: BTreeMap::new(),
        value_stack: Vec::with_capacity(32),
        metrics: CanonicalVmMetrics {
            bytecode_ops: bytecode_op_count(project),
            peak_frames: 1,
            ..CanonicalVmMetrics::default()
        },
    };
    for global in &project.globals {
        let index = global.symbol.0 as usize;
        if vm.globals.len() <= index {
            vm.globals.resize_with(index + 1, || None);
        }
        vm.globals[index] = Some(slot(None));
    }
    let locals = vm.allocate_locals(&project.entry.frame);
    vm.frames.push(Frame {
        code: CodeUnit::Entry,
        ip: 0,
        locals,
        stack_base: 0,
        iterators: Vec::new(),
        match_restore: Vec::new(),
    });
    vm.run()?;
    if PROFILE {
        vm.metrics.allocations = vm
            .memory
            .telemetry()
            .iter()
            .filter(|event| matches!(event, AllocationEvent::Alloc { .. }))
            .count() as u64;
    }
    Ok(ProfiledCanonicalVmExecution {
        execution: CanonicalVmExecution {
            output: vm.output,
            allocator_events: vm.memory.telemetry().to_vec(),
            live_allocations: vm.memory.live_allocations(),
        },
        metrics: vm.metrics,
    })
}

fn bytecode_op_count(project: &CanonicalBytecodeProject) -> usize {
    project.entry.instructions.len()
        + project
            .functions
            .iter()
            .map(|function| function.body.instructions.len())
            .sum::<usize>()
        + project
            .closures
            .iter()
            .map(|closure| closure.body.instructions.len())
            .sum::<usize>()
}

fn opcode_name(op: &CanonicalOpcode) -> &'static str {
    match op {
        CanonicalOpcode::PushUnit => "PUSH_UNIT",
        CanonicalOpcode::PushInteger(_) => "PUSH_INTEGER",
        CanonicalOpcode::PushLayout { .. } => "PUSH_LAYOUT",
        CanonicalOpcode::PushFloat(_) => "PUSH_FLOAT",
        CanonicalOpcode::PushBool(_) => "PUSH_BOOL",
        CanonicalOpcode::PushString(_) => "PUSH_STRING",
        CanonicalOpcode::PushBytes(_) => "PUSH_BYTES",
        CanonicalOpcode::PushNull => "PUSH_NULL",
        CanonicalOpcode::LoadLocal { .. } => "LOAD_LOCAL",
        CanonicalOpcode::LoadGlobal { .. } => "LOAD_GLOBAL",
        CanonicalOpcode::PushFunction { .. } => "PUSH_FUNCTION",
        CanonicalOpcode::MakeList(_) => "MAKE_LIST",
        CanonicalOpcode::Binary { operand, .. } => match operand {
            CanonicalOperandType::Integer(IntegerType::I8) => "BINARY_I8",
            CanonicalOperandType::Integer(IntegerType::U8) => "BINARY_U8",
            CanonicalOperandType::Integer(IntegerType::I16) => "BINARY_I16",
            CanonicalOperandType::Integer(IntegerType::U16) => "BINARY_U16",
            CanonicalOperandType::Integer(IntegerType::I32) => "BINARY_I32",
            CanonicalOperandType::Integer(IntegerType::U32) => "BINARY_U32",
            CanonicalOperandType::Integer(IntegerType::I64) => "BINARY_I64",
            CanonicalOperandType::Integer(IntegerType::U64) => "BINARY_U64",
            CanonicalOperandType::Integer(IntegerType::Usize) => "BINARY_USIZE",
            CanonicalOperandType::Integer(IntegerType::LegacyInt) => "BINARY_LEGACY_INT",
            CanonicalOperandType::Bool => "BINARY_BOOL",
            CanonicalOperandType::Float => "BINARY_F64",
            CanonicalOperandType::F32 => "BINARY_F32",
            CanonicalOperandType::F64 => "BINARY_F64_EXPLICIT",
            CanonicalOperandType::Pointer => "BINARY_PTR",
            CanonicalOperandType::String => "BINARY_STRING",
            CanonicalOperandType::Other(_) => "BINARY_AGGREGATE",
        },
        CanonicalOpcode::Unary { .. } => "UNARY",
        CanonicalOpcode::Cast(_) => "CAST",
        CanonicalOpcode::Bitcast { .. } => "BITCAST",
        CanonicalOpcode::Length => "LENGTH",
        CanonicalOpcode::CallDirect { .. } => "CALL_DIRECT",
        CanonicalOpcode::CallIndirect { .. } => "CALL_INDIRECT",
        CanonicalOpcode::Intrinsic { .. } => "INTRINSIC",
        CanonicalOpcode::HostCall { .. } => "HOST_CALL",
        CanonicalOpcode::Format { .. } => "FORMAT",
        CanonicalOpcode::MakeStruct { .. } => "MAKE_STRUCT",
        CanonicalOpcode::MakeEnum { .. } => "MAKE_ENUM",
        CanonicalOpcode::ResolvePlace(_) => "RESOLVE_PLACE",
        CanonicalOpcode::ReadPlace(_) => "READ_PLACE",
        CanonicalOpcode::BorrowPlace { .. } => "BORROW_PLACE",
        CanonicalOpcode::StorePlace => "STORE_PLACE",
        CanonicalOpcode::ClearPlace => "CLEAR_PLACE",
        CanonicalOpcode::StoreLocal(_) => "STORE_LOCAL",
        CanonicalOpcode::StoreGlobal(_) => "STORE_GLOBAL",
        CanonicalOpcode::Pop => "POP",
        CanonicalOpcode::Jump(_) => "JUMP",
        CanonicalOpcode::JumpIfFalse(_) => "JUMP_IF_FALSE",
        CanonicalOpcode::ResultUnwrapOrJump { .. } => "RESULT_UNWRAP",
        CanonicalOpcode::IteratorBegin { .. } => "ITERATOR_BEGIN",
        CanonicalOpcode::IteratorNext { .. } => "ITERATOR_NEXT",
        CanonicalOpcode::IteratorEnd { .. } => "ITERATOR_END",
        CanonicalOpcode::MatchDispatch { .. } => "MATCH_DISPATCH",
        CanonicalOpcode::MatchRestore => "MATCH_RESTORE",
        CanonicalOpcode::MakeClosure { .. } => "MAKE_CLOSURE",
        CanonicalOpcode::Return => "RETURN",
        CanonicalOpcode::Halt => "HALT",
    }
}

fn body(project: &CanonicalBytecodeProject, code: CodeUnit) -> &CanonicalBytecodeBody {
    match code {
        CodeUnit::Entry => &project.entry,
        CodeUnit::Function(instance) => &project.functions[instance.0 as usize].body,
        CodeUnit::Closure(closure) => &project.closures[closure.0 as usize].body,
    }
}

impl<const PROFILE: bool> CanonicalVm<'_, PROFILE> {
    fn run(&mut self) -> Result<(), VmError> {
        // Keep the immutable bytecode project outside the mutable VM borrow so
        // the dispatcher can execute instructions in place. Cloning an opcode
        // on every dispatch made flat bytecode slower precisely on the tight
        // local/branch workloads this representation is meant to improve.
        let project = self.project;
        while !self.frames.is_empty() {
            let instruction = {
                let frame = self.frames.last().expect("checked frame");
                body(project, frame.code)
                    .instructions
                    .get(frame.ip)
                    .ok_or_else(|| error("instruction pointer out of range", Span::zero()))?
            };
            self.frames.last_mut().expect("checked frame").ip += 1;
            if PROFILE {
                self.metrics.total_opcodes += 1;
                *self
                    .metrics
                    .per_opcode
                    .entry(opcode_name(&instruction.opcode))
                    .or_default() += 1;
            }
            // The common scalar/local/control instructions are interpreted by
            // reference. Cold aggregate/host instructions retain the simpler
            // owned executor, so their variable-sized operands are cloned only
            // when they actually execute.
            let handled = match &instruction.opcode {
                CanonicalOpcode::PushUnit => {
                    self.value_stack.push(VmValue::Unit);
                    true
                }
                CanonicalOpcode::PushInteger(value) => {
                    self.value_stack.push(VmValue::Integer(*value));
                    true
                }
                CanonicalOpcode::PushBool(value) => {
                    self.value_stack.push(VmValue::Bool(*value));
                    true
                }
                CanonicalOpcode::LoadLocal { local, action } => {
                    let value = self.read_local(*local, *action, instruction.span)?;
                    self.value_stack.push(value);
                    true
                }
                CanonicalOpcode::StoreLocal(local) => {
                    let value = pop(&mut self.value_stack, instruction.span)?;
                    self.write_local(*local, value, instruction.span)?;
                    true
                }
                CanonicalOpcode::Binary { op, operand } => {
                    let right = pop(&mut self.value_stack, instruction.span)?;
                    let left = pop(&mut self.value_stack, instruction.span)?;
                    self.value_stack.push(binary(
                        left,
                        op,
                        right,
                        *operand,
                        self.project.semantic.layout,
                        instruction.span,
                    )?);
                    true
                }
                CanonicalOpcode::Pop => {
                    pop(&mut self.value_stack, instruction.span)?;
                    true
                }
                CanonicalOpcode::Jump(target) => {
                    self.set_ip(*target, instruction.span)?;
                    true
                }
                CanonicalOpcode::JumpIfFalse(target) => {
                    if !pop(&mut self.value_stack, instruction.span)?.truthy(instruction.span)? {
                        self.set_ip(*target, instruction.span)?;
                    }
                    true
                }
                _ => false,
            };
            if !handled {
                self.execute(instruction.clone())?;
            }
            if PROFILE {
                self.metrics.peak_value_stack =
                    self.metrics.peak_value_stack.max(self.value_stack.len());
                self.metrics.peak_live_allocations = self
                    .metrics
                    .peak_live_allocations
                    .max(self.memory.live_allocations());
            }
        }
        Ok(())
    }

    fn execute(&mut self, instruction: CanonicalInstruction) -> Result<(), VmError> {
        let span = instruction.span;
        match instruction.opcode {
            CanonicalOpcode::PushUnit => self.value_stack.push(VmValue::Unit),
            CanonicalOpcode::PushInteger(value) => self.value_stack.push(VmValue::Integer(value)),
            CanonicalOpcode::PushLayout { target, alignment } => {
                let layout = layout_of(&self.project.semantic, target)
                    .map_err(|e| error(&e.to_string(), span))?;
                self.value_stack
                    .push(VmValue::Integer(i64::from(if alignment {
                        layout.align
                    } else {
                        layout.size
                    })));
            }
            CanonicalOpcode::PushFloat(value) => self.value_stack.push(VmValue::Float(value)),
            CanonicalOpcode::PushBool(value) => self.value_stack.push(VmValue::Bool(value)),
            CanonicalOpcode::PushString(value) => self.value_stack.push(VmValue::String(value)),
            CanonicalOpcode::PushBytes(value) => self.value_stack.push(VmValue::Bytes(value)),
            CanonicalOpcode::PushNull => self.value_stack.push(VmValue::Null),
            CanonicalOpcode::LoadLocal { local, action } => {
                let value = self.read_local(local, action, span)?;
                self.value_stack.push(value);
            }
            CanonicalOpcode::LoadGlobal { symbol, action } => {
                self.value_stack
                    .push(read(&self.global(symbol, span)?, action, span)?)
            }
            CanonicalOpcode::PushFunction { instance, .. } => {
                self.value_stack.push(VmValue::Function(instance))
            }
            CanonicalOpcode::MakeList(count) => {
                let values = pop_many(&mut self.value_stack, count, span)?;
                self.value_stack.push(VmValue::List(
                    values.into_iter().map(|value| slot(Some(value))).collect(),
                ));
            }
            CanonicalOpcode::Binary { op, operand } => {
                let right = pop(&mut self.value_stack, span)?;
                let left = pop(&mut self.value_stack, span)?;
                self.value_stack.push(binary(
                    left,
                    &op,
                    right,
                    operand,
                    self.project.semantic.layout,
                    span,
                )?);
            }
            CanonicalOpcode::Unary { op, operand: _ } => {
                let value = pop(&mut self.value_stack, span)?;
                self.value_stack.push(unary(&op, value, span)?);
            }
            CanonicalOpcode::Cast(target) => {
                let value = pop(&mut self.value_stack, span)?;
                self.value_stack
                    .push(cast(value, self.project.semantic.type_of(target), span)?);
            }
            CanonicalOpcode::Bitcast { source, target } => {
                let value = pop(&mut self.value_stack, span)?;
                self.value_stack.push(bitcast(
                    value,
                    self.project.semantic.type_of(source),
                    self.project.semantic.type_of(target),
                    self.project.semantic.layout,
                    span,
                )?);
            }
            CanonicalOpcode::Length => {
                let value = pop(&mut self.value_stack, span)?;
                self.value_stack.push(length(value, span)?);
            }
            CanonicalOpcode::CallDirect {
                callee,
                argument_count,
                ..
            } => {
                let args = pop_many(&mut self.value_stack, argument_count, span)?;
                self.push_function(callee, args, span)?;
            }
            CanonicalOpcode::CallIndirect { argument_count, .. } => {
                let args = pop_many(&mut self.value_stack, argument_count, span)?;
                let callee = pop(&mut self.value_stack, span)?;
                match callee {
                    VmValue::Function(instance) => self.push_function(instance, args, span)?,
                    VmValue::Closure(closure) => self.push_closure(&closure, args, span)?,
                    _ => return Err(error("indirect callee is not callable", span)),
                }
            }
            CanonicalOpcode::Intrinsic {
                intrinsic,
                argument_count,
                result,
            } if intrinsic.is_runtime_host_memory() => {
                let args = pop_many(&mut self.value_stack, argument_count, span)?;
                let value = self.runtime_memory(intrinsic, &args, result, span)?;
                self.value_stack.push(value);
            }
            CanonicalOpcode::Intrinsic {
                intrinsic,
                argument_count,
                result,
            } if intrinsic.is_runtime_view() => {
                let args = pop_many(&mut self.value_stack, argument_count, span)?;
                let value = self.runtime_view(intrinsic, &args, result, span)?;
                self.value_stack.push(value);
            }
            CanonicalOpcode::Intrinsic { .. } => {
                return Err(error("target intrinsic in hosted VM bytecode", span))
            }
            CanonicalOpcode::HostCall {
                capability,
                argument_count,
            } => {
                let args = pop_many(&mut self.value_stack, argument_count, span)?;
                match capability {
                    dbyte_hir::HostCapabilityId::Print => {
                        for (index, value) in args.iter().enumerate() {
                            if index != 0 {
                                self.output.push(' ');
                            }
                            self.output.push_str(&value.display());
                        }
                        self.output.push('\n');
                        self.value_stack.push(VmValue::Unit);
                    }
                    dbyte_hir::HostCapabilityId::EnvArgs => {
                        self.value_stack.push(VmValue::List(
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
                            dbyte_interp::execute_resolved_host_capability(capability, &args)
                                .map_err(|message| error(&message, span))?;
                        self.value_stack.push(from_host_value(value)?);
                    }
                }
            }
            CanonicalOpcode::Format { parts, value_count } => {
                let values = pop_many(&mut self.value_stack, value_count, span)?;
                let mut values = values.into_iter();
                let mut bytes = Vec::new();
                for part in parts {
                    match part {
                        CanonicalFormatPart::Literal(value) => bytes.extend_from_slice(&value),
                        CanonicalFormatPart::Value => bytes.extend_from_slice(
                            values
                                .next()
                                .ok_or_else(|| error("format stack underflow", span))?
                                .display()
                                .as_bytes(),
                        ),
                    }
                }
                self.value_stack.push(VmValue::String(bytes));
            }
            CanonicalOpcode::MakeStruct { fields, .. } => {
                let values = pop_many(&mut self.value_stack, fields.len(), span)?;
                self.value_stack.push(VmValue::Struct(
                    fields
                        .iter()
                        .copied()
                        .zip(values)
                        .map(|(field, value)| (field, slot(Some(value))))
                        .collect(),
                ));
            }
            CanonicalOpcode::MakeEnum {
                variant,
                payload_count,
                ..
            } => {
                let values = pop_many(&mut self.value_stack, payload_count, span)?;
                self.value_stack.push(VmValue::Enum {
                    variant,
                    payload: values.into_iter().map(|value| slot(Some(value))).collect(),
                });
            }
            CanonicalOpcode::ResolvePlace(place) => {
                let place = self.resolve_place(&place, span)?;
                self.value_stack.push(VmValue::Place(place));
            }
            CanonicalOpcode::ReadPlace(action) => {
                let place = take_place(&mut self.value_stack, span)?;
                let value = self.read_place(place, action, span)?;
                self.value_stack.push(value);
            }
            CanonicalOpcode::BorrowPlace { .. } => {
                let place = take_place(&mut self.value_stack, span)?;
                let value = self.borrow_place(place, span)?;
                self.value_stack.push(value);
            }
            CanonicalOpcode::StorePlace => {
                let place = take_place(&mut self.value_stack, span)?;
                let value = pop(&mut self.value_stack, span)?;
                self.write_place(place, value, span)?;
            }
            CanonicalOpcode::ClearPlace => {
                let place = take_place(&mut self.value_stack, span)?;
                self.clear_place(place, span)?;
            }
            CanonicalOpcode::StoreLocal(local) => {
                let value = pop(&mut self.value_stack, span)?;
                self.write_local(local, value, span)?;
            }
            CanonicalOpcode::StoreGlobal(symbol) => {
                let value = pop(&mut self.value_stack, span)?;
                self.global(symbol, span)?.borrow_mut().replace(value);
            }
            CanonicalOpcode::Pop => {
                pop(&mut self.value_stack, span)?;
            }
            CanonicalOpcode::Jump(target) => self.set_ip(target, span)?,
            CanonicalOpcode::JumpIfFalse(target) => {
                if !pop(&mut self.value_stack, span)?.truthy(span)? {
                    self.set_ip(target, span)?;
                }
            }
            CanonicalOpcode::ResultUnwrapOrJump {
                ok_variant,
                err_variant,
                error_target,
            } => {
                let value = pop(&mut self.value_stack, span)?;
                let VmValue::Enum { variant, payload } = value else {
                    return Err(error("? operand is not Result", span));
                };
                if variant == ok_variant {
                    self.value_stack.push(
                        payload
                            .first()
                            .map(|value| {
                                value
                                    .borrow_mut()
                                    .take()
                                    .ok_or_else(|| error("Ok payload moved", span))
                            })
                            .transpose()?
                            .unwrap_or(VmValue::Unit),
                    );
                } else if variant == err_variant {
                    self.value_stack.push(VmValue::Enum { variant, payload });
                    self.set_ip(error_target, span)?;
                } else {
                    return Err(error("invalid Result tag", span));
                }
            }
            CanonicalOpcode::IteratorBegin { local, .. } => {
                let value = pop(&mut self.value_stack, span)?;
                let items = iterable_items(value, span)?;
                let previous = self.take_local(local, span)?;
                self.frames
                    .last_mut()
                    .expect("active frame")
                    .iterators
                    .push(IteratorState {
                        local,
                        items,
                        next: 0,
                        previous,
                    });
            }
            CanonicalOpcode::IteratorNext { local, end_target } => {
                let next = {
                    let state = self
                        .frames
                        .last_mut()
                        .expect("active frame")
                        .iterators
                        .last_mut()
                        .ok_or_else(|| error("iterator state missing", span))?;
                    if state.local != local {
                        return Err(error("iterator local mismatch", span));
                    }
                    if state.next == state.items.len() {
                        None
                    } else {
                        let value = state.items[state.next].clone();
                        state.next += 1;
                        Some(value)
                    }
                };
                if let Some(value) = next {
                    self.write_local(local, value, span)?;
                } else {
                    self.set_ip(end_target, span)?;
                }
            }
            CanonicalOpcode::IteratorEnd { local } => {
                let state = self
                    .frames
                    .last_mut()
                    .expect("active frame")
                    .iterators
                    .pop()
                    .ok_or_else(|| error("iterator state missing", span))?;
                if state.local != local {
                    return Err(error("iterator local mismatch", span));
                }
                self.replace_local(local, state.previous, span)?;
            }
            CanonicalOpcode::MatchDispatch { mode, arms } => {
                let value = pop(&mut self.value_stack, span)?;
                let VmValue::Enum { variant, payload } = value else {
                    return Err(error("match value is not enum", span));
                };
                let arm = arms
                    .into_iter()
                    .find(|arm| arm.variant.is_none_or(|candidate| candidate == variant))
                    .ok_or_else(|| error("exhaustive match has no arm", span))?;
                let mut restore = Vec::new();
                for (binding, source) in arm.bindings.iter().zip(payload) {
                    let value = match mode {
                        MatchMode::Move => source
                            .borrow_mut()
                            .take()
                            .ok_or_else(|| error("match payload moved", binding.span))?,
                        MatchMode::Borrow | MatchMode::BorrowMut => VmValue::Reference(source),
                    };
                    restore.push((binding.id, self.take_local(binding.id, binding.span)?));
                    self.write_local(binding.id, value, binding.span)?;
                }
                self.frames
                    .last_mut()
                    .expect("active frame")
                    .match_restore
                    .push(restore);
                self.set_ip(arm.target, span)?;
            }
            CanonicalOpcode::MatchRestore => {
                let restore = self
                    .frames
                    .last_mut()
                    .expect("active frame")
                    .match_restore
                    .pop()
                    .ok_or_else(|| error("match restore state missing", span))?;
                for (local, prior) in restore.into_iter().rev() {
                    self.replace_local(local, prior, span)?;
                }
            }
            CanonicalOpcode::MakeClosure { closure, captures } => {
                let mut captured = Vec::with_capacity(captures.len());
                for capture in captures {
                    let stored = match capture.mode {
                        CaptureMode::Move => slot(Some(
                            self.take_local(capture.local, capture.span)?
                                .ok_or_else(|| error("capture moved", capture.span))?,
                        )),
                        CaptureMode::Borrow | CaptureMode::BorrowMut => {
                            self.local_cell(capture.local, capture.span)?
                        }
                    };
                    captured.push((capture.local, stored));
                }
                self.value_stack.push(VmValue::Closure(Rc::new(VmClosure {
                    closure,
                    captures: captured,
                })));
            }
            CanonicalOpcode::Return => self.return_frame(span)?,
            CanonicalOpcode::Halt => {
                if !matches!(
                    self.frames.last().map(|frame| frame.code),
                    Some(CodeUnit::Entry)
                ) {
                    return Err(error("HALT outside entry bytecode", span));
                }
                self.pop_frame(false, span)?;
            }
        }
        Ok(())
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

    fn resolve_place(
        &mut self,
        place: &CanonicalPlace,
        span: Span,
    ) -> Result<ResolvedPlace, VmError> {
        let operand_count = usize::from(matches!(place.base, CanonicalPlaceBase::StackPointer))
            + place
                .projections
                .iter()
                .filter(|projection| matches!(projection, CanonicalProjection::StackIndex { .. }))
                .count();
        let operands = pop_many(&mut self.value_stack, operand_count, span)?;
        let mut operands = operands.into_iter();
        let frame = self
            .frames
            .len()
            .checked_sub(1)
            .ok_or_else(|| error("execution frame missing", span))?;
        let mut resolved = match place.base {
            CanonicalPlaceBase::Local(local) => ResolvedPlace::Local {
                frame,
                local: local.0 as usize,
            },
            CanonicalPlaceBase::Global(symbol) => ResolvedPlace::Global(symbol.0 as usize),
            CanonicalPlaceBase::StackPointer => match operands
                .next()
                .ok_or_else(|| error("pointer place operand missing", span))?
            {
                VmValue::Reference(slot) => ResolvedPlace::Cell(dereference_slot(slot)),
                VmValue::MemoryPointer { address, pointee } => ResolvedPlace::Memory {
                    address,
                    ty: pointee,
                },
                VmValue::View {
                    address, element, ..
                } => ResolvedPlace::Memory {
                    address,
                    ty: element,
                },
                _ => return Err(error("pointer place is invalid", span)),
            },
        };
        let mut view_length = self
            .place_value(&resolved, span)?
            .and_then(|value| match value {
                VmValue::View { length, .. } => Some(length),
                _ => None,
            });
        for projection in &place.projections {
            resolved = match projection {
                CanonicalProjection::Field { field, ty, offset } => match resolved {
                    ResolvedPlace::Memory { address, .. } => ResolvedPlace::Memory {
                        address: address
                            .checked_add(u64::from(*offset))
                            .ok_or_else(|| error("runtime memory address overflow", span))?,
                        ty: *ty,
                    },
                    other => {
                        let cell = self.place_cell(other, span)?;
                        let mut value = cell.borrow_mut();
                        if value.is_none() {
                            *value = Some(VmValue::Struct(BTreeMap::new()));
                        }
                        match value.as_mut() {
                            Some(VmValue::Struct(fields)) => ResolvedPlace::Cell(
                                fields.entry(*field).or_insert_with(|| slot(None)).clone(),
                            ),
                            _ => return Err(error("field base is not struct", span)),
                        }
                    }
                },
                CanonicalProjection::Payload {
                    variant,
                    index,
                    ty,
                    offset,
                } => match resolved {
                    ResolvedPlace::Memory { address, .. } => ResolvedPlace::Memory {
                        address: address
                            .checked_add(u64::from(*offset))
                            .ok_or_else(|| error("runtime memory address overflow", span))?,
                        ty: *ty,
                    },
                    other => {
                        let cell = self.place_cell(other, span)?;
                        let borrowed = cell.borrow();
                        match borrowed.as_ref() {
                            Some(VmValue::Enum {
                                variant: active,
                                payload,
                            }) if active == variant => ResolvedPlace::Cell(
                                payload
                                    .get(*index as usize)
                                    .cloned()
                                    .ok_or_else(|| error("payload missing", span))?,
                            ),
                            Some(VmValue::Enum { .. }) => {
                                return Err(error("inactive payload", span))
                            }
                            _ => return Err(error("payload base is not enum", span)),
                        }
                    }
                },
                CanonicalProjection::StackIndex {
                    element,
                    element_size,
                } => {
                    let raw = operands
                        .next()
                        .ok_or_else(|| error("index operand missing", span))?
                        .integer(span)?;
                    if !matches!(resolved, ResolvedPlace::Memory { .. }) {
                        if let Some(value) = self.place_value(&resolved, span)? {
                            match value {
                                VmValue::MemoryPointer { address, pointee } => {
                                    resolved = ResolvedPlace::Memory {
                                        address,
                                        ty: pointee,
                                    };
                                    view_length = None;
                                }
                                VmValue::View {
                                    address,
                                    length,
                                    element,
                                    ..
                                } => {
                                    resolved = ResolvedPlace::Memory {
                                        address,
                                        ty: element,
                                    };
                                    view_length = Some(length);
                                }
                                _ => {}
                            }
                        }
                    }
                    match resolved {
                        ResolvedPlace::Memory { address, .. } => {
                            let index = u64::try_from(raw)
                                .map_err(|_| error("negative runtime memory index", span))?;
                            if view_length.is_some_and(|length| index >= length) {
                                return Err(error("view index out of bounds", span));
                            }
                            let offset = index
                                .checked_mul(u64::from(*element_size))
                                .ok_or_else(|| error("runtime memory index overflow", span))?;
                            ResolvedPlace::Memory {
                                address: address.checked_add(offset).ok_or_else(|| {
                                    error("runtime memory address overflow", span)
                                })?,
                                ty: *element,
                            }
                        }
                        other => {
                            let cell = self.place_cell(other, span)?;
                            let length = match cell.borrow().as_ref() {
                                Some(VmValue::List(values)) => values.len(),
                                Some(VmValue::String(bytes) | VmValue::Bytes(bytes)) => bytes.len(),
                                _ => 0,
                            };
                            let normalized = if raw < 0 {
                                (length as i64).checked_add(raw)
                            } else {
                                Some(raw)
                            };
                            let index = normalized
                                .and_then(|value| usize::try_from(value).ok())
                                .filter(|value| *value < length)
                                .ok_or_else(|| {
                                    error(
                                        &format!("index out of range: index={raw} length={length}"),
                                        span,
                                    )
                                })?;
                            let resolved = match cell.borrow().as_ref() {
                                Some(VmValue::List(values)) => {
                                    ResolvedPlace::Cell(values[index].clone())
                                }
                                Some(VmValue::String(bytes) | VmValue::Bytes(bytes)) => {
                                    ResolvedPlace::Cell(slot(Some(VmValue::Integer(i64::from(
                                        bytes[index],
                                    )))))
                                }
                                _ => return Err(error("index base is not list", span)),
                            };
                            resolved
                        }
                    }
                }
            };
            view_length = None;
        }
        Ok(resolved)
    }

    fn place_value(
        &mut self,
        place: &ResolvedPlace,
        span: Span,
    ) -> Result<Option<VmValue>, VmError> {
        Ok(match place {
            ResolvedPlace::Local { frame, local } => self.local_value(*frame, *local, span)?,
            ResolvedPlace::Global(index) => self
                .globals
                .get(*index)
                .and_then(Option::as_ref)
                .and_then(|slot| slot.borrow().clone()),
            ResolvedPlace::Cell(slot) => slot.borrow().clone(),
            ResolvedPlace::Memory { .. } => None,
        })
    }

    fn place_cell(&mut self, place: ResolvedPlace, span: Span) -> Result<Slot, VmError> {
        match place {
            ResolvedPlace::Local { frame, local } => self.local_cell_at(frame, local, span),
            ResolvedPlace::Global(index) => self
                .globals
                .get(index)
                .and_then(Option::as_ref)
                .cloned()
                .ok_or_else(|| error("global slot missing", span)),
            ResolvedPlace::Cell(slot) => Ok(slot),
            ResolvedPlace::Memory { .. } => Err(error("memory place is not a hosted cell", span)),
        }
    }

    fn read_place(
        &mut self,
        place: ResolvedPlace,
        action: OwnershipAction,
        span: Span,
    ) -> Result<VmValue, VmError> {
        match place {
            ResolvedPlace::Memory { address, ty } => {
                self.read_memory_value(address, ty, action, span)
            }
            ResolvedPlace::Local { frame, local } => self.read_local_at(frame, local, action, span),
            other => read(&self.place_cell(other, span)?, action, span),
        }
    }

    fn borrow_place(&mut self, place: ResolvedPlace, span: Span) -> Result<VmValue, VmError> {
        match place {
            ResolvedPlace::Memory { address, ty } => Ok(VmValue::MemoryPointer {
                address,
                pointee: ty,
            }),
            other => Ok(VmValue::Reference(self.place_cell(other, span)?)),
        }
    }

    fn write_place(
        &mut self,
        place: ResolvedPlace,
        value: VmValue,
        span: Span,
    ) -> Result<(), VmError> {
        match place {
            ResolvedPlace::Memory { address, ty } => {
                self.write_memory_value(address, ty, value, span)
            }
            ResolvedPlace::Local { frame, local } => {
                self.replace_local_at(frame, local, Some(value), span)
            }
            other => {
                self.place_cell(other, span)?.borrow_mut().replace(value);
                Ok(())
            }
        }
    }

    fn clear_place(&mut self, place: ResolvedPlace, span: Span) -> Result<(), VmError> {
        match place {
            ResolvedPlace::Memory { address, .. } => {
                self.memory_values.remove(&address);
                Ok(())
            }
            ResolvedPlace::Local { frame, local } => {
                self.replace_local_at(frame, local, None, span)
            }
            other => {
                self.place_cell(other, span)?.borrow_mut().take();
                Ok(())
            }
        }
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
            CanonicalType::F32 => Ok(VmValue::F32Bits(raw as u32)),
            CanonicalType::F64 => Ok(VmValue::F64Bits(raw)),
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
            VmValue::F32Bits(value) => u64::from(value),
            VmValue::F64Bits(value) => value,
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

    fn body(&self, code: CodeUnit) -> &CanonicalBytecodeBody {
        match code {
            CodeUnit::Entry => &self.project.entry,
            CodeUnit::Function(instance) => &self.project.functions[instance.0 as usize].body,
            CodeUnit::Closure(closure) => &self.project.closures[closure.0 as usize].body,
        }
    }

    fn allocate_locals(&mut self, layout: &CanonicalFrameLayout) -> Vec<LocalSlot> {
        let mut locals = self.frame_pool.pop().unwrap_or_default();
        locals.clear();
        locals.extend((0..layout.local_count).map(|index| {
            if layout.stable_cells.get(index).copied().unwrap_or(false) {
                LocalSlot::Cell(slot(None))
            } else {
                LocalSlot::Inline(None)
            }
        }));
        locals
    }

    fn push_function(
        &mut self,
        instance: FunctionInstanceId,
        args: Vec<VmValue>,
        span: Span,
    ) -> Result<(), VmError> {
        if self.frames.len().saturating_sub(1) >= 16 {
            return Err(error("maximum call depth exceeded", span));
        }
        let function = self
            .project
            .functions
            .get(instance.0 as usize)
            .ok_or_else(|| error("function instance missing", span))?;
        if function.instance != instance {
            return Err(error("function instance table is not canonical", span));
        }
        if function.declaration_only || args.len() != function.parameters.len() {
            return Err(error("invalid canonical function entry", span));
        }
        let parameters = function.parameters.clone();
        let layout = function.body.frame.clone();
        let mut locals = self.allocate_locals(&layout);
        for (parameter, value) in parameters.iter().zip(args) {
            replace_local_slot(&mut locals, parameter.id.0 as usize, Some(value), span)?;
        }
        let stack_base = self.value_stack.len();
        self.frames.push(Frame {
            code: CodeUnit::Function(instance),
            ip: 0,
            locals,
            stack_base,
            iterators: Vec::new(),
            match_restore: Vec::new(),
        });
        if PROFILE {
            self.metrics.function_calls += 1;
            self.metrics.peak_frames = self.metrics.peak_frames.max(self.frames.len());
        }
        Ok(())
    }

    fn push_closure(
        &mut self,
        closure: &VmClosure,
        args: Vec<VmValue>,
        span: Span,
    ) -> Result<(), VmError> {
        if self.frames.len().saturating_sub(1) >= 16 {
            return Err(error("maximum call depth exceeded", span));
        }
        let definition = self
            .project
            .closures
            .get(closure.closure.0 as usize)
            .ok_or_else(|| error("closure code missing", span))?;
        if definition.closure != closure.closure || args.len() != definition.parameters.len() {
            return Err(error("closure arity mismatch", span));
        }
        let parameters = definition.parameters.clone();
        let layout = definition.body.frame.clone();
        let mut locals = self.allocate_locals(&layout);
        for (local, value) in &closure.captures {
            let index = local.0 as usize;
            if index >= locals.len() {
                return Err(error("closure capture slot missing", span));
            }
            locals[index] = LocalSlot::Cell(value.clone());
        }
        for (parameter, value) in parameters.iter().zip(args) {
            replace_local_slot(&mut locals, parameter.id.0 as usize, Some(value), span)?;
        }
        let stack_base = self.value_stack.len();
        self.frames.push(Frame {
            code: CodeUnit::Closure(closure.closure),
            ip: 0,
            locals,
            stack_base,
            iterators: Vec::new(),
            match_restore: Vec::new(),
        });
        if PROFILE {
            self.metrics.function_calls += 1;
            self.metrics.peak_frames = self.metrics.peak_frames.max(self.frames.len());
        }
        Ok(())
    }

    fn return_frame(&mut self, span: Span) -> Result<(), VmError> {
        let value = pop(&mut self.value_stack, span)?;
        self.pop_frame(true, span)?;
        self.value_stack.push(value);
        Ok(())
    }

    fn pop_frame(&mut self, require_caller: bool, span: Span) -> Result<(), VmError> {
        let frame = self
            .frames
            .pop()
            .ok_or_else(|| error("execution frame missing", span))?;
        if require_caller && self.frames.is_empty() {
            return Err(error("function return escaped entry", span));
        }
        self.value_stack.truncate(frame.stack_base);
        let mut locals = frame.locals;
        for local in &mut locals {
            *local = LocalSlot::Inline(None);
        }
        self.frame_pool.push(locals);
        Ok(())
    }

    fn set_ip(&mut self, target: usize, span: Span) -> Result<(), VmError> {
        let code = self
            .frames
            .last()
            .ok_or_else(|| error("execution frame missing", span))?
            .code;
        if target >= self.body(code).instructions.len() {
            return Err(error("branch target out of range", span));
        }
        self.frames.last_mut().expect("checked frame").ip = target;
        Ok(())
    }

    fn read_local(
        &mut self,
        id: LocalId,
        action: OwnershipAction,
        span: Span,
    ) -> Result<VmValue, VmError> {
        let frame = self
            .frames
            .len()
            .checked_sub(1)
            .ok_or_else(|| error("execution frame missing", span))?;
        self.read_local_at(frame, id.0 as usize, action, span)
    }

    fn read_local_at(
        &mut self,
        frame: usize,
        local: usize,
        action: OwnershipAction,
        span: Span,
    ) -> Result<VmValue, VmError> {
        let slot = self
            .frames
            .get_mut(frame)
            .and_then(|frame| frame.locals.get_mut(local))
            .ok_or_else(|| error("local slot missing", span))?;
        match slot {
            LocalSlot::Inline(value) => match action {
                OwnershipAction::Move => value
                    .take()
                    .ok_or_else(|| error("value already moved", span)),
                _ => value
                    .clone()
                    .ok_or_else(|| error("value already moved", span)),
            },
            LocalSlot::Cell(value) => read(value, action, span),
        }
    }

    fn write_local(&mut self, id: LocalId, value: VmValue, span: Span) -> Result<(), VmError> {
        self.replace_local(id, Some(value), span)
    }

    fn replace_local(
        &mut self,
        id: LocalId,
        value: Option<VmValue>,
        span: Span,
    ) -> Result<(), VmError> {
        let frame = self
            .frames
            .len()
            .checked_sub(1)
            .ok_or_else(|| error("execution frame missing", span))?;
        self.replace_local_at(frame, id.0 as usize, value, span)
    }

    fn replace_local_at(
        &mut self,
        frame: usize,
        local: usize,
        value: Option<VmValue>,
        span: Span,
    ) -> Result<(), VmError> {
        let target = self
            .frames
            .get_mut(frame)
            .and_then(|frame| frame.locals.get_mut(local))
            .ok_or_else(|| error("local slot missing", span))?;
        match target {
            LocalSlot::Inline(current) => *current = value,
            LocalSlot::Cell(cell) => *cell.borrow_mut() = value,
        }
        Ok(())
    }

    fn take_local(&mut self, id: LocalId, span: Span) -> Result<Option<VmValue>, VmError> {
        let frame = self
            .frames
            .last_mut()
            .ok_or_else(|| error("execution frame missing", span))?;
        let target = frame
            .locals
            .get_mut(id.0 as usize)
            .ok_or_else(|| error("local slot missing", span))?;
        Ok(match target {
            LocalSlot::Inline(value) => value.take(),
            LocalSlot::Cell(cell) => cell.borrow_mut().take(),
        })
    }

    fn local_value(
        &self,
        frame: usize,
        local: usize,
        span: Span,
    ) -> Result<Option<VmValue>, VmError> {
        let target = self
            .frames
            .get(frame)
            .and_then(|frame| frame.locals.get(local))
            .ok_or_else(|| error("local slot missing", span))?;
        Ok(match target {
            LocalSlot::Inline(value) => value.clone(),
            LocalSlot::Cell(cell) => cell.borrow().clone(),
        })
    }

    fn local_cell(&mut self, id: LocalId, span: Span) -> Result<Slot, VmError> {
        let frame = self
            .frames
            .len()
            .checked_sub(1)
            .ok_or_else(|| error("execution frame missing", span))?;
        self.local_cell_at(frame, id.0 as usize, span)
    }

    fn local_cell_at(&mut self, frame: usize, local: usize, span: Span) -> Result<Slot, VmError> {
        let target = self
            .frames
            .get_mut(frame)
            .and_then(|frame| frame.locals.get_mut(local))
            .ok_or_else(|| error("local slot missing", span))?;
        if let LocalSlot::Inline(value) = target {
            let cell = slot(value.take());
            *target = LocalSlot::Cell(cell);
        }
        match target {
            LocalSlot::Cell(cell) => Ok(cell.clone()),
            LocalSlot::Inline(_) => unreachable!(),
        }
    }

    fn global(&self, id: SymbolId, span: Span) -> Result<Slot, VmError> {
        self.globals
            .get(id.0 as usize)
            .and_then(Option::as_ref)
            .cloned()
            .ok_or_else(|| error("global slot missing", span))
    }
}

fn replace_local_slot(
    locals: &mut [LocalSlot],
    index: usize,
    value: Option<VmValue>,
    span: Span,
) -> Result<(), VmError> {
    let target = locals
        .get_mut(index)
        .ok_or_else(|| error("local slot missing", span))?;
    match target {
        LocalSlot::Inline(current) => *current = value,
        LocalSlot::Cell(cell) => *cell.borrow_mut() = value,
    }
    Ok(())
}

fn take_place(stack: &mut Vec<VmValue>, span: Span) -> Result<ResolvedPlace, VmError> {
    match pop(stack, span)? {
        VmValue::Place(place) => Ok(place),
        _ => Err(error("resolved place missing", span)),
    }
}

fn iterable_items(value: VmValue, span: Span) -> Result<Vec<VmValue>, VmError> {
    match value {
        VmValue::List(items) => items
            .into_iter()
            .map(|item| {
                item.borrow()
                    .as_ref()
                    .cloned()
                    .ok_or_else(|| error("for item was moved", span))
            })
            .collect(),
        VmValue::String(bytes) => Ok(String::from_utf8(bytes)
            .map_err(|_| error("hosted str is not UTF-8", span))?
            .chars()
            .map(|value| VmValue::String(value.to_string().into_bytes()))
            .collect::<Vec<_>>()),
        _ => Err(error("value is not iterable", span)),
    }
}

impl VmValue {
    fn display(&self) -> String {
        match self {
            Self::Unit => String::new(),
            Self::Integer(v) => v.to_string(),
            Self::Float(v) => v.to_string(),
            Self::F32Bits(v) => format!("<f32 bits=0x{v:08x}>"),
            Self::F64Bits(v) => format!("<f64 bits=0x{v:016x}>"),
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
            Self::Place(_) => "<place>".into(),
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
        (UnaryOp::Neg, VmValue::F32Bits(value)) => Ok(VmValue::F32Bits(value ^ 0x8000_0000)),
        (UnaryOp::Neg, VmValue::F64Bits(value)) => {
            Ok(VmValue::F64Bits(value ^ 0x8000_0000_0000_0000))
        }
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
fn binary(
    left: VmValue,
    op: &BinOp,
    right: VmValue,
    operand: CanonicalOperandType,
    layout: dbyte_hir::TargetDataLayout,
    span: Span,
) -> Result<VmValue, VmError> {
    use BinOp::*;
    match (left, right) {
        (VmValue::MemoryPointer { address, pointee }, VmValue::Integer(offset))
            if matches!(op, Add | Sub) =>
        {
            // Concrete HIR has already scaled the element count to bytes.
            let bits = if *op == Add {
                address.wrapping_add(offset as u64)
            } else {
                address.wrapping_sub(offset as u64)
            };
            Ok(VmValue::MemoryPointer {
                address: IntegerValue::from_bits(bits, IntegerType::Usize, layout).bits(),
                pointee,
            })
        }
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
        (VmValue::Integer(a), VmValue::Integer(b)) => {
            let CanonicalOperandType::Integer(kind) = operand else {
                return Err(error("integer opcode has non-integer type", span));
            };
            if kind == IntegerType::LegacyInt {
                return legacy_integer_binary(a, op, b, span);
            }
            let left = IntegerValue::from_bits(a as u64, kind, layout);
            let right = IntegerValue::from_bits(b as u64, kind, layout);
            let comparison = if is_signed(kind) {
                (left.signed(layout), right.signed(layout))
            } else {
                (left.bits() as i64, right.bits() as i64)
            };
            match op {
                EqEq => Ok(VmValue::Bool(left.bits() == right.bits())),
                NotEq => Ok(VmValue::Bool(left.bits() != right.bits())),
                Lt if is_signed(kind) => Ok(VmValue::Bool(comparison.0 < comparison.1)),
                LtEq if is_signed(kind) => Ok(VmValue::Bool(comparison.0 <= comparison.1)),
                Gt if is_signed(kind) => Ok(VmValue::Bool(comparison.0 > comparison.1)),
                GtEq if is_signed(kind) => Ok(VmValue::Bool(comparison.0 >= comparison.1)),
                Lt => Ok(VmValue::Bool(left.bits() < right.bits())),
                LtEq => Ok(VmValue::Bool(left.bits() <= right.bits())),
                Gt => Ok(VmValue::Bool(left.bits() > right.bits())),
                GtEq => Ok(VmValue::Bool(left.bits() >= right.bits())),
                operation => {
                    let operation = match operation {
                        Add => IntegerBinaryOp::Add,
                        Sub => IntegerBinaryOp::Sub,
                        Mul => IntegerBinaryOp::Mul,
                        Div => IntegerBinaryOp::Div,
                        Rem => IntegerBinaryOp::Rem,
                        BitAnd => IntegerBinaryOp::BitAnd,
                        BitOr => IntegerBinaryOp::BitOr,
                        BitXor => IntegerBinaryOp::BitXor,
                        Shl => IntegerBinaryOp::ShiftLeft,
                        Shr => IntegerBinaryOp::ShiftRight,
                        EqEq | NotEq | Lt | LtEq | Gt | GtEq => unreachable!(),
                    };
                    left.binary(operation, right, layout)
                        .map(|value| VmValue::Integer(value.bits() as i64))
                        .map_err(|failure| {
                            error(
                                match failure {
                                    IntegerError::DivisionByZero => "division by zero",
                                    IntegerError::SignedDivisionOverflow => "integer overflow",
                                    IntegerError::LiteralOutOfRange
                                    | IntegerError::TypeMismatch => "integer type mismatch",
                                },
                                span,
                            )
                        })
                }
            }
        }
        (VmValue::F32Bits(left), VmValue::F32Bits(right)) => {
            let left = f32::from_bits(left);
            let right = f32::from_bits(right);
            match op {
                Add => Ok(VmValue::F32Bits((left + right).to_bits())),
                Sub => Ok(VmValue::F32Bits((left - right).to_bits())),
                Mul => Ok(VmValue::F32Bits((left * right).to_bits())),
                Div => Ok(VmValue::F32Bits((left / right).to_bits())),
                EqEq => Ok(VmValue::Bool(left == right)),
                NotEq => Ok(VmValue::Bool(left != right)),
                Lt => Ok(VmValue::Bool(left < right)),
                LtEq => Ok(VmValue::Bool(left <= right)),
                Gt => Ok(VmValue::Bool(left > right)),
                GtEq => Ok(VmValue::Bool(left >= right)),
                _ => Err(error("invalid f32 operation", span)),
            }
        }
        (VmValue::F64Bits(left), VmValue::F64Bits(right)) => {
            let left = f64::from_bits(left);
            let right = f64::from_bits(right);
            match op {
                Add => Ok(VmValue::F64Bits((left + right).to_bits())),
                Sub => Ok(VmValue::F64Bits((left - right).to_bits())),
                Mul => Ok(VmValue::F64Bits((left * right).to_bits())),
                Div => Ok(VmValue::F64Bits((left / right).to_bits())),
                EqEq => Ok(VmValue::Bool(left == right)),
                NotEq => Ok(VmValue::Bool(left != right)),
                Lt => Ok(VmValue::Bool(left < right)),
                LtEq => Ok(VmValue::Bool(left <= right)),
                Gt => Ok(VmValue::Bool(left > right)),
                GtEq => Ok(VmValue::Bool(left >= right)),
                _ => Err(error("invalid f64 operation", span)),
            }
        }
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

fn legacy_integer_binary(
    left: i64,
    op: &BinOp,
    right: i64,
    span: Span,
) -> Result<VmValue, VmError> {
    use BinOp::*;
    let checked = match op {
        Add => left.checked_add(right),
        Sub => left.checked_sub(right),
        Mul => left.checked_mul(right),
        Div => left.checked_div(right),
        Rem => left.checked_rem(right),
        Shl => left.checked_shl(right as u32),
        Shr => left.checked_shr(right as u32),
        BitAnd => return Ok(VmValue::Integer(left & right)),
        BitOr => return Ok(VmValue::Integer(left | right)),
        BitXor => return Ok(VmValue::Integer(left ^ right)),
        EqEq => return Ok(VmValue::Bool(left == right)),
        NotEq => return Ok(VmValue::Bool(left != right)),
        Lt => return Ok(VmValue::Bool(left < right)),
        LtEq => return Ok(VmValue::Bool(left <= right)),
        Gt => return Ok(VmValue::Bool(left > right)),
        GtEq => return Ok(VmValue::Bool(left >= right)),
    };
    checked.map(VmValue::Integer).ok_or_else(|| {
        error(
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
fn bitcast(
    value: VmValue,
    source: &CanonicalType,
    target: &CanonicalType,
    layout: dbyte_hir::TargetDataLayout,
    span: Span,
) -> Result<VmValue, VmError> {
    let bits = match (value, source) {
        (VmValue::Integer(value), CanonicalType::Integer(kind)) => {
            IntegerValue::from_bits(value as u64, *kind, layout).bits()
        }
        (VmValue::F32Bits(value), CanonicalType::F32) => u64::from(value),
        (VmValue::F64Bits(value), CanonicalType::F64) => value,
        _ => return Err(error("invalid exact-bit source representation", span)),
    };
    match target {
        CanonicalType::Integer(kind) => Ok(VmValue::Integer(
            IntegerValue::from_bits(bits, *kind, layout).bits() as i64,
        )),
        CanonicalType::F32 => Ok(VmValue::F32Bits(bits as u32)),
        CanonicalType::F64 => Ok(VmValue::F64Bits(bits)),
        _ => Err(error("invalid exact-bit target representation", span)),
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
    use dbyte_compiler::{
        lower_concrete_project_to_bytecode, lower_concrete_project_to_bytecode_at_level,
    };
    use dbyte_hir::{
        analyze_project, load_source_project, load_source_project_from_program,
        monomorphize_project, ModuleId, Profile, ProjectInput, ProjectModuleInput,
        TargetDataLayout,
    };
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;
    use std::collections::BTreeSet;

    #[test]
    fn nmos_6502_legal_opcode_matrix_is_complete_and_unique() {
        const MATRIX: &str = include_str!("../../../benchmarks/vm/6502/opcodes.csv");
        const EXPECTED: &[u8] = &[
            0x00, 0x01, 0x05, 0x06, 0x08, 0x09, 0x0a, 0x0d, 0x0e, 0x10, 0x11, 0x15, 0x16, 0x18,
            0x19, 0x1d, 0x1e, 0x20, 0x21, 0x24, 0x25, 0x26, 0x28, 0x29, 0x2a, 0x2c, 0x2d, 0x2e,
            0x30, 0x31, 0x35, 0x36, 0x38, 0x39, 0x3d, 0x3e, 0x40, 0x41, 0x45, 0x46, 0x48, 0x49,
            0x4a, 0x4c, 0x4d, 0x4e, 0x50, 0x51, 0x55, 0x56, 0x58, 0x59, 0x5d, 0x5e, 0x60, 0x61,
            0x65, 0x66, 0x68, 0x69, 0x6a, 0x6c, 0x6d, 0x6e, 0x70, 0x71, 0x75, 0x76, 0x78, 0x79,
            0x7d, 0x7e, 0x81, 0x84, 0x85, 0x86, 0x88, 0x8a, 0x8c, 0x8d, 0x8e, 0x90, 0x91, 0x94,
            0x95, 0x96, 0x98, 0x99, 0x9a, 0x9d, 0xa0, 0xa1, 0xa2, 0xa4, 0xa5, 0xa6, 0xa8, 0xa9,
            0xaa, 0xac, 0xad, 0xae, 0xb0, 0xb1, 0xb4, 0xb5, 0xb6, 0xb8, 0xb9, 0xba, 0xbc, 0xbd,
            0xbe, 0xc0, 0xc1, 0xc4, 0xc5, 0xc6, 0xc8, 0xc9, 0xca, 0xcc, 0xcd, 0xce, 0xd0, 0xd1,
            0xd5, 0xd6, 0xd8, 0xd9, 0xdd, 0xde, 0xe0, 0xe1, 0xe4, 0xe5, 0xe6, 0xe8, 0xe9, 0xea,
            0xec, 0xed, 0xee, 0xf0, 0xf1, 0xf5, 0xf6, 0xf8, 0xf9, 0xfd, 0xfe,
        ];
        let modes = [
            "implied",
            "accumulator",
            "immediate",
            "zero_page",
            "zero_page_x",
            "zero_page_y",
            "relative",
            "absolute",
            "absolute_x",
            "absolute_y",
            "indirect",
            "indexed_indirect",
            "indirect_indexed",
        ];
        let mut actual = Vec::new();
        for (line, record) in MATRIX.lines().enumerate().skip(1) {
            let fields = record.split(',').collect::<Vec<_>>();
            assert_eq!(fields.len(), 3, "malformed opcode row {}", line + 1);
            let opcode = u8::from_str_radix(fields[0], 16).expect("hex opcode");
            assert!(!fields[1].is_empty(), "empty mnemonic at row {}", line + 1);
            assert!(
                modes.contains(&fields[2]),
                "unknown mode at row {}",
                line + 1
            );
            actual.push(opcode);
        }
        let unique = actual.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(actual.len(), 151, "expected legal opcode rows");
        assert_eq!(unique.len(), 151, "duplicate legal opcode byte");
        assert_eq!(actual, EXPECTED, "missing or reordered legal opcode byte");
    }

    fn bytecode(source: &str) -> CanonicalBytecodeProject {
        bytecode_at(source, VmOptimizationLevel::O1)
    }

    fn bytecode_at(source: &str, optimization: VmOptimizationLevel) -> CanonicalBytecodeProject {
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
        lower_concrete_project_to_bytecode_at_level(&concrete, optimization).unwrap()
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
    fn flat_dispatcher_enforces_the_exact_call_depth_boundary() {
        let source = "fn recurse() -> int:\n    return recurse()\n\nprint(recurse())\n";
        let failure = execute_canonical_bytecode(&bytecode(source)).unwrap_err();
        assert_eq!(failure.msg, "maximum call depth exceeded");
        assert_eq!(failure.span, Span { line: 2, col: 13 });
    }

    #[test]
    fn flat_dispatcher_repeats_control_flow_without_rust_recursion() {
        let source = "let count: u32 = 0 as u32\nwhile count < (50000 as u32):\n    count = count + (1 as u32)\nprint(count)\n";
        let execution = execute_canonical_bytecode(&bytecode(source)).unwrap();
        assert_eq!(execution.output, "50000\n");
    }

    #[test]
    fn profiled_executor_is_semantically_identical_and_reports_canonical_ops() {
        let source = "fn add(value: int) -> int:\n    return value + 1\n\nlet answer = add(41)\nprint(answer)\n";
        let bytecode = bytecode(source);
        let normal = execute_canonical_bytecode(&bytecode).unwrap();
        let profiled = execute_canonical_bytecode_profiled(&bytecode).unwrap();
        assert_eq!(profiled.execution, normal);
        assert_eq!(profiled.metrics.function_calls, 1);
        assert!(profiled.metrics.total_opcodes >= 6);
        assert_eq!(profiled.metrics.per_opcode["CALL_DIRECT"], 1);
        assert_eq!(profiled.metrics.per_opcode["BINARY_LEGACY_INT"], 1);
        assert!(profiled.metrics.bytecode_ops >= profiled.metrics.total_opcodes as usize);
        assert_eq!(profiled.metrics.peak_frames, 2);
    }

    #[test]
    fn optimized_and_o0_bytecode_preserve_output_drop_and_allocator_events() {
        let source = "struct Resource:\n    value: int\n\ndrop fn destroy(value: borrow mut Resource):\n    print((*value).value)\n\nlet resource = Resource(value = 40 + 2)\nprint(6 * 7)\ndrop(resource)\n";
        let o0 = execute_canonical_bytecode(&bytecode_at(source, VmOptimizationLevel::O0)).unwrap();
        let optimized =
            execute_canonical_bytecode(&bytecode_at(source, VmOptimizationLevel::O1)).unwrap();
        assert_eq!(optimized, o0);
        assert_eq!(optimized.output, "42\n42\n");
    }

    #[test]
    fn strict_portable_same_source_executes_at_o0_and_o1() {
        let source =
            include_str!("../../../examples/native_v13_machine/portable_runtime_parity.dby");
        for (label, level) in [
            ("O0", VmOptimizationLevel::O0),
            ("O1", VmOptimizationLevel::O1),
        ] {
            let execution = execute_canonical_bytecode(&bytecode_at(source, level)).unwrap();
            assert_eq!(
                execution.output, "",
                "strict portable {label} output drifted"
            );
            assert_eq!(
                execution.live_allocations, 0,
                "strict portable {label} leaked"
            );
            println!("PORTABLE_VM_{label}=PASS");
        }
    }

    #[test]
    fn exact_bitcast_preserves_explicit_float_payload_bits_at_o0_and_o1() {
        let source = "fn round32(bits: u32) -> u32:\n    let value: f32 = bits as! f32\n    return value as! u32\n\nfn round64(bits: u64) -> u64:\n    let value: f64 = bits as! f64\n    return value as! u64\n\nlet negzero32: u32 = 2147483648 as u32\nlet qnan32: u32 = 2143289345 as u32\nlet qnan64: u64 = ((1 as u64) << 63) | ((2047 as u64) << 52) | ((1 as u64) << 51) | 74565 as u64\nprint(round32(negzero32) == negzero32)\nprint(round32(qnan32) == qnan32)\nprint(round64(qnan64) == qnan64)\n";
        for layout in [TargetDataLayout::I686, TargetDataLayout::X86_64] {
            let program = Parser::new(Lexer::new(source).tokenize().unwrap())
                .parse_program()
                .unwrap();
            let analyzed = analyze_project(
                &ProjectInput {
                    root: ModuleId(0),
                    modules: vec![ProjectModuleInput {
                        id: ModuleId(0),
                        name: "bitcast".into(),
                        program,
                        imports: Vec::new(),
                    }],
                },
                Profile::Vm,
                layout,
            )
            .unwrap();
            let concrete = monomorphize_project(&analyzed).unwrap();
            for level in [VmOptimizationLevel::O0, VmOptimizationLevel::O1] {
                let code = lower_concrete_project_to_bytecode_at_level(&concrete, level).unwrap();
                let execution = execute_canonical_bytecode(&code).unwrap();
                assert_eq!(
                    execution.output, "true\ntrue\ntrue\n",
                    "{layout:?} {level:?}"
                );
            }
        }
    }

    #[test]
    fn explicit_float_arithmetic_matches_ieee_bits_at_o0_and_o1() {
        let source = "let one: f32 = (1065353216 as u32) as! f32\nlet two: f32 = (1073741824 as u32) as! f32\nlet three: f32 = one + two\nprint((three as! u32) == (1077936128 as u32))\nlet zero: f32 = (0 as u32) as! f32\nprint(((-zero) as! u32) == (2147483648 as u32))\nlet inf: f32 = one / zero\nprint((inf as! u32) == (2139095040 as u32))\nlet nan: f32 = (2143289345 as u32) as! f32\nprint(nan != nan)\nprint(nan == nan)\nlet one64: f64 = (4607182418800017408 as u64) as! f64\nlet two64: f64 = (4611686018427387904 as u64) as! f64\nprint(((one64 + two64) as! u64) == (4613937818241073152 as u64))\n";
        for optimization in [VmOptimizationLevel::O0, VmOptimizationLevel::O1] {
            let execution = execute_canonical_bytecode(&bytecode_at(source, optimization)).unwrap();
            assert_eq!(execution.output, "true\ntrue\ntrue\ntrue\nfalse\ntrue\n");
        }
    }

    #[test]
    fn nmos_6502_matrix_and_trace_match_tree_contract_at_o0_and_o1() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (relative, expected) in [
            ("benchmarks/vm/6502/matrix.dby", "151\n151\n0\n"),
            (
                "benchmarks/vm/6502/trace.dby",
                "true\ntrue\ntrue\ntrue\ntrue\ntrue\n1\n",
            ),
        ] {
            let input = load_source_project(&root.join(relative)).unwrap();
            let analyzed = analyze_project(&input, Profile::Vm, TargetDataLayout::I686).unwrap();
            let concrete = monomorphize_project(&analyzed).unwrap();
            let o0 =
                lower_concrete_project_to_bytecode_at_level(&concrete, VmOptimizationLevel::O0)
                    .unwrap();
            let o1 =
                lower_concrete_project_to_bytecode_at_level(&concrete, VmOptimizationLevel::O1)
                    .unwrap();
            let o0_execution = execute_canonical_bytecode(&o0).unwrap();
            let o1_execution = execute_canonical_bytecode(&o1).unwrap();
            assert_eq!(o0_execution.output, expected, "O0 {relative}");
            assert_eq!(o1_execution, o0_execution, "O1 parity {relative}");
        }
    }

    #[test]
    fn pointer_stride_and_target_width_match_hosted_o0_o1() {
        for layout in [TargetDataLayout::I686, TargetDataLayout::X86_64] {
            for (ty, stride) in [("u8", 1), ("u16", 2), ("u32", 4), ("u64", 8)] {
                let source = format!("fn step[T](p: ptr[T], n: usize) -> ptr[T]:\n    return p + n\n\nlet p: ptr[{ty}] = (4096 as usize) as ptr[{ty}]\nlet q: ptr[{ty}] = step[{ty}](p, 3 as usize)\nprint(q as usize)\nprint((q - (2 as usize)) as usize)\nlet edge: ptr[u8] = (4294967295 as usize) as ptr[u8]\nprint((edge + (2 as usize)) as usize)\n");
                let program = Parser::new(Lexer::new(&source).tokenize().unwrap())
                    .parse_program()
                    .unwrap();
                let analyzed = analyze_project(
                    &ProjectInput {
                        root: ModuleId(0),
                        modules: vec![ProjectModuleInput {
                            id: ModuleId(0),
                            name: "pointer-test".into(),
                            program,
                            imports: Vec::new(),
                        }],
                    },
                    Profile::Hosted,
                    layout,
                )
                .unwrap();
                let concrete = monomorphize_project(&analyzed).unwrap();
                let expected = format!(
                    "{}\n{}\n{}\n",
                    4096 + 3 * stride,
                    4096 + stride,
                    if layout.pointer_size == 4 {
                        1u64
                    } else {
                        4294967297
                    }
                );
                let hosted = dbyte_interp::execute_concrete_project(&concrete).unwrap();
                assert_eq!(hosted.output, expected, "{ty} {layout:?}");
                for level in [VmOptimizationLevel::O0, VmOptimizationLevel::O1] {
                    let code =
                        lower_concrete_project_to_bytecode_at_level(&concrete, level).unwrap();
                    let vm = execute_canonical_bytecode(&code).unwrap();
                    assert_eq!(vm.output, expected, "{ty} {layout:?} {level:?}");
                    assert_eq!(vm.allocator_events, hosted.allocator_events);
                    assert_eq!(vm.live_allocations, 0);
                }
            }
        }
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

    #[test]
    fn versioned_v123_collections_preserve_hosted_o0_o1_and_ownership_parity() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let input = load_source_project(&root.join("runtime/v12.3.0/collections_smoke.dby"))
            .expect("load tracked v12.3 collections proof");
        let analyzed = analyze_project(&input, Profile::Hosted, TargetDataLayout::I686)
            .expect("analyze collections through canonical HIR");
        let concrete = monomorphize_project(&analyzed).expect("monomorphize collections");

        let hosted = dbyte_interp::execute_concrete_project(&concrete)
            .expect("execute collections through hosted HIR");
        let o0 = lower_concrete_project_to_bytecode_at_level(&concrete, VmOptimizationLevel::O0)
            .expect("lower O0 collections");
        let o1 = lower_concrete_project_to_bytecode_at_level(&concrete, VmOptimizationLevel::O1)
            .expect("lower optimized collections");
        let o0 = execute_canonical_bytecode(&o0).expect("execute O0 collections");
        let o1 = execute_canonical_bytecode(&o1).expect("execute optimized collections");

        assert_eq!(hosted.output, "1\n1\n1\n1\n1\n1\n");
        assert_eq!(o0.output, hosted.output);
        assert_eq!(o1.output, hosted.output);
        assert_eq!(o0.allocator_events, hosted.allocator_events);
        assert_eq!(o1.allocator_events, hosted.allocator_events);
        assert_eq!(
            hosted.live_allocations, 0,
            "hosted collection allocations leaked: {:?}",
            hosted.allocator_events
        );
        assert_eq!(
            o0.live_allocations, 0,
            "O0 collection allocations leaked: {:?}",
            o0.allocator_events
        );
        assert_eq!(
            o1.live_allocations, 0,
            "O1 collection allocations leaked: {:?}",
            o1.allocator_events
        );
        let allocations = hosted
            .allocator_events
            .iter()
            .filter(|event| matches!(event, AllocationEvent::Alloc { .. }))
            .count();
        let frees = hosted
            .allocator_events
            .iter()
            .filter(|event| matches!(event, AllocationEvent::Free { .. }))
            .count();
        assert!(
            allocations >= 7,
            "collections proof did not exercise growth"
        );
        assert_eq!(
            frees, allocations,
            "every collection allocation must be freed"
        );
    }
}
