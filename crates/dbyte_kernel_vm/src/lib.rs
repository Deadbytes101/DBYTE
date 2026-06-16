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
    }

    pub trait VmHost {
        fn call<O: VmOutput>(&mut self, service_id: u8, output: &mut O) -> Result<(), VmError>;
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
        fn call<O: VmOutput>(&mut self, service_id: u8, _output: &mut O) -> Result<(), VmError> {
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
                        host.call(service_id, output)?;
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
pub use vm::{NoHost, Vm, VmError, VmHost, VmOutput, STACK_CAPACITY};

#[cfg(test)]
mod tests {
    use super::opcode;
    use super::{NoHost, Vm, VmError, VmHost, VmOutput};

    #[derive(Default)]
    struct FixedOutput {
        strings: [&'static str; 4],
        ints: [i32; 2],
        string_len: usize,
        int_len: usize,
    }

    struct MockHost;

    impl VmOutput for FixedOutput {
        fn write_str(&mut self, value: &str) {
            self.strings[self.string_len] = match value {
                "DBYTE VM ONLINE" => "DBYTE VM ONLINE",
                "KERNEL ONLINE" => "KERNEL ONLINE",
                "GRAPHICS MODE 13H" => "GRAPHICS MODE 13H",
                "hello" => "hello",
                _ => "",
            };
            self.string_len += 1;
        }

        fn write_i32(&mut self, value: i32) {
            self.ints[self.int_len] = value;
            self.int_len += 1;
        }
    }

    impl VmHost for MockHost {
        fn call<O: VmOutput>(&mut self, service_id: u8, output: &mut O) -> Result<(), VmError> {
            match service_id {
                1 => {
                    output.write_str("KERNEL ONLINE");
                    output.write_str("DBYTE VM ONLINE");
                    output.write_str("GRAPHICS MODE 13H");
                    Ok(())
                }
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
    fn kcall_unsupported_service_fails_deterministically() {
        let bytecode = [opcode::KCALL, 2, opcode::HALT];
        let strings = [];
        let mut output = FixedOutput::default();
        let mut host = MockHost;
        let mut vm = Vm::new(&bytecode, &strings);

        assert_eq!(
            vm.run_with_host(&mut output, &mut host),
            Err(VmError::UnsupportedService(2))
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
