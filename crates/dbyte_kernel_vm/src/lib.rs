#![no_std]

pub mod opcode {
    pub const PUSH_INT: u8 = 0x01;
    pub const PUSH_STR_CONST: u8 = 0x02;
    pub const ADD: u8 = 0x03;
    pub const PRINT: u8 = 0x04;
    pub const KCALL: u8 = 0x05;
    pub const HALT: u8 = 0xff;
}

pub mod value {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Value {
        Int(i32),
        StrConst(u16),
    }
}

pub mod vm {
    use crate::opcode;
    use crate::value::Value;

    pub const STACK_CAPACITY: usize = 16;

    pub trait VmOutput {
        fn write_str(&mut self, value: &str);
        fn write_i32(&mut self, value: i32);

        fn clear_log(&mut self) {}
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum VmHostResult {
        None,
        PushI32(i32),
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum VmHostArgSpec {
        None,
        I32,
        StrConst,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum VmHostArgs<'a> {
        None,
        I32(i32),
        StrConst(&'a str),
    }

    pub trait VmHost {
        fn arg_spec(&self, service_id: u8) -> Result<VmHostArgSpec, VmError>;

        fn call<O: VmOutput>(
            &mut self,
            service_id: u8,
            args: VmHostArgs<'_>,
            output: &mut O,
        ) -> Result<VmHostResult, VmError>;
    }

    pub struct NoHost;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum VmError {
        StackOverflow,
        StackUnderflow,
        TypeMismatch,
        StrConstIndexOutOfBounds,
        UnexpectedEnd,
        UnsupportedService(u8),
        UnknownOpcode(u8),
        MissingHalt,
    }

    impl VmHost for NoHost {
        fn arg_spec(&self, service_id: u8) -> Result<VmHostArgSpec, VmError> {
            Err(VmError::UnsupportedService(service_id))
        }

        fn call<O: VmOutput>(
            &mut self,
            service_id: u8,
            _args: VmHostArgs<'_>,
            _output: &mut O,
        ) -> Result<VmHostResult, VmError> {
            Err(VmError::UnsupportedService(service_id))
        }
    }

    pub struct Vm<'a> {
        bytecode: &'a [u8],
        strings: &'a [&'a str],
        ip: usize,
        stack: [Option<Value>; STACK_CAPACITY],
        stack_len: usize,
    }

    impl<'a> Vm<'a> {
        pub fn new(bytecode: &'a [u8], strings: &'a [&'a str]) -> Self {
            Self {
                bytecode,
                strings,
                ip: 0,
                stack: [None; STACK_CAPACITY],
                stack_len: 0,
            }
        }

        pub fn run<O: VmOutput>(&mut self, output: &mut O) -> Result<(), VmError> {
            let mut host = NoHost;
            self.run_with_host(output, &mut host)
        }

        pub fn run_with_host<O: VmOutput, H: VmHost>(
            &mut self,
            output: &mut O,
            host: &mut H,
        ) -> Result<(), VmError> {
            loop {
                let op = self.read_u8()?;
                match op {
                    opcode::PUSH_INT => {
                        let value = self.read_i32_le()?;
                        self.push(Value::Int(value))?;
                    }
                    opcode::PUSH_STR_CONST => {
                        let index = self.read_u16_le()?;
                        self.push(Value::StrConst(index))?;
                    }
                    opcode::ADD => {
                        let rhs = self.pop()?;
                        let lhs = self.pop()?;
                        match (lhs, rhs) {
                            (Value::Int(a), Value::Int(b)) => self.push(Value::Int(a + b))?,
                            _ => return Err(VmError::TypeMismatch),
                        }
                    }
                    opcode::PRINT => match self.pop()? {
                        Value::Int(value) => output.write_i32(value),
                        Value::StrConst(index) => {
                            let text = self
                                .strings
                                .get(index as usize)
                                .ok_or(VmError::StrConstIndexOutOfBounds)?;
                            output.write_str(text);
                        }
                    },
                    opcode::KCALL => {
                        let service_id = self.read_u8()?;
                        let args = self.read_host_args(host.arg_spec(service_id)?)?;
                        match host.call(service_id, args, output)? {
                            VmHostResult::None => {}
                            VmHostResult::PushI32(value) => self.push(Value::Int(value))?,
                        }
                    }
                    opcode::HALT => return Ok(()),
                    other => return Err(VmError::UnknownOpcode(other)),
                }

                if self.ip >= self.bytecode.len() {
                    return Err(VmError::MissingHalt);
                }
            }
        }

        fn push(&mut self, value: Value) -> Result<(), VmError> {
            if self.stack_len >= STACK_CAPACITY {
                return Err(VmError::StackOverflow);
            }
            self.stack[self.stack_len] = Some(value);
            self.stack_len += 1;
            Ok(())
        }

        fn pop(&mut self) -> Result<Value, VmError> {
            if self.stack_len == 0 {
                return Err(VmError::StackUnderflow);
            }
            self.stack_len -= 1;
            self.stack[self.stack_len]
                .take()
                .ok_or(VmError::StackUnderflow)
        }

        fn pop_i32(&mut self) -> Result<i32, VmError> {
            match self.pop()? {
                Value::Int(value) => Ok(value),
                Value::StrConst(_) => Err(VmError::TypeMismatch),
            }
        }

        fn pop_str_const(&mut self) -> Result<&'a str, VmError> {
            match self.pop()? {
                Value::StrConst(index) => self
                    .strings
                    .get(index as usize)
                    .copied()
                    .ok_or(VmError::StrConstIndexOutOfBounds),
                Value::Int(_) => Err(VmError::TypeMismatch),
            }
        }

        fn read_host_args(&mut self, spec: VmHostArgSpec) -> Result<VmHostArgs<'a>, VmError> {
            match spec {
                VmHostArgSpec::None => Ok(VmHostArgs::None),
                VmHostArgSpec::I32 => Ok(VmHostArgs::I32(self.pop_i32()?)),
                VmHostArgSpec::StrConst => Ok(VmHostArgs::StrConst(self.pop_str_const()?)),
            }
        }

        fn read_u8(&mut self) -> Result<u8, VmError> {
            let byte = *self.bytecode.get(self.ip).ok_or(VmError::UnexpectedEnd)?;
            self.ip += 1;
            Ok(byte)
        }

        fn read_u16_le(&mut self) -> Result<u16, VmError> {
            let lo = self.read_u8()? as u16;
            let hi = self.read_u8()? as u16;
            Ok(lo | (hi << 8))
        }

        fn read_i32_le(&mut self) -> Result<i32, VmError> {
            let b0 = self.read_u8()? as u32;
            let b1 = self.read_u8()? as u32;
            let b2 = self.read_u8()? as u32;
            let b3 = self.read_u8()? as u32;
            Ok((b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)) as i32)
        }
    }
}

pub use value::Value;
pub use vm::{
    NoHost, Vm, VmError, VmHost, VmHostArgSpec, VmHostArgs, VmHostResult, VmOutput, STACK_CAPACITY,
};

#[cfg(test)]
mod tests {
    use super::opcode;
    use super::{NoHost, Vm, VmError, VmHost, VmHostArgSpec, VmHostArgs, VmHostResult, VmOutput};

    #[derive(Default)]
    struct FixedOutput {
        strings: [&'static str; 8],
        ints: [i32; 2],
        string_len: usize,
        int_len: usize,
        clears: usize,
    }

    struct MockHost;

    impl VmOutput for FixedOutput {
        fn write_str(&mut self, value: &str) {
            self.strings[self.string_len] = match value {
                "DBYTE VM ONLINE" => "DBYTE VM ONLINE",
                "KERNEL ONLINE" => "KERNEL ONLINE",
                "GRAPHICS MODE 13H" => "GRAPHICS MODE 13H",
                "TICKS SERVICE OK" => "TICKS SERVICE OK",
                "MASK SERVICE OK" => "MASK SERVICE OK",
                "ARG TEXT DBYTE SERVICE ARG" => "ARG TEXT DBYTE SERVICE ARG",
                "HELLO GRAPHICS LOG" => "HELLO GRAPHICS LOG",
                "LOG CLEARED" => "LOG CLEARED",
                "hello" => "hello",
                _ => "",
            };
            self.string_len += 1;
        }

        fn write_i32(&mut self, value: i32) {
            self.ints[self.int_len] = value;
            self.int_len += 1;
        }

        fn clear_log(&mut self) {
            self.clears += 1;
        }
    }

    impl VmHost for MockHost {
        fn arg_spec(&self, service_id: u8) -> Result<VmHostArgSpec, VmError> {
            match service_id {
                1..=3 | 7 => Ok(VmHostArgSpec::None),
                4 => Ok(VmHostArgSpec::I32),
                5 | 6 => Ok(VmHostArgSpec::StrConst),
                _ => Err(VmError::UnsupportedService(service_id)),
            }
        }

        fn call<O: VmOutput>(
            &mut self,
            service_id: u8,
            args: VmHostArgs<'_>,
            output: &mut O,
        ) -> Result<VmHostResult, VmError> {
            match service_id {
                1 => {
                    assert_eq!(args, VmHostArgs::None);
                    output.write_str("KERNEL ONLINE");
                    output.write_str("DBYTE VM ONLINE");
                    output.write_str("GRAPHICS MODE 13H");
                    Ok(VmHostResult::None)
                }
                2 => {
                    assert_eq!(args, VmHostArgs::None);
                    output.write_str("TICKS SERVICE OK");
                    output.write_str("MASK SERVICE OK");
                    Ok(VmHostResult::None)
                }
                3 => {
                    assert_eq!(args, VmHostArgs::None);
                    Ok(VmHostResult::PushI32(8))
                }
                4 => match args {
                    VmHostArgs::I32(value) => {
                        output.write_i32(value);
                        Ok(VmHostResult::None)
                    }
                    VmHostArgs::None | VmHostArgs::StrConst(_) => Err(VmError::TypeMismatch),
                },
                5 => match args {
                    VmHostArgs::StrConst(value) => {
                        if value == "DBYTE SERVICE ARG" {
                            output.write_str("ARG TEXT DBYTE SERVICE ARG");
                            Ok(VmHostResult::None)
                        } else {
                            Err(VmError::TypeMismatch)
                        }
                    }
                    VmHostArgs::None | VmHostArgs::I32(_) => Err(VmError::TypeMismatch),
                },
                6 => match args {
                    VmHostArgs::StrConst(value) => {
                        output.write_str(value);
                        Ok(VmHostResult::None)
                    }
                    VmHostArgs::None | VmHostArgs::I32(_) => Err(VmError::TypeMismatch),
                },
                7 => match args {
                    VmHostArgs::None => {
                        output.clear_log();
                        output.write_str("LOG CLEARED");
                        Ok(VmHostResult::None)
                    }
                    VmHostArgs::I32(_) | VmHostArgs::StrConst(_) => Err(VmError::TypeMismatch),
                },
                _ => Err(VmError::UnsupportedService(service_id)),
            }
        }
    }

    #[test]
    fn runs_probe_program() {
        let bytecode = [
            opcode::PUSH_STR_CONST,
            0,
            0,
            opcode::PRINT,
            opcode::PUSH_INT,
            40,
            0,
            0,
            0,
            opcode::PUSH_INT,
            2,
            0,
            0,
            0,
            opcode::ADD,
            opcode::PRINT,
            opcode::HALT,
        ];
        let strings = ["DBYTE VM ONLINE"];
        let mut output = FixedOutput::default();
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run(&mut output), Ok(()));
        assert_eq!(output.strings[0], "DBYTE VM ONLINE");
        assert_eq!(output.ints[0], 42);
    }

    #[test]
    fn adds_two_integers() {
        let bytecode = [
            opcode::PUSH_INT,
            7,
            0,
            0,
            0,
            opcode::PUSH_INT,
            35,
            0,
            0,
            0,
            opcode::ADD,
            opcode::PRINT,
            opcode::HALT,
        ];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run(&mut output), Ok(()));
        assert_eq!(output.ints[0], 42);
    }

    #[test]
    fn prints_string_constant() {
        let bytecode = [opcode::PUSH_STR_CONST, 0, 0, opcode::PRINT, opcode::HALT];
        let strings = ["hello"];
        let mut output = FixedOutput::default();
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run(&mut output), Ok(()));
        assert_eq!(output.strings[0], "hello");
    }

    #[test]
    fn prints_integer() {
        let bytecode = [opcode::PUSH_INT, 42, 0, 0, 0, opcode::PRINT, opcode::HALT];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run(&mut output), Ok(()));
        assert_eq!(output.ints[0], 42);
    }

    #[test]
    fn kcall_supported_service_succeeds_with_host() {
        let bytecode = [opcode::KCALL, 1, opcode::HALT];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run_with_host(&mut output, &mut host), Ok(()));
        assert_eq!(output.strings[0], "KERNEL ONLINE");
        assert_eq!(output.strings[1], "DBYTE VM ONLINE");
        assert_eq!(output.strings[2], "GRAPHICS MODE 13H");
    }

    #[test]
    fn kcall_ticks_service_succeeds_with_host() {
        let bytecode = [opcode::KCALL, 2, opcode::HALT];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run_with_host(&mut output, &mut host), Ok(()));
        assert_eq!(output.strings[0], "TICKS SERVICE OK");
        assert_eq!(output.strings[1], "MASK SERVICE OK");
    }

    #[test]
    fn kcall_tick_value_pushes_i32_with_host() {
        let bytecode = [opcode::KCALL, 3, opcode::HALT];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run_with_host(&mut output, &mut host), Ok(()));
    }

    #[test]
    fn kcall_tick_value_can_be_added_and_printed() {
        let bytecode = [
            opcode::KCALL,
            3,
            opcode::PUSH_INT,
            1,
            0,
            0,
            0,
            opcode::ADD,
            opcode::PRINT,
            opcode::HALT,
        ];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run_with_host(&mut output, &mut host), Ok(()));
        assert_eq!(output.ints[0], 9);
    }

    #[test]
    fn kcall_tick_value_stack_overflow_fails_deterministically() {
        let bytecode = [
            opcode::PUSH_INT,
            0,
            0,
            0,
            0,
            opcode::PUSH_INT,
            1,
            0,
            0,
            0,
            opcode::PUSH_INT,
            2,
            0,
            0,
            0,
            opcode::PUSH_INT,
            3,
            0,
            0,
            0,
            opcode::PUSH_INT,
            4,
            0,
            0,
            0,
            opcode::PUSH_INT,
            5,
            0,
            0,
            0,
            opcode::PUSH_INT,
            6,
            0,
            0,
            0,
            opcode::PUSH_INT,
            7,
            0,
            0,
            0,
            opcode::PUSH_INT,
            8,
            0,
            0,
            0,
            opcode::PUSH_INT,
            9,
            0,
            0,
            0,
            opcode::PUSH_INT,
            10,
            0,
            0,
            0,
            opcode::PUSH_INT,
            11,
            0,
            0,
            0,
            opcode::PUSH_INT,
            12,
            0,
            0,
            0,
            opcode::PUSH_INT,
            13,
            0,
            0,
            0,
            opcode::PUSH_INT,
            14,
            0,
            0,
            0,
            opcode::PUSH_INT,
            15,
            0,
            0,
            0,
            opcode::KCALL,
            3,
            opcode::HALT,
        ];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(
            vm.run_with_host(&mut output, &mut host),
            Err(VmError::StackOverflow)
        );
    }

    #[test]
    fn kcall_echo_i32_consumes_argument_with_host() {
        let bytecode = [opcode::PUSH_INT, 7, 0, 0, 0, opcode::KCALL, 4, opcode::HALT];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run_with_host(&mut output, &mut host), Ok(()));
        assert_eq!(output.ints[0], 7);
    }

    #[test]
    fn kcall_echo_i32_stack_underflow_fails_deterministically() {
        let bytecode = [opcode::KCALL, 4, opcode::HALT];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(
            vm.run_with_host(&mut output, &mut host),
            Err(VmError::StackUnderflow)
        );
    }

    #[test]
    fn kcall_echo_str_consumes_string_constant_with_host() {
        let bytecode = [opcode::PUSH_STR_CONST, 0, 0, opcode::KCALL, 5, opcode::HALT];
        let strings = ["DBYTE SERVICE ARG"];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run_with_host(&mut output, &mut host), Ok(()));
        assert_eq!(output.strings[0], "ARG TEXT DBYTE SERVICE ARG");
    }

    #[test]
    fn kcall_echo_str_wrong_type_fails_deterministically() {
        let bytecode = [opcode::PUSH_INT, 7, 0, 0, 0, opcode::KCALL, 5, opcode::HALT];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(
            vm.run_with_host(&mut output, &mut host),
            Err(VmError::TypeMismatch)
        );
    }

    #[test]
    fn kcall_echo_str_stack_underflow_fails_deterministically() {
        let bytecode = [opcode::KCALL, 5, opcode::HALT];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(
            vm.run_with_host(&mut output, &mut host),
            Err(VmError::StackUnderflow)
        );
    }

    #[test]
    fn kcall_echo_str_invalid_const_index_fails_deterministically() {
        let bytecode = [opcode::PUSH_STR_CONST, 1, 0, opcode::KCALL, 5, opcode::HALT];
        let strings = ["DBYTE SERVICE ARG"];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(
            vm.run_with_host(&mut output, &mut host),
            Err(VmError::StrConstIndexOutOfBounds)
        );
    }

    #[test]
    fn kcall_graphics_log_consumes_string_constant_with_host() {
        let bytecode = [opcode::PUSH_STR_CONST, 0, 0, opcode::KCALL, 6, opcode::HALT];
        let strings = ["HELLO GRAPHICS LOG"];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run_with_host(&mut output, &mut host), Ok(()));
        assert_eq!(output.strings[0], "HELLO GRAPHICS LOG");
    }

    #[test]
    fn kcall_graphics_log_wrong_type_fails_deterministically() {
        let bytecode = [opcode::PUSH_INT, 7, 0, 0, 0, opcode::KCALL, 6, opcode::HALT];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(
            vm.run_with_host(&mut output, &mut host),
            Err(VmError::TypeMismatch)
        );
    }

    #[test]
    fn kcall_graphics_log_stack_underflow_fails_deterministically() {
        let bytecode = [opcode::KCALL, 6, opcode::HALT];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(
            vm.run_with_host(&mut output, &mut host),
            Err(VmError::StackUnderflow)
        );
    }

    #[test]
    fn kcall_graphics_log_clear_succeeds_with_host() {
        let bytecode = [opcode::KCALL, 7, opcode::HALT];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run_with_host(&mut output, &mut host), Ok(()));
        assert_eq!(output.clears, 1);
        assert_eq!(output.strings[0], "LOG CLEARED");
    }

    #[test]
    fn kcall_unsupported_service_fails_deterministically() {
        let bytecode = [opcode::KCALL, 99, opcode::HALT];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(
            vm.run_with_host(&mut output, &mut host),
            Err(VmError::UnsupportedService(99))
        );
    }

    #[test]
    fn kcall_truncated_service_id_fails_deterministically() {
        let bytecode = [opcode::KCALL];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run(&mut output), Err(VmError::UnexpectedEnd));
    }

    #[test]
    fn kcall_without_host_fails_deterministically() {
        let bytecode = [opcode::KCALL, 1, opcode::HALT];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut host = NoHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(
            vm.run_with_host(&mut output, &mut host),
            Err(VmError::UnsupportedService(1))
        );
    }

    #[test]
    fn rejects_unknown_opcode() {
        let bytecode = [0x7f];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run(&mut output), Err(VmError::UnknownOpcode(0x7f)));
    }

    #[test]
    fn rejects_truncated_push_int() {
        let bytecode = [opcode::PUSH_INT, 1, 0];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run(&mut output), Err(VmError::UnexpectedEnd));
    }

    #[test]
    fn rejects_missing_halt() {
        let bytecode = [opcode::PUSH_INT, 1, 0, 0, 0];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run(&mut output), Err(VmError::MissingHalt));
    }

    #[test]
    fn rejects_stack_underflow() {
        let bytecode = [opcode::ADD, opcode::HALT];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run(&mut output), Err(VmError::StackUnderflow));
    }

    #[test]
    fn rejects_stack_overflow() {
        let bytecode = [
            opcode::PUSH_INT,
            0,
            0,
            0,
            0,
            opcode::PUSH_INT,
            1,
            0,
            0,
            0,
            opcode::PUSH_INT,
            2,
            0,
            0,
            0,
            opcode::PUSH_INT,
            3,
            0,
            0,
            0,
            opcode::PUSH_INT,
            4,
            0,
            0,
            0,
            opcode::PUSH_INT,
            5,
            0,
            0,
            0,
            opcode::PUSH_INT,
            6,
            0,
            0,
            0,
            opcode::PUSH_INT,
            7,
            0,
            0,
            0,
            opcode::PUSH_INT,
            8,
            0,
            0,
            0,
            opcode::PUSH_INT,
            9,
            0,
            0,
            0,
            opcode::PUSH_INT,
            10,
            0,
            0,
            0,
            opcode::PUSH_INT,
            11,
            0,
            0,
            0,
            opcode::PUSH_INT,
            12,
            0,
            0,
            0,
            opcode::PUSH_INT,
            13,
            0,
            0,
            0,
            opcode::PUSH_INT,
            14,
            0,
            0,
            0,
            opcode::PUSH_INT,
            15,
            0,
            0,
            0,
            opcode::PUSH_INT,
            16,
            0,
            0,
            0,
            opcode::HALT,
        ];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run(&mut output), Err(VmError::StackOverflow));
    }

    #[test]
    fn rejects_invalid_string_constant_index() {
        let bytecode = [opcode::PUSH_STR_CONST, 1, 0, opcode::PRINT, opcode::HALT];
        let strings = ["hello"];
        let mut output = FixedOutput::default();
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(vm.run(&mut output), Err(VmError::StrConstIndexOutOfBounds));
    }
}
