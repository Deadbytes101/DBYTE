use dbyte_ast::{FStrPart, Span};
use dbyte_bytecode::{format_op, BytecodeFunction, Chunk, ModuleMember, ModuleValue, Op, Value};
use dbyte_compiler::{CompileError, Compiler};
use dbyte_lexer::Lexer;
use dbyte_module::{resolve_import, ImportTarget, ModuleError, ModuleState};
use dbyte_parser::Parser;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
enum StackValue {
    Value(Value),
    Iter(IteratorState),
}

pub struct Vm {
    stack: Vec<StackValue>,
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
                    frame[slot] = Value::Module(module);
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
                    let args = self.pop_args(argc)?;
                    let module = self.pop_module()?;
                    let member = module.members.get(&property).cloned().ok_or_else(|| {
                        VmError::new(format!(
                            "module '{}' has no public member '{}'",
                            module.alias, property
                        ))
                    })?;
                    let value = self.call_module_member(&property, member, args)?;
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
                    let args = self.pop_args(argc)?;
                    let value = self.call(&name, args, chunk)?;
                    self.push(value);
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
        self.stack.push(StackValue::Value(value));
    }

    fn pop(&mut self) -> Result<Value, VmError> {
        match self.stack.pop() {
            Some(StackValue::Value(value)) => Ok(value),
            Some(StackValue::Iter(_)) => Err(VmError::new("expected value, found iterator")),
            None => Err(VmError::new("stack underflow")),
        }
    }

    fn pop_args(&mut self, argc: usize) -> Result<Vec<Value>, VmError> {
        let mut args = Vec::with_capacity(argc);
        for _ in 0..argc {
            args.push(self.pop()?);
        }
        args.reverse();
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
        self.stack
            .iter()
            .filter_map(|v| match v {
                StackValue::Value(value) => Some(format!("{}", value)),
                StackValue::Iter(_) => None,
            })
            .collect()
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
                    return Err(VmError::new("index out of range"));
                }
                self.push(items[normalized as usize].clone());
                Ok(())
            }
            Value::Bytes(bs) => {
                let normalized = if idx < 0 { bs.len() as i64 + idx } else { idx };
                if normalized < 0 || normalized as usize >= bs.len() {
                    return Err(VmError::new("index out of range"));
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
        self.stack
            .push(StackValue::Iter(IteratorState { items, index: 0 }));
        Ok(())
    }

    fn iter_next(&mut self, slot: usize) -> Result<bool, VmError> {
        let iter = match self.stack.last_mut() {
            Some(StackValue::Iter(iter)) => iter,
            _ => return Err(VmError::new("expected iterator")),
        };
        if iter.index >= iter.items.len() {
            self.stack.pop();
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

    fn call(&mut self, name: &str, args: Vec<Value>, chunk: &Chunk) -> Result<Value, VmError> {
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
        let function = chunk
            .functions
            .get(name)
            .ok_or_else(|| VmError::new(format!("undefined function `{}`", name)))?;
        self.call_function(function, args)
    }

    fn call_function(
        &mut self,
        function: &BytecodeFunction,
        args: Vec<Value>,
    ) -> Result<Value, VmError> {
        if args.len() != function.params.len() {
            return Err(VmError::new(format!(
                "function `{}` expects {} args, got {}",
                function.name,
                function.params.len(),
                args.len()
            )));
        }
        let mut frame = vec![Value::Void; function.chunk.local_names.len()];
        for (idx, arg) in args.into_iter().enumerate() {
            if idx >= frame.len() {
                frame.push(arg);
            } else {
                frame[idx] = arg;
            }
        }
        self.frames.push(frame);
        let result = self.run_chunk(&function.chunk)?;
        self.frames.pop();
        Ok(result.unwrap_or(Value::Void))
    }

    fn pop_module(&mut self) -> Result<ModuleValue, VmError> {
        match self.pop()? {
            Value::Module(module) => Ok(module),
            other => Err(VmError::new(format!(
                "member access not supported for `{}`",
                other
            ))),
        }
    }

    fn call_module_member(
        &mut self,
        name: &str,
        member: ModuleMember,
        args: Vec<Value>,
    ) -> Result<Value, VmError> {
        match member {
            ModuleMember::Function(function) => self.call_function(&function, args),
            ModuleMember::Native(native) => self.call_native(&native, args),
            ModuleMember::Value(_) => Err(VmError::new(format!(
                "module member `{}` is not callable",
                name
            ))),
        }
    }

    fn call_native(&self, name: &str, args: Vec<Value>) -> Result<Value, VmError> {
        match name {
            "std.math.abs" => Ok(Value::Int(expect_int(&args, 0)?.abs())),
            "std.math.min" => Ok(Value::Int(expect_int(&args, 0)?.min(expect_int(&args, 1)?))),
            "std.math.max" => Ok(Value::Int(expect_int(&args, 0)?.max(expect_int(&args, 1)?))),
            "std.fs.read_text" => {
                let path = expect_str(&args, 0)?;
                std::fs::read_to_string(path)
                    .map(Value::Str)
                    .map_err(|e| VmError::new(format!("fs.read_text failed for `{}`: {}", path, e)))
            }
            "std.fs.write_text" => {
                let path = expect_str(&args, 0)?;
                let text = expect_str(&args, 1)?;
                std::fs::write(path, text)
                    .map(|_| Value::Void)
                    .map_err(|e| {
                        VmError::new(format!("fs.write_text failed for `{}`: {}", path, e))
                    })
            }
            "std.fs.read_bytes" => {
                let path = expect_str(&args, 0)?;
                std::fs::read(path).map(Value::Bytes).map_err(|e| {
                    VmError::new(format!("fs.read_bytes failed for `{}`: {}", path, e))
                })
            }
            "std.fs.write_bytes" => {
                let path = expect_str(&args, 0)?;
                let bytes = expect_bytes(&args, 1)?;
                std::fs::write(path, bytes)
                    .map(|_| Value::Void)
                    .map_err(|e| {
                        VmError::new(format!("fs.write_bytes failed for `{}`: {}", path, e))
                    })
            }
            "std.encoding.hex_encode" => {
                let bytes = expect_bytes(&args, 0)?;
                Ok(Value::Str(hex::encode(bytes)))
            }
            "std.encoding.hex_decode" => {
                let s = expect_str(&args, 0)?;
                hex::decode(s)
                    .map(Value::Bytes)
                    .map_err(|e| VmError::new(format!("hex_decode failed: {}", e)))
            }
            "std.hash.sha256" => {
                use sha2::{Digest, Sha256};
                let bytes = expect_bytes(&args, 0)?;
                let mut hasher = Sha256::new();
                hasher.update(bytes);
                let result = hasher.finalize();
                Ok(Value::Bytes(result.to_vec()))
            }
            "std.env.args" => Ok(Value::List(std::env::args().map(Value::Str).collect())),
            _ => Err(VmError::new(format!("unknown native `{}`", name))),
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
        let names = match name {
            "std.math" => vec!["abs", "min", "max"],
            "std.fs" => vec!["read_text", "write_text", "read_bytes", "write_bytes"],
            "std.encoding" => vec!["hex_encode", "hex_decode"],
            "std.hash" => vec!["sha256"],
            "std.env" => vec!["args"],
            _ => {
                return Err(VmError::new(format!(
                    "ImportError: standard module not found: {}",
                    name
                )))
            }
        };
        let mut members = HashMap::new();
        for member in names {
            members.insert(
                member.to_string(),
                ModuleMember::Native(format!("{}.{}", name, member)),
            );
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
                members.insert(name.clone(), ModuleMember::Function(function));
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
            other
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
            other
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
            other
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
