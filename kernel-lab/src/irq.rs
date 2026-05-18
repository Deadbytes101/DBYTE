#![allow(dead_code)]

//! IRQ handler skeleton foundation.
//!
//! This module documents the first hardware IRQ handler shapes without making
//! them active. It contains no assembly stubs, no active ABI entrypoints, no
//! PIC EOI writes, and no port I/O. The symbols are compiled so verification
//! can guard the intended IRQ0/IRQ1 contract before any IDT binding exists.

/// Planned CPU vector for IRQ0 after the future PIC remap.
pub const IRQ0_VECTOR: u8 = 32;

/// Planned CPU vector for IRQ1 after the future PIC remap.
pub const IRQ1_VECTOR: u8 = 33;

/// Planned display name for IRQ0.
pub const IRQ0_NAME: &str = "timer";

/// Planned display name for IRQ1.
pub const IRQ1_NAME: &str = "keyboard";

/// Dormant gate state shared by planned IRQ gate descriptors.
pub const IRQ_GATE_STATE_DORMANT: &str = "dormant / disabled";

/// Disabled IDT binding state for planned IRQ gates.
pub const IRQ_IDT_BINDING_DISABLED: &str = "disabled";

/// Disabled PIC remap state for planned IRQ gates.
pub const IRQ_PIC_REMAP_DISABLED: &str = "disabled";

/// Disabled EOI dispatch state for planned IRQ gates.
pub const IRQ_EOI_DISPATCH_DISABLED: &str = "disabled";

/// Disabled maskable interrupt state for planned IRQ gates.
pub const IRQ_INTERRUPTS_DISABLED: &str = "disabled";

/// Disabled IDT install state for the future IRQ gate bind path.
pub const IRQ_IDT_INSTALL_DISABLED: &str = "planned / not installed";

/// Disabled bind helper symbol name exposed for telemetry.
pub const IRQ_BIND_DISABLED_HELPER: &str = "bind_irq_gates_disabled";

/// Boot call state for the disabled bind helper.
pub const IRQ_BIND_BOOT_CALL_DISABLED: &str = "no";

/// Runtime IDT vector state for planned IRQ gates.
pub const IRQ_VECTOR_UNBOUND: &str = "unbound";

/// Runtime handler state for planned IRQ gates.
pub const IRQ_ACTIVE_HANDLER_NONE: &str = "none";

/// Keyboard input mode while IRQ1 remains disabled.
pub const IRQ_KEYBOARD_INPUT_POLLING_ONLY: &str = "polling-only";

/// Disabled bind path note for each planned IRQ gate.
pub const IRQ_BIND_PATH_DISABLED_ONLY: &str = "disabled bind path only";

/// Documentation-only representation of a future IRQ handler.
pub struct IrqHandlerSkeleton {
    pub irq: u8,
    pub vector: u8,
    pub name: &'static str,
    pub state: &'static str,
}

/// Documentation-only representation of a future IRQ IDT gate plan.
#[derive(Copy, Clone)]
pub struct IrqGatePlan {
    pub irq: u8,
    pub vector: u8,
    pub name: &'static str,
    pub gate_state: &'static str,
    pub idt_binding: &'static str,
    pub pic_remap: &'static str,
    pub eoi_dispatch: &'static str,
    pub interrupts: &'static str,
}

/// Documentation-only step in the future IRQ gate binding path.
#[derive(Copy, Clone)]
pub struct IrqGateBindDisabledStep {
    pub irq: u8,
    pub vector: u8,
    pub name: &'static str,
    pub bind_path: &'static str,
    pub idt_install: &'static str,
    pub pic_remap: &'static str,
    pub eoi_dispatch: &'static str,
    pub interrupts: &'static str,
}

/// Documentation-only status for the disabled IRQ gate binding helper.
#[derive(Copy, Clone)]
pub struct IrqGateBindDisabledStatus {
    pub helper: &'static str,
    pub boot_call: &'static str,
    pub irq0_vector: u8,
    pub irq0_state: &'static str,
    pub irq1_vector: u8,
    pub irq1_state: &'static str,
    pub irq0_active_handler: &'static str,
    pub irq1_active_handler: &'static str,
    pub keyboard_input: &'static str,
    pub steps: [IrqGateBindDisabledStep; 2],
}

/// Documentation-only timer IRQ skeleton.
pub fn irq0_timer_skeleton() -> IrqHandlerSkeleton {
    IrqHandlerSkeleton {
        irq: 0,
        vector: IRQ0_VECTOR,
        name: IRQ0_NAME,
        state: "skeleton / disabled",
    }
}

/// Documentation-only keyboard IRQ skeleton.
pub fn irq1_keyboard_skeleton() -> IrqHandlerSkeleton {
    IrqHandlerSkeleton {
        irq: 1,
        vector: IRQ1_VECTOR,
        name: IRQ1_NAME,
        state: "skeleton / disabled",
    }
}

/// Returns the planned IRQ0/IRQ1 skeleton contract without touching hardware.
pub fn irq_handler_skeletons() -> [IrqHandlerSkeleton; 2] {
    [irq0_timer_skeleton(), irq1_keyboard_skeleton()]
}

/// Documentation-only timer IRQ gate plan.
pub fn irq0_timer_gate_plan() -> IrqGatePlan {
    IrqGatePlan {
        irq: 0,
        vector: IRQ0_VECTOR,
        name: IRQ0_NAME,
        gate_state: IRQ_GATE_STATE_DORMANT,
        idt_binding: IRQ_IDT_BINDING_DISABLED,
        pic_remap: IRQ_PIC_REMAP_DISABLED,
        eoi_dispatch: IRQ_EOI_DISPATCH_DISABLED,
        interrupts: IRQ_INTERRUPTS_DISABLED,
    }
}

/// Documentation-only keyboard IRQ gate plan.
pub fn irq1_keyboard_gate_plan() -> IrqGatePlan {
    IrqGatePlan {
        irq: 1,
        vector: IRQ1_VECTOR,
        name: IRQ1_NAME,
        gate_state: IRQ_GATE_STATE_DORMANT,
        idt_binding: IRQ_IDT_BINDING_DISABLED,
        pic_remap: IRQ_PIC_REMAP_DISABLED,
        eoi_dispatch: IRQ_EOI_DISPATCH_DISABLED,
        interrupts: IRQ_INTERRUPTS_DISABLED,
    }
}

/// Returns the planned IRQ0/IRQ1 gate contract without touching hardware.
pub fn irq_gate_plan() -> [IrqGatePlan; 2] {
    [irq0_timer_gate_plan(), irq1_keyboard_gate_plan()]
}

/// Returns the disabled IRQ0/IRQ1 gate bind path without installing gates.
pub fn bind_irq_gates_disabled() -> IrqGateBindDisabledStatus {
    IrqGateBindDisabledStatus {
        helper: IRQ_BIND_DISABLED_HELPER,
        boot_call: IRQ_BIND_BOOT_CALL_DISABLED,
        irq0_vector: IRQ0_VECTOR,
        irq0_state: IRQ_VECTOR_UNBOUND,
        irq1_vector: IRQ1_VECTOR,
        irq1_state: IRQ_VECTOR_UNBOUND,
        irq0_active_handler: IRQ_ACTIVE_HANDLER_NONE,
        irq1_active_handler: IRQ_ACTIVE_HANDLER_NONE,
        keyboard_input: IRQ_KEYBOARD_INPUT_POLLING_ONLY,
        steps: [
            IrqGateBindDisabledStep {
                irq: 0,
                vector: IRQ0_VECTOR,
                name: IRQ0_NAME,
                bind_path: IRQ_BIND_PATH_DISABLED_ONLY,
                idt_install: IRQ_IDT_INSTALL_DISABLED,
                pic_remap: IRQ_PIC_REMAP_DISABLED,
                eoi_dispatch: IRQ_EOI_DISPATCH_DISABLED,
                interrupts: IRQ_INTERRUPTS_DISABLED,
            },
            IrqGateBindDisabledStep {
                irq: 1,
                vector: IRQ1_VECTOR,
                name: IRQ1_NAME,
                bind_path: IRQ_BIND_PATH_DISABLED_ONLY,
                idt_install: IRQ_IDT_INSTALL_DISABLED,
                pic_remap: IRQ_PIC_REMAP_DISABLED,
                eoi_dispatch: IRQ_EOI_DISPATCH_DISABLED,
                interrupts: IRQ_INTERRUPTS_DISABLED,
            },
        ],
    }
}
