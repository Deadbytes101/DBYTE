use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};

use dbyte_kernel_vm::{
    opcode, Vm, VmError, VmHost, VmHostArgSpec, VmHostArgs, VmHostResult, VmOutput,
};

use crate::{irq0_ticks_status_snapshot, kernel_clock_status_snapshot, serial, vga};

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
const KERNEL_TICK_VALUE: u8 = 3;
const KERNEL_ECHO_I32: u8 = 4;
const KERNEL_ECHO_STR: u8 = 5;
const KERNEL_GRAPHICS_LOG: u8 = 6;
const KERNEL_GRAPHICS_LOG_CLEAR: u8 = 7;
const KERNEL_CLOCK_STATUS: u8 = 8;
const KERNEL_STATUS_LINE: &str = "KERNEL ONLINE";
const DBYTE_VM_STATUS_LINE: &str = "DBYTE VM ONLINE";
const GRAPHICS_STATUS_LINE: &str = "GRAPHICS MODE 13H";
const ARG_VALUE_7_LINE: &str = "ARG VALUE 7";
const ARG_TEXT_DBYTE_SERVICE_ARG_LINE: &str = "ARG TEXT DBYTE SERVICE ARG";
const HELLO_GRAPHICS_LOG_LINE: &str = "HELLO GRAPHICS LOG";
const LOG_CLEARED_LINE: &str = "LOG CLEARED";
const GRAPHICS_LOG_READY_LINE: &str = "GRAPHICS LOG READY";
const APP_OK_LINE: &str = "APP OK";
const APP_NOT_FOUND_LINE: &str = "APP NOT FOUND";
const APP_INFO_RESULT_READY: &str = "READY";
const APP_INFO_RESULT_VM_ERROR_TEST: &str = "VM ERROR TEST";
const APP_INFO_SERVICES_NONE: &str = "none";
const APP_INFO_SERVICES_SYSINFO: &str = "1";
const APP_INFO_SERVICES_TICKS: &str = "2";
const APP_INFO_SERVICES_TICKMATH: &str = "3";
const APP_INFO_SERVICES_ARGTEST: &str = "4";
const APP_INFO_SERVICES_STRTEST: &str = "5";
const APP_INFO_SERVICES_LOGTEST: &str = "6";
const APP_INFO_SERVICES_LOGCLEAR: &str = "7";
const APP_INFO_SERVICES_UIDEMO: &str = "7 6 1 2";
const APP_INFO_SERVICES_ERRTEST: &str = "99";
const APP_INFO_SERVICES_CLOCKINFO: &str = "8";
const IRQ0_TICKS_0008_LINE: &str = "IRQ0 TICKS 0008";
const IRQ0_MASKED_LINE: &str = "IRQ0 MASKED";
const IRQ0_UNMASKED_LINE: &str = "IRQ0 UNMASKED";
const IRQ0_TICKS_UNKNOWN_LINE: &str = "IRQ0 TICKS UNKNOWN";
const KERNEL_CLOCK_LINE: &str = "KERNEL CLOCK";
const KERNEL_CLOCK_RUNTIME_RUNNING_LINE: &str = "runtime: running";
const KERNEL_CLOCK_RUNTIME_STOPPED_LINE: &str = "runtime: stopped";

pub struct EmbeddedDbyteApp {
    pub name: &'static str,
    pub bytecode: &'static [u8],
    pub consts: &'static [&'static str],
    pub output_lines: &'static [&'static str],
    // Bounded graphics projection only. App execution is proven by output_lines capture.
    pub display_lines: &'static [&'static str],
    pub info_services: &'static str,
    pub info_result: &'static str,
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

static DBYTE_APP_TICKMATH_STRINGS: [&str; 1] = ["APP TICKMATH"];
static DBYTE_APP_TICKMATH_OUTPUT_LINES: [&str; 2] = ["APP TICKMATH", "9"];
static DBYTE_APP_TICKMATH_BYTECODE: [u8; 14] = [
    opcode::PUSH_STR_CONST,
    0x00,
    0x00,          // PUSH_STR_CONST 0
    opcode::PRINT, // PRINT
    opcode::KCALL,
    KERNEL_TICK_VALUE, // KCALL KERNEL_TICK_VALUE
    opcode::PUSH_INT,
    0x01,
    0x00,
    0x00,
    0x00,        // PUSH_INT 1
    opcode::ADD, // ADD
    opcode::PRINT,
    opcode::HALT, // HALT
];

static DBYTE_APP_ARGTEST_STRINGS: [&str; 1] = ["APP ARGTEST"];
static DBYTE_APP_ARGTEST_OUTPUT_LINES: [&str; 2] = ["APP ARGTEST", ARG_VALUE_7_LINE];
static DBYTE_APP_ARGTEST_BYTECODE: [u8; 12] = [
    opcode::PUSH_STR_CONST,
    0x00,
    0x00,          // PUSH_STR_CONST 0
    opcode::PRINT, // PRINT
    opcode::PUSH_INT,
    0x07,
    0x00,
    0x00,
    0x00, // PUSH_INT 7
    opcode::KCALL,
    KERNEL_ECHO_I32, // KCALL KERNEL_ECHO_I32
    opcode::HALT,    // HALT
];

static DBYTE_APP_STRTEST_STRINGS: [&str; 2] = ["APP STRTEST", "DBYTE SERVICE ARG"];
static DBYTE_APP_STRTEST_OUTPUT_LINES: [&str; 2] = ["APP STRTEST", ARG_TEXT_DBYTE_SERVICE_ARG_LINE];
static DBYTE_APP_STRTEST_BYTECODE: [u8; 10] = [
    opcode::PUSH_STR_CONST,
    0x00,
    0x00,          // PUSH_STR_CONST 0
    opcode::PRINT, // PRINT
    opcode::PUSH_STR_CONST,
    0x01,
    0x00, // PUSH_STR_CONST 1
    opcode::KCALL,
    KERNEL_ECHO_STR, // KCALL KERNEL_ECHO_STR
    opcode::HALT,    // HALT
];

static DBYTE_APP_LOGTEST_STRINGS: [&str; 2] = ["APP LOGTEST", HELLO_GRAPHICS_LOG_LINE];
static DBYTE_APP_LOGTEST_OUTPUT_LINES: [&str; 2] = ["APP LOGTEST", HELLO_GRAPHICS_LOG_LINE];
static DBYTE_APP_LOGTEST_BYTECODE: [u8; 10] = [
    opcode::PUSH_STR_CONST,
    0x00,
    0x00,          // PUSH_STR_CONST 0
    opcode::PRINT, // PRINT
    opcode::PUSH_STR_CONST,
    0x01,
    0x00, // PUSH_STR_CONST 1
    opcode::KCALL,
    KERNEL_GRAPHICS_LOG, // KCALL KERNEL_GRAPHICS_LOG
    opcode::HALT,        // HALT
];

static DBYTE_APP_LOGCLEAR_STRINGS: [&str; 1] = ["APP LOGCLEAR"];
static DBYTE_APP_LOGCLEAR_OUTPUT_LINES: [&str; 2] = ["APP LOGCLEAR", LOG_CLEARED_LINE];
static DBYTE_APP_LOGCLEAR_BYTECODE: [u8; 7] = [
    opcode::PUSH_STR_CONST,
    0x00,
    0x00,          // PUSH_STR_CONST 0
    opcode::PRINT, // PRINT
    opcode::KCALL,
    KERNEL_GRAPHICS_LOG_CLEAR, // KCALL KERNEL_GRAPHICS_LOG_CLEAR
    opcode::HALT,              // HALT
];

static DBYTE_APP_UIDEMO_STRINGS: [&str; 2] = ["APP UIDEMO", GRAPHICS_LOG_READY_LINE];
static DBYTE_APP_UIDEMO_OUTPUT_LINES: [&str; 8] = [
    "APP UIDEMO",
    LOG_CLEARED_LINE,
    GRAPHICS_LOG_READY_LINE,
    KERNEL_STATUS_LINE,
    DBYTE_VM_STATUS_LINE,
    GRAPHICS_STATUS_LINE,
    IRQ0_TICKS_0008_LINE,
    IRQ0_MASKED_LINE,
];
static DBYTE_APP_UIDEMO_DISPLAY_LINES: [&str; 4] = [
    "APP UIDEMO",
    GRAPHICS_LOG_READY_LINE,
    KERNEL_STATUS_LINE,
    IRQ0_TICKS_0008_LINE,
];
static DBYTE_APP_UIDEMO_BYTECODE: [u8; 16] = [
    opcode::PUSH_STR_CONST,
    0x00,
    0x00,          // PUSH_STR_CONST 0
    opcode::PRINT, // PRINT
    opcode::KCALL,
    KERNEL_GRAPHICS_LOG_CLEAR, // KCALL KERNEL_GRAPHICS_LOG_CLEAR
    opcode::PUSH_STR_CONST,
    0x01,
    0x00, // PUSH_STR_CONST 1
    opcode::KCALL,
    KERNEL_GRAPHICS_LOG, // KCALL KERNEL_GRAPHICS_LOG
    opcode::KCALL,
    KERNEL_STATUS, // KCALL KERNEL_STATUS
    opcode::KCALL,
    KERNEL_TICKS, // KCALL KERNEL_TICKS
    opcode::HALT, // HALT
];

static DBYTE_APP_ERRTEST_STRINGS: [&str; 1] = ["APP ERRTEST"];
static DBYTE_APP_ERRTEST_OUTPUT_LINES: [&str; 1] = ["APP ERRTEST"];
static DBYTE_APP_ERRTEST_BYTECODE: [u8; 7] = [
    opcode::PUSH_STR_CONST,
    0x00,
    0x00,          // PUSH_STR_CONST 0
    opcode::PRINT, // PRINT
    opcode::KCALL,
    99,           // KCALL unsupported service proof
    opcode::HALT, // HALT
];

static DBYTE_APP_CLOCKINFO_STRINGS: [&str; 1] = ["APP CLOCKINFO"];
static DBYTE_APP_CLOCKINFO_OUTPUT_LINES: [&str; 4] = [
    "APP CLOCKINFO",
    KERNEL_CLOCK_LINE,
    "runtime: <running|stopped>",
    "ticks: <current>",
];
static DBYTE_APP_CLOCKINFO_DISPLAY_LINES: [&str; 2] = ["APP CLOCKINFO", KERNEL_CLOCK_LINE];
static DBYTE_APP_CLOCKINFO_BYTECODE: [u8; 7] = [
    opcode::PUSH_STR_CONST,
    0x00,
    0x00,          // PUSH_STR_CONST 0
    opcode::PRINT, // PRINT
    opcode::KCALL,
    KERNEL_CLOCK_STATUS, // KCALL KERNEL_CLOCK_STATUS
    opcode::HALT,        // HALT
];

#[allow(dead_code)]
pub const EMBEDDED_DBYTE_APPS: [EmbeddedDbyteApp; 12] = [
    EmbeddedDbyteApp {
        name: "hello",
        bytecode: &DBYTE_APP_HELLO_BYTECODE,
        consts: &DBYTE_APP_HELLO_STRINGS,
        output_lines: &DBYTE_APP_HELLO_OUTPUT_LINES,
        display_lines: &DBYTE_APP_HELLO_OUTPUT_LINES,
        info_services: APP_INFO_SERVICES_NONE,
        info_result: APP_INFO_RESULT_READY,
    },
    EmbeddedDbyteApp {
        name: "math",
        bytecode: &DBYTE_APP_MATH_BYTECODE,
        consts: &DBYTE_APP_MATH_STRINGS,
        output_lines: &DBYTE_APP_MATH_OUTPUT_LINES,
        display_lines: &DBYTE_APP_MATH_OUTPUT_LINES,
        info_services: APP_INFO_SERVICES_NONE,
        info_result: APP_INFO_RESULT_READY,
    },
    EmbeddedDbyteApp {
        name: "sysinfo",
        bytecode: &DBYTE_APP_SYSINFO_BYTECODE,
        consts: &DBYTE_APP_SYSINFO_STRINGS,
        output_lines: &DBYTE_APP_SYSINFO_OUTPUT_LINES,
        display_lines: &DBYTE_APP_SYSINFO_OUTPUT_LINES,
        info_services: APP_INFO_SERVICES_SYSINFO,
        info_result: APP_INFO_RESULT_READY,
    },
    EmbeddedDbyteApp {
        name: "ticks",
        bytecode: &DBYTE_APP_TICKS_BYTECODE,
        consts: &DBYTE_APP_TICKS_STRINGS,
        output_lines: &DBYTE_APP_TICKS_OUTPUT_LINES,
        display_lines: &DBYTE_APP_TICKS_OUTPUT_LINES,
        info_services: APP_INFO_SERVICES_TICKS,
        info_result: APP_INFO_RESULT_READY,
    },
    EmbeddedDbyteApp {
        name: "tickmath",
        bytecode: &DBYTE_APP_TICKMATH_BYTECODE,
        consts: &DBYTE_APP_TICKMATH_STRINGS,
        output_lines: &DBYTE_APP_TICKMATH_OUTPUT_LINES,
        display_lines: &DBYTE_APP_TICKMATH_OUTPUT_LINES,
        info_services: APP_INFO_SERVICES_TICKMATH,
        info_result: APP_INFO_RESULT_READY,
    },
    EmbeddedDbyteApp {
        name: "argtest",
        bytecode: &DBYTE_APP_ARGTEST_BYTECODE,
        consts: &DBYTE_APP_ARGTEST_STRINGS,
        output_lines: &DBYTE_APP_ARGTEST_OUTPUT_LINES,
        display_lines: &DBYTE_APP_ARGTEST_OUTPUT_LINES,
        info_services: APP_INFO_SERVICES_ARGTEST,
        info_result: APP_INFO_RESULT_READY,
    },
    EmbeddedDbyteApp {
        name: "strtest",
        bytecode: &DBYTE_APP_STRTEST_BYTECODE,
        consts: &DBYTE_APP_STRTEST_STRINGS,
        output_lines: &DBYTE_APP_STRTEST_OUTPUT_LINES,
        display_lines: &DBYTE_APP_STRTEST_OUTPUT_LINES,
        info_services: APP_INFO_SERVICES_STRTEST,
        info_result: APP_INFO_RESULT_READY,
    },
    EmbeddedDbyteApp {
        name: "logtest",
        bytecode: &DBYTE_APP_LOGTEST_BYTECODE,
        consts: &DBYTE_APP_LOGTEST_STRINGS,
        output_lines: &DBYTE_APP_LOGTEST_OUTPUT_LINES,
        display_lines: &DBYTE_APP_LOGTEST_OUTPUT_LINES,
        info_services: APP_INFO_SERVICES_LOGTEST,
        info_result: APP_INFO_RESULT_READY,
    },
    EmbeddedDbyteApp {
        name: "logclear",
        bytecode: &DBYTE_APP_LOGCLEAR_BYTECODE,
        consts: &DBYTE_APP_LOGCLEAR_STRINGS,
        output_lines: &DBYTE_APP_LOGCLEAR_OUTPUT_LINES,
        display_lines: &DBYTE_APP_LOGCLEAR_OUTPUT_LINES,
        info_services: APP_INFO_SERVICES_LOGCLEAR,
        info_result: APP_INFO_RESULT_READY,
    },
    EmbeddedDbyteApp {
        name: "uidemo",
        bytecode: &DBYTE_APP_UIDEMO_BYTECODE,
        consts: &DBYTE_APP_UIDEMO_STRINGS,
        output_lines: &DBYTE_APP_UIDEMO_OUTPUT_LINES,
        display_lines: &DBYTE_APP_UIDEMO_DISPLAY_LINES,
        info_services: APP_INFO_SERVICES_UIDEMO,
        info_result: APP_INFO_RESULT_READY,
    },
    EmbeddedDbyteApp {
        name: "errtest",
        bytecode: &DBYTE_APP_ERRTEST_BYTECODE,
        consts: &DBYTE_APP_ERRTEST_STRINGS,
        output_lines: &DBYTE_APP_ERRTEST_OUTPUT_LINES,
        display_lines: &DBYTE_APP_ERRTEST_OUTPUT_LINES,
        info_services: APP_INFO_SERVICES_ERRTEST,
        info_result: APP_INFO_RESULT_VM_ERROR_TEST,
    },
    EmbeddedDbyteApp {
        name: "clockinfo",
        bytecode: &DBYTE_APP_CLOCKINFO_BYTECODE,
        consts: &DBYTE_APP_CLOCKINFO_STRINGS,
        output_lines: &DBYTE_APP_CLOCKINFO_OUTPUT_LINES,
        display_lines: &DBYTE_APP_CLOCKINFO_DISPLAY_LINES,
        info_services: APP_INFO_SERVICES_CLOCKINFO,
        info_result: APP_INFO_RESULT_READY,
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
    pub clock_status: Option<EmbeddedDbyteClockStatus>,
}

#[derive(Clone, Copy)]
pub struct EmbeddedDbyteClockStatus {
    pub runtime: &'static str,
    pub ticks: u32,
}

pub struct EmbeddedDbyteAppError {
    pub app: &'static EmbeddedDbyteApp,
    pub error: VmError,
}

pub enum EmbeddedDbyteAppRunResult {
    Ok(EmbeddedDbyteAppCapture),
    NotFound,
    VmError(EmbeddedDbyteAppError),
}

struct ProbeCaptureOutput {
    banner: bool,
    value: bool,
}

struct KernelServiceHost;

struct FixedLineBuffer<'a> {
    bytes: &'a mut [u8],
    len: usize,
}

struct DbyteAppCaptureOutput {
    app: &'static EmbeddedDbyteApp,
    line_index: usize,
    matched: bool,
    clock_runtime: Option<&'static str>,
    clock_ticks: Option<u32>,
}

impl<'a> FixedLineBuffer<'a> {
    fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes, len: 0 }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

impl Write for FixedLineBuffer<'_> {
    fn write_str(&mut self, value: &str) -> core::fmt::Result {
        let available = self.bytes.len().saturating_sub(self.len);
        if value.len() > available {
            return Err(core::fmt::Error);
        }
        let end = self.len + value.len();
        self.bytes[self.len..end].copy_from_slice(value.as_bytes());
        self.len = end;
        Ok(())
    }
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
    fn arg_spec(&self, service_id: u8) -> Result<VmHostArgSpec, VmError> {
        match service_id {
            KERNEL_STATUS
            | KERNEL_TICKS
            | KERNEL_TICK_VALUE
            | KERNEL_GRAPHICS_LOG_CLEAR
            | KERNEL_CLOCK_STATUS => Ok(VmHostArgSpec::None),
            KERNEL_ECHO_I32 => Ok(VmHostArgSpec::I32),
            KERNEL_ECHO_STR => Ok(VmHostArgSpec::StrConst),
            KERNEL_GRAPHICS_LOG => Ok(VmHostArgSpec::StrConst),
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
            KERNEL_STATUS => {
                output.write_str(KERNEL_STATUS_LINE);
                output.write_str(DBYTE_VM_STATUS_LINE);
                output.write_str(GRAPHICS_STATUS_LINE);
                Ok(VmHostResult::None)
            }
            KERNEL_TICKS => {
                write_kernel_ticks(output);
                Ok(VmHostResult::None)
            }
            KERNEL_TICK_VALUE => Ok(VmHostResult::PushI32(kernel_tick_value())),
            KERNEL_ECHO_I32 => match args {
                VmHostArgs::I32(value) => {
                    write_kernel_echo_i32(value, output);
                    Ok(VmHostResult::None)
                }
                VmHostArgs::None | VmHostArgs::StrConst(_) => Err(VmError::TypeMismatch),
            },
            KERNEL_ECHO_STR => match args {
                VmHostArgs::StrConst(value) => {
                    write_kernel_echo_str(value, output)?;
                    Ok(VmHostResult::None)
                }
                VmHostArgs::None | VmHostArgs::I32(_) => Err(VmError::TypeMismatch),
            },
            KERNEL_GRAPHICS_LOG => match args {
                VmHostArgs::StrConst(value) => {
                    output.write_str(value);
                    Ok(VmHostResult::None)
                }
                VmHostArgs::None | VmHostArgs::I32(_) => Err(VmError::TypeMismatch),
            },
            KERNEL_GRAPHICS_LOG_CLEAR => match args {
                VmHostArgs::None => {
                    output.clear_log();
                    output.write_str(LOG_CLEARED_LINE);
                    Ok(VmHostResult::None)
                }
                VmHostArgs::I32(_) | VmHostArgs::StrConst(_) => Err(VmError::TypeMismatch),
            },
            KERNEL_CLOCK_STATUS => {
                write_kernel_clock_status(output);
                Ok(VmHostResult::None)
            }
            _ => Err(VmError::UnsupportedService(service_id)),
        }
    }
}

fn kernel_tick_value() -> i32 {
    let ticks = irq0_ticks_status_snapshot();
    ticks.target_ticks as i32
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

fn write_kernel_clock_status<O: VmOutput>(output: &mut O) {
    let clock = kernel_clock_status_snapshot();
    output.write_str(KERNEL_CLOCK_LINE);
    output.write_str(if clock.runtime == "running" {
        KERNEL_CLOCK_RUNTIME_RUNNING_LINE
    } else {
        KERNEL_CLOCK_RUNTIME_STOPPED_LINE
    });

    let mut bytes = [0u8; 24];
    let mut line = FixedLineBuffer::new(&mut bytes);
    let _ = write!(line, "ticks: {:04}", clock.ticks);
    output.write_str(line.as_str());
}

fn write_kernel_echo_i32<O: VmOutput>(value: i32, output: &mut O) {
    let mut bytes = [0u8; 24];
    let mut line = FixedLineBuffer::new(&mut bytes);
    let _ = write!(line, "ARG VALUE {}", value);
    output.write_str(line.as_str());
}

fn write_kernel_echo_str<O: VmOutput>(value: &str, output: &mut O) -> Result<(), VmError> {
    let mut bytes = [0u8; 64];
    let mut line = FixedLineBuffer::new(&mut bytes);
    write!(line, "ARG TEXT {}", value).map_err(|_| VmError::TypeMismatch)?;
    output.write_str(line.as_str());
    Ok(())
}

impl VmOutput for DbyteAppCaptureOutput {
    fn write_str(&mut self, value: &str) {
        // Full app output is the execution contract; display_lines must not bypass this capture.
        if self.app.name == "clockinfo" {
            match self.line_index {
                0 if value == "APP CLOCKINFO" => {}
                1 if value == KERNEL_CLOCK_LINE => {}
                2 if value == KERNEL_CLOCK_RUNTIME_RUNNING_LINE => {
                    self.clock_runtime = Some("running");
                }
                2 if value == KERNEL_CLOCK_RUNTIME_STOPPED_LINE => {
                    self.clock_runtime = Some("stopped");
                }
                3 => match decode_kernel_clock_ticks_line(value) {
                    Some(ticks) => self.clock_ticks = Some(ticks),
                    None => self.matched = false,
                },
                _ => self.matched = false,
            }
        } else if self.line_index >= self.app.output_lines.len()
            || value != self.app.output_lines[self.line_index]
        {
            self.matched = false;
        }
        self.line_index += 1;
    }

    fn write_i32(&mut self, value: i32) {
        if self.app.name == "clockinfo"
            || self.line_index >= self.app.output_lines.len()
            || Some(value) != expected_i32_value(self.app.output_lines[self.line_index])
        {
            self.matched = false;
        }
        self.line_index += 1;
    }
}

fn decode_kernel_clock_ticks_line(value: &str) -> Option<u32> {
    const PREFIX: &[u8] = b"ticks: ";
    let bytes = value.as_bytes();
    if !bytes.starts_with(PREFIX) || bytes.len() < PREFIX.len() + 4 {
        return None;
    }

    let mut ticks: u32 = 0;
    for byte in &bytes[PREFIX.len()..] {
        if *byte < b'0' || *byte > b'9' {
            return None;
        }
        ticks = ticks.checked_mul(10)?.checked_add((*byte - b'0') as u32)?;
    }
    Some(ticks)
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

pub fn run_embedded_app(name: &[u8]) -> EmbeddedDbyteAppRunResult {
    let app = match find_embedded_app(name) {
        Some(app) => app,
        None => return EmbeddedDbyteAppRunResult::NotFound,
    };
    let mut output = DbyteAppCaptureOutput {
        app,
        line_index: 0,
        matched: true,
        clock_runtime: None,
        clock_ticks: None,
    };

    let result = run_embedded_app_program(app.bytecode, app.consts, &mut output);
    let clock_status = match (output.clock_runtime, output.clock_ticks) {
        (Some(runtime), Some(ticks)) => Some(EmbeddedDbyteClockStatus { runtime, ticks }),
        _ => None,
    };
    let dynamic_output_complete = app.name != "clockinfo" || clock_status.is_some();
    // A display projection is renderable only after bytecode produced every expected output line.
    if output.matched && output.line_index == app.output_lines.len() && dynamic_output_complete {
        match result {
            Ok(()) => EmbeddedDbyteAppRunResult::Ok(EmbeddedDbyteAppCapture { app, clock_status }),
            Err(error) => EmbeddedDbyteAppRunResult::VmError(EmbeddedDbyteAppError { app, error }),
        }
    } else {
        EmbeddedDbyteAppRunResult::VmError(EmbeddedDbyteAppError {
            app,
            error: VmError::TypeMismatch,
        })
    }
}

#[allow(dead_code)]
pub fn run_embedded_app_capture(name: &[u8]) -> Option<Result<EmbeddedDbyteAppCapture, VmError>> {
    match run_embedded_app(name) {
        EmbeddedDbyteAppRunResult::Ok(capture) => Some(Ok(capture)),
        EmbeddedDbyteAppRunResult::VmError(error) => Some(Err(error.error)),
        EmbeddedDbyteAppRunResult::NotFound => None,
    }
}

pub fn app_ok_line() -> &'static str {
    APP_OK_LINE
}

pub fn app_not_found_line() -> &'static str {
    APP_NOT_FOUND_LINE
}

pub fn vm_error_graphics_name(error: VmError) -> &'static str {
    match error {
        VmError::StackOverflow => "StackOverflow",
        VmError::StackUnderflow => "StackUnderflow",
        VmError::TypeMismatch => "TypeMismatch",
        VmError::StrConstIndexOutOfBounds => "StrConstIndexOutOfBounds",
        VmError::UnexpectedEnd => "UnexpectedEnd",
        VmError::UnsupportedService(_) => "UnsupportedService",
        VmError::UnknownOpcode(_) => "UnknownOpcode",
        VmError::MissingHalt => "MissingHalt",
    }
}

pub fn vm_error_graphics_u8_payload(error: VmError) -> Option<u8> {
    match error {
        VmError::UnsupportedService(value) | VmError::UnknownOpcode(value) => Some(value),
        VmError::StackOverflow
        | VmError::StackUnderflow
        | VmError::TypeMismatch
        | VmError::StrConstIndexOutOfBounds
        | VmError::UnexpectedEnd
        | VmError::MissingHalt => None,
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
