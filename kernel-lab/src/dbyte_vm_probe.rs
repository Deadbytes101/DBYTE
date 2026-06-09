use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};

use dbyte_kernel_vm::{Vm, VmError, VmOutput};

use crate::{serial, vga};

const DBYTE_VM_PROBE_STRINGS: [&str; 1] = ["DBYTE VM ONLINE"];
const DBYTE_VM_PROBE_BYTECODE: [u8; 17] = [
    0x02, 0x00, 0x00, // PUSH_STR_CONST 0
    0x04, // PRINT
    0x01, 0x28, 0x00, 0x00, 0x00, // PUSH_INT 40
    0x01, 0x02, 0x00, 0x00, 0x00, // PUSH_INT 2
    0x03, // ADD
    0x04, // PRINT
    0xff, // HALT
];

const DBYTE_VM_BOOT_SCRIPT_STRINGS: [&str; 1] = ["DBYTE BOOT SCRIPT"];
const DBYTE_VM_BOOT_SCRIPT_BYTECODE: [u8; 17] = [
    0x02, 0x00, 0x00, // PUSH_STR_CONST 0
    0x04, // PRINT
    0x01, 0x01, 0x00, 0x00, 0x00, // PUSH_INT 1
    0x01, 0x01, 0x00, 0x00, 0x00, // PUSH_INT 1
    0x03, // ADD
    0x04, // PRINT
    0xff, // HALT
];

static BOOT_SCRIPT_EXECUTED: AtomicBool = AtomicBool::new(false);
static BOOT_SCRIPT_OK: AtomicBool = AtomicBool::new(false);

struct KernelVmOutput;

impl VmOutput for KernelVmOutput {
    fn write_str(&mut self, value: &str) {
        vga::print(value);
        vga::print("\n");
        serial::print(value);
        serial::print("\n");
    }

    fn write_i32(&mut self, value: i32) {
        let mut vga_writer = vga::VgaWriter;
        let mut serial_writer = serial::SerialWriter;
        let _ = writeln!(vga_writer, "{}", value);
        let _ = writeln!(serial_writer, "{}", value);
    }
}

pub fn print_status() {
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let executed = if BOOT_SCRIPT_EXECUTED.load(Ordering::SeqCst) {
        "executed"
    } else {
        "not executed"
    };
    let result = if BOOT_SCRIPT_OK.load(Ordering::SeqCst) {
        "ok"
    } else {
        "unknown"
    };
    let _ = write!(
        vga_writer,
        "DByte kernel VM\nstate: ready\nmode: embedded bytecode\nheap: none\nfilesystem: none\nboot script: {}\nboot script result: {}\n",
        executed, result
    );
    let _ = write!(
        serial_writer,
        "DByte kernel VM\nstate: ready\nmode: embedded bytecode\nheap: none\nfilesystem: none\nboot script: {}\nboot script result: {}\n",
        executed, result
    );
}

pub fn run_boot_script() {
    let mut output = KernelVmOutput;
    BOOT_SCRIPT_EXECUTED.store(true, Ordering::SeqCst);
    match run_program(
        &DBYTE_VM_BOOT_SCRIPT_BYTECODE,
        &DBYTE_VM_BOOT_SCRIPT_STRINGS,
        &mut output,
    ) {
        Ok(()) => BOOT_SCRIPT_OK.store(true, Ordering::SeqCst),
        Err(error) => {
            BOOT_SCRIPT_OK.store(false, Ordering::SeqCst);
            print_error("DByte boot script error: ", error);
        }
    }
}

pub fn run_probe() {
    let mut output = KernelVmOutput;
    if let Err(error) = run_program(
        &DBYTE_VM_PROBE_BYTECODE,
        &DBYTE_VM_PROBE_STRINGS,
        &mut output,
    ) {
        print_error("DByte kernel VM error: ", error);
    }
}

fn run_program<O: VmOutput>(
    bytecode: &[u8],
    strings: &[&str],
    output: &mut O,
) -> Result<(), VmError> {
    let mut vm = Vm::new(bytecode, strings);
    vm.run(output)
}

fn print_error(prefix: &str, error: VmError) {
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = writeln!(vga_writer, "{}{}", prefix, vm_error_name(error));
    let _ = writeln!(serial_writer, "{}{}", prefix, vm_error_name(error));
}

fn vm_error_name(error: VmError) -> &'static str {
    match error {
        VmError::StackOverflow => "stack overflow",
        VmError::StackUnderflow => "stack underflow",
        VmError::TypeMismatch => "type mismatch",
        VmError::StrConstIndexOutOfBounds => "string constant index out of bounds",
        VmError::UnexpectedEnd => "unexpected end",
        VmError::UnknownOpcode(_) => "unknown opcode",
        VmError::MissingHalt => "missing halt",
    }
}

#[cfg(test)]
mod tests {
    use super::{run_program, vm_error_name};
    use dbyte_kernel_vm::{VmError, VmOutput};

    #[derive(Default)]
    struct TestOutput {
        strings: [&'static str; 1],
        ints: [i32; 1],
        string_len: usize,
        int_len: usize,
    }

    impl VmOutput for TestOutput {
        fn write_str(&mut self, value: &str) {
            self.strings[self.string_len] = match value {
                "DBYTE BOOT SCRIPT" => "DBYTE BOOT SCRIPT",
                _ => "",
            };
            self.string_len += 1;
        }

        fn write_i32(&mut self, value: i32) {
            self.ints[self.int_len] = value;
            self.int_len += 1;
        }
    }

    #[test]
    fn boot_script_bytecode_succeeds_through_runner() {
        let bytecode = [
            0x02, 0x00, 0x00, 0x04, 0x01, 0x01, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00,
            0x03, 0x04, 0xff,
        ];
        let strings = ["DBYTE BOOT SCRIPT"];
        let mut output = TestOutput::default();

        assert_eq!(run_program(&bytecode, &strings, &mut output), Ok(()));
        assert_eq!(output.strings[0], "DBYTE BOOT SCRIPT");
        assert_eq!(output.ints[0], 2);
    }

    #[test]
    fn malformed_boot_script_bytecode_reports_deterministic_error() {
        let bytecode = [0x01, 0x01];
        let strings = ["DBYTE BOOT SCRIPT"];
        let mut output = TestOutput::default();

        assert_eq!(
            run_program(&bytecode, &strings, &mut output),
            Err(VmError::UnexpectedEnd)
        );
        assert_eq!(vm_error_name(VmError::UnexpectedEnd), "unexpected end");
    }
}
