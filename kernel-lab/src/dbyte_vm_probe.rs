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

pub struct EmbeddedDbyteApp {
    pub name: &'static str,
    pub bytecode: &'static [u8],
    pub consts: &'static [&'static str],
    pub output_lines: &'static [&'static str],
}

static DBYTE_APP_HELLO_STRINGS: [&str; 1] = ["HELLO FROM DBYTE APP"];
static DBYTE_APP_HELLO_OUTPUT_LINES: [&str; 1] = ["HELLO FROM DBYTE APP"];
static DBYTE_APP_HELLO_BYTECODE: [u8; 5] = [
    0x02, 0x00, 0x00, // PUSH_STR_CONST 0
    0x04, // PRINT
    0xff, // HALT
];

static DBYTE_APP_MATH_STRINGS: [&str; 1] = ["APP MATH"];
static DBYTE_APP_MATH_OUTPUT_LINES: [&str; 2] = ["APP MATH", "7"];
static DBYTE_APP_MATH_BYTECODE: [u8; 17] = [
    0x02, 0x00, 0x00, // PUSH_STR_CONST 0
    0x04, // PRINT
    0x01, 0x03, 0x00, 0x00, 0x00, // PUSH_INT 3
    0x01, 0x04, 0x00, 0x00, 0x00, // PUSH_INT 4
    0x03, // ADD
    0x04, // PRINT
    0xff, // HALT
];

#[allow(dead_code)]
pub const EMBEDDED_DBYTE_APPS: [EmbeddedDbyteApp; 2] = [
    EmbeddedDbyteApp {
        name: "hello",
        bytecode: &DBYTE_APP_HELLO_BYTECODE,
        consts: &DBYTE_APP_HELLO_STRINGS,
        output_lines: &DBYTE_APP_HELLO_OUTPUT_LINES,
    },
    EmbeddedDbyteApp {
        name: "math",
        bytecode: &DBYTE_APP_MATH_BYTECODE,
        consts: &DBYTE_APP_MATH_STRINGS,
        output_lines: &DBYTE_APP_MATH_OUTPUT_LINES,
    },
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

pub struct VmProbeCapture {
    pub banner: bool,
    pub value: bool,
}

pub struct EmbeddedDbyteAppCapture {
    pub app: &'static EmbeddedDbyteApp,
}

struct ProbeCaptureOutput {
    banner: bool,
    value: bool,
}

struct DbyteAppCaptureOutput {
    app: &'static EmbeddedDbyteApp,
    line_index: usize,
    matched: bool,
}

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

impl VmOutput for ProbeCaptureOutput {
    fn write_str(&mut self, value: &str) {
        if value == DBYTE_VM_PROBE_STRINGS[0] {
            self.banner = true;
        }
    }

    fn write_i32(&mut self, value: i32) {
        if value == 42 {
            self.value = true;
        }
    }
}

impl VmOutput for DbyteAppCaptureOutput {
    fn write_str(&mut self, value: &str) {
        if self.line_index >= self.app.output_lines.len()
            || value != self.app.output_lines[self.line_index]
        {
            self.matched = false;
        }
        self.line_index += 1;
    }

    fn write_i32(&mut self, value: i32) {
        if self.line_index >= self.app.output_lines.len()
            || Some(value) != expected_i32_value(self.app.output_lines[self.line_index])
        {
            self.matched = false;
        }
        self.line_index += 1;
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

pub fn run_probe_capture() -> Result<VmProbeCapture, VmError> {
    let mut output = ProbeCaptureOutput {
        banner: false,
        value: false,
    };
    run_program(
        &DBYTE_VM_PROBE_BYTECODE,
        &DBYTE_VM_PROBE_STRINGS,
        &mut output,
    )?;
    Ok(VmProbeCapture {
        banner: output.banner,
        value: output.value,
    })
}

pub fn find_embedded_app(name: &[u8]) -> Option<&'static EmbeddedDbyteApp> {
    for app in &EMBEDDED_DBYTE_APPS {
        if name == app.name.as_bytes() {
            return Some(app);
        }
    }

    None
}

pub fn run_embedded_app_capture(name: &[u8]) -> Option<Result<EmbeddedDbyteAppCapture, VmError>> {
    let app = find_embedded_app(name)?;
    let mut output = DbyteAppCaptureOutput {
        app,
        line_index: 0,
        matched: true,
    };

    let result =
        run_program(app.bytecode, app.consts, &mut output).map(|_| EmbeddedDbyteAppCapture { app });
    if output.matched && output.line_index == app.output_lines.len() {
        Some(result)
    } else {
        Some(Err(VmError::TypeMismatch))
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

fn expected_i32_value(value: &str) -> Option<i32> {
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return None;
    }

    let mut number: i32 = 0;
    let mut index: usize = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte < b'0' || byte > b'9' {
            return None;
        }
        number = number * 10 + (byte - b'0') as i32;
        index += 1;
    }

    Some(number)
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
