use byteorder::{ByteOrder, BE, LE};
use dbyte_ast::{FStrPart, Span};
use dbyte_bytecode::{
    format_op, BytecodeFunction, Chunk, ModuleMember, ModuleValue, NativeFn, Op, Value,
};
use dbyte_compiler::{CompileError, Compiler};
use dbyte_lexer::Lexer;
use dbyte_module::{resolve_import, ImportTarget, ModuleError, ModuleState};
use dbyte_parser::Parser;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

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

pub struct Vm {
    stack: Vec<Value>,
    iter_stack: Vec<IteratorState>,
    frames: Vec<Vec<Value>>,
    trace: bool,
    current_file: Option<PathBuf>,
    module_cache: HashMap<String, ModuleState<ModuleValue>>,
    loading_stack: Vec<String>,
}

impl Vm {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            iter_stack: Vec::new(),
            frames: Vec::new(),
            trace: false,
            current_file: None,
            module_cache: HashMap::new(),
            loading_stack: Vec::new(),
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
        self.frames.push(vec![Value::Void; chunk.local_names.len()]);
        let result = self.run_chunk(chunk);
        self.frames.pop();
        match result {
            Ok(Some(_)) => Err(VmError::new("return outside function")),
            Ok(None) => Ok(()),
            Err(err) => Err(err),
        }
    }

    fn run_chunk(&mut self, chunk: &Chunk) -> Result<Option<Value>, VmError> {
        let mut ip = 0usize;
        while ip < chunk.code.len() {
            let op = chunk.code[ip].clone();
            if self.trace {
                println!(
                    "ip={:04} op={} stack=[{}]",
                    ip,
                    format_op(&op, chunk),
                    self.value_stack().join(", ")
                );
            }
            ip += 1;
            match op {
                Op::Const(idx) => self.push(chunk.constants[idx].clone()),
                Op::FStr(parts) => self.eval_fstr(&parts, chunk)?,
                Op::Add => self.binary_add()?,
                Op::Sub => self.binary_num("sub", |a, b| a - b, |a, b| a - b)?,
                Op::Mul => self.binary_num("mul", |a, b| a * b, |a, b| a * b)?,
                Op::Div => self.binary_div()?,
                Op::Eq => self.binary_cmp(|a, b| a == b)?,
                Op::Ne => self.binary_cmp(|a, b| a != b)?,
                Op::Lt => self.binary_ord("<", |a, b| a < b, |a, b| a < b)?,
                Op::Le => self.binary_ord("<=", |a, b| a <= b, |a, b| a <= b)?,
                Op::Gt => self.binary_ord(">", |a, b| a > b, |a, b| a > b)?,
                Op::Ge => self.binary_ord(">=", |a, b| a >= b, |a, b| a >= b)?,
                Op::Neg => self.unary_neg()?,
                Op::Not => self.unary_not()?,
                Op::MakeList(count) => self.make_list(count)?,
                Op::Index => self.index()?,
                Op::LoadLocal(slot) => {
                    let value = self
                        .current_frame()?
                        .get(slot)
                        .cloned()
                        .ok_or_else(|| VmError::new(format!("invalid local slot {}", slot)))?;
                    self.push(value);
                }
                Op::StoreLocal(slot) => {
                    let value = self.pop()?;
                    let frame = self.current_frame_mut()?;
                    if slot >= frame.len() {
                        frame.resize(slot + 1, Value::Void);
                    }
                    frame[slot] = value;
                }
                Op::Import(path, slot) => {
                    let alias = chunk.local_names.get(slot).cloned().unwrap_or_default();
                    let module = self.load_module(&path, &alias)?;
                    let frame = self.current_frame_mut()?;
                    if slot >= frame.len() {
                        frame.resize(slot + 1, Value::Void);
                    }
                    frame[slot] = Value::Module(Rc::new(module));
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
                        ModuleMember::Function(f) => self.call_function(&f, args_start)?,
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
                Op::IterInit => self.iter_init()?,
                Op::IterNext { slot, jump } => {
                    let should_continue = self.iter_next(slot)?;
                    if !should_continue {
                        ip = jump;
                    }
                }
                Op::Jump(target) => ip = target,
                Op::JumpIfFalse(target) => {
                    if !self.pop_bool("jump condition")? {
                        ip = target;
                    }
                }
                Op::Call(name, argc) => {
                    let args_start = self.stack.len() - argc;
                    if name == "print" || name == "len" {
                        let value = self.call_builtin(&name, &self.stack[args_start..])?;
                        self.stack.truncate(args_start);
                        self.push(value);
                    } else {
                        let function = chunk.functions.get(&name).ok_or_else(|| {
                            VmError::new(format!("undefined function `{}`", name))
                        })?;
                        let value = self.call_function(function, args_start)?;
                        self.push(value);
                    }
                }
                Op::Return => return Ok(Some(self.pop()?)),
                Op::Pop => {
                    self.pop()?;
                }
                Op::Halt => break,
            }
        }
        Ok(None)
    }

    fn current_frame(&self) -> Result<&Vec<Value>, VmError> {
        self.frames
            .last()
            .ok_or_else(|| VmError::new("no active VM frame"))
    }

    fn current_frame_mut(&mut self) -> Result<&mut Vec<Value>, VmError> {
        self.frames
            .last_mut()
            .ok_or_else(|| VmError::new("no active VM frame"))
    }

    fn push(&mut self, value: Value) {
        self.stack.push(value);
    }

    fn pop(&mut self) -> Result<Value, VmError> {
        self.stack
            .pop()
            .ok_or_else(|| VmError::new("stack underflow"))
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
                    out.push_str(&format!("{}", self.current_frame()?[slot]));
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
            (Value::Int(a), Value::Int(b)) => Value::Int(a + b),
            (Value::Float(a), Value::Float(b)) => Value::Float(a + b),
            (Value::Str(a), Value::Str(b)) => Value::Str(a + &b),
            _ => return Err(VmError::new("type mismatch in binary expression")),
        };
        self.push(value);
        Ok(())
    }

    fn binary_num(
        &mut self,
        label: &str,
        int_op: fn(i64, i64) -> i64,
        float_op: fn(f64, f64) -> f64,
    ) -> Result<(), VmError> {
        let b = self.pop()?;
        let a = self.pop()?;
        let value = match (a, b) {
            (Value::Int(a), Value::Int(b)) => Value::Int(int_op(a, b)),
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
            (Value::Int(a), Value::Int(b)) => Value::Int(a / b),
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
            Value::Int(n) => Value::Int(-n),
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

    fn iter_next(&mut self, slot: usize) -> Result<bool, VmError> {
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
        let frame = self.current_frame_mut()?;
        if slot >= frame.len() {
            frame.resize(slot + 1, Value::Void);
        }
        frame[slot] = value;
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

    fn call_function(
        &mut self,
        function: &BytecodeFunction,
        args_start: usize,
    ) -> Result<Value, VmError> {
        let argc = self.stack.len() - args_start;
        if argc != function.params.len() {
            return Err(VmError::new(format!(
                "function `{}` expects {} args, got {}",
                function.name,
                function.params.len(),
                argc
            )));
        }
        let mut frame = vec![Value::Void; function.chunk.local_names.len()];
        for (idx, arg) in self.stack.drain(args_start..).enumerate() {
            if idx < frame.len() {
                frame[idx] = arg;
            }
        }
        self.frames.push(frame);
        let result = self.run_chunk(&function.chunk)?;
        self.frames.pop();
        Ok(result.unwrap_or(Value::Void))
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
        self.frames.push(vec![Value::Void; chunk.local_names.len()]);
        let executed = self.run_chunk(&chunk);
        let frame = self.frames.pop().unwrap_or_default();
        self.current_file = saved_file;
        executed?;

        let mut members = HashMap::new();
        for (name, slot) in &chunk.public_values {
            if let Some(value) = frame.get(*slot).cloned() {
                members.insert(name.clone(), ModuleMember::Value(value));
            }
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
}
