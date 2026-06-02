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

use core::sync::atomic::{AtomicBool, Ordering};

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

    fn eoi_write_hw_smoke_from_state(
        writes_this_command: u8,
        hardware_mutation: &'static str,
        fire_result: &'static str,
    ) -> EoiWriteHwSmokeStatus {
        let armed = EOI_WRITE_HW_SMOKE_ARMED.load(Ordering::SeqCst);
        let consumed = EOI_WRITE_HW_SMOKE_CONSUMED.load(Ordering::SeqCst);
        let performed = EOI_WRITE_HW_SMOKE_PERFORMED.load(Ordering::SeqCst);
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
            write_pic_port(PIC_MASTER_COMMAND, PIC_EOI);
        }
        EOI_WRITE_HW_SMOKE_CONSUMED.store(true, Ordering::SeqCst);
        EOI_WRITE_HW_SMOKE_PERFORMED.store(true, Ordering::SeqCst);
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

/// Writes one byte to a PIC command/data port for the controlled smoke path.
unsafe fn write_pic_port(port: u16, value: u8) {
    core::arch::asm!(
        "out dx, al",
        in("dx") port,
        in("al") value,
        options(nomem, nostack, preserves_flags)
    );
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
