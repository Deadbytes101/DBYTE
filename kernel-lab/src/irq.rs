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
