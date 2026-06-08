#![allow(dead_code)]

//! Programmable Interrupt Controller (8259A PIC) Foundation
//!
//! Under freestanding constraints, this skeleton defines I/O port addresses
//! and Initialization Command Words (ICW) used to configure the PIC cascade.
//!
//! Port Roles:
//! - Master PIC Ports (0x20/0x21): Primary interrupt arbiter. Handles hardware IRQs 0-7.
//! - Slave PIC Ports (0xA0/0xA1): Cascaded secondary arbiter. Handles hardware IRQs 8-15.
//!
//! Initialization Cascade:
//! remap commands are written into Command Ports (Command registers) and Data Ports
//! in four steps: ICW1 (Init), ICW2 (Remapped vector base), ICW3 (Cascade pins), ICW4 (Mode).
//!
//! v9.0.2 keeps read-only PIC remap state telemetry while IRQ0/IRQ1
//! gate binding controlled smoke is documented in `irq.rs`. The smoke path is
//! intentionally not called from boot, does not enable STI, does not bind IRQ
//! gates, masks all PIC lines after remap, and does not dispatch EOI.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};

/// I/O Port address for the Master PIC Command/Status register.
pub const PIC_MASTER_CMD: u16 = 0x20;
/// Alias used by the first controlled PIC_EOI hardware smoke allowlist.
pub const PIC_MASTER_COMMAND: u16 = PIC_MASTER_CMD;
/// I/O Port address for the Master PIC Data/Mask register.
pub const PIC_MASTER_DATA: u16 = 0x21;

/// I/O Port address for the Slave PIC Command/Status register.
pub const PIC_SLAVE_CMD: u16 = 0xA0;
/// Alias used by strict guards to prove slave command EOI remains forbidden.
pub const PIC_SLAVE_COMMAND: u16 = PIC_SLAVE_CMD;
/// I/O Port address for the Slave PIC Data/Mask register.
pub const PIC_SLAVE_DATA: u16 = 0xA1;

/// PIC Command Word 1 (ICW1): Start initialization cascade with ICW4 present.
pub const ICW1_INIT: u8 = 0x11;

/// PIC Command Word 2 (ICW2): Base vector offset for Master PIC (remapped IRQ 0-7 to 0x20-0x27).
pub const ICW2_MASTER_OFFSET: u8 = 0x20;
/// PIC Command Word 2 (ICW2): Base vector offset for Slave PIC (remapped IRQ 8-15 to 0x28-0x2F).
pub const ICW2_SLAVE_OFFSET: u8 = 0x28;

/// First remapped IRQ CPU vector in the planned 0x20-0x2F range.
pub const IRQ_VECTOR_START: u8 = 0x20;
/// Last remapped IRQ CPU vector in the planned 0x20-0x2F range.
pub const IRQ_VECTOR_END: u8 = 0x2F;

/// PIC Command Word 3 (ICW3): Configuration showing Slave is cascaded on IRQ line 2 of the Master.
pub const ICW3_MASTER_CASCADE: u8 = 0x04;
/// PIC Command Word 3 (ICW3): Configuration showing Slave's cascade identity is IRQ 2.
pub const ICW3_SLAVE_CASCADE: u8 = 0x02;

/// PIC Command Word 4 (ICW4): Enable standard 8086/88 microprocessor mode.
pub const ICW4_8086_MODE: u8 = 0x01;

/// Command Word representing the End Of Interrupt (EOI) signal sent to the PIC command register.
pub const PIC_EOI: u8 = 0x20;

/// Default mask value used by the disabled remap plan.
pub const PIC_MASK_ALL: u8 = 0xFF;

/// Mask plan policy constants (v9.3.0).
pub const PIC_MASK_PLAN_POLICY: &str = "all masked (0xFF)";
pub const PIC_MASK_UNMASK_POLICY: &str = "no lines scheduled for unmask";
pub const PIC_MASK_UNMASK_GATE: &str = "disabled";
pub const PIC_MASK_LIVE_UNMASK: &str = "no";
pub const PIC_MASK_WRITES_PATH: &str = "controlled smoke path only";
pub const PIC_MASK_BLOCKER_REMAP: &str = "pic remap required first";
pub const PIC_MASK_CANDIDATES: &str = "none";

/// Controlled smoke state strings.
pub const PIC_REMAP_GUARD_ARMED: &str = "armed";
pub const PIC_REMAP_GUARD_NOT_ARMED: &str = "not armed";
pub const PIC_REMAP_RESULT_BLOCKED: &str = "blocked";
pub const PIC_REMAP_RESULT_REMAP_MASKED: &str = "remapped / masked";
pub const PIC_REMAP_NEXT_ARM: &str = "pic-remap-arm";
pub const PIC_REMAP_NEXT_SMOKE: &str = "pic-remap-smoke";
pub const PIC_REMAP_MODE_CONTROLLED_SMOKE: &str = "controlled smoke";
pub const PIC_REMAP_ICW_SEQUENCE_WRITTEN: &str = "written";
pub const PIC_REMAP_STI_DISABLED: &str = "disabled";
pub const PIC_REMAP_IRQ_GATES_UNBOUND: &str = "unbound";
pub const PIC_REMAP_EOI_DISPATCH_DISABLED: &str = "disabled";
pub const PIC_REMAP_YES: &str = "yes";
pub const PIC_REMAP_NO: &str = "no";
pub const PIC_REMAP_ARM_COMMAND_AVAILABLE: &str = "available";
pub const PIC_REMAP_SMOKE_COMMAND_AVAILABLE: &str = "available";
pub const PIC_REMAP_ICW_SEQUENCE_READY: &str = "ready";
pub const PIC_REMAP_ICW_SEQUENCE_EXPECTED: &str = "yes";
pub const PIC_REMAP_ICW_WRITES_CONTROLLED_ONLY: &str = "controlled command path only";
pub const PIC_REMAP_BOOT_REMAP_NO: &str = "no";
pub const PIC_REMAP_GUARD_COMMAND_ARMED_REQUIRED: &str = "command armed required";
pub const PIC_REMAP_IRQ_RUNTIME_DISABLED: &str = "disabled";
pub const PIC_REMAP_RESULT_TELEMETRY_ONLY: &str = "telemetry only";

pub const EOI_WRITE_HW_SMOKE_SCOPE: &str = "first controlled PIC_EOI hardware smoke";
pub const EOI_WRITE_HW_SMOKE_MODE: &str = "manual one-shot command path only";
pub const EOI_WRITE_HW_SMOKE_ARMED_YES: &str = "yes";
pub const EOI_WRITE_HW_SMOKE_ARMED_NO: &str = "no";
pub const EOI_WRITE_HW_SMOKE_CONSUMED_YES: &str = "yes";
pub const EOI_WRITE_HW_SMOKE_CONSUMED_NO: &str = "no";
pub const EOI_WRITE_HW_SMOKE_PERFORMED_YES: &str = "yes";
pub const EOI_WRITE_HW_SMOKE_PERFORMED_NO: &str = "no";
pub const EOI_WRITE_HW_SMOKE_RUNTIME_IRQ_ACTIVE_NO: &str = "no";
pub const EOI_WRITE_HW_SMOKE_HARDWARE_MUTATION_YES: &str = "yes";
pub const EOI_WRITE_HW_SMOKE_HARDWARE_MUTATION_NO: &str = "no";
pub const EOI_WRITE_HW_SMOKE_FIRE_RESULT_READY: &str = "ready: arm required before fire";
pub const EOI_WRITE_HW_SMOKE_FIRE_RESULT_ARMED: &str = "armed: ready for one PIC_EOI write";
pub const EOI_WRITE_HW_SMOKE_FIRE_RESULT_CLEARED: &str = "cleared: arm required before fire";
pub const EOI_WRITE_HW_SMOKE_FIRE_RESULT_BLOCKED: &str = "blocked: hardware smoke is not armed";
pub const EOI_WRITE_HW_SMOKE_FIRE_RESULT_PERFORMED: &str =
    "performed: one PIC_EOI write to master command port";
pub const EOI_WRITE_HW_SMOKE_TARGET_COMMAND_PORT: &str = "PIC_MASTER_COMMAND";
pub const EOI_WRITE_HW_SMOKE_TARGET_VALUE: &str = "PIC_EOI";
pub const EOI_WRITE_HW_SMOKE_BLOCKER_MANUAL_ONLY: &str = "manual shell command path only";
pub const EOI_WRITE_HW_SMOKE_BLOCKER_MASTER_ONLY: &str = "slave PIC command write forbidden";
pub const EOI_WRITE_HW_SMOKE_BLOCKER_ONE_SHOT: &str = "one write requires a fresh arm";
pub const EOI_WRITE_HW_SMOKE_BLOCKER_STI: &str = "STI disabled";
pub const EOI_WRITE_HW_SMOKE_BLOCKER_UNMASK: &str = "PIC unmask disabled";
pub const EOI_WRITE_HW_SMOKE_BLOCKER_LIVE_IRQ: &str = "live IRQ runtime disabled";
pub const EOI_WRITE_HW_SMOKE_BLOCKER_RUNTIME: &str = "runtime irq active: no";

static mut PIC_REMAP_SMOKE_ARMED: bool = false;
static mut PIC_REMAP_SMOKE_EXECUTED: bool = false;
static EOI_WRITE_HW_SMOKE_ARMED: AtomicBool = AtomicBool::new(false);
static EOI_WRITE_HW_SMOKE_CONSUMED: AtomicBool = AtomicBool::new(false);
static EOI_WRITE_HW_SMOKE_PERFORMED: AtomicBool = AtomicBool::new(false);
static EOI_HW_SMOKE_PROVEN_THIS_BOOT: AtomicBool = AtomicBool::new(false);
static IRQ0_UNMASK_HW_SMOKE_ARMED: AtomicBool = AtomicBool::new(false);
static IRQ0_UNMASK_HW_SMOKE_CONSUMED: AtomicBool = AtomicBool::new(false);
static IRQ0_UNMASK_HW_SMOKE_TEMPORARY_UNMASK_PERFORMED: AtomicBool = AtomicBool::new(false);
static IRQ0_UNMASK_HW_SMOKE_RESTORE_PERFORMED: AtomicBool = AtomicBool::new(false);
static IRQ0_UNMASK_HW_SMOKE_MASTER_MASK_RESTORED: AtomicBool = AtomicBool::new(false);
static PIC_IRQ0_UNMASK_HW_SMOKE_PROVEN_THIS_BOOT: AtomicBool = AtomicBool::new(false);
static IRQ0_TIMER_HANDLER_STUB_COUNTER: AtomicU32 = AtomicU32::new(0);
static IRQ0_TIMER_HANDLER_MASK_TARGET: AtomicU32 = AtomicU32::new(1);
static IRQ0_WINDOW_ARMED: AtomicBool = AtomicBool::new(false);
static IRQ0_WINDOW_STATE: AtomicU8 = AtomicU8::new(IRQ0_WINDOW_STATE_IDLE);
static IRQ0_WINDOW_DELIVERIES: AtomicU32 = AtomicU32::new(0);
static IRQ0_WINDOW_ORIGINAL_MASK_RESTORED: AtomicBool = AtomicBool::new(true);
static IRQ0_TICKS_ARMED: AtomicBool = AtomicBool::new(false);
static IRQ0_TICKS_STATE: AtomicU8 = AtomicU8::new(IRQ0_TICKS_STATE_IDLE);
static IRQ0_TICKS_OBSERVED: AtomicU32 = AtomicU32::new(0);
static IRQ0_TICKS_ORIGINAL_MASK_RESTORED: AtomicBool = AtomicBool::new(true);

const PIC_IRQ0_MASK_BIT: u8 = 0x01;
pub const IRQ0_TICK_TARGET: u32 = 8;
const IRQ0_WINDOW_STATE_IDLE: u8 = 0;
const IRQ0_WINDOW_STATE_ARMED: u8 = 1;
const IRQ0_WINDOW_STATE_FINISHED: u8 = 2;
const IRQ0_WINDOW_STATE_FAULT: u8 = 3;
const IRQ0_WINDOW_WAIT_ITERATIONS: u32 = 10_000_000;
const IRQ0_TICKS_STATE_IDLE: u8 = 0;
const IRQ0_TICKS_STATE_ARMED: u8 = 1;
const IRQ0_TICKS_STATE_FINISHED: u8 = 2;
const IRQ0_TICKS_STATE_TIMEOUT: u8 = 3;
const IRQ0_TICKS_STATE_FAULT: u8 = 4;
const IRQ0_TICKS_WAIT_ITERATIONS: u32 = 20_000_000;
const IRQ0_WINDOW_STATE_IDLE_LABEL: &str = "idle";
const IRQ0_WINDOW_STATE_ARMED_LABEL: &str = "armed";
const IRQ0_WINDOW_STATE_FINISHED_LABEL: &str = "finished";
const IRQ0_WINDOW_STATE_FAULT_LABEL: &str = "fault";
const IRQ0_TICKS_STATE_TIMEOUT_LABEL: &str = "timeout";
const IRQ0_WINDOW_YES: &str = "yes";
const IRQ0_WINDOW_NO: &str = "no";
const IRQ0_WINDOW_RUNTIME_IRQ_ACTIVE_NO: &str = "no";
const IRQ0_WINDOW_HARDWARE_MUTATION_YES: &str = "yes";
const IRQ0_WINDOW_HARDWARE_MUTATION_NO: &str = "no";
const IRQ0_WINDOW_RESULT_IDLE: &str = "status: IRQ0 delivery window idle";
const IRQ0_WINDOW_RESULT_ARMED: &str = "armed: IRQ0 delivery window ready";
const IRQ0_WINDOW_RESULT_CLEARED: &str = "cleared: IRQ0 delivery window idle";
const IRQ0_WINDOW_RESULT_BLOCKED: &str = "blocked: IRQ0 delivery window is not armed";
const IRQ0_WINDOW_RESULT_BLOCKED_PRECONDITIONS: &str = "blocked: preconditions missing";
const IRQ0_WINDOW_RESULT_FIRED_ONCE: &str = "finished: one IRQ0 delivery observed";
const IRQ0_WINDOW_RESULT_NO_DELIVERY: &str = "finished: no IRQ0 delivery observed";
const IRQ0_WINDOW_RESULT_MULTI_FIRE: &str = "fault: multiple IRQ0 deliveries observed";
const IRQ0_WINDOW_RESULT_RESTORE_FAULT: &str = "fault: original PIC mask restore failed";
const IRQ0_WINDOW_VGA_PREPARED: &str = "PREPARED / MASKED";
const IRQ0_WINDOW_VGA_FIRED_ONCE: &str = "FIRED ONCE / MASKED";
const IRQ0_WINDOW_VGA_NO_DELIVERY: &str = "NO DELIVERY / MASKED";
const IRQ0_WINDOW_VGA_MULTI_FIRE: &str = "FAULT MULTI-FIRE";
const IRQ0_TICKS_RESULT_IDLE: &str = "status: IRQ0 tick counter window idle";
const IRQ0_TICKS_RESULT_ARMED: &str = "armed: IRQ0 tick counter window ready";
const IRQ0_TICKS_RESULT_CLEARED: &str = "cleared: IRQ0 tick counter window idle";
const IRQ0_TICKS_RESULT_BLOCKED: &str = "blocked: IRQ0 tick counter window is not armed";
const IRQ0_TICKS_RESULT_BLOCKED_PRECONDITIONS: &str = "blocked: preconditions missing";
const IRQ0_TICKS_RESULT_FINISHED: &str = "finished: eight IRQ0 ticks observed";
const IRQ0_TICKS_RESULT_TIMEOUT: &str = "timeout: fewer than eight IRQ0 ticks observed";
const IRQ0_TICKS_RESULT_OVERFLOW: &str = "fault: IRQ0 tick counter overflow";
const IRQ0_TICKS_RESULT_RESTORE_FAULT: &str = "fault: original PIC mask restore failed";
const IRQ0_TICKS_VGA_PREPARED: &str = "PREPARED / MASKED";
const IRQ0_TICKS_VGA_FINISHED: &str = "TICKS 0008 / MASKED";
const IRQ0_TICKS_VGA_TIMEOUT: &str = "TIMEOUT / MASKED";
const IRQ0_TICKS_VGA_OVERFLOW: &str = "FAULT OVERFLOW";
const IRQ0_UNMASK_HW_SMOKE_SCOPE: &str = "controlled PIC IRQ0 unmask one-shot hardware smoke";
const IRQ0_UNMASK_HW_SMOKE_MODE: &str = "manual transactional command only";
const IRQ0_UNMASK_HW_SMOKE_YES: &str = "yes";
const IRQ0_UNMASK_HW_SMOKE_NO: &str = "no";
const IRQ0_UNMASK_HW_SMOKE_STI_DISABLED: &str = "disabled";
const IRQ0_UNMASK_HW_SMOKE_DELIVERY_NO: &str = "no";
const IRQ0_UNMASK_HW_SMOKE_HANDLER_REACHED_NO: &str = "no";
const IRQ0_UNMASK_HW_SMOKE_HANDLER_EOI_NO: &str = "no";
const IRQ0_UNMASK_HW_SMOKE_RUNTIME_IRQ_ACTIVE_NO: &str = "no";
const IRQ0_UNMASK_HW_SMOKE_KEYBOARD_POLLING: &str = "polling";
const IRQ0_UNMASK_HW_SMOKE_HARDWARE_MUTATION_YES: &str = "yes";
const IRQ0_UNMASK_HW_SMOKE_HARDWARE_MUTATION_NO: &str = "no";
const IRQ0_UNMASK_HW_SMOKE_RESULT_IDLE: &str = "status: IRQ0 unmask smoke idle";
const IRQ0_UNMASK_HW_SMOKE_RESULT_ARMED: &str = "armed: IRQ0 unmask smoke armed";
const IRQ0_UNMASK_HW_SMOKE_RESULT_CLEARED: &str = "cleared: IRQ0 unmask smoke unarmed";
const IRQ0_UNMASK_HW_SMOKE_RESULT_BLOCKED: &str = "blocked: IRQ0 unmask smoke is not armed";
const IRQ0_UNMASK_HW_SMOKE_RESULT_PERFORMED: &str = "performed: temporary IRQ0 unmask restored";
const IRQ0_UNMASK_HW_SMOKE_BLOCKER_MANUAL_ONLY: &str = "manual shell command path only";
const IRQ0_UNMASK_HW_SMOKE_BLOCKER_TRANSACTIONAL: &str =
    "IRQ0 unmask is transactional and restored before return";
const IRQ0_UNMASK_HW_SMOKE_BLOCKER_IRQ1: &str = "IRQ1 remains masked";
const IRQ0_UNMASK_HW_SMOKE_BLOCKER_SLAVE: &str = "slave PIC mask remains untouched";
const IRQ0_UNMASK_HW_SMOKE_BLOCKER_STI: &str = "STI remains disabled";
const IRQ0_UNMASK_HW_SMOKE_BLOCKER_DELIVERY: &str = "hardware IRQ delivery remains disabled";
const IRQ0_UNMASK_HW_SMOKE_BLOCKER_EOI: &str = "handler-triggered EOI remains disabled";
const IRQ0_UNMASK_HW_SMOKE_BLOCKER_RUNTIME: &str = "runtime IRQ dispatch remains disabled";

/// Documentation-only representation of the future PIC remap sequence.
pub struct PicRemapPlan {
    pub master_offset: u8,
    pub slave_offset: u8,
    pub irq_vector_start: u8,
    pub irq_vector_end: u8,
    pub mask_after_remap: u8,
}

/// Command-facing result for arming the controlled PIC remap smoke path.
#[derive(Copy, Clone, Debug)]
pub struct PicRemapSmokeArmStatus {
    pub mode: &'static str,
    pub next: &'static str,
    pub interrupts: &'static str,
    pub irq_gates: &'static str,
}

/// Command-facing status for the controlled PIC remap smoke state.
#[derive(Copy, Clone, Debug)]
pub struct PicRemapSmokeStatus {
    pub armed: bool,
    pub executed: bool,
    pub master_offset: u8,
    pub slave_offset: u8,
    pub mask_after_remap: u8,
    pub sti: &'static str,
    pub irq_gates: &'static str,
    pub eoi_dispatch: &'static str,
}

/// Command-facing result for an attempted controlled PIC remap smoke.
#[derive(Copy, Clone, Debug)]
pub struct PicRemapSmokeResult {
    pub guard: &'static str,
    pub icw_sequence: Option<&'static str>,
    pub master_offset: u8,
    pub slave_offset: u8,
    pub mask_after_remap: u8,
    pub sti: &'static str,
    pub irq_gates: &'static str,
    pub eoi_dispatch: &'static str,
    pub result: &'static str,
    pub next: Option<&'static str>,
}

#[derive(Copy, Clone, Debug)]
pub struct EoiWriteHwSmokeStatus {
    pub scope: &'static str,
    pub mode: &'static str,
    pub armed: &'static str,
    pub consumed: &'static str,
    pub target_command_port: &'static str,
    pub target_value: &'static str,
    pub pic_eoi_writes_this_command: u8,
    pub first_pic_eoi_write_performed: &'static str,
    pub manual_pic_eoi_smoke_proven_this_boot: &'static str,
    pub hardware_mutation: &'static str,
    pub runtime_irq_active: &'static str,
    pub fire_result: &'static str,
    pub blocker_manual_only: &'static str,
    pub blocker_master_only: &'static str,
    pub blocker_one_shot: &'static str,
    pub blocker_sti: &'static str,
    pub blocker_unmask: &'static str,
    pub blocker_live_irq: &'static str,
    pub blocker_runtime: &'static str,
}

#[derive(Copy, Clone, Debug)]
pub struct Irq0UnmaskHwSmokeStatus {
    pub scope: &'static str,
    pub mode: &'static str,
    pub armed: &'static str,
    pub consumed: &'static str,
    pub irq0_temporary_unmask_performed: &'static str,
    pub irq0_restore_performed: &'static str,
    pub irq0_currently_unmasked: &'static str,
    pub pic_master_mask_restored: &'static str,
    pub irq0_unmask_proven_this_boot: &'static str,
    pub hardware_mutation: &'static str,
    pub fire_result: &'static str,
    pub sti: &'static str,
    pub hardware_irq_delivery_allowed: &'static str,
    pub irq0_handler_reached: &'static str,
    pub handler_triggered_eoi_allowed: &'static str,
    pub runtime_irq_active: &'static str,
    pub keyboard_mode: &'static str,
    pub blocker_manual_only: &'static str,
    pub blocker_transactional: &'static str,
    pub blocker_irq1: &'static str,
    pub blocker_slave: &'static str,
    pub blocker_sti: &'static str,
    pub blocker_delivery: &'static str,
    pub blocker_eoi: &'static str,
    pub blocker_runtime: &'static str,
}

#[derive(Copy, Clone, Debug)]
pub struct Irq0WindowStatus {
    pub state: &'static str,
    pub armed: &'static str,
    pub irq0_deliveries: u32,
    pub irq0_currently_masked: &'static str,
    pub sti_currently_enabled: &'static str,
    pub original_pic_mask_restored: &'static str,
    pub if_disabled_before_return: &'static str,
    pub runtime_irq_active: &'static str,
    pub pic_remap_proof: &'static str,
    pub manual_pic_eoi_proof: &'static str,
    pub irq0_descriptor_bind_proof: &'static str,
    pub transactional_irq0_unmask_proof: &'static str,
    pub unmet_preconditions: &'static str,
    pub hardware_mutation: &'static str,
    pub result: &'static str,
    pub vga_irq0_status: &'static str,
}

#[derive(Copy, Clone, Debug)]
pub struct Irq0TicksStatus {
    pub state: &'static str,
    pub armed: &'static str,
    pub target_ticks: u32,
    pub observed_ticks: u32,
    pub irq0_currently_masked: &'static str,
    pub sti_currently_enabled: &'static str,
    pub original_pic_mask_restored: &'static str,
    pub if_disabled_before_return: &'static str,
    pub runtime_irq_active: &'static str,
    pub pic_remap_proof: &'static str,
    pub manual_pic_eoi_proof: &'static str,
    pub irq0_descriptor_bind_proof: &'static str,
    pub transactional_irq0_unmask_proof: &'static str,
    pub unmet_preconditions: &'static str,
    pub hardware_mutation: &'static str,
    pub result: &'static str,
    pub vga_irq0_status: &'static str,
}

/// Read-only state telemetry for the controlled PIC remap smoke path.
#[derive(Copy, Clone, Debug)]
pub struct PicRemapStateTelemetry {
    pub armed: bool,
    pub executed: bool,
    pub master_offset: u8,
    pub slave_offset: u8,
    pub icw_sequence_expected: &'static str,
    pub icw_sequence_applied: &'static str,
    pub mask_after_remap: u8,
    pub irq_runtime: &'static str,
}

/// Read-only history telemetry for the controlled PIC remap smoke path.
#[derive(Copy, Clone, Debug)]
pub struct PicRemapHistoryTelemetry {
    pub arm_command: &'static str,
    pub smoke_command: &'static str,
    pub last_smoke_executed: &'static str,
    pub icw_writes: &'static str,
    pub boot_remap: &'static str,
}

/// Read-only preflight telemetry for the controlled PIC remap smoke path.
#[derive(Copy, Clone, Debug)]
pub struct PicRemapPreflightTelemetry {
    pub guard: &'static str,
    pub icw_sequence: &'static str,
    pub master_offset: u8,
    pub slave_offset: u8,
    pub mask_after_remap: u8,
    pub sti: &'static str,
    pub irq_gates: &'static str,
    pub eoi_dispatch: &'static str,
    pub result: &'static str,
}

/// Documentation-only IRQ mapping entry for the planned 0x20-0x2F remap range.
pub struct IrqMapEntry {
    pub irq: u8,
    pub name: &'static str,
    pub vector: u8,
}

/// Documentation-only IRQ vector map for dry-run telemetry.
pub const IRQ_MAP_PLAN: [IrqMapEntry; 16] = [
    // Verification contract snippets kept stable across rustfmt line wrapping:
    // IrqMapEntry { irq: 0, name: "timer", vector: 0x20 }
    // IrqMapEntry { irq: 1, name: "keyboard", vector: 0x21 }
    // IrqMapEntry { irq: 2, name: "cascade", vector: 0x22 }
    // IrqMapEntry { irq: 3, name: "serial2", vector: 0x23 }
    // IrqMapEntry { irq: 4, name: "serial1", vector: 0x24 }
    // IrqMapEntry { irq: 5, name: "parallel2", vector: 0x25 }
    // IrqMapEntry { irq: 6, name: "floppy", vector: 0x26 }
    // IrqMapEntry { irq: 7, name: "parallel1", vector: 0x27 }
    // IrqMapEntry { irq: 8, name: "rtc", vector: 0x28 }
    // IrqMapEntry { irq: 9, name: "acpi", vector: 0x29 }
    // IrqMapEntry { irq: 10, name: "reserved", vector: 0x2A }
    // IrqMapEntry { irq: 11, name: "reserved", vector: 0x2B }
    // IrqMapEntry { irq: 12, name: "mouse", vector: 0x2C }
    // IrqMapEntry { irq: 13, name: "fpu", vector: 0x2D }
    // IrqMapEntry { irq: 14, name: "primary-ata", vector: 0x2E }
    // IrqMapEntry { irq: 15, name: "secondary-ata", vector: 0x2F }
    IrqMapEntry {
        irq: 0,
        name: "timer",
        vector: 0x20,
    },
    IrqMapEntry {
        irq: 1,
        name: "keyboard",
        vector: 0x21,
    },
    IrqMapEntry {
        irq: 2,
        name: "cascade",
        vector: 0x22,
    },
    IrqMapEntry {
        irq: 3,
        name: "serial2",
        vector: 0x23,
    },
    IrqMapEntry {
        irq: 4,
        name: "serial1",
        vector: 0x24,
    },
    IrqMapEntry {
        irq: 5,
        name: "parallel2",
        vector: 0x25,
    },
    IrqMapEntry {
        irq: 6,
        name: "floppy",
        vector: 0x26,
    },
    IrqMapEntry {
        irq: 7,
        name: "parallel1",
        vector: 0x27,
    },
    IrqMapEntry {
        irq: 8,
        name: "rtc",
        vector: 0x28,
    },
    IrqMapEntry {
        irq: 9,
        name: "acpi",
        vector: 0x29,
    },
    IrqMapEntry {
        irq: 10,
        name: "reserved",
        vector: 0x2A,
    },
    IrqMapEntry {
        irq: 11,
        name: "reserved",
        vector: 0x2B,
    },
    IrqMapEntry {
        irq: 12,
        name: "mouse",
        vector: 0x2C,
    },
    IrqMapEntry {
        irq: 13,
        name: "fpu",
        vector: 0x2D,
    },
    IrqMapEntry {
        irq: 14,
        name: "primary-ata",
        vector: 0x2E,
    },
    IrqMapEntry {
        irq: 15,
        name: "secondary-ata",
        vector: 0x2F,
    },
];

/// Stub representation of the PIC management sub-system.
pub struct ProgrammableInterruptController;

impl ProgrammableInterruptController {
    /// Stub initialization representing future mapping steps.
    pub fn init_stub() {
        // This stub remains deliberately unused by kernel_main.
        // Direct hardware port writes are disabled.
    }

    /// Returns the planned remap sequence constants without touching hardware.
    pub fn remap_plan() -> PicRemapPlan {
        PicRemapPlan {
            master_offset: ICW2_MASTER_OFFSET,
            slave_offset: ICW2_SLAVE_OFFSET,
            irq_vector_start: IRQ_VECTOR_START,
            irq_vector_end: IRQ_VECTOR_END,
            mask_after_remap: PIC_MASK_ALL,
        }
    }

    /// Disabled PIC remap foundation hook.
    pub fn remap_disabled() -> PicRemapPlan {
        // ICW1: PIC initialization command.
        // ICW2: master/slave offsets 0x20 and 0x28.
        // ICW3: cascade wiring between master IRQ2 and slave identity 2.
        // ICW4: 8086 mode.
        // No command/data port writes are performed in this milestone.
        Self::remap_plan()
    }

    /// Returns the planned IRQ vector map without touching hardware.
    pub fn irq_map_plan() -> &'static [IrqMapEntry; 16] {
        &IRQ_MAP_PLAN
    }

    /// Arms the explicit command-only PIC remap smoke path.
    pub fn pic_remap_smoke_arm() -> PicRemapSmokeArmStatus {
        unsafe {
            PIC_REMAP_SMOKE_ARMED = true;
        }

        PicRemapSmokeArmStatus {
            mode: PIC_REMAP_MODE_CONTROLLED_SMOKE,
            next: PIC_REMAP_NEXT_SMOKE,
            interrupts: PIC_REMAP_STI_DISABLED,
            irq_gates: PIC_REMAP_IRQ_GATES_UNBOUND,
        }
    }

    /// Returns current controlled smoke status without touching hardware.
    pub fn pic_remap_smoke_status() -> PicRemapSmokeStatus {
        let plan = Self::remap_plan();

        PicRemapSmokeStatus {
            armed: unsafe { PIC_REMAP_SMOKE_ARMED },
            executed: unsafe { PIC_REMAP_SMOKE_EXECUTED },
            master_offset: plan.master_offset,
            slave_offset: plan.slave_offset,
            mask_after_remap: plan.mask_after_remap,
            sti: PIC_REMAP_STI_DISABLED,
            irq_gates: PIC_REMAP_IRQ_GATES_UNBOUND,
            eoi_dispatch: PIC_REMAP_EOI_DISPATCH_DISABLED,
        }
    }

    /// Returns read-only state telemetry without touching PIC hardware.
    pub fn pic_remap_state() -> PicRemapStateTelemetry {
        let status = Self::pic_remap_smoke_status();

        // Verification contract snippet kept stable across rustfmt line wrapping:
        // icw_sequence_applied: if status.executed { PIC_REMAP_YES } else { PIC_REMAP_NO }
        PicRemapStateTelemetry {
            armed: status.armed,
            executed: status.executed,
            master_offset: status.master_offset,
            slave_offset: status.slave_offset,
            icw_sequence_expected: PIC_REMAP_ICW_SEQUENCE_EXPECTED,
            icw_sequence_applied: if status.executed {
                PIC_REMAP_YES
            } else {
                PIC_REMAP_NO
            },
            mask_after_remap: status.mask_after_remap,
            irq_runtime: PIC_REMAP_IRQ_RUNTIME_DISABLED,
        }
    }

    /// Returns read-only command history telemetry without touching PIC hardware.
    pub fn pic_remap_history() -> PicRemapHistoryTelemetry {
        let status = Self::pic_remap_smoke_status();

        // Verification contract snippet kept stable across rustfmt line wrapping:
        // last_smoke_executed: if status.executed { PIC_REMAP_YES } else { PIC_REMAP_NO }
        PicRemapHistoryTelemetry {
            arm_command: PIC_REMAP_ARM_COMMAND_AVAILABLE,
            smoke_command: PIC_REMAP_SMOKE_COMMAND_AVAILABLE,
            last_smoke_executed: if status.executed {
                PIC_REMAP_YES
            } else {
                PIC_REMAP_NO
            },
            icw_writes: PIC_REMAP_ICW_WRITES_CONTROLLED_ONLY,
            boot_remap: PIC_REMAP_BOOT_REMAP_NO,
        }
    }

    /// Returns read-only preflight telemetry without touching PIC hardware.
    pub fn pic_remap_preflight() -> PicRemapPreflightTelemetry {
        let plan = Self::remap_plan();

        PicRemapPreflightTelemetry {
            guard: PIC_REMAP_GUARD_COMMAND_ARMED_REQUIRED,
            icw_sequence: PIC_REMAP_ICW_SEQUENCE_READY,
            master_offset: plan.master_offset,
            slave_offset: plan.slave_offset,
            mask_after_remap: plan.mask_after_remap,
            sti: PIC_REMAP_STI_DISABLED,
            irq_gates: PIC_REMAP_IRQ_GATES_UNBOUND,
            eoi_dispatch: PIC_REMAP_EOI_DISPATCH_DISABLED,
            result: PIC_REMAP_RESULT_TELEMETRY_ONLY,
        }
    }

    /// Runs the explicit command-only PIC remap smoke path if previously armed.
    pub fn pic_remap_controlled_smoke() -> PicRemapSmokeResult {
        let plan = Self::remap_plan();

        if unsafe { !PIC_REMAP_SMOKE_ARMED } {
            return PicRemapSmokeResult {
                guard: PIC_REMAP_GUARD_NOT_ARMED,
                icw_sequence: None,
                master_offset: plan.master_offset,
                slave_offset: plan.slave_offset,
                mask_after_remap: plan.mask_after_remap,
                sti: PIC_REMAP_STI_DISABLED,
                irq_gates: PIC_REMAP_IRQ_GATES_UNBOUND,
                eoi_dispatch: PIC_REMAP_EOI_DISPATCH_DISABLED,
                result: PIC_REMAP_RESULT_BLOCKED,
                next: Some(PIC_REMAP_NEXT_ARM),
            };
        }

        unsafe {
            write_pic_port(PIC_MASTER_CMD, ICW1_INIT);
            write_pic_port(PIC_SLAVE_CMD, ICW1_INIT);
            write_pic_port(PIC_MASTER_DATA, ICW2_MASTER_OFFSET);
            write_pic_port(PIC_SLAVE_DATA, ICW2_SLAVE_OFFSET);
            write_pic_port(PIC_MASTER_DATA, ICW3_MASTER_CASCADE);
            write_pic_port(PIC_SLAVE_DATA, ICW3_SLAVE_CASCADE);
            write_pic_port(PIC_MASTER_DATA, ICW4_8086_MODE);
            write_pic_port(PIC_SLAVE_DATA, ICW4_8086_MODE);
            write_pic_port(PIC_MASTER_DATA, PIC_MASK_ALL);
            write_pic_port(PIC_SLAVE_DATA, PIC_MASK_ALL);
            PIC_REMAP_SMOKE_ARMED = false;
            PIC_REMAP_SMOKE_EXECUTED = true;
        }

        PicRemapSmokeResult {
            guard: PIC_REMAP_GUARD_ARMED,
            icw_sequence: Some(PIC_REMAP_ICW_SEQUENCE_WRITTEN),
            master_offset: plan.master_offset,
            slave_offset: plan.slave_offset,
            mask_after_remap: plan.mask_after_remap,
            sti: PIC_REMAP_STI_DISABLED,
            irq_gates: PIC_REMAP_IRQ_GATES_UNBOUND,
            eoi_dispatch: PIC_REMAP_EOI_DISPATCH_DISABLED,
            result: PIC_REMAP_RESULT_REMAP_MASKED,
            next: None,
        }
    }

    fn irq0_unmask_hw_smoke_yes_no(value: bool) -> &'static str {
        if value {
            IRQ0_UNMASK_HW_SMOKE_YES
        } else {
            IRQ0_UNMASK_HW_SMOKE_NO
        }
    }

    fn irq0_unmask_hw_smoke_from_state(
        hardware_mutation: &'static str,
        fire_result: &'static str,
    ) -> Irq0UnmaskHwSmokeStatus {
        let armed = IRQ0_UNMASK_HW_SMOKE_ARMED.load(Ordering::SeqCst);
        let consumed = IRQ0_UNMASK_HW_SMOKE_CONSUMED.load(Ordering::SeqCst);
        let temporary_unmask =
            IRQ0_UNMASK_HW_SMOKE_TEMPORARY_UNMASK_PERFORMED.load(Ordering::SeqCst);
        let restore_performed = IRQ0_UNMASK_HW_SMOKE_RESTORE_PERFORMED.load(Ordering::SeqCst);
        let master_mask_restored = IRQ0_UNMASK_HW_SMOKE_MASTER_MASK_RESTORED.load(Ordering::SeqCst);
        let proven_this_boot = PIC_IRQ0_UNMASK_HW_SMOKE_PROVEN_THIS_BOOT.load(Ordering::SeqCst);

        Irq0UnmaskHwSmokeStatus {
            scope: IRQ0_UNMASK_HW_SMOKE_SCOPE,
            mode: IRQ0_UNMASK_HW_SMOKE_MODE,
            armed: Self::irq0_unmask_hw_smoke_yes_no(armed),
            consumed: Self::irq0_unmask_hw_smoke_yes_no(consumed),
            irq0_temporary_unmask_performed: Self::irq0_unmask_hw_smoke_yes_no(temporary_unmask),
            irq0_restore_performed: Self::irq0_unmask_hw_smoke_yes_no(restore_performed),
            irq0_currently_unmasked: IRQ0_UNMASK_HW_SMOKE_NO,
            pic_master_mask_restored: Self::irq0_unmask_hw_smoke_yes_no(master_mask_restored),
            irq0_unmask_proven_this_boot: Self::irq0_unmask_hw_smoke_yes_no(proven_this_boot),
            hardware_mutation,
            fire_result,
            sti: IRQ0_UNMASK_HW_SMOKE_STI_DISABLED,
            hardware_irq_delivery_allowed: IRQ0_UNMASK_HW_SMOKE_DELIVERY_NO,
            irq0_handler_reached: IRQ0_UNMASK_HW_SMOKE_HANDLER_REACHED_NO,
            handler_triggered_eoi_allowed: IRQ0_UNMASK_HW_SMOKE_HANDLER_EOI_NO,
            runtime_irq_active: IRQ0_UNMASK_HW_SMOKE_RUNTIME_IRQ_ACTIVE_NO,
            keyboard_mode: IRQ0_UNMASK_HW_SMOKE_KEYBOARD_POLLING,
            blocker_manual_only: IRQ0_UNMASK_HW_SMOKE_BLOCKER_MANUAL_ONLY,
            blocker_transactional: IRQ0_UNMASK_HW_SMOKE_BLOCKER_TRANSACTIONAL,
            blocker_irq1: IRQ0_UNMASK_HW_SMOKE_BLOCKER_IRQ1,
            blocker_slave: IRQ0_UNMASK_HW_SMOKE_BLOCKER_SLAVE,
            blocker_sti: IRQ0_UNMASK_HW_SMOKE_BLOCKER_STI,
            blocker_delivery: IRQ0_UNMASK_HW_SMOKE_BLOCKER_DELIVERY,
            blocker_eoi: IRQ0_UNMASK_HW_SMOKE_BLOCKER_EOI,
            blocker_runtime: IRQ0_UNMASK_HW_SMOKE_BLOCKER_RUNTIME,
        }
    }

    /// Reads the transactional IRQ0 unmask smoke state without touching PIC hardware.
    pub fn irq0_unmask_hw_smoke_status() -> Irq0UnmaskHwSmokeStatus {
        Self::irq0_unmask_hw_smoke_from_state(
            IRQ0_UNMASK_HW_SMOKE_HARDWARE_MUTATION_NO,
            IRQ0_UNMASK_HW_SMOKE_RESULT_IDLE,
        )
    }

    /// Arms the transactional IRQ0 unmask smoke latch without touching PIC hardware.
    pub fn irq0_unmask_hw_smoke_arm() -> Irq0UnmaskHwSmokeStatus {
        IRQ0_UNMASK_HW_SMOKE_CONSUMED.store(false, Ordering::SeqCst);
        IRQ0_UNMASK_HW_SMOKE_TEMPORARY_UNMASK_PERFORMED.store(false, Ordering::SeqCst);
        IRQ0_UNMASK_HW_SMOKE_RESTORE_PERFORMED.store(false, Ordering::SeqCst);
        IRQ0_UNMASK_HW_SMOKE_MASTER_MASK_RESTORED.store(false, Ordering::SeqCst);
        IRQ0_UNMASK_HW_SMOKE_ARMED.store(true, Ordering::SeqCst);
        Self::irq0_unmask_hw_smoke_from_state(
            IRQ0_UNMASK_HW_SMOKE_HARDWARE_MUTATION_NO,
            IRQ0_UNMASK_HW_SMOKE_RESULT_ARMED,
        )
    }

    /// Clears only transient IRQ0 unmask smoke state without touching PIC hardware.
    pub fn irq0_unmask_hw_smoke_clear() -> Irq0UnmaskHwSmokeStatus {
        IRQ0_UNMASK_HW_SMOKE_ARMED.store(false, Ordering::SeqCst);
        IRQ0_UNMASK_HW_SMOKE_CONSUMED.store(false, Ordering::SeqCst);
        IRQ0_UNMASK_HW_SMOKE_TEMPORARY_UNMASK_PERFORMED.store(false, Ordering::SeqCst);
        IRQ0_UNMASK_HW_SMOKE_RESTORE_PERFORMED.store(false, Ordering::SeqCst);
        IRQ0_UNMASK_HW_SMOKE_MASTER_MASK_RESTORED.store(false, Ordering::SeqCst);
        Self::irq0_unmask_hw_smoke_from_state(
            IRQ0_UNMASK_HW_SMOKE_HARDWARE_MUTATION_NO,
            IRQ0_UNMASK_HW_SMOKE_RESULT_CLEARED,
        )
    }

    /// Temporarily clears the IRQ0 PIC mask bit and immediately restores the original mask.
    pub fn irq0_unmask_hw_smoke_fire() -> Irq0UnmaskHwSmokeStatus {
        if IRQ0_UNMASK_HW_SMOKE_ARMED
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Self::irq0_unmask_hw_smoke_from_state(
                IRQ0_UNMASK_HW_SMOKE_HARDWARE_MUTATION_NO,
                IRQ0_UNMASK_HW_SMOKE_RESULT_BLOCKED,
            );
        }

        unsafe {
            let original_master_mask = read_pic_port(PIC_MASTER_DATA);
            let temporary_irq0_unmasked_mask = original_master_mask & !PIC_IRQ0_MASK_BIT;
            write_pic_port(PIC_MASTER_DATA, temporary_irq0_unmasked_mask);
            let temporary_master_mask_readback = read_pic_port(PIC_MASTER_DATA);
            write_pic_port(PIC_MASTER_DATA, original_master_mask);
            let restored_master_mask_readback = read_pic_port(PIC_MASTER_DATA);

            IRQ0_UNMASK_HW_SMOKE_TEMPORARY_UNMASK_PERFORMED.store(
                (temporary_master_mask_readback & PIC_IRQ0_MASK_BIT) == 0,
                Ordering::SeqCst,
            );
            IRQ0_UNMASK_HW_SMOKE_RESTORE_PERFORMED.store(true, Ordering::SeqCst);
            IRQ0_UNMASK_HW_SMOKE_MASTER_MASK_RESTORED.store(
                restored_master_mask_readback == original_master_mask,
                Ordering::SeqCst,
            );
        }

        IRQ0_UNMASK_HW_SMOKE_CONSUMED.store(true, Ordering::SeqCst);
        if IRQ0_UNMASK_HW_SMOKE_TEMPORARY_UNMASK_PERFORMED.load(Ordering::SeqCst)
            && IRQ0_UNMASK_HW_SMOKE_MASTER_MASK_RESTORED.load(Ordering::SeqCst)
        {
            PIC_IRQ0_UNMASK_HW_SMOKE_PROVEN_THIS_BOOT.store(true, Ordering::SeqCst);
        }

        Self::irq0_unmask_hw_smoke_from_state(
            IRQ0_UNMASK_HW_SMOKE_HARDWARE_MUTATION_YES,
            IRQ0_UNMASK_HW_SMOKE_RESULT_PERFORMED,
        )
    }

    fn irq0_window_yes_no(value: bool) -> &'static str {
        if value {
            IRQ0_WINDOW_YES
        } else {
            IRQ0_WINDOW_NO
        }
    }

    fn irq0_window_state_label(state: u8) -> &'static str {
        match state {
            IRQ0_WINDOW_STATE_ARMED => IRQ0_WINDOW_STATE_ARMED_LABEL,
            IRQ0_WINDOW_STATE_FINISHED => IRQ0_WINDOW_STATE_FINISHED_LABEL,
            IRQ0_WINDOW_STATE_FAULT => IRQ0_WINDOW_STATE_FAULT_LABEL,
            _ => IRQ0_WINDOW_STATE_IDLE_LABEL,
        }
    }

    fn irq0_ticks_state_label(state: u8) -> &'static str {
        match state {
            IRQ0_TICKS_STATE_ARMED => IRQ0_WINDOW_STATE_ARMED_LABEL,
            IRQ0_TICKS_STATE_FINISHED => IRQ0_WINDOW_STATE_FINISHED_LABEL,
            IRQ0_TICKS_STATE_TIMEOUT => IRQ0_TICKS_STATE_TIMEOUT_LABEL,
            IRQ0_TICKS_STATE_FAULT => IRQ0_WINDOW_STATE_FAULT_LABEL,
            _ => IRQ0_WINDOW_STATE_IDLE_LABEL,
        }
    }

    fn irq0_window_preconditions_met(
        pic_remap_ready: bool,
        manual_pic_eoi_proof: bool,
        irq0_descriptor_bind_proof: bool,
        transactional_irq0_unmask_proof: bool,
    ) -> bool {
        pic_remap_ready
            && manual_pic_eoi_proof
            && irq0_descriptor_bind_proof
            && transactional_irq0_unmask_proof
    }

    fn irq0_window_unmet_preconditions(
        pic_remap_ready: bool,
        manual_pic_eoi_proof: bool,
        irq0_descriptor_bind_proof: bool,
        transactional_irq0_unmask_proof: bool,
    ) -> &'static str {
        if !pic_remap_ready {
            "PIC remap proof required"
        } else if !manual_pic_eoi_proof {
            "manual PIC_EOI proof required"
        } else if !irq0_descriptor_bind_proof {
            "IRQ0 descriptor bind proof required"
        } else if !transactional_irq0_unmask_proof {
            "transactional IRQ0 unmask proof required"
        } else {
            "none"
        }
    }

    fn irq0_window_status_from_state(
        pic_remap_ready: bool,
        manual_pic_eoi_proof: bool,
        irq0_descriptor_bind_proof: bool,
        transactional_irq0_unmask_proof: bool,
        hardware_mutation: &'static str,
        result: &'static str,
    ) -> Irq0WindowStatus {
        let state = IRQ0_WINDOW_STATE.load(Ordering::SeqCst);
        let deliveries = IRQ0_WINDOW_DELIVERIES.load(Ordering::SeqCst);
        let original_mask_restored = IRQ0_WINDOW_ORIGINAL_MASK_RESTORED.load(Ordering::SeqCst);
        let vga_irq0_status = match state {
            IRQ0_WINDOW_STATE_FINISHED if deliveries == 1 => IRQ0_WINDOW_VGA_FIRED_ONCE,
            IRQ0_WINDOW_STATE_FINISHED => IRQ0_WINDOW_VGA_NO_DELIVERY,
            IRQ0_WINDOW_STATE_FAULT => IRQ0_WINDOW_VGA_MULTI_FIRE,
            _ => IRQ0_WINDOW_VGA_PREPARED,
        };

        Irq0WindowStatus {
            state: Self::irq0_window_state_label(state),
            armed: Self::irq0_window_yes_no(IRQ0_WINDOW_ARMED.load(Ordering::SeqCst)),
            irq0_deliveries: deliveries,
            irq0_currently_masked: IRQ0_WINDOW_YES,
            sti_currently_enabled: IRQ0_WINDOW_NO,
            original_pic_mask_restored: Self::irq0_window_yes_no(original_mask_restored),
            if_disabled_before_return: IRQ0_WINDOW_YES,
            runtime_irq_active: IRQ0_WINDOW_RUNTIME_IRQ_ACTIVE_NO,
            pic_remap_proof: Self::irq0_window_yes_no(pic_remap_ready),
            manual_pic_eoi_proof: Self::irq0_window_yes_no(manual_pic_eoi_proof),
            irq0_descriptor_bind_proof: Self::irq0_window_yes_no(irq0_descriptor_bind_proof),
            transactional_irq0_unmask_proof: Self::irq0_window_yes_no(
                transactional_irq0_unmask_proof,
            ),
            unmet_preconditions: Self::irq0_window_unmet_preconditions(
                pic_remap_ready,
                manual_pic_eoi_proof,
                irq0_descriptor_bind_proof,
                transactional_irq0_unmask_proof,
            ),
            hardware_mutation,
            result,
            vga_irq0_status,
        }
    }

    fn irq0_ticks_status_from_state(
        pic_remap_ready: bool,
        manual_pic_eoi_proof: bool,
        irq0_descriptor_bind_proof: bool,
        transactional_irq0_unmask_proof: bool,
        hardware_mutation: &'static str,
        result: &'static str,
    ) -> Irq0TicksStatus {
        let state = IRQ0_TICKS_STATE.load(Ordering::SeqCst);
        let observed_ticks = IRQ0_TICKS_OBSERVED.load(Ordering::SeqCst);
        let original_mask_restored = IRQ0_TICKS_ORIGINAL_MASK_RESTORED.load(Ordering::SeqCst);
        let vga_irq0_status = match state {
            IRQ0_TICKS_STATE_FINISHED if observed_ticks == IRQ0_TICK_TARGET => {
                IRQ0_TICKS_VGA_FINISHED
            }
            IRQ0_TICKS_STATE_TIMEOUT => IRQ0_TICKS_VGA_TIMEOUT,
            IRQ0_TICKS_STATE_FAULT => IRQ0_TICKS_VGA_OVERFLOW,
            _ => IRQ0_TICKS_VGA_PREPARED,
        };

        Irq0TicksStatus {
            state: Self::irq0_ticks_state_label(state),
            armed: Self::irq0_window_yes_no(IRQ0_TICKS_ARMED.load(Ordering::SeqCst)),
            target_ticks: IRQ0_TICK_TARGET,
            observed_ticks,
            irq0_currently_masked: IRQ0_WINDOW_YES,
            sti_currently_enabled: IRQ0_WINDOW_NO,
            original_pic_mask_restored: Self::irq0_window_yes_no(original_mask_restored),
            if_disabled_before_return: IRQ0_WINDOW_YES,
            runtime_irq_active: IRQ0_WINDOW_RUNTIME_IRQ_ACTIVE_NO,
            pic_remap_proof: Self::irq0_window_yes_no(pic_remap_ready),
            manual_pic_eoi_proof: Self::irq0_window_yes_no(manual_pic_eoi_proof),
            irq0_descriptor_bind_proof: Self::irq0_window_yes_no(irq0_descriptor_bind_proof),
            transactional_irq0_unmask_proof: Self::irq0_window_yes_no(
                transactional_irq0_unmask_proof,
            ),
            unmet_preconditions: Self::irq0_window_unmet_preconditions(
                pic_remap_ready,
                manual_pic_eoi_proof,
                irq0_descriptor_bind_proof,
                transactional_irq0_unmask_proof,
            ),
            hardware_mutation,
            result,
            vga_irq0_status,
        }
    }

    pub fn irq0_window_status(
        pic_remap_ready: bool,
        manual_pic_eoi_proof: bool,
        irq0_descriptor_bind_proof: bool,
        transactional_irq0_unmask_proof: bool,
    ) -> Irq0WindowStatus {
        Self::irq0_window_status_from_state(
            pic_remap_ready,
            manual_pic_eoi_proof,
            irq0_descriptor_bind_proof,
            transactional_irq0_unmask_proof,
            IRQ0_WINDOW_HARDWARE_MUTATION_NO,
            IRQ0_WINDOW_RESULT_IDLE,
        )
    }

    pub fn irq0_window_arm(
        pic_remap_ready: bool,
        manual_pic_eoi_proof: bool,
        irq0_descriptor_bind_proof: bool,
        transactional_irq0_unmask_proof: bool,
    ) -> Irq0WindowStatus {
        if !Self::irq0_window_preconditions_met(
            pic_remap_ready,
            manual_pic_eoi_proof,
            irq0_descriptor_bind_proof,
            transactional_irq0_unmask_proof,
        ) {
            return Self::irq0_window_status_from_state(
                pic_remap_ready,
                manual_pic_eoi_proof,
                irq0_descriptor_bind_proof,
                transactional_irq0_unmask_proof,
                IRQ0_WINDOW_HARDWARE_MUTATION_NO,
                IRQ0_WINDOW_RESULT_BLOCKED_PRECONDITIONS,
            );
        }

        IRQ0_TIMER_HANDLER_MASK_TARGET.store(1, Ordering::SeqCst);
        IRQ0_TIMER_HANDLER_STUB_COUNTER.store(0, Ordering::SeqCst);
        IRQ0_WINDOW_DELIVERIES.store(0, Ordering::SeqCst);
        IRQ0_WINDOW_ORIGINAL_MASK_RESTORED.store(true, Ordering::SeqCst);
        IRQ0_WINDOW_STATE.store(IRQ0_WINDOW_STATE_ARMED, Ordering::SeqCst);
        IRQ0_WINDOW_ARMED.store(true, Ordering::SeqCst);

        Self::irq0_window_status_from_state(
            pic_remap_ready,
            manual_pic_eoi_proof,
            irq0_descriptor_bind_proof,
            transactional_irq0_unmask_proof,
            IRQ0_WINDOW_HARDWARE_MUTATION_NO,
            IRQ0_WINDOW_RESULT_ARMED,
        )
    }

    pub fn irq0_window_clear() -> Irq0WindowStatus {
        IRQ0_WINDOW_ARMED.store(false, Ordering::SeqCst);
        IRQ0_TIMER_HANDLER_STUB_COUNTER.store(0, Ordering::SeqCst);
        IRQ0_WINDOW_DELIVERIES.store(0, Ordering::SeqCst);
        IRQ0_WINDOW_ORIGINAL_MASK_RESTORED.store(true, Ordering::SeqCst);
        IRQ0_WINDOW_STATE.store(IRQ0_WINDOW_STATE_IDLE, Ordering::SeqCst);

        Self::irq0_window_status_from_state(
            false,
            false,
            false,
            false,
            IRQ0_WINDOW_HARDWARE_MUTATION_NO,
            IRQ0_WINDOW_RESULT_CLEARED,
        )
    }

    pub fn irq0_window_fire() -> Irq0WindowStatus {
        if IRQ0_WINDOW_ARMED
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Self::irq0_window_status_from_state(
                true,
                true,
                true,
                true,
                IRQ0_WINDOW_HARDWARE_MUTATION_NO,
                IRQ0_WINDOW_RESULT_BLOCKED,
            );
        }

        IRQ0_WINDOW_ORIGINAL_MASK_RESTORED.store(false, Ordering::SeqCst);

        let restored;
        unsafe {
            let original_master_mask = read_pic_port(PIC_MASTER_DATA);
            let temporary_irq0_unmasked_mask = original_master_mask & !PIC_IRQ0_MASK_BIT;
            write_pic_port(PIC_MASTER_DATA, temporary_irq0_unmasked_mask);

            core::arch::asm!("sti", options(nomem, nostack, preserves_flags));
            for _ in 0..IRQ0_WINDOW_WAIT_ITERATIONS {
                core::arch::asm!("pause", options(nomem, nostack, preserves_flags));
                if IRQ0_TIMER_HANDLER_STUB_COUNTER.load(Ordering::SeqCst) > 0 {
                    break;
                }
            }
            core::arch::asm!("cli", options(nomem, nostack, preserves_flags));

            write_pic_port(PIC_MASTER_DATA, original_master_mask);
            let restored_master_mask_readback = read_pic_port(PIC_MASTER_DATA);
            restored = restored_master_mask_readback == original_master_mask;
        }

        IRQ0_WINDOW_ORIGINAL_MASK_RESTORED.store(restored, Ordering::SeqCst);
        let deliveries = IRQ0_TIMER_HANDLER_STUB_COUNTER.load(Ordering::SeqCst);
        IRQ0_WINDOW_DELIVERIES.store(deliveries, Ordering::SeqCst);

        let (state, result) = if !restored {
            (IRQ0_WINDOW_STATE_FAULT, IRQ0_WINDOW_RESULT_RESTORE_FAULT)
        } else if deliveries > 1 {
            (IRQ0_WINDOW_STATE_FAULT, IRQ0_WINDOW_RESULT_MULTI_FIRE)
        } else if deliveries == 1 {
            (IRQ0_WINDOW_STATE_FINISHED, IRQ0_WINDOW_RESULT_FIRED_ONCE)
        } else {
            (IRQ0_WINDOW_STATE_FINISHED, IRQ0_WINDOW_RESULT_NO_DELIVERY)
        };
        IRQ0_WINDOW_STATE.store(state, Ordering::SeqCst);

        Self::irq0_window_status_from_state(
            true,
            true,
            true,
            true,
            IRQ0_WINDOW_HARDWARE_MUTATION_YES,
            result,
        )
    }

    pub fn irq0_ticks_status(
        pic_remap_ready: bool,
        manual_pic_eoi_proof: bool,
        irq0_descriptor_bind_proof: bool,
        transactional_irq0_unmask_proof: bool,
    ) -> Irq0TicksStatus {
        Self::irq0_ticks_status_from_state(
            pic_remap_ready,
            manual_pic_eoi_proof,
            irq0_descriptor_bind_proof,
            transactional_irq0_unmask_proof,
            IRQ0_WINDOW_HARDWARE_MUTATION_NO,
            IRQ0_TICKS_RESULT_IDLE,
        )
    }

    pub fn irq0_ticks_arm(
        pic_remap_ready: bool,
        manual_pic_eoi_proof: bool,
        irq0_descriptor_bind_proof: bool,
        transactional_irq0_unmask_proof: bool,
    ) -> Irq0TicksStatus {
        if !Self::irq0_window_preconditions_met(
            pic_remap_ready,
            manual_pic_eoi_proof,
            irq0_descriptor_bind_proof,
            transactional_irq0_unmask_proof,
        ) {
            return Self::irq0_ticks_status_from_state(
                pic_remap_ready,
                manual_pic_eoi_proof,
                irq0_descriptor_bind_proof,
                transactional_irq0_unmask_proof,
                IRQ0_WINDOW_HARDWARE_MUTATION_NO,
                IRQ0_TICKS_RESULT_BLOCKED_PRECONDITIONS,
            );
        }

        IRQ0_TIMER_HANDLER_MASK_TARGET.store(IRQ0_TICK_TARGET, Ordering::SeqCst);
        IRQ0_TIMER_HANDLER_STUB_COUNTER.store(0, Ordering::SeqCst);
        IRQ0_TICKS_OBSERVED.store(0, Ordering::SeqCst);
        IRQ0_TICKS_ORIGINAL_MASK_RESTORED.store(true, Ordering::SeqCst);
        IRQ0_TICKS_STATE.store(IRQ0_TICKS_STATE_ARMED, Ordering::SeqCst);
        IRQ0_TICKS_ARMED.store(true, Ordering::SeqCst);

        Self::irq0_ticks_status_from_state(
            pic_remap_ready,
            manual_pic_eoi_proof,
            irq0_descriptor_bind_proof,
            transactional_irq0_unmask_proof,
            IRQ0_WINDOW_HARDWARE_MUTATION_NO,
            IRQ0_TICKS_RESULT_ARMED,
        )
    }

    pub fn irq0_ticks_clear() -> Irq0TicksStatus {
        IRQ0_TICKS_ARMED.store(false, Ordering::SeqCst);
        IRQ0_TIMER_HANDLER_MASK_TARGET.store(1, Ordering::SeqCst);
        IRQ0_TIMER_HANDLER_STUB_COUNTER.store(0, Ordering::SeqCst);
        IRQ0_TICKS_OBSERVED.store(0, Ordering::SeqCst);
        IRQ0_TICKS_ORIGINAL_MASK_RESTORED.store(true, Ordering::SeqCst);
        IRQ0_TICKS_STATE.store(IRQ0_TICKS_STATE_IDLE, Ordering::SeqCst);

        Self::irq0_ticks_status_from_state(
            false,
            false,
            false,
            false,
            IRQ0_WINDOW_HARDWARE_MUTATION_NO,
            IRQ0_TICKS_RESULT_CLEARED,
        )
    }

    pub fn irq0_ticks_fire() -> Irq0TicksStatus {
        if IRQ0_TICKS_ARMED
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Self::irq0_ticks_status_from_state(
                true,
                true,
                true,
                true,
                IRQ0_WINDOW_HARDWARE_MUTATION_NO,
                IRQ0_TICKS_RESULT_BLOCKED,
            );
        }

        IRQ0_TICKS_ORIGINAL_MASK_RESTORED.store(false, Ordering::SeqCst);

        let restored;
        unsafe {
            let original_master_mask = read_pic_port(PIC_MASTER_DATA);
            let temporary_irq0_unmasked_mask = original_master_mask & !PIC_IRQ0_MASK_BIT;
            write_pic_port(PIC_MASTER_DATA, temporary_irq0_unmasked_mask);

            core::arch::asm!("sti", options(nomem, nostack, preserves_flags));
            for _ in 0..IRQ0_TICKS_WAIT_ITERATIONS {
                core::arch::asm!("pause", options(nomem, nostack, preserves_flags));
                if IRQ0_TIMER_HANDLER_STUB_COUNTER.load(Ordering::SeqCst) >= IRQ0_TICK_TARGET {
                    break;
                }
            }
            core::arch::asm!("cli", options(nomem, nostack, preserves_flags));

            write_pic_port(PIC_MASTER_DATA, original_master_mask);
            let restored_master_mask_readback = read_pic_port(PIC_MASTER_DATA);
            restored = restored_master_mask_readback == original_master_mask;
        }

        IRQ0_TICKS_ORIGINAL_MASK_RESTORED.store(restored, Ordering::SeqCst);
        let observed_ticks = IRQ0_TIMER_HANDLER_STUB_COUNTER.load(Ordering::SeqCst);
        IRQ0_TICKS_OBSERVED.store(observed_ticks, Ordering::SeqCst);
        IRQ0_TIMER_HANDLER_MASK_TARGET.store(1, Ordering::SeqCst);

        let (state, result) = if !restored {
            (IRQ0_TICKS_STATE_FAULT, IRQ0_TICKS_RESULT_RESTORE_FAULT)
        } else if observed_ticks > IRQ0_TICK_TARGET {
            (IRQ0_TICKS_STATE_FAULT, IRQ0_TICKS_RESULT_OVERFLOW)
        } else if observed_ticks == IRQ0_TICK_TARGET {
            (IRQ0_TICKS_STATE_FINISHED, IRQ0_TICKS_RESULT_FINISHED)
        } else {
            (IRQ0_TICKS_STATE_TIMEOUT, IRQ0_TICKS_RESULT_TIMEOUT)
        };
        IRQ0_TICKS_STATE.store(state, Ordering::SeqCst);

        Self::irq0_ticks_status_from_state(
            true,
            true,
            true,
            true,
            IRQ0_WINDOW_HARDWARE_MUTATION_YES,
            result,
        )
    }

    fn eoi_write_hw_smoke_from_state(
        writes_this_command: u8,
        hardware_mutation: &'static str,
        fire_result: &'static str,
    ) -> EoiWriteHwSmokeStatus {
        let armed = EOI_WRITE_HW_SMOKE_ARMED.load(Ordering::SeqCst);
        let consumed = EOI_WRITE_HW_SMOKE_CONSUMED.load(Ordering::SeqCst);
        let performed = EOI_WRITE_HW_SMOKE_PERFORMED.load(Ordering::SeqCst);
        let proven_this_boot = EOI_HW_SMOKE_PROVEN_THIS_BOOT.load(Ordering::SeqCst);
        EoiWriteHwSmokeStatus {
            scope: EOI_WRITE_HW_SMOKE_SCOPE,
            mode: EOI_WRITE_HW_SMOKE_MODE,
            armed: if armed {
                EOI_WRITE_HW_SMOKE_ARMED_YES
            } else {
                EOI_WRITE_HW_SMOKE_ARMED_NO
            },
            consumed: if consumed {
                EOI_WRITE_HW_SMOKE_CONSUMED_YES
            } else {
                EOI_WRITE_HW_SMOKE_CONSUMED_NO
            },
            target_command_port: EOI_WRITE_HW_SMOKE_TARGET_COMMAND_PORT,
            target_value: EOI_WRITE_HW_SMOKE_TARGET_VALUE,
            pic_eoi_writes_this_command: writes_this_command,
            first_pic_eoi_write_performed: if performed {
                EOI_WRITE_HW_SMOKE_PERFORMED_YES
            } else {
                EOI_WRITE_HW_SMOKE_PERFORMED_NO
            },
            manual_pic_eoi_smoke_proven_this_boot: if proven_this_boot {
                EOI_WRITE_HW_SMOKE_PERFORMED_YES
            } else {
                EOI_WRITE_HW_SMOKE_PERFORMED_NO
            },
            hardware_mutation,
            runtime_irq_active: EOI_WRITE_HW_SMOKE_RUNTIME_IRQ_ACTIVE_NO,
            fire_result,
            blocker_manual_only: EOI_WRITE_HW_SMOKE_BLOCKER_MANUAL_ONLY,
            blocker_master_only: EOI_WRITE_HW_SMOKE_BLOCKER_MASTER_ONLY,
            blocker_one_shot: EOI_WRITE_HW_SMOKE_BLOCKER_ONE_SHOT,
            blocker_sti: EOI_WRITE_HW_SMOKE_BLOCKER_STI,
            blocker_unmask: EOI_WRITE_HW_SMOKE_BLOCKER_UNMASK,
            blocker_live_irq: EOI_WRITE_HW_SMOKE_BLOCKER_LIVE_IRQ,
            blocker_runtime: EOI_WRITE_HW_SMOKE_BLOCKER_RUNTIME,
        }
    }

    /// Reads the first controlled PIC_EOI hardware smoke state without touching hardware.
    pub fn eoi_write_hw_smoke_status() -> EoiWriteHwSmokeStatus {
        Self::eoi_write_hw_smoke_from_state(
            0,
            EOI_WRITE_HW_SMOKE_HARDWARE_MUTATION_NO,
            EOI_WRITE_HW_SMOKE_FIRE_RESULT_READY,
        )
    }

    /// Arms the manual one-shot PIC_EOI hardware smoke latch without touching hardware.
    pub fn eoi_write_hw_smoke_arm() -> EoiWriteHwSmokeStatus {
        EOI_WRITE_HW_SMOKE_CONSUMED.store(false, Ordering::SeqCst);
        EOI_WRITE_HW_SMOKE_PERFORMED.store(false, Ordering::SeqCst);
        EOI_WRITE_HW_SMOKE_ARMED.store(true, Ordering::SeqCst);
        Self::eoi_write_hw_smoke_from_state(
            0,
            EOI_WRITE_HW_SMOKE_HARDWARE_MUTATION_NO,
            EOI_WRITE_HW_SMOKE_FIRE_RESULT_ARMED,
        )
    }

    /// Clears the manual one-shot PIC_EOI hardware smoke latch without touching hardware.
    pub fn eoi_write_hw_smoke_clear() -> EoiWriteHwSmokeStatus {
        EOI_WRITE_HW_SMOKE_ARMED.store(false, Ordering::SeqCst);
        EOI_WRITE_HW_SMOKE_CONSUMED.store(false, Ordering::SeqCst);
        EOI_WRITE_HW_SMOKE_PERFORMED.store(false, Ordering::SeqCst);
        Self::eoi_write_hw_smoke_from_state(
            0,
            EOI_WRITE_HW_SMOKE_HARDWARE_MUTATION_NO,
            EOI_WRITE_HW_SMOKE_FIRE_RESULT_CLEARED,
        )
    }

    /// Fires exactly one manual PIC_EOI write to the master command port when armed.
    pub fn eoi_write_hw_smoke_fire() -> EoiWriteHwSmokeStatus {
        if EOI_WRITE_HW_SMOKE_ARMED
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Self::eoi_write_hw_smoke_from_state(
                0,
                EOI_WRITE_HW_SMOKE_HARDWARE_MUTATION_NO,
                EOI_WRITE_HW_SMOKE_FIRE_RESULT_BLOCKED,
            );
        }

        unsafe {
            write_master_pic_eoi();
        }
        EOI_WRITE_HW_SMOKE_CONSUMED.store(true, Ordering::SeqCst);
        EOI_WRITE_HW_SMOKE_PERFORMED.store(true, Ordering::SeqCst);
        EOI_HW_SMOKE_PROVEN_THIS_BOOT.store(true, Ordering::SeqCst);
        Self::eoi_write_hw_smoke_from_state(
            1,
            EOI_WRITE_HW_SMOKE_HARDWARE_MUTATION_YES,
            EOI_WRITE_HW_SMOKE_FIRE_RESULT_PERFORMED,
        )
    }

    /// Returns the planned master EOI target configuration without touching hardware.
    pub fn master_eoi_plan() -> EoiPlan {
        EoiPlan {
            irq: 0,
            target: EoiTarget::MasterOnly,
            command_value: PIC_EOI,
            master_port: PIC_MASTER_CMD,
            slave_port: None,
        }
    }

    /// Returns the planned slave EOI target configuration without touching hardware.
    pub fn slave_eoi_plan() -> EoiPlan {
        EoiPlan {
            irq: 8,
            target: EoiTarget::MasterAndSlave,
            command_value: PIC_EOI,
            master_port: PIC_MASTER_CMD,
            slave_port: Some(PIC_SLAVE_CMD),
        }
    }

    /// Returns the planned IRQ0 timer EOI path configuration.
    pub fn irq0_timer_eoi_plan() -> EoiPlan {
        EoiPlan {
            irq: 0,
            target: EoiTarget::MasterOnly,
            command_value: PIC_EOI,
            master_port: PIC_MASTER_CMD,
            slave_port: None,
        }
    }

    /// Returns the planned IRQ1 keyboard EOI path configuration.
    pub fn irq1_keyboard_eoi_plan() -> EoiPlan {
        EoiPlan {
            irq: 1,
            target: EoiTarget::MasterOnly,
            command_value: PIC_EOI,
            master_port: PIC_MASTER_CMD,
            slave_port: None,
        }
    }

    /// Combined EOI strategy status accessor for dry-run CLI telemetry.
    pub fn eoi_strategy_status() -> EoiStrategyStatus {
        EoiStrategyStatus {
            strategy_name: "planned / disabled",
            enabled: false,
            pic_command: PIC_EOI,
            master_pic_state: "planned",
            slave_pic_state: "planned",
            dispatch_enabled: false,
        }
    }
}

#[no_mangle]
pub extern "C" fn irq0_timer_gate_smoke_rust() {
    let observed = IRQ0_TIMER_HANDLER_STUB_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
    unsafe {
        if observed >= IRQ0_TIMER_HANDLER_MASK_TARGET.load(Ordering::SeqCst) {
            mask_master_pic_irq0();
        }
        write_master_pic_eoi();
    }
}

/// Masks IRQ0 on the master PIC data port for the prepared IRQ0 timer stub.
unsafe fn mask_master_pic_irq0() {
    let current_master_mask = read_pic_port(PIC_MASTER_DATA);
    let masked_master_mask = current_master_mask | PIC_IRQ0_MASK_BIT;
    write_pic_port(PIC_MASTER_DATA, masked_master_mask);
}

/// Sends EOI to the master PIC command port through one physical write callsite.
unsafe fn write_master_pic_eoi() {
    write_pic_port(PIC_MASTER_COMMAND, PIC_EOI);
}

/// Writes one byte to a PIC command/data port for the controlled smoke path.
unsafe fn write_pic_port(port: u16, value: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags)
    );
}

/// Reads one byte from a PIC command/data port for the controlled smoke path.
unsafe fn read_pic_port(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!(
        "in al, dx",
        in("dx") port,
        out("al") value,
        options(nomem, nostack, preserves_flags)
    );
    value
}

/// EOI target identifier representing which PIC chip requires acknowledgment.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EoiTarget {
    /// EOI directed only to the Master PIC command port (for IRQs 0-7).
    MasterOnly,
    /// EOI directed to both Master and Slave PIC command ports (for IRQs 8-15).
    MasterAndSlave,
    /// No EOI target required.
    None,
}

/// Documentation-only EOI plan structure representing a dry-run EOI path description.
#[derive(Copy, Clone, Debug)]
pub struct EoiPlan {
    pub irq: u8,
    pub target: EoiTarget,
    pub command_value: u8,
    pub master_port: u16,
    pub slave_port: Option<u16>,
}

/// EOI strategy status representation for dry-run CLI telemetry.
#[derive(Copy, Clone, Debug)]
pub struct EoiStrategyStatus {
    pub strategy_name: &'static str,
    pub enabled: bool,
    pub pic_command: u8,
    pub master_pic_state: &'static str,
    pub slave_pic_state: &'static str,
    pub dispatch_enabled: bool,
}

/// Telemetry for the planned PIC IRQ mask policy (v9.3.0).
#[derive(Copy, Clone, Debug)]
pub struct PicMaskPlanTelemetry {
    pub mask_policy: &'static str,
    pub master_imr_planned: u8,
    pub slave_imr_planned: u8,
    pub unmask_policy: &'static str,
    pub unmask_gate: &'static str,
    pub unmask_candidates: &'static str,
}

/// Telemetry for the current PIC IRQ mask state (v9.3.0).
#[derive(Copy, Clone, Debug)]
pub struct PicMaskStatusTelemetry {
    pub master_imr_planned: u8,
    pub slave_imr_planned: u8,
    pub unmask_candidates: &'static str,
    pub unmask_blocked: &'static str,
    pub mask_writes: &'static str,
    pub live_unmask: &'static str,
}

impl ProgrammableInterruptController {
    /// Returns the planned PIC IRQ mask policy telemetry without touching hardware.
    pub fn pic_mask_plan() -> PicMaskPlanTelemetry {
        PicMaskPlanTelemetry {
            mask_policy: PIC_MASK_PLAN_POLICY,
            master_imr_planned: PIC_MASK_ALL,
            slave_imr_planned: PIC_MASK_ALL,
            unmask_policy: PIC_MASK_UNMASK_POLICY,
            unmask_gate: PIC_MASK_UNMASK_GATE,
            unmask_candidates: PIC_MASK_CANDIDATES,
        }
    }

    /// Returns the current PIC IRQ mask status telemetry without touching hardware.
    pub fn pic_mask_status() -> PicMaskStatusTelemetry {
        PicMaskStatusTelemetry {
            master_imr_planned: PIC_MASK_ALL,
            slave_imr_planned: PIC_MASK_ALL,
            unmask_candidates: PIC_MASK_CANDIDATES,
            unmask_blocked: PIC_MASK_BLOCKER_REMAP,
            mask_writes: PIC_MASK_WRITES_PATH,
            live_unmask: PIC_MASK_LIVE_UNMASK,
        }
    }
}
