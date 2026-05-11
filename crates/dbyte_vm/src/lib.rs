use byteorder::{ByteOrder, BE, LE};
use dbyte_ast::{FStrPart, Span};
use dbyte_bytecode::{
    format_op, BytecodeFunction, Chunk, LocalKind, ModuleMember, ModuleValue, NativeFn, Op, Value,
};
use dbyte_compiler::{CompileError, Compiler};
use dbyte_lexer::Lexer;
use dbyte_module::{resolve_import, ImportTarget, ModuleError, ModuleState};
use dbyte_parser::Parser;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

// Keep this below the host thread stack ceiling so recursion fails as a DByte
// RuntimeError instead of aborting the Rust process.
const MAX_CALL_DEPTH: usize = 32;

#[derive(Debug)]
pub struct VmError {
    pub msg: String,
    pub span: Span,
}

impl VmError {
    fn new(msg: impl Into<String>) -> Self {
        Self {
            msg: msg.into(),
            span: Span::zero(),
        }
    }
}

#[derive(Debug, Clone)]
struct IteratorState {
    items: Vec<Value>,
    index: usize,
}

#[derive(Debug, Clone)]
struct Frame {
    values: Vec<Value>,
    i64s: Vec<i64>,
}

#[derive(Debug, Clone, Copy)]
enum ReturnMode {
    TopLevel,
    Push { stack_base: usize },
    PushI64 { stack_base: usize },
    StoreI64 { stack_base: usize, dst: usize },
    Discard { stack_base: usize },
}

#[derive(Debug, Clone)]
enum FrameChunk {
    TopLevel(Rc<Chunk>),
    Function(Rc<BytecodeFunction>),
}

impl FrameChunk {
    fn chunk(&self) -> &Chunk {
        match self {
            Self::TopLevel(chunk) => chunk,
            Self::Function(function) => &function.chunk,
        }
    }
}

#[derive(Debug, Clone)]
struct ExecFrame {
    chunk: FrameChunk,
    locals: Frame,
    ip: usize,
    return_mode: ReturnMode,
}

impl Frame {
    fn new(chunk: &Chunk) -> Self {
        let i64_count = if chunk.i64_local_count > 0 {
            chunk.i64_local_count
        } else {
            chunk
                .local_kinds
                .iter()
                .filter(|kind| **kind == LocalKind::I64)
                .count()
        };
        let has_value_locals = chunk.local_kinds.len() < chunk.local_names.len()
            || chunk.local_kinds.contains(&LocalKind::Value);
        Self {
            values: if has_value_locals {
                vec![Value::Void; chunk.local_names.len()]
            } else {
                Vec::new()
            },
            i64s: vec![0; i64_count],
        }
    }

    fn reset(&mut self, chunk: &Chunk) {
        let i64_count = if chunk.i64_local_count > 0 {
            chunk.i64_local_count
        } else {
            chunk
                .local_kinds
                .iter()
                .filter(|kind| **kind == LocalKind::I64)
                .count()
        };
        let has_value_locals = chunk.local_kinds.len() < chunk.local_names.len()
            || chunk.local_kinds.contains(&LocalKind::Value);

        if has_value_locals {
            self.values.resize(chunk.local_names.len(), Value::Void);
            self.values.fill(Value::Void);
        } else {
            self.values.clear();
        }
        self.i64s.resize(i64_count, 0);
        self.i64s.fill(0);
    }

    fn ensure_slot(&mut self, slot: usize) {
        if slot >= self.values.len() {
            self.values.resize(slot + 1, Value::Void);
        }
    }

    fn get_value(&self, chunk: &Chunk, slot: usize) -> Result<Value, VmError> {
        match i64_local_slot(chunk, slot) {
            Some(i64_slot) => self
                .i64s
                .get(i64_slot)
                .copied()
                .map(Value::Int)
                .ok_or_else(|| VmError::new(format!("invalid i64 local slot {}", slot))),
            None => self
                .values
                .get(slot)
                .cloned()
                .ok_or_else(|| VmError::new(format!("invalid local slot {}", slot))),
        }
    }

    fn set_value(&mut self, chunk: &Chunk, slot: usize, value: Value) -> Result<(), VmError> {
        match i64_local_slot(chunk, slot) {
            Some(i64_slot) => match value {
                Value::Int(n) => {
                    self.i64s[i64_slot] = n;
                    Ok(())
                }
                other => Err(VmError::new(format!(
                    "expected int local {}, found {}",
                    slot,
                    other.kind_name()
                ))),
            },
            None => {
                self.ensure_slot(slot);
                self.values[slot] = value;
                Ok(())
            }
        }
    }

    fn set_i64(&mut self, chunk: &Chunk, slot: usize, value: i64) {
        match i64_local_slot(chunk, slot) {
            Some(i64_slot) => self.i64s[i64_slot] = value,
            None => {
                self.ensure_slot(slot);
                self.values[slot] = Value::Int(value);
            }
        }
    }
}

fn i64_local_slot(chunk: &Chunk, slot: usize) -> Option<usize> {
    if let Some(mapped) = chunk.local_i64_slots.get(slot).copied().flatten() {
        return Some(mapped);
    }
    if chunk.local_kinds.get(slot).copied() != Some(LocalKind::I64) {
        return None;
    }
    Some(
        chunk.local_kinds[..slot]
            .iter()
            .filter(|kind| **kind == LocalKind::I64)
            .count(),
    )
}

fn required_i64_slot(chunk: &Chunk, slot: usize) -> Result<usize, VmError> {
    i64_local_slot(chunk, slot)
        .ok_or_else(|| VmError::new(format!("invalid i64 local slot {}", slot)))
}

pub struct Vm {
    stack: Vec<Value>,
    i64_stack: Vec<i64>,
    iter_stack: Vec<IteratorState>,
    frames: Vec<ExecFrame>,
    free_frames: Vec<Frame>,
    trace: bool,
    current_file: Option<PathBuf>,
    module_cache: HashMap<String, ModuleState<ModuleValue>>,
    loading_stack: Vec<String>,
    active_function_tables: Vec<Vec<Rc<BytecodeFunction>>>,
}

impl Vm {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            i64_stack: Vec::new(),
            iter_stack: Vec::new(),
            frames: Vec::new(),
            free_frames: Vec::new(),
            trace: false,
            current_file: None,
            module_cache: HashMap::new(),
            loading_stack: Vec::new(),
            active_function_tables: Vec::new(),
        }
    }

    pub fn with_entry_path(path: impl Into<PathBuf>) -> Self {
        let mut vm = Self::new();
        vm.current_file = Some(path.into());
        vm
    }

    pub fn set_trace(&mut self, trace: bool) {
        self.trace = trace;
    }

    pub fn run(&mut self, chunk: &Chunk) -> Result<(), VmError> {
        let chunk = Rc::new(chunk.clone());
        self.active_function_tables
            .push(chunk.functions_by_id.iter().cloned().map(Rc::new).collect());
        let result = self.run_top_chunk(chunk).map(|_| ());
        self.active_function_tables.pop();
        result
    }

    fn run_top_chunk(&mut self, chunk: Rc<Chunk>) -> Result<Option<Frame>, VmError> {
        let locals = Frame::new(&chunk);
        self.frames.push(ExecFrame {
            chunk: FrameChunk::TopLevel(chunk),
            locals,
            ip: 0,
            return_mode: ReturnMode::TopLevel,
        });
        self.run_loop()
    }

    fn run_loop(&mut self) -> Result<Option<Frame>, VmError> {
        'dispatch: while !self.frames.is_empty() {
            let frame_chunk = self
                .frames
                .last()
                .ok_or_else(|| VmError::new("no active VM frame"))?
                .chunk
                .clone();
            let chunk = frame_chunk.chunk();

            loop {
                let (op, ip) = {
                    let frame = self
                        .frames
                        .last_mut()
                        .ok_or_else(|| VmError::new("no active VM frame"))?;
                    if frame.ip >= chunk.code.len() {
                        self.finish_frame(Value::Void)?;
                        continue 'dispatch;
                    }
                    let ip = frame.ip;
                    let op = chunk.code[ip].clone();
                    frame.ip += 1;
                    (op, ip)
                };
                if self.trace {
                    println!(
                        "ip={:04} op={} stack=[{}]",
                        ip,
                        format_op(&op, chunk),
                        self.value_stack().join(", ")
                    );
                }
                match op {
                    Op::Const(idx) => self.push(chunk.constants[idx].clone()),
                    Op::ConstI64(n) => self.push(Value::Int(n)),
                    Op::ConstI64Stack(n) => self.i64_stack.push(n),
                    Op::FStr(parts) => self.eval_fstr(&parts, chunk)?,
                    Op::Add => self.binary_add()?,
                    Op::Sub => self.binary_num("sub", |a, b| a.checked_sub(b), |a, b| a - b)?,
                    Op::Mul => self.binary_num("mul", |a, b| a.checked_mul(b), |a, b| a * b)?,
                    Op::Div => self.binary_div()?,
                    Op::AddI64 => self.binary_i64(|a, b| checked_i64(a.checked_add(b)))?,
                    Op::SubI64 => self.binary_i64(|a, b| checked_i64(a.checked_sub(b)))?,
                    Op::MulI64 => self.binary_i64(|a, b| checked_i64(a.checked_mul(b)))?,
                    Op::DivI64 => self.binary_i64(|a, b| {
                        if b == 0 {
                            Err(VmError::new("division by zero"))
                        } else {
                            checked_i64(a.checked_div(b))
                        }
                    })?,
                    Op::AddI64Stack => {
                        self.binary_i64_stack(|a, b| checked_i64(a.checked_add(b)))?
                    }
                    Op::SubI64Stack => {
                        self.binary_i64_stack(|a, b| checked_i64(a.checked_sub(b)))?
                    }
                    Op::MulI64Stack => {
                        self.binary_i64_stack(|a, b| checked_i64(a.checked_mul(b)))?
                    }
                    Op::DivI64Stack => self.binary_i64_stack(|a, b| {
                        if b == 0 {
                            Err(VmError::new("division by zero"))
                        } else {
                            checked_i64(a.checked_div(b))
                        }
                    })?,
                    Op::Eq => self.binary_cmp(|a, b| a == b)?,
                    Op::Ne => self.binary_cmp(|a, b| a != b)?,
                    Op::Lt => self.binary_ord("<", |a, b| a < b, |a, b| a < b)?,
                    Op::Le => self.binary_ord("<=", |a, b| a <= b, |a, b| a <= b)?,
                    Op::Gt => self.binary_ord(">", |a, b| a > b, |a, b| a > b)?,
                    Op::Ge => self.binary_ord(">=", |a, b| a >= b, |a, b| a >= b)?,
                    Op::LtI64 => self.binary_i64_cmp(|a, b| a < b)?,
                    Op::LeI64 => self.binary_i64_cmp(|a, b| a <= b)?,
                    Op::GtI64 => self.binary_i64_cmp(|a, b| a > b)?,
                    Op::GeI64 => self.binary_i64_cmp(|a, b| a >= b)?,
                    Op::Neg => self.unary_neg()?,
                    Op::Not => self.unary_not()?,
                    Op::MakeList(count) => self.make_list(count)?,
                    Op::Index => self.index()?,
                    Op::LoadLocal(slot) => {
                        let value = self.current_frame()?.get_value(chunk, slot)?;
                        self.push(value);
                    }
                    Op::LoadLocalI64(slot) => {
                        let i64_slot = required_i64_slot(chunk, slot)?;
                        let value = *self
                            .frames
                            .last()
                            .and_then(|frame| frame.locals.i64s.get(i64_slot))
                            .ok_or_else(|| {
                                VmError::new(format!("invalid i64 local slot {}", slot))
                            })?;
                        self.push(Value::Int(value));
                    }
                    Op::LoadLocalI64Stack(slot) => {
                        let value = self.load_i64_local(chunk, slot)?;
                        self.i64_stack.push(value);
                    }
                    Op::StoreLocal(slot) => {
                        let value = self.pop()?;
                        self.current_frame_mut()?.set_value(chunk, slot, value)?;
                    }
                    Op::StoreLocalI64(slot) => {
                        let value = self.pop_i64()?;
                        let i64_slot = required_i64_slot(chunk, slot)?;
                        let target = self
                            .frames
                            .last_mut()
                            .and_then(|frame| frame.locals.i64s.get_mut(i64_slot))
                            .ok_or_else(|| {
                                VmError::new(format!("invalid i64 local slot {}", slot))
                            })?;
                        *target = value;
                    }
                    Op::StoreLocalI64Stack(slot) => {
                        let value = self.pop_i64_stack()?;
                        self.store_i64_in_frame(chunk, slot, value)?;
                    }
                    Op::AddLocalI64 { dst, src } => {
                        let src_i64_slot = required_i64_slot(chunk, src)?;
                        let dst_i64_slot = required_i64_slot(chunk, dst)?;
                        let frame = self
                            .frames
                            .last_mut()
                            .ok_or_else(|| VmError::new("no active VM frame"))?;
                        let value = *frame.locals.i64s.get(src_i64_slot).ok_or_else(|| {
                            VmError::new(format!("invalid i64 local slot {}", src))
                        })?;
                        let target = frame.locals.i64s.get_mut(dst_i64_slot).ok_or_else(|| {
                            VmError::new(format!("invalid i64 local slot {}", dst))
                        })?;
                        *target = checked_i64(target.checked_add(value))?;
                    }
                    Op::AddLocalConstI64 { slot, value } => {
                        let i64_slot = required_i64_slot(chunk, slot)?;
                        let target = self
                            .frames
                            .last_mut()
                            .and_then(|frame| frame.locals.i64s.get_mut(i64_slot))
                            .ok_or_else(|| {
                                VmError::new(format!("invalid i64 local slot {}", slot))
                            })?;
                        *target = checked_i64(target.checked_add(value))?;
                    }
                    Op::LtLocalConstI64 { slot, value } => {
                        let i64_slot = required_i64_slot(chunk, slot)?;
                        let left = *self
                            .frames
                            .last()
                            .and_then(|frame| frame.locals.i64s.get(i64_slot))
                            .ok_or_else(|| {
                                VmError::new(format!("invalid i64 local slot {}", slot))
                            })?;
                        self.push(Value::Bool(left < value));
                    }
                    Op::LeLocalConstI64 { slot, value } => {
                        let i64_slot = required_i64_slot(chunk, slot)?;
                        let left = *self
                            .frames
                            .last()
                            .and_then(|frame| frame.locals.i64s.get(i64_slot))
                            .ok_or_else(|| {
                                VmError::new(format!("invalid i64 local slot {}", slot))
                            })?;
                        self.push(Value::Bool(left <= value));
                    }
                    Op::GtLocalConstI64 { slot, value } => {
                        let i64_slot = required_i64_slot(chunk, slot)?;
                        let left = *self
                            .frames
                            .last()
                            .and_then(|frame| frame.locals.i64s.get(i64_slot))
                            .ok_or_else(|| {
                                VmError::new(format!("invalid i64 local slot {}", slot))
                            })?;
                        self.push(Value::Bool(left > value));
                    }
                    Op::GeLocalConstI64 { slot, value } => {
                        let i64_slot = required_i64_slot(chunk, slot)?;
                        let left = *self
                            .frames
                            .last()
                            .and_then(|frame| frame.locals.i64s.get(i64_slot))
                            .ok_or_else(|| {
                                VmError::new(format!("invalid i64 local slot {}", slot))
                            })?;
                        self.push(Value::Bool(left >= value));
                    }
                    Op::JumpIfNotLtLocalConstI64 {
                        slot,
                        value,
                        target,
                    } => {
                        self.jump_if_not_local_const_i64(chunk, slot, value, target, |a, b| a < b)?
                    }
                    Op::JumpIfNotLeLocalConstI64 {
                        slot,
                        value,
                        target,
                    } => {
                        self.jump_if_not_local_const_i64(chunk, slot, value, target, |a, b| a <= b)?
                    }
                    Op::JumpIfNotGtLocalConstI64 {
                        slot,
                        value,
                        target,
                    } => {
                        self.jump_if_not_local_const_i64(chunk, slot, value, target, |a, b| a > b)?
                    }
                    Op::JumpIfNotGeLocalConstI64 {
                        slot,
                        value,
                        target,
                    } => {
                        self.jump_if_not_local_const_i64(chunk, slot, value, target, |a, b| a >= b)?
                    }
                    Op::Import(path, slot) => {
                        let alias = chunk.local_names.get(slot).cloned().unwrap_or_default();
                        let module = self.load_module(&path, &alias)?;
                        self.current_frame_mut()?.set_value(
                            chunk,
                            slot,
                            Value::Module(Rc::new(module)),
                        )?;
                    }
                    Op::Member(property) => {
                        let module = self.pop_module()?;
                        let member = module.members.get(&property).cloned().ok_or_else(|| {
                            VmError::new(format!(
                                "module '{}' has no public member '{}'",
                                module.alias, property
                            ))
                        })?;
                        match member {
                            ModuleMember::Value(value) => self.push(value),
                            ModuleMember::Function(_) | ModuleMember::Native(_) => {
                                return Err(VmError::new(format!(
                                    "module member '{}' is callable",
                                    property
                                )));
                            }
                        }
                    }
                    Op::MemberCall(property, argc) => {
                        let args_start = self.stack.len() - argc;
                        let module = match self.stack.get(args_start - 1) {
                            Some(Value::Module(m)) => m.clone(),
                            _ => return Err(VmError::new("member call requires a module")),
                        };
                        let member = module.members.get(&property).cloned().ok_or_else(|| {
                            VmError::new(format!(
                                "module '{}' has no public member '{}'",
                                module.alias, property
                            ))
                        })?;
                        let value = match member {
                            ModuleMember::Native(id) => {
                                self.call_native(id, &self.stack[args_start..])?
                            }
                            ModuleMember::Function(f) => {
                                self.push_call_frame(
                                    Rc::new(*f),
                                    args_start,
                                    ReturnMode::Push {
                                        stack_base: args_start - 1,
                                    },
                                )?;
                                continue 'dispatch;
                            }
                            ModuleMember::Value(_) => {
                                return Err(VmError::new(format!(
                                    "module member '{}' is not callable",
                                    property
                                )))
                            }
                        };
                        self.stack.truncate(args_start - 1);
                        self.push(value);
                    }
                    Op::ReadU32Le => self.read_u32_le()?,
                    Op::BufferFind => self.buffer_find()?,
                    Op::BufferReplace => self.buffer_replace()?,
                    Op::IterInit => self.iter_init()?,
                    Op::IterNext { slot, jump } => {
                        let should_continue = self.iter_next(chunk, slot)?;
                        if !should_continue {
                            self.current_exec_frame_mut()?.ip = jump;
                        }
                    }
                    Op::Jump(target) => {
                        self.frames
                            .last_mut()
                            .ok_or_else(|| VmError::new("no active VM frame"))?
                            .ip = target;
                    }
                    Op::JumpIfFalse(target) => {
                        if !self.pop_bool("jump condition")? {
                            self.frames
                                .last_mut()
                                .ok_or_else(|| VmError::new("no active VM frame"))?
                                .ip = target;
                        }
                    }
                    Op::Call(name, argc) => {
                        let args_start = self.stack.len() - argc;
                        if name == "print" || name == "len" {
                            let value = self.call_builtin(&name, &self.stack[args_start..])?;
                            self.stack.truncate(args_start);
                            self.push(value);
                        } else {
                            let function =
                                chunk.functions.get(&name).cloned().ok_or_else(|| {
                                    VmError::new(format!("undefined function `{}`", name))
                                })?;
                            self.push_call_frame(
                                Rc::new(function),
                                args_start,
                                ReturnMode::Push {
                                    stack_base: args_start,
                                },
                            )?;
                            continue 'dispatch;
                        }
                    }
                    Op::CallFn { id, argc } => {
                        let args_start = self.stack.len() - argc;
                        let function = self.resolve_function(chunk, id)?;
                        self.push_call_frame(
                            function,
                            args_start,
                            ReturnMode::Push {
                                stack_base: args_start,
                            },
                        )?;
                        continue 'dispatch;
                    }
                    Op::CallFnI64ToI64Stack { id, argc } => {
                        if self.i64_stack.len() < argc {
                            return Err(VmError::new("i64 stack underflow"));
                        }
                        let args_start = self.i64_stack.len() - argc;
                        let function = self.resolve_function(chunk, id)?;
                        self.push_call_frame_i64(
                            function,
                            args_start,
                            ReturnMode::PushI64 {
                                stack_base: args_start,
                            },
                        )?;
                        continue 'dispatch;
                    }
                    Op::CallFnI64ToLocal { id, argc, dst } => {
                        let args_start = self.stack.len() - argc;
                        let function = self.resolve_function(chunk, id)?;
                        self.push_call_frame(
                            function,
                            args_start,
                            ReturnMode::StoreI64 {
                                stack_base: args_start,
                                dst,
                            },
                        )?;
                        continue 'dispatch;
                    }
                    Op::CallFnDiscard { id, argc } => {
                        let args_start = self.stack.len() - argc;
                        let function = self.resolve_function(chunk, id)?;
                        self.push_call_frame(
                            function,
                            args_start,
                            ReturnMode::Discard {
                                stack_base: args_start,
                            },
                        )?;
                        continue 'dispatch;
                    }
                    Op::Return => {
                        let value = self.pop()?;
                        self.finish_frame(value)?;
                        continue 'dispatch;
                    }
                    Op::ReturnI64 => {
                        let value = self.pop_i64()?;
                        self.finish_frame_i64(value)?;
                        continue 'dispatch;
                    }
                    Op::ReturnI64ToI64Stack => {
                        let value = self.pop_i64_stack()?;
                        self.finish_frame_i64(value)?;
                        continue 'dispatch;
                    }
                    Op::Pop => {
                        if self.stack.pop().is_none() {
                            return Err(VmError::new("stack underflow"));
                        }
                    }
                    Op::Halt => {
                        let frame = self
                            .frames
                            .pop()
                            .ok_or_else(|| VmError::new("no active VM frame"))?;
                        match frame.return_mode {
                            ReturnMode::TopLevel => return Ok(Some(frame.locals)),
                            ReturnMode::Push { stack_base } => {
                                self.release_frame(frame.locals);
                                self.stack.truncate(stack_base);
                                self.push(Value::Void);
                            }
                            ReturnMode::PushI64 { stack_base } => {
                                self.release_frame(frame.locals);
                                self.i64_stack.truncate(stack_base);
                            }
                            ReturnMode::StoreI64 { stack_base, dst } => {
                                self.release_frame(frame.locals);
                                self.stack.truncate(stack_base);
                                let _ = dst;
                            }
                            ReturnMode::Discard { stack_base } => {
                                self.release_frame(frame.locals);
                                self.stack.truncate(stack_base);
                            }
                        }
                        continue 'dispatch;
                    }
                }
            }
        }
        Ok(None)
    }

    fn current_frame(&self) -> Result<&Frame, VmError> {
        self.frames
            .last()
            .map(|frame| &frame.locals)
            .ok_or_else(|| VmError::new("no active VM frame"))
    }

    fn current_frame_mut(&mut self) -> Result<&mut Frame, VmError> {
        self.frames
            .last_mut()
            .map(|frame| &mut frame.locals)
            .ok_or_else(|| VmError::new("no active VM frame"))
    }

    fn current_exec_frame_mut(&mut self) -> Result<&mut ExecFrame, VmError> {
        self.frames
            .last_mut()
            .ok_or_else(|| VmError::new("no active VM frame"))
    }

    fn acquire_frame(&mut self, chunk: &Chunk) -> Frame {
        match self.free_frames.pop() {
            Some(mut frame) => {
                frame.reset(chunk);
                frame
            }
            None => Frame::new(chunk),
        }
    }

    fn release_frame(&mut self, frame: Frame) {
        self.free_frames.push(frame);
    }

    fn finish_frame(&mut self, value: Value) -> Result<(), VmError> {
        let frame = self
            .frames
            .pop()
            .ok_or_else(|| VmError::new("no active VM frame"))?;
        let return_mode = frame.return_mode;
        self.release_frame(frame.locals);
        match return_mode {
            ReturnMode::TopLevel => Err(VmError::new("return outside function")),
            ReturnMode::Push { stack_base } => {
                self.stack.truncate(stack_base);
                self.push(value);
                Ok(())
            }
            ReturnMode::PushI64 { stack_base } => {
                self.i64_stack.truncate(stack_base);
                let Value::Int(value) = value else {
                    return Err(VmError::new("expected int return value"));
                };
                self.i64_stack.push(value);
                Ok(())
            }
            ReturnMode::StoreI64 { stack_base, dst } => {
                self.stack.truncate(stack_base);
                let Value::Int(value) = value else {
                    return Err(VmError::new("expected int return value"));
                };
                self.store_i64_in_current_frame(dst, value)
            }
            ReturnMode::Discard { stack_base } => {
                self.stack.truncate(stack_base);
                Ok(())
            }
        }
    }

    fn finish_frame_i64(&mut self, value: i64) -> Result<(), VmError> {
        let frame = self
            .frames
            .pop()
            .ok_or_else(|| VmError::new("no active VM frame"))?;
        let return_mode = frame.return_mode;
        self.release_frame(frame.locals);
        match return_mode {
            ReturnMode::TopLevel => Err(VmError::new("return outside function")),
            ReturnMode::Push { stack_base } => {
                self.stack.truncate(stack_base);
                self.push(Value::Int(value));
                Ok(())
            }
            ReturnMode::PushI64 { stack_base } => {
                self.i64_stack.truncate(stack_base);
                self.i64_stack.push(value);
                Ok(())
            }
            ReturnMode::StoreI64 { stack_base, dst } => {
                self.stack.truncate(stack_base);
                self.store_i64_in_current_frame(dst, value)
            }
            ReturnMode::Discard { stack_base } => {
                self.stack.truncate(stack_base);
                Ok(())
            }
        }
    }

    fn push_call_frame(
        &mut self,
        function: Rc<BytecodeFunction>,
        args_start: usize,
        return_mode: ReturnMode,
    ) -> Result<(), VmError> {
        let argc = self.stack.len() - args_start;
        if argc != function.params.len() {
            return Err(VmError::new(format!(
                "function `{}` expects {} args, got {}",
                function.name,
                function.params.len(),
                argc
            )));
        }
        if self.frames.len() >= MAX_CALL_DEPTH {
            return Err(VmError::new("maximum call depth exceeded"));
        }

        let mut locals = self.acquire_frame(&function.chunk);
        for idx in (0..argc).rev() {
            let arg = self.pop()?;
            if idx < function.chunk.local_names.len() {
                match (i64_local_slot(&function.chunk, idx), arg) {
                    (Some(_), Value::Int(n)) => locals.set_i64(&function.chunk, idx, n),
                    (_, value) => locals.set_value(&function.chunk, idx, value)?,
                }
            }
        }
        self.frames.push(ExecFrame {
            chunk: FrameChunk::Function(function),
            locals,
            ip: 0,
            return_mode,
        });
        Ok(())
    }

    fn push_call_frame_i64(
        &mut self,
        function: Rc<BytecodeFunction>,
        args_start: usize,
        return_mode: ReturnMode,
    ) -> Result<(), VmError> {
        let argc = self.i64_stack.len() - args_start;
        if argc != function.params.len() {
            return Err(VmError::new(format!(
                "function `{}` expects {} args, got {}",
                function.name,
                function.params.len(),
                argc
            )));
        }
        if self.frames.len() >= MAX_CALL_DEPTH {
            return Err(VmError::new("maximum call depth exceeded"));
        }

        let mut locals = self.acquire_frame(&function.chunk);
        for idx in (0..argc).rev() {
            let arg = self.pop_i64_stack()?;
            if idx < function.chunk.local_names.len() {
                match i64_local_slot(&function.chunk, idx) {
                    Some(_) => locals.set_i64(&function.chunk, idx, arg),
                    None => locals.set_value(&function.chunk, idx, Value::Int(arg))?,
                }
            }
        }
        self.frames.push(ExecFrame {
            chunk: FrameChunk::Function(function),
            locals,
            ip: 0,
            return_mode,
        });
        Ok(())
    }

    fn resolve_function(&self, chunk: &Chunk, id: usize) -> Result<Rc<BytecodeFunction>, VmError> {
        if let Some(function) = self
            .active_function_tables
            .last()
            .and_then(|functions| functions.get(id))
        {
            return Ok(function.clone());
        }
        chunk
            .functions_by_id
            .get(id)
            .cloned()
            .map(Rc::new)
            .ok_or_else(|| VmError::new(format!("invalid function id {}", id)))
    }

    fn store_i64_in_current_frame(&mut self, dst: usize, value: i64) -> Result<(), VmError> {
        let caller_chunk = self
            .frames
            .last()
            .ok_or_else(|| VmError::new("no caller frame"))?
            .chunk
            .clone();
        let caller_chunk = caller_chunk.chunk();
        let i64_slot = required_i64_slot(caller_chunk, dst)?;
        let target = self
            .frames
            .last_mut()
            .and_then(|frame| frame.locals.i64s.get_mut(i64_slot))
            .ok_or_else(|| VmError::new(format!("invalid i64 local slot {}", dst)))?;
        *target = value;
        Ok(())
    }

    fn load_i64_local(&self, chunk: &Chunk, slot: usize) -> Result<i64, VmError> {
        let i64_slot = required_i64_slot(chunk, slot)?;
        self.frames
            .last()
            .and_then(|frame| frame.locals.i64s.get(i64_slot))
            .copied()
            .ok_or_else(|| VmError::new(format!("invalid i64 local slot {}", slot)))
    }

    fn jump_if_not_local_const_i64(
        &mut self,
        chunk: &Chunk,
        slot: usize,
        value: i64,
        target: usize,
        predicate: impl FnOnce(i64, i64) -> bool,
    ) -> Result<(), VmError> {
        let left = self.load_i64_local(chunk, slot)?;
        if !predicate(left, value) {
            self.frames
                .last_mut()
                .ok_or_else(|| VmError::new("no active VM frame"))?
                .ip = target;
        }
        Ok(())
    }

    fn store_i64_in_frame(
        &mut self,
        chunk: &Chunk,
        slot: usize,
        value: i64,
    ) -> Result<(), VmError> {
        let i64_slot = required_i64_slot(chunk, slot)?;
        let target = self
            .frames
            .last_mut()
            .and_then(|frame| frame.locals.i64s.get_mut(i64_slot))
            .ok_or_else(|| VmError::new(format!("invalid i64 local slot {}", slot)))?;
        *target = value;
        Ok(())
    }

    fn push(&mut self, value: Value) {
        self.stack.push(value);
    }

    fn pop(&mut self) -> Result<Value, VmError> {
        self.stack
            .pop()
            .ok_or_else(|| VmError::new("stack underflow"))
    }

    fn pop_i64(&mut self) -> Result<i64, VmError> {
        match self.pop()? {
            Value::Int(n) => Ok(n),
            other => Err(VmError::new(format!(
                "expected int on stack, found {}",
                other.kind_name()
            ))),
        }
    }

    fn pop_i64_stack(&mut self) -> Result<i64, VmError> {
        self.i64_stack
            .pop()
            .ok_or_else(|| VmError::new("i64 stack underflow"))
    }

    fn pop_args(&mut self, argc: usize) -> Result<Vec<Value>, VmError> {
        if self.stack.len() < argc {
            return Err(VmError::new("stack underflow"));
        }
        let args = self.stack.split_off(self.stack.len() - argc);
        Ok(args)
    }

    fn pop_bool(&mut self, label: &str) -> Result<bool, VmError> {
        match self.pop()? {
            Value::Bool(b) => Ok(b),
            other => Err(VmError::new(format!(
                "{} must be bool, found {}",
                label, other
            ))),
        }
    }

    fn value_stack(&self) -> Vec<String> {
        self.stack.iter().map(|v| format!("{}", v)).collect()
    }

    fn eval_fstr(&mut self, parts: &[FStrPart], chunk: &Chunk) -> Result<(), VmError> {
        let mut out = String::new();
        for part in parts {
            match part {
                FStrPart::Literal(s) => out.push_str(s),
                FStrPart::Interp(name) => {
                    let slot = chunk
                        .local_names
                        .iter()
                        .position(|n| n == name)
                        .ok_or_else(|| {
                            VmError::new(format!(
                                "undefined variable `{}` in string interpolation",
                                name
                            ))
                        })?;
                    out.push_str(&format!(
                        "{}",
                        self.current_frame()?.get_value(chunk, slot)?
                    ));
                }
            }
        }
        self.push(Value::Str(out));
        Ok(())
    }

    fn binary_add(&mut self) -> Result<(), VmError> {
        let b = self.pop()?;
        let a = self.pop()?;
        let value = match (a, b) {
            (Value::Int(a), Value::Int(b)) => Value::Int(checked_i64(a.checked_add(b))?),
            (Value::Float(a), Value::Float(b)) => Value::Float(a + b),
            (Value::Str(a), Value::Str(b)) => Value::Str(a + &b),
            _ => return Err(VmError::new("type mismatch in binary expression")),
        };
        self.push(value);
        Ok(())
    }

    fn binary_i64(&mut self, op: fn(i64, i64) -> Result<i64, VmError>) -> Result<(), VmError> {
        let b = self.pop_i64()?;
        let a = self.pop_i64()?;
        self.push(Value::Int(op(a, b)?));
        Ok(())
    }

    fn binary_i64_stack(
        &mut self,
        op: fn(i64, i64) -> Result<i64, VmError>,
    ) -> Result<(), VmError> {
        let b = self.pop_i64_stack()?;
        let a = self.pop_i64_stack()?;
        self.i64_stack.push(op(a, b)?);
        Ok(())
    }

    fn binary_i64_cmp(&mut self, op: fn(i64, i64) -> bool) -> Result<(), VmError> {
        let b = self.pop_i64()?;
        let a = self.pop_i64()?;
        self.push(Value::Bool(op(a, b)));
        Ok(())
    }

    fn binary_num(
        &mut self,
        label: &str,
        int_op: fn(i64, i64) -> Option<i64>,
        float_op: fn(f64, f64) -> f64,
    ) -> Result<(), VmError> {
        let b = self.pop()?;
        let a = self.pop()?;
        let value = match (a, b) {
            (Value::Int(a), Value::Int(b)) => Value::Int(checked_i64(int_op(a, b))?),
            (Value::Float(a), Value::Float(b)) => Value::Float(float_op(a, b)),
            _ => return Err(VmError::new(format!("unsupported {} operation", label))),
        };
        self.push(value);
        Ok(())
    }

    fn binary_div(&mut self) -> Result<(), VmError> {
        let b = self.pop()?;
        let a = self.pop()?;
        let value = match (a, b) {
            (Value::Int(_), Value::Int(0)) => return Err(VmError::new("division by zero")),
            (Value::Int(a), Value::Int(b)) => Value::Int(checked_i64(a.checked_div(b))?),
            (Value::Float(a), Value::Float(b)) => Value::Float(a / b),
            _ => return Err(VmError::new("unsupported div operation")),
        };
        self.push(value);
        Ok(())
    }

    fn binary_cmp(&mut self, op: fn(Value, Value) -> bool) -> Result<(), VmError> {
        let b = self.pop()?;
        let a = self.pop()?;
        self.push(Value::Bool(op(a, b)));
        Ok(())
    }

    fn binary_ord(
        &mut self,
        label: &str,
        int_op: fn(i64, i64) -> bool,
        float_op: fn(f64, f64) -> bool,
    ) -> Result<(), VmError> {
        let b = self.pop()?;
        let a = self.pop()?;
        let value = match (a, b) {
            (Value::Int(a), Value::Int(b)) => int_op(a, b),
            (Value::Float(a), Value::Float(b)) => float_op(a, b),
            _ => return Err(VmError::new(format!("unsupported {} comparison", label))),
        };
        self.push(Value::Bool(value));
        Ok(())
    }

    fn unary_neg(&mut self) -> Result<(), VmError> {
        let value = match self.pop()? {
            Value::Int(n) => Value::Int(checked_i64(n.checked_neg())?),
            Value::Float(n) => Value::Float(-n),
            _ => return Err(VmError::new("unary `-` requires numeric")),
        };
        self.push(value);
        Ok(())
    }

    fn unary_not(&mut self) -> Result<(), VmError> {
        let value = match self.pop()? {
            Value::Bool(b) => Value::Bool(!b),
            _ => return Err(VmError::new("unary `!` requires bool")),
        };
        self.push(value);
        Ok(())
    }

    fn make_list(&mut self, count: usize) -> Result<(), VmError> {
        let mut items = self.pop_args(count)?;
        items.shrink_to_fit();
        self.push(Value::List(items));
        Ok(())
    }

    fn index(&mut self) -> Result<(), VmError> {
        let index = self.pop()?;
        let target = self.pop()?;
        let idx = match index {
            Value::Int(n) => n,
            _ => return Err(VmError::new("list index must be int")),
        };
        match target {
            Value::List(items) => {
                let normalized = if idx < 0 {
                    items.len() as i64 + idx
                } else {
                    idx
                };
                if normalized < 0 || normalized as usize >= items.len() {
                    return Err(VmError::new(format!(
                        "index out of range: list length is {}, but index is {}",
                        items.len(),
                        idx
                    )));
                }
                self.push(items[normalized as usize].clone());
                Ok(())
            }
            Value::Bytes(bs) => {
                let normalized = if idx < 0 { bs.len() as i64 + idx } else { idx };
                if normalized < 0 || normalized as usize >= bs.len() {
                    return Err(VmError::new(format!(
                        "index out of range: bytes length is {}, but index is {}",
                        bs.len(),
                        idx
                    )));
                }
                self.push(Value::Int(bs[normalized as usize] as i64));
                Ok(())
            }
            _ => Err(VmError::new("value is not indexable")),
        }
    }

    fn iter_init(&mut self) -> Result<(), VmError> {
        let value = self.pop()?;
        let items = match value {
            Value::List(items) => items,
            Value::Str(s) => s.chars().map(|c| Value::Str(c.to_string())).collect(),
            _ => return Err(VmError::new("value is not iterable")),
        };
        self.iter_stack.push(IteratorState { items, index: 0 });
        Ok(())
    }

    fn iter_next(&mut self, chunk: &Chunk, slot: usize) -> Result<bool, VmError> {
        let iter = match self.iter_stack.last_mut() {
            Some(iter) => iter,
            _ => return Err(VmError::new("expected iterator")),
        };
        if iter.index >= iter.items.len() {
            self.iter_stack.pop();
            return Ok(false);
        }
        let value = iter.items[iter.index].clone();
        iter.index += 1;
        self.current_frame_mut()?.set_value(chunk, slot, value)?;
        Ok(true)
    }

    fn call_builtin(&self, name: &str, args: &[Value]) -> Result<Value, VmError> {
        if name == "print" {
            let rendered: Vec<String> = args.iter().map(|v| format!("{}", v)).collect();
            println!("{}", rendered.join(" "));
            return Ok(Value::Void);
        }
        if name == "len" {
            if args.len() != 1 {
                return Err(VmError::new("len() expects 1 argument"));
            }
            let length = match &args[0] {
                Value::Str(s) => s.len(),
                Value::List(l) => l.len(),
                Value::Bytes(b) => b.len(),
                _ => return Err(VmError::new("len() expects str, list, or bytes")),
            };
            return Ok(Value::Int(length as i64));
        }
        Err(VmError::new(format!("undefined builtin `{}`", name)))
    }

    fn pop_module(&mut self) -> Result<Rc<ModuleValue>, VmError> {
        match self.pop()? {
            Value::Module(module) => Ok(module),
            other => Err(VmError::new(format!(
                "member access not supported for `{}`",
                other
            ))),
        }
    }

    fn call_native(&self, id: NativeFn, args: &[Value]) -> Result<Value, VmError> {
        use dbyte_bytecode::NativeFn::*;
        match id {
            MathAbs => Ok(Value::Int(expect_int(args, 0)?.abs())),
            MathMin => Ok(Value::Int(expect_int(args, 0)?.min(expect_int(args, 1)?))),
            MathMax => Ok(Value::Int(expect_int(args, 0)?.max(expect_int(args, 1)?))),
            FsReadText => {
                let path = expect_str(args, 0)?;
                std::fs::read_to_string(path)
                    .map(Value::Str)
                    .map_err(|e| VmError::new(format!("fs.read_text failed for `{}`: {}", path, e)))
            }
            FsWriteText => {
                let path = expect_str(args, 0)?;
                let text = expect_str(args, 1)?;
                std::fs::write(path, text)
                    .map(|_| Value::Void)
                    .map_err(|e| {
                        VmError::new(format!("fs.write_text failed for `{}`: {}", path, e))
                    })
            }
            FsReadBytes => {
                let path = expect_str(args, 0)?;
                std::fs::read(path).map(Value::Bytes).map_err(|e| {
                    VmError::new(format!("fs.read_bytes failed for `{}`: {}", path, e))
                })
            }
            FsWriteBytes => {
                let path = expect_str(args, 0)?;
                let bytes = expect_bytes(args, 1)?;
                std::fs::write(path, bytes)
                    .map(|_| Value::Void)
                    .map_err(|e| {
                        VmError::new(format!("fs.write_bytes failed for `{}`: {}", path, e))
                    })
            }
            EncodingHexEncode => {
                let bytes = expect_bytes(args, 0)?;
                Ok(Value::Str(hex::encode(bytes)))
            }
            EncodingHexDecode => {
                let s = expect_str(args, 0)?;
                hex::decode(s)
                    .map(Value::Bytes)
                    .map_err(|e| VmError::new(format!("hex_decode failed: {}", e)))
            }
            HashSha256 => {
                use sha2::{Digest, Sha256};
                let bytes = expect_bytes(args, 0)?;
                let mut hasher = Sha256::new();
                hasher.update(bytes);
                let result = hasher.finalize();
                Ok(Value::Bytes(result.to_vec()))
            }
            EnvArgs => Ok(Value::List(std::env::args().map(Value::Str).collect())),
            BufferNew | BufferFromBytes | BufferToBytes | BufferLen | BufferGet | BufferSet
            | BufferSlice | BufferLoad | BufferSave | BufferFind | BufferReplace => {
                self.native_buffer_dispatch(id, args)
            }
            BinaryU8 | BinaryI8 | BinaryU16Le | BinaryU16Be | BinaryI16Le | BinaryI16Be
            | BinaryU32Le | BinaryU32Be | BinaryI32Le | BinaryI32Be | BinaryPackU16Le
            | BinaryPackU16Be | BinaryPackU32Le | BinaryPackU32Be | BinaryWriteU16Le
            | BinaryWriteU16Be | BinaryWriteU32Le | BinaryWriteU32Be => {
                self.native_binary_dispatch(id, args)
            }
        }
    }

    fn module_key(target: &ImportTarget) -> String {
        match target {
            ImportTarget::File(path) => path.to_string_lossy().to_string(),
            ImportTarget::Std(name) => name.clone(),
        }
    }

    fn load_module(&mut self, path: &str, alias: &str) -> Result<ModuleValue, VmError> {
        let target = resolve_import(path, self.current_file.as_deref())
            .map_err(|e| VmError::new(format!("ImportError: {}", format_module_error(&e))))?;
        let key = Self::module_key(&target);
        match self.module_cache.get(&key) {
            Some(ModuleState::Loaded(module)) => {
                let mut module = module.clone();
                module.alias = alias.to_string();
                return Ok(module);
            }
            Some(ModuleState::Loading) => {
                let mut chain = self.loading_stack.clone();
                chain.push(key);
                return Err(VmError::new(format!(
                    "ImportError: circular import detected: {}",
                    chain.join(" -> ")
                )));
            }
            None => {}
        }

        self.module_cache.insert(key.clone(), ModuleState::Loading);
        self.loading_stack.push(key.clone());
        let loaded = match target {
            ImportTarget::Std(name) => self.load_std_module(&name, alias),
            ImportTarget::File(path) => self.load_file_module(&path, alias),
        };
        self.loading_stack.pop();

        match loaded {
            Ok(module) => {
                self.module_cache
                    .insert(key, ModuleState::Loaded(module.clone()));
                Ok(module)
            }
            Err(err) => {
                self.module_cache.remove(&key);
                Err(err)
            }
        }
    }

    fn load_std_module(&self, name: &str, alias: &str) -> Result<ModuleValue, VmError> {
        use dbyte_bytecode::NativeFn;
        use dbyte_module::stdlib_exports;
        let exports = stdlib_exports(name).ok_or_else(|| {
            VmError::new(format!("ImportError: standard module not found: {}", name))
        })?;

        let mut members = HashMap::new();
        for (member_name, _) in exports {
            let full_name = format!("{}.{}", name, member_name);
            let id = NativeFn::from_name(&full_name)
                .ok_or_else(|| VmError::new(format!("unknown native `{}`", full_name)))?;
            members.insert(member_name.clone(), ModuleMember::Native(id));
        }

        Ok(ModuleValue {
            alias: alias.to_string(),
            members,
        })
    }

    fn load_file_module(&mut self, path: &Path, alias: &str) -> Result<ModuleValue, VmError> {
        let program = parse_program(path)?;
        let chunk = Compiler::with_entry_path(path.to_path_buf())
            .compile_program(&program)
            .map_err(compile_error_to_vm)?;

        let saved_file = self.current_file.replace(path.to_path_buf());
        let chunk = Rc::new(chunk);
        self.active_function_tables
            .push(chunk.functions_by_id.iter().cloned().map(Rc::new).collect());
        let executed = self.run_top_chunk(chunk.clone());
        self.active_function_tables.pop();
        self.current_file = saved_file;
        let frame = executed?;

        let mut members = HashMap::new();
        let frame = frame.ok_or_else(|| VmError::new("no module VM frame"))?;
        for (name, slot) in &chunk.public_values {
            let value = frame.get_value(&chunk, *slot)?;
            members.insert(name.clone(), ModuleMember::Value(value));
        }
        for name in &chunk.public_functions {
            if let Some(function) = chunk.functions.get(name).cloned() {
                members.insert(name.clone(), ModuleMember::Function(Box::new(function)));
            }
        }
        Ok(ModuleValue {
            alias: alias.to_string(),
            members,
        })
    }

    fn read_u32_le(&mut self) -> Result<(), VmError> {
        let offset_value = match self.stack.pop() {
            Some(Value::Int(n)) => n,
            Some(other) => {
                return Err(VmError::new(format!(
                    "expected int on stack, found {}",
                    other.kind_name()
                )))
            }
            None => return Err(VmError::new("stack underflow")),
        };
        let offset = self.checked_offset(offset_value)?;
        let data = match self.pop()? {
            Value::Bytes(bytes) => bytes,
            other => {
                return Err(VmError::new(format!(
                    "expected bytes argument 1, found {}",
                    other.kind_name()
                )))
            }
        };
        self.require_len(&data, offset, 4)?;
        self.push(Value::Int(LE::read_u32(&data[offset..]) as i64));
        Ok(())
    }

    fn buffer_find(&mut self) -> Result<(), VmError> {
        let pattern = match self.pop()? {
            Value::Bytes(bytes) => bytes,
            other => {
                return Err(VmError::new(format!(
                    "expected bytes argument 2, found {}",
                    other.kind_name()
                )))
            }
        };
        if pattern.is_empty() {
            return Err(VmError::new("buffer.find: pattern cannot be empty"));
        }
        let buffer = match self.pop()? {
            Value::Buffer(buffer) => buffer,
            other => {
                return Err(VmError::new(format!(
                    "expected buffer argument 1, found {}",
                    other.kind_name()
                )))
            }
        };
        let buf = buffer.borrow();
        let pos = buf
            .windows(pattern.len())
            .position(|w| w == pattern)
            .map(|p| p as i64)
            .unwrap_or(-1);
        self.push(Value::Int(pos));
        Ok(())
    }

    fn buffer_replace(&mut self) -> Result<(), VmError> {
        let data = match self.pop()? {
            Value::Bytes(bytes) => bytes,
            other => {
                return Err(VmError::new(format!(
                    "expected bytes argument 3, found {}",
                    other.kind_name()
                )))
            }
        };
        let offset_value = match self.stack.pop() {
            Some(Value::Int(n)) => n,
            Some(other) => {
                return Err(VmError::new(format!(
                    "expected int on stack, found {}",
                    other.kind_name()
                )))
            }
            None => return Err(VmError::new("stack underflow")),
        };
        let offset = self.checked_offset(offset_value)?;
        let buffer = match self.pop()? {
            Value::Buffer(buffer) => buffer,
            other => {
                return Err(VmError::new(format!(
                    "expected buffer argument 1, found {}",
                    other.kind_name()
                )))
            }
        };
        let mut buf = buffer.borrow_mut();
        let end = offset + data.len();
        if end > buf.len() {
            return Err(VmError::new(format!(
                "buffer.replace out of range: need {} bytes at offset {}, but length is {}",
                data.len(),
                offset,
                buf.len()
            )));
        }
        buf[offset..end].copy_from_slice(&data);
        self.push(Value::Void);
        Ok(())
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_program(path: &Path) -> Result<dbyte_ast::Program, VmError> {
    let src = std::fs::read_to_string(path).map_err(|e| {
        VmError::new(format!(
            "ImportError: cannot read `{}`: {}",
            path.display(),
            e
        ))
    })?;
    let tokens = Lexer::new(&src).tokenize().map_err(|e| VmError {
        msg: e.msg,
        span: e.span,
    })?;
    Parser::new(tokens).parse_program().map_err(|e| VmError {
        msg: e.msg,
        span: e.span,
    })
}

fn compile_error_to_vm(error: CompileError) -> VmError {
    VmError {
        msg: error.msg,
        span: error.span,
    }
}

fn expect_int(args: &[Value], idx: usize) -> Result<i64, VmError> {
    match args.get(idx) {
        Some(Value::Int(n)) => Ok(*n),
        Some(other) => Err(VmError::new(format!(
            "expected int argument {}, found {}",
            idx + 1,
            other.kind_name()
        ))),
        None => Err(VmError::new(format!("missing argument {}", idx + 1))),
    }
}

fn checked_i64(value: Option<i64>) -> Result<i64, VmError> {
    value.ok_or_else(|| VmError::new("integer overflow"))
}

fn expect_str(args: &[Value], idx: usize) -> Result<&str, VmError> {
    match args.get(idx) {
        Some(Value::Str(s)) => Ok(s),
        Some(other) => Err(VmError::new(format!(
            "expected str argument {}, found {}",
            idx + 1,
            other.kind_name()
        ))),
        None => Err(VmError::new(format!("missing argument {}", idx + 1))),
    }
}

fn expect_bytes(args: &[Value], idx: usize) -> Result<&[u8], VmError> {
    match args.get(idx) {
        Some(Value::Bytes(bs)) => Ok(bs),
        Some(other) => Err(VmError::new(format!(
            "expected bytes argument {}, found {}",
            idx + 1,
            other.kind_name()
        ))),
        None => Err(VmError::new(format!("missing argument {}", idx + 1))),
    }
}

fn expect_buffer(args: &[Value], idx: usize) -> Result<Rc<RefCell<Vec<u8>>>, VmError> {
    match args.get(idx) {
        Some(Value::Buffer(b)) => Ok(b.clone()),
        Some(other) => Err(VmError::new(format!(
            "expected buffer argument {}, found {}",
            idx + 1,
            other.kind_name()
        ))),
        None => Err(VmError::new(format!("missing argument {}", idx + 1))),
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

impl Vm {
    fn native_binary_dispatch(&self, id: NativeFn, args: &[Value]) -> Result<Value, VmError> {
        use dbyte_bytecode::NativeFn::*;
        match id {
            BinaryU8 | BinaryI8 | BinaryU16Le | BinaryU16Be | BinaryI16Le | BinaryI16Be
            | BinaryU32Le | BinaryU32Be | BinaryI32Le | BinaryI32Be => {
                self.native_binary_read(id, args)
            }
            BinaryPackU16Le | BinaryPackU16Be | BinaryPackU32Le | BinaryPackU32Be => {
                self.native_binary_pack(id, args)
            }
            BinaryWriteU16Le | BinaryWriteU16Be | BinaryWriteU32Le | BinaryWriteU32Be => {
                self.native_binary_write(id, args)
            }
            _ => unreachable!(),
        }
    }

    fn native_buffer_dispatch(&self, id: NativeFn, args: &[Value]) -> Result<Value, VmError> {
        use dbyte_bytecode::NativeFn::*;
        match id {
            BufferNew => {
                let size = expect_int(args, 0)?;
                if size < 0 {
                    return Err(VmError::new("buffer size must be non-negative"));
                }
                Ok(Value::Buffer(Rc::new(RefCell::new(vec![
                    0u8;
                    size as usize
                ]))))
            }
            BufferFromBytes => {
                let bs = expect_bytes(args, 0)?;
                Ok(Value::Buffer(Rc::new(RefCell::new(bs.to_vec()))))
            }
            BufferToBytes => {
                let b = expect_buffer(args, 0)?;
                let val = Value::Bytes(b.borrow().clone());
                Ok(val)
            }
            BufferLen => {
                let b = expect_buffer(args, 0)?;
                let len = b.borrow().len() as i64;
                Ok(Value::Int(len))
            }
            BufferGet => {
                let b = expect_buffer(args, 0)?;
                let offset = self.checked_offset(expect_int(args, 1)?)?;
                let buf = b.borrow();
                if offset >= buf.len() {
                    return Err(VmError::new(format!(
                        "buffer get out of range: offset {}, but length is {}",
                        offset,
                        buf.len()
                    )));
                }
                Ok(Value::Int(buf[offset] as i64))
            }
            BufferSet => {
                let b = expect_buffer(args, 0)?;
                let offset = self.checked_offset(expect_int(args, 1)?)?;
                let val = expect_int(args, 2)?;
                if !(0..=255).contains(&val) {
                    return Err(VmError::new(format!(
                        "buffer set value out of range: {}",
                        val
                    )));
                }
                let mut buf = b.borrow_mut();
                if offset >= buf.len() {
                    return Err(VmError::new(format!(
                        "buffer set out of range: offset {}, but length is {}",
                        offset,
                        buf.len()
                    )));
                }
                buf[offset] = val as u8;
                Ok(Value::Void)
            }
            BufferSlice => {
                let b = expect_buffer(args, 0)?;
                let offset = self.checked_offset(expect_int(args, 1)?)?;
                let length = expect_int(args, 2)?;
                if length < 0 {
                    return Err(VmError::new("length must be non-negative"));
                }
                let buf = b.borrow();
                if offset
                    .checked_add(length as usize)
                    .is_none_or(|end| end > buf.len())
                {
                    return Err(VmError::new(format!(
                        "buffer slice out of range: need {} bytes at offset {}, but length is {}",
                        length,
                        offset,
                        buf.len()
                    )));
                }
                let start = offset;
                let end = start + length as usize;
                Ok(Value::Bytes(buf[start..end].to_vec()))
            }
            BufferLoad => {
                let path = expect_str(args, 0)?;
                let data = std::fs::read(path).map_err(|e| {
                    VmError::new(format!("buffer.load failed for `{}`: {}", path, e))
                })?;
                Ok(Value::Buffer(Rc::new(RefCell::new(data))))
            }
            BufferSave => {
                let path = expect_str(args, 0)?;
                let b = expect_buffer(args, 1)?;
                std::fs::write(path, &*b.borrow()).map_err(|e| {
                    VmError::new(format!("buffer.save failed for `{}`: {}", path, e))
                })?;
                Ok(Value::Void)
            }
            BufferFind => {
                let b = expect_buffer(args, 0)?;
                let pattern = expect_bytes(args, 1)?;
                if pattern.is_empty() {
                    return Err(VmError::new("buffer.find: pattern cannot be empty"));
                }
                let buf = b.borrow();
                let pos = buf
                    .windows(pattern.len())
                    .position(|w| w == pattern)
                    .map(|p| p as i64)
                    .unwrap_or(-1);
                Ok(Value::Int(pos))
            }
            BufferReplace => {
                let b = expect_buffer(args, 0)?;
                let offset = self.checked_offset(expect_int(args, 1)?)?;
                let data = expect_bytes(args, 2)?;
                let mut buf = b.borrow_mut();
                let end = offset + data.len();
                if end > buf.len() {
                    return Err(VmError::new(format!(
                        "buffer.replace out of range: need {} bytes at offset {}, but length is {}",
                        data.len(),
                        offset,
                        buf.len()
                    )));
                }
                buf[offset..end].copy_from_slice(data);
                Ok(Value::Void)
            }
            _ => unreachable!(),
        }
    }

    fn native_binary_write(&self, id: NativeFn, args: &[Value]) -> Result<Value, VmError> {
        use dbyte_bytecode::NativeFn::*;
        let b = expect_buffer(args, 0)?;
        let offset = self.checked_offset(expect_int(args, 1)?)?;
        let val = expect_int(args, 2)?;
        let mut buf = b.borrow_mut();

        match id {
            BinaryWriteU16Le => {
                if !(0..=65535).contains(&val) {
                    return Err(VmError::new(format!("value {} out of u16 range", val)));
                }
                if offset + 2 > buf.len() {
                    return Err(VmError::new(format!(
                        "write out of range: need 2 bytes at offset {}, but length is {}",
                        offset,
                        buf.len()
                    )));
                }
                LE::write_u16(&mut buf[offset..], val as u16);
            }
            BinaryWriteU16Be => {
                if !(0..=65535).contains(&val) {
                    return Err(VmError::new(format!("value {} out of u16 range", val)));
                }
                if offset + 2 > buf.len() {
                    return Err(VmError::new(format!(
                        "write out of range: need 2 bytes at offset {}, but length is {}",
                        offset,
                        buf.len()
                    )));
                }
                BE::write_u16(&mut buf[offset..], val as u16);
            }
            BinaryWriteU32Le => {
                if !(0..=4294967295).contains(&val) {
                    return Err(VmError::new(format!("value {} out of u32 range", val)));
                }
                if offset + 4 > buf.len() {
                    return Err(VmError::new(format!(
                        "write out of range: need 4 bytes at offset {}, but length is {}",
                        offset,
                        buf.len()
                    )));
                }
                LE::write_u32(&mut buf[offset..], val as u32);
            }
            BinaryWriteU32Be => {
                if !(0..=4294967295).contains(&val) {
                    return Err(VmError::new(format!("value {} out of u32 range", val)));
                }
                if offset + 4 > buf.len() {
                    return Err(VmError::new(format!(
                        "write out of range: need 4 bytes at offset {}, but length is {}",
                        offset,
                        buf.len()
                    )));
                }
                BE::write_u32(&mut buf[offset..], val as u32);
            }
            _ => unreachable!(),
        }
        Ok(Value::Void)
    }

    fn checked_offset(&self, offset: i64) -> Result<usize, VmError> {
        if offset < 0 {
            return Err(VmError::new("offset must be non-negative"));
        }
        Ok(offset as usize)
    }

    fn require_len(&self, data: &[u8], offset: usize, width: usize) -> Result<(), VmError> {
        if offset.checked_add(width).is_none_or(|end| end > data.len()) {
            return Err(VmError::new(format!(
                "read out of range: need {} bytes at offset {}, but length is {}",
                width,
                offset,
                data.len()
            )));
        }
        Ok(())
    }

    fn native_binary_read(&self, id: NativeFn, args: &[Value]) -> Result<Value, VmError> {
        use dbyte_bytecode::NativeFn::*;
        let bs = expect_bytes(args, 0)?;
        let offset = self.checked_offset(expect_int(args, 1)?)?;

        match id {
            BinaryU8 => {
                self.require_len(bs, offset, 1)?;
                Ok(Value::Int(bs[offset] as i64))
            }
            BinaryI8 => {
                self.require_len(bs, offset, 1)?;
                Ok(Value::Int(bs[offset] as i8 as i64))
            }
            BinaryU16Le => {
                self.require_len(bs, offset, 2)?;
                Ok(Value::Int(LE::read_u16(&bs[offset..]) as i64))
            }
            BinaryU16Be => {
                self.require_len(bs, offset, 2)?;
                Ok(Value::Int(BE::read_u16(&bs[offset..]) as i64))
            }
            BinaryI16Le => {
                self.require_len(bs, offset, 2)?;
                Ok(Value::Int(LE::read_i16(&bs[offset..]) as i64))
            }
            BinaryI16Be => {
                self.require_len(bs, offset, 2)?;
                Ok(Value::Int(BE::read_i16(&bs[offset..]) as i64))
            }
            BinaryU32Le => {
                self.require_len(bs, offset, 4)?;
                Ok(Value::Int(LE::read_u32(&bs[offset..]) as i64))
            }
            BinaryU32Be => {
                self.require_len(bs, offset, 4)?;
                Ok(Value::Int(BE::read_u32(&bs[offset..]) as i64))
            }
            BinaryI32Le => {
                self.require_len(bs, offset, 4)?;
                Ok(Value::Int(LE::read_i32(&bs[offset..]) as i64))
            }
            BinaryI32Be => {
                self.require_len(bs, offset, 4)?;
                Ok(Value::Int(BE::read_i32(&bs[offset..]) as i64))
            }
            _ => unreachable!(),
        }
    }

    fn native_binary_pack(&self, id: NativeFn, args: &[Value]) -> Result<Value, VmError> {
        use dbyte_bytecode::NativeFn::*;
        let val = expect_int(args, 0)?;
        let mut buf = vec![
            0u8;
            match id {
                BinaryPackU16Le | BinaryPackU16Be => 2,
                BinaryPackU32Le | BinaryPackU32Be => 4,
                _ => unreachable!(),
            }
        ];

        match id {
            BinaryPackU16Le => {
                if !(0..=65535).contains(&val) {
                    return Err(VmError::new(format!(
                        "std.binary.pack_u16_le failed: value {} out of u16 range",
                        val
                    )));
                }
                LE::write_u16(&mut buf, val as u16);
            }
            BinaryPackU16Be => {
                if !(0..=65535).contains(&val) {
                    return Err(VmError::new(format!(
                        "std.binary.pack_u16_be failed: value {} out of u16 range",
                        val
                    )));
                }
                BE::write_u16(&mut buf, val as u16);
            }
            BinaryPackU32Le => {
                if !(0..=4294967295).contains(&val) {
                    return Err(VmError::new(format!(
                        "std.binary.pack_u32_le failed: value {} out of u32 range",
                        val
                    )));
                }
                LE::write_u32(&mut buf, val as u32);
            }
            BinaryPackU32Be => {
                if !(0..=4294967295).contains(&val) {
                    return Err(VmError::new(format!(
                        "std.binary.pack_u32_be failed: value {} out of u32 range",
                        val
                    )));
                }
                BE::write_u32(&mut buf, val as u32);
            }
            _ => unreachable!(),
        }
        Ok(Value::Bytes(buf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbyte_bytecode::{BytecodeFunction, Chunk, Op, Value};

    #[test]
    fn reports_stack_underflow() {
        let mut chunk = Chunk::new("test");
        chunk.code.push(Op::Pop);
        chunk.code.push(Op::Halt);

        let mut vm = Vm::new();
        let err = vm.run(&chunk).unwrap_err();

        assert!(err.msg.contains("stack underflow"));
    }

    #[test]
    fn reports_invalid_local_slot() {
        let mut chunk = Chunk::new("test");
        chunk.code.push(Op::LoadLocal(99));
        chunk.code.push(Op::Halt);

        let mut vm = Vm::new();
        let err = vm.run(&chunk).unwrap_err();

        assert!(err.msg.contains("invalid local slot 99"));
    }

    #[test]
    fn reports_function_arity_mismatch() {
        let mut function_chunk = Chunk::new("add");
        function_chunk.local_names = vec!["a".into(), "b".into()];
        let zero = function_chunk.add_const(Value::Int(0));
        function_chunk.code.push(Op::Const(zero));
        function_chunk.code.push(Op::Return);

        let function = BytecodeFunction {
            name: "add".into(),
            params: vec!["a".into(), "b".into()],
            chunk: function_chunk,
        };

        let mut chunk = Chunk::new("test");
        let one = chunk.add_const(Value::Int(1));
        chunk.functions.insert("add".into(), function);
        chunk.code.push(Op::Const(one));
        chunk.code.push(Op::Call("add".into(), 1));
        chunk.code.push(Op::Halt);

        let mut vm = Vm::new();
        let err = vm.run(&chunk).unwrap_err();

        assert!(err.msg.contains("function `add` expects 2 args, got 1"));
    }

    #[test]
    fn reports_i64_stack_call_underflow() {
        let mut function_chunk = Chunk::new("id");
        function_chunk.local_names = vec!["n".into()];
        function_chunk.code.push(Op::ConstI64Stack(0));
        function_chunk.code.push(Op::ReturnI64ToI64Stack);

        let function = BytecodeFunction {
            name: "id".into(),
            params: vec!["n".into()],
            chunk: function_chunk,
        };

        let mut chunk = Chunk::new("test");
        chunk.functions_by_id.push(function);
        chunk.code.push(Op::CallFnI64ToI64Stack { id: 0, argc: 1 });
        chunk.code.push(Op::Halt);

        let mut vm = Vm::new();
        let err = vm.run(&chunk).unwrap_err();

        assert!(err.msg.contains("i64 stack underflow"));
    }

    #[test]
    fn reports_i64_stack_function_arity_mismatch() {
        let mut function_chunk = Chunk::new("add");
        function_chunk.local_names = vec!["a".into(), "b".into()];
        function_chunk.code.push(Op::ConstI64Stack(0));
        function_chunk.code.push(Op::ReturnI64ToI64Stack);

        let function = BytecodeFunction {
            name: "add".into(),
            params: vec!["a".into(), "b".into()],
            chunk: function_chunk,
        };

        let mut chunk = Chunk::new("test");
        chunk.functions_by_id.push(function);
        chunk.code.push(Op::ConstI64Stack(1));
        chunk.code.push(Op::CallFnI64ToI64Stack { id: 0, argc: 1 });
        chunk.code.push(Op::Halt);

        let mut vm = Vm::new();
        let err = vm.run(&chunk).unwrap_err();

        assert!(err.msg.contains("function `add` expects 2 args, got 1"));
    }

    #[test]
    fn reports_i64_stack_recursion_depth() {
        let mut function_chunk = Chunk::new("spin");
        function_chunk.code.push(Op::ConstI64Stack(1));
        function_chunk
            .code
            .push(Op::CallFnI64ToI64Stack { id: 0, argc: 1 });
        function_chunk.code.push(Op::ReturnI64ToI64Stack);

        let function = BytecodeFunction {
            name: "spin".into(),
            params: vec!["n".into()],
            chunk: function_chunk,
        };

        let mut chunk = Chunk::new("test");
        chunk.functions_by_id.push(function);
        chunk.code.push(Op::ConstI64Stack(0));
        chunk.code.push(Op::CallFnI64ToI64Stack { id: 0, argc: 1 });
        chunk.code.push(Op::Halt);

        let mut vm = Vm::new();
        let err = vm.run(&chunk).unwrap_err();

        assert!(err.msg.contains("maximum call depth exceeded"));
    }
}
