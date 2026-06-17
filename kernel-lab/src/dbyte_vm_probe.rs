use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};

use dbyte_kernel_vm::{opcode, Vm, VmError, VmHost, VmOutput};

use crate::{irq0_ticks_status_snapshot, serial, vga};

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
const KERNEL_STATUS: u8 = 1;
const KERNEL_TICKS: u8 = 2;
const KERNEL_STATUS_LINE: &str = "KERNEL ONLINE";
const DBYTE_VM_STATUS_LINE: &str = "DBYTE VM ONLINE";
const GRAPHICS_STATUS_LINE: &str = "GRAPHICS MODE 13H";
const IRQ0_TICKS_0008_LINE: &str = "IRQ0 TICKS 0008";
const IRQ0_MASKED_LINE: &str = "IRQ0 MASKED";
const IRQ0_UNMASKED_LINE: &str = "IRQ0 UNMASKED";
const IRQ0_TICKS_UNKNOWN_LINE: &str = "IRQ0 TICKS UNKNOWN";

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

static DBYTE_APP_SYSINFO_STRINGS: [&str; 1] = ["APP SYSINFO"];
static DBYTE_APP_SYSINFO_OUTPUT_LINES: [&str; 4] = [
    "APP SYSINFO",
    KERNEL_STATUS_LINE,
    DBYTE_VM_STATUS_LINE,
    GRAPHICS_STATUS_LINE,
];
static DBYTE_APP_SYSINFO_BYTECODE: [u8; 7] = [
    opcode::PUSH_STR_CONST,
    0x00,
    0x00,          // PUSH_STR_CONST 0
    opcode::PRINT, // PRINT
    opcode::KCALL,
    KERNEL_STATUS, // KCALL KERNEL_STATUS
    opcode::HALT,  // HALT
];

static DBYTE_APP_TICKS_STRINGS: [&str; 1] = ["APP TICKS"];
static DBYTE_APP_TICKS_OUTPUT_LINES: [&str; 3] =
    ["APP TICKS", IRQ0_TICKS_0008_LINE, IRQ0_MASKED_LINE];
static DBYTE_APP_TICKS_BYTECODE: [u8; 7] = [
    opcode::PUSH_STR_CONST,
    0x00,
    0x00,          // PUSH_STR_CONST 0
    opcode::PRINT, // PRINT
    opcode::KCALL,
    KERNEL_TICKS, // KCALL KERNEL_TICKS
    opcode::HALT, // HALT
];

#[allow(dead_code)]
pub const EMBEDDED_DBYTE_APPS: [EmbeddedDbyteApp; 4] = [
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
    EmbeddedDbyteApp {
        name: "sysinfo",
        bytecode: &DBYTE_APP_SYSINFO_BYTECODE,
        consts: &DBYTE_APP_SYSINFO_STRINGS,
        output_lines: &DBYTE_APP_SYSINFO_OUTPUT_LINES,
    },
    EmbeddedDbyteApp {
        name: "ticks",
        bytecode: &DBYTE_APP_TICKS_BYTECODE,
        consts: &DBYTE_APP_TICKS_STRINGS,
        output_lines: &DBYTE_APP_TICKS_OUTPUT_LINES,
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

struct KernelServiceHost;

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

impl VmHost for KernelServiceHost {
    fn call<O: VmOutput>(&mut self, service_id: u8, output: &mut O) -> Result<(), VmError> {
        match service_id {
            KERNEL_STATUS => {
                output.write_str(KERNEL_STATUS_LINE);
                output.write_str(DBYTE_VM_STATUS_LINE);
                output.write_str(GRAPHICS_STATUS_LINE);
                Ok(())
            }
            KERNEL_TICKS => {
                write_kernel_ticks(output);
                Ok(())
            }
            _ => Err(VmError::UnsupportedService(service_id)),
        }
    }
}

fn write_kernel_ticks<O: VmOutput>(output: &mut O) {
    let ticks = irq0_ticks_status_snapshot();
    match ticks.target_ticks {
        8 => output.write_str(IRQ0_TICKS_0008_LINE),
        _ => output.write_str(IRQ0_TICKS_UNKNOWN_LINE),
    }
    match ticks.irq0_currently_masked {
        "yes" => output.write_str(IRQ0_MASKED_LINE),
        _ => output.write_str(IRQ0_UNMASKED_LINE),
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

    let result = run_embedded_app_program(app.bytecode, app.consts, &mut output)
        .map(|_| EmbeddedDbyteAppCapture { app });
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

fn run_embedded_app_program<O: VmOutput>(
    bytecode: &[u8],
    strings: &[&str],
    output: &mut O,
) -> Result<(), VmError> {
    let mut vm = Vm::new(bytecode, strings);
    let mut host = KernelServiceHost;
    vm.run_with_host(output, &mut host)
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
        VmError::UnsupportedService(_) => "unsupported service",
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
