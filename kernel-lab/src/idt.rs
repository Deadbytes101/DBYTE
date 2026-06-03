#![allow(dead_code)]

//! Interrupt Descriptor Table (IDT) Foundation for x86
//!
//! Under freestanding constraints, this skeleton defines Gate Descriptors
//! (IDT entries) and the base pointer representation to be loaded via LIDT.

use core::sync::atomic::{AtomicBool, Ordering};

use crate::{interrupts, pic};

pub const IDT_BIND_HW_SMOKE_VECTOR: usize = 0x81;
const IDT_BIND_HW_SMOKE_VECTOR_LABEL: &str = "0x81";
const IDT_BIND_HW_SMOKE_TARGET_HANDLER: &str = "inert test stub";
const IDT_BIND_HW_SMOKE_SCOPE: &str = "controlled IDT bind one-shot hardware smoke";
const IDT_BIND_HW_SMOKE_MODE: &str = "manual shell command only";
const IDT_BIND_HW_SMOKE_MUTATION_ALLOWED: &str = "one IDT descriptor bind only";
const IDT_BIND_HW_SMOKE_LIVE_IRQ_BIND_NO: &str = "no";
const IDT_BIND_HW_SMOKE_IRQ0_BIND_NO: &str = "no";
const IDT_BIND_HW_SMOKE_IRQ1_BIND_NO: &str = "no";
const IDT_BIND_HW_SMOKE_INTERRUPT_INVOCATION_NO: &str = "no";
const IDT_BIND_HW_SMOKE_RUNTIME_IRQ_ACTIVE_NO: &str = "no";
const IDT_BIND_HW_SMOKE_STI_DISABLED: &str = "disabled";
const IDT_BIND_HW_SMOKE_PIC_UNMASK_DISABLED: &str = "disabled";
const IDT_BIND_HW_SMOKE_KEYBOARD_POLLING: &str = "polling";
const IDT_BIND_HW_SMOKE_YES: &str = "yes";
const IDT_BIND_HW_SMOKE_NO: &str = "no";
const IDT_BIND_HW_SMOKE_BINDS_ZERO: &str = "0";
const IDT_BIND_HW_SMOKE_BINDS_ONE: &str = "1";
const IDT_BIND_HW_SMOKE_RESULT_IDLE: &str = "status: IDT bind hardware smoke idle";
const IDT_BIND_HW_SMOKE_RESULT_ARMED: &str = "armed: IDT bind hardware smoke armed";
const IDT_BIND_HW_SMOKE_RESULT_CLEARED: &str = "cleared: IDT bind hardware smoke unarmed";
const IDT_BIND_HW_SMOKE_RESULT_BLOCKED: &str = "blocked: hardware smoke is not armed";
const IDT_BIND_HW_SMOKE_RESULT_PERFORMED: &str =
    "performed: one IDT descriptor bind to vector 0x81";
const IDT_BIND_HW_SMOKE_BLOCKER_MANUAL_ONLY: &str = "manual shell command path only";
const IDT_BIND_HW_SMOKE_BLOCKER_TEST_VECTOR: &str = "dedicated non-IRQ test vector only";
const IDT_BIND_HW_SMOKE_BLOCKER_INERT_STUB: &str = "inert test stub only";
const IDT_BIND_HW_SMOKE_BLOCKER_NO_INVOCATION: &str = "interrupt invocation remains disabled";
const IDT_BIND_HW_SMOKE_BLOCKER_NO_LIVE_IRQ: &str =
    "live IRQ0/IRQ1 binding remains disabled";
const IDT_BIND_HW_SMOKE_BLOCKER_RUNTIME: &str = "runtime IRQ dispatch remains disabled";

static IDT_BIND_HW_SMOKE_ARMED: AtomicBool = AtomicBool::new(false);
static IDT_BIND_HW_SMOKE_CONSUMED: AtomicBool = AtomicBool::new(false);
static IDT_BIND_HW_SMOKE_PERFORMED: AtomicBool = AtomicBool::new(false);
static IDT_BIND_HW_SMOKE_PROVEN_THIS_BOOT: AtomicBool = AtomicBool::new(false);

pub const IRQ0_BIND_HW_SMOKE_VECTOR: usize = pic::ICW2_MASTER_OFFSET as usize;
const IRQ0_BIND_HW_SMOKE_VECTOR_LABEL: &str = "0x20";
const IRQ0_BIND_HW_SMOKE_TARGET_HANDLER: &str = "inert IRQ0 timer stub";
const IRQ0_BIND_HW_SMOKE_SCOPE: &str = "controlled IRQ0 timer bind one-shot hardware smoke";
const IRQ0_BIND_HW_SMOKE_MODE: &str = "manual shell command only";
const IRQ0_BIND_HW_SMOKE_MUTATION_ALLOWED: &str = "one IRQ0 IDT descriptor bind only";
const IRQ0_BIND_HW_SMOKE_HARDWARE_DELIVERY_NO: &str = "no";
const IRQ0_BIND_HW_SMOKE_HANDLER_EOI_NO: &str = "no";
const IRQ0_BIND_HW_SMOKE_RUNTIME_IRQ_ACTIVE_NO: &str = "no";
const IRQ0_BIND_HW_SMOKE_STI_DISABLED: &str = "disabled";
const IRQ0_BIND_HW_SMOKE_PIC_IRQ0_UNMASK_DISABLED: &str = "disabled";
const IRQ0_BIND_HW_SMOKE_KEYBOARD_POLLING: &str = "polling";
const IRQ0_BIND_HW_SMOKE_BINDS_ZERO: &str = "0";
const IRQ0_BIND_HW_SMOKE_BINDS_ONE: &str = "1";
const IRQ0_BIND_HW_SMOKE_RESULT_IDLE: &str = "status: IRQ0 timer bind smoke idle";
const IRQ0_BIND_HW_SMOKE_RESULT_ARMED: &str = "armed: IRQ0 timer bind smoke armed";
const IRQ0_BIND_HW_SMOKE_RESULT_CLEARED: &str = "cleared: IRQ0 timer bind smoke unarmed";
const IRQ0_BIND_HW_SMOKE_RESULT_BLOCKED: &str = "blocked: IRQ0 bind smoke is not armed";
const IRQ0_BIND_HW_SMOKE_RESULT_PERFORMED: &str =
    "performed: one IRQ0 IDT descriptor bind";
const IRQ0_BIND_HW_SMOKE_BLOCKER_MANUAL_ONLY: &str = "manual shell command path only";
const IRQ0_BIND_HW_SMOKE_BLOCKER_IRQ0_ONLY: &str = "IRQ0 timer vector bind only";
const IRQ0_BIND_HW_SMOKE_BLOCKER_NO_UNMASK: &str = "PIC IRQ0 unmask remains disabled";
const IRQ0_BIND_HW_SMOKE_BLOCKER_STI: &str = "STI remains disabled";
const IRQ0_BIND_HW_SMOKE_BLOCKER_NO_DELIVERY: &str =
    "timer interrupt delivery remains disabled";
const IRQ0_BIND_HW_SMOKE_BLOCKER_NO_EOI: &str = "handler-triggered EOI remains disabled";
const IRQ0_BIND_HW_SMOKE_BLOCKER_RUNTIME: &str = "runtime IRQ dispatch remains disabled";

static IRQ0_BIND_HW_SMOKE_ARMED: AtomicBool = AtomicBool::new(false);
static IRQ0_BIND_HW_SMOKE_CONSUMED: AtomicBool = AtomicBool::new(false);
static IRQ0_BIND_HW_SMOKE_PERFORMED: AtomicBool = AtomicBool::new(false);
static IRQ0_BIND_HW_SMOKE_HANDLER_REACHED: AtomicBool = AtomicBool::new(false);
static IRQ0_BIND_HW_SMOKE_PROVEN_THIS_BOOT: AtomicBool = AtomicBool::new(false);

const IDT_INVOKE_HW_SMOKE_SCOPE: &str = "controlled IDT vector invocation one-shot hardware smoke";
const IDT_INVOKE_HW_SMOKE_TARGET_HANDLER: &str = "inert test stub";
const IDT_INVOKE_HW_SMOKE_RUNTIME_IRQ_ACTIVE_NO: &str = "no";
const IDT_INVOKE_HW_SMOKE_STI_DISABLED: &str = "disabled";
const IDT_INVOKE_HW_SMOKE_PIC_UNMASK_DISABLED: &str = "disabled";
const IDT_INVOKE_HW_SMOKE_KEYBOARD_POLLING: &str = "polling";
const IDT_INVOKE_HW_SMOKE_YES: &str = "yes";
const IDT_INVOKE_HW_SMOKE_NO: &str = "no";
const IDT_INVOKE_HW_SMOKE_INVOCATIONS_ZERO: &str = "0";
const IDT_INVOKE_HW_SMOKE_INVOCATIONS_ONE: &str = "1";
const IDT_INVOKE_HW_SMOKE_RESULT_IDLE: &str = "status: IDT vector invocation smoke idle";
const IDT_INVOKE_HW_SMOKE_RESULT_ARMED: &str = "armed: IDT vector invocation smoke armed";
const IDT_INVOKE_HW_SMOKE_RESULT_CLEARED: &str = "cleared: IDT vector invocation smoke unarmed";
const IDT_INVOKE_HW_SMOKE_RESULT_BLOCKED_BIND: &str =
    "blocked: manual IDT bind proof is required";
const IDT_INVOKE_HW_SMOKE_RESULT_BLOCKED_UNARMED: &str =
    "blocked: invocation smoke is not armed";
const IDT_INVOKE_HW_SMOKE_RESULT_PERFORMED: &str = "performed: one int 0x81 invocation";
const IDT_INVOKE_HW_SMOKE_BLOCKER_BIND_PROOF: &str =
    "manual IDT bind proof is required";
const IDT_INVOKE_HW_SMOKE_BLOCKER_MANUAL_ONLY: &str = "manual shell command path only";
const IDT_INVOKE_HW_SMOKE_BLOCKER_VECTOR: &str = "dedicated vector 0x81 only";
const IDT_INVOKE_HW_SMOKE_BLOCKER_NO_IRQ: &str = "IRQ0/IRQ1 binding remains disabled";
const IDT_INVOKE_HW_SMOKE_BLOCKER_RUNTIME: &str =
    "runtime IRQ dispatch remains disabled";

static IDT_INVOKE_HW_SMOKE_ARMED: AtomicBool = AtomicBool::new(false);
static IDT_INVOKE_HW_SMOKE_CONSUMED: AtomicBool = AtomicBool::new(false);
static IDT_INVOKE_HW_SMOKE_PERFORMED: AtomicBool = AtomicBool::new(false);
static IDT_INVOKE_HW_SMOKE_STUB_REACHED: AtomicBool = AtomicBool::new(false);
static IDT_INVOKE_HW_SMOKE_PROVEN_THIS_BOOT: AtomicBool = AtomicBool::new(false);

#[derive(Copy, Clone, Debug)]
pub struct IdtBindHwSmokeStatus {
    pub scope: &'static str,
    pub mode: &'static str,
    pub armed: &'static str,
    pub consumed: &'static str,
    pub target_vector: &'static str,
    pub target_handler: &'static str,
    pub live_irq_bind: &'static str,
    pub irq0_bind: &'static str,
    pub irq1_bind: &'static str,
    pub interrupt_invocation: &'static str,
    pub hardware_mutation_allowed: &'static str,
    pub idt_descriptor_binds_this_command: &'static str,
    pub first_idt_bind_performed: &'static str,
    pub manual_idt_bind_smoke_proven_this_boot: &'static str,
    pub hardware_mutation: &'static str,
    pub runtime_irq_active: &'static str,
    pub sti: &'static str,
    pub pic_unmask: &'static str,
    pub keyboard_mode: &'static str,
    pub fire_result: &'static str,
    pub blocker_manual_only: &'static str,
    pub blocker_test_vector: &'static str,
    pub blocker_inert_stub: &'static str,
    pub blocker_no_invocation: &'static str,
    pub blocker_no_live_irq: &'static str,
    pub blocker_runtime: &'static str,
}

#[derive(Copy, Clone, Debug)]
pub struct IdtInvokeHwSmokeStatus {
    pub scope: &'static str,
    pub bind_proven_this_boot: &'static str,
    pub armed: &'static str,
    pub consumed: &'static str,
    pub target_vector: &'static str,
    pub target_handler: &'static str,
    pub interrupt_invocations_this_command: &'static str,
    pub inert_stub_reached: &'static str,
    pub first_idt_invocation_performed: &'static str,
    pub manual_idt_invocation_smoke_proven_this_boot: &'static str,
    pub hardware_mutation: &'static str,
    pub runtime_irq_active: &'static str,
    pub sti: &'static str,
    pub pic_unmask: &'static str,
    pub keyboard_mode: &'static str,
    pub fire_result: &'static str,
    pub blocker_bind_proof: &'static str,
    pub blocker_manual_only: &'static str,
    pub blocker_vector: &'static str,
    pub blocker_no_irq: &'static str,
    pub blocker_runtime: &'static str,
}

#[derive(Copy, Clone, Debug)]
pub struct Irq0BindHwSmokeStatus {
    pub scope: &'static str,
    pub mode: &'static str,
    pub armed: &'static str,
    pub consumed: &'static str,
    pub irq0_bind_smoke_vector: &'static str,
    pub target_handler: &'static str,
    pub hardware_mutation_allowed: &'static str,
    pub irq0_descriptor_binds_this_command: &'static str,
    pub irq0_descriptor_bound: &'static str,
    pub irq0_bind_proven_this_boot: &'static str,
    pub irq0_handler_reached: &'static str,
    pub irq0_hardware_delivery_allowed: &'static str,
    pub pic_irq0_unmask: &'static str,
    pub sti: &'static str,
    pub handler_triggered_eoi_allowed: &'static str,
    pub runtime_irq_active: &'static str,
    pub keyboard_mode: &'static str,
    pub hardware_mutation: &'static str,
    pub fire_result: &'static str,
    pub blocker_manual_only: &'static str,
    pub blocker_irq0_only: &'static str,
    pub blocker_no_unmask: &'static str,
    pub blocker_sti: &'static str,
    pub blocker_no_delivery: &'static str,
    pub blocker_no_eoi: &'static str,
    pub blocker_runtime: &'static str,
}

fn yes_no(value: bool) -> &'static str {
    if value {
        IDT_BIND_HW_SMOKE_YES
    } else {
        IDT_BIND_HW_SMOKE_NO
    }
}

fn irq0_bind_hw_smoke_from_state(
    armed: bool,
    consumed: bool,
    irq0_descriptor_binds_this_command: &'static str,
    hardware_mutation: &'static str,
    fire_result: &'static str,
) -> Irq0BindHwSmokeStatus {
    let proven = IRQ0_BIND_HW_SMOKE_PROVEN_THIS_BOOT.load(Ordering::SeqCst);
    Irq0BindHwSmokeStatus {
        scope: IRQ0_BIND_HW_SMOKE_SCOPE,
        mode: IRQ0_BIND_HW_SMOKE_MODE,
        armed: yes_no(armed),
        consumed: yes_no(consumed),
        irq0_bind_smoke_vector: IRQ0_BIND_HW_SMOKE_VECTOR_LABEL,
        target_handler: IRQ0_BIND_HW_SMOKE_TARGET_HANDLER,
        hardware_mutation_allowed: IRQ0_BIND_HW_SMOKE_MUTATION_ALLOWED,
        irq0_descriptor_binds_this_command,
        irq0_descriptor_bound: yes_no(proven),
        irq0_bind_proven_this_boot: yes_no(proven),
        irq0_handler_reached: yes_no(IRQ0_BIND_HW_SMOKE_HANDLER_REACHED.load(Ordering::SeqCst)),
        irq0_hardware_delivery_allowed: IRQ0_BIND_HW_SMOKE_HARDWARE_DELIVERY_NO,
        pic_irq0_unmask: IRQ0_BIND_HW_SMOKE_PIC_IRQ0_UNMASK_DISABLED,
        sti: IRQ0_BIND_HW_SMOKE_STI_DISABLED,
        handler_triggered_eoi_allowed: IRQ0_BIND_HW_SMOKE_HANDLER_EOI_NO,
        runtime_irq_active: IRQ0_BIND_HW_SMOKE_RUNTIME_IRQ_ACTIVE_NO,
        keyboard_mode: IRQ0_BIND_HW_SMOKE_KEYBOARD_POLLING,
        hardware_mutation,
        fire_result,
        blocker_manual_only: IRQ0_BIND_HW_SMOKE_BLOCKER_MANUAL_ONLY,
        blocker_irq0_only: IRQ0_BIND_HW_SMOKE_BLOCKER_IRQ0_ONLY,
        blocker_no_unmask: IRQ0_BIND_HW_SMOKE_BLOCKER_NO_UNMASK,
        blocker_sti: IRQ0_BIND_HW_SMOKE_BLOCKER_STI,
        blocker_no_delivery: IRQ0_BIND_HW_SMOKE_BLOCKER_NO_DELIVERY,
        blocker_no_eoi: IRQ0_BIND_HW_SMOKE_BLOCKER_NO_EOI,
        blocker_runtime: IRQ0_BIND_HW_SMOKE_BLOCKER_RUNTIME,
    }
}

pub fn irq0_bind_hw_smoke_status() -> Irq0BindHwSmokeStatus {
    let armed = IRQ0_BIND_HW_SMOKE_ARMED.load(Ordering::SeqCst);
    let consumed = IRQ0_BIND_HW_SMOKE_CONSUMED.load(Ordering::SeqCst);
    irq0_bind_hw_smoke_from_state(
        armed,
        consumed,
        IRQ0_BIND_HW_SMOKE_BINDS_ZERO,
        IDT_BIND_HW_SMOKE_NO,
        IRQ0_BIND_HW_SMOKE_RESULT_IDLE,
    )
}

pub fn irq0_bind_hw_smoke_arm() -> Irq0BindHwSmokeStatus {
    IRQ0_BIND_HW_SMOKE_CONSUMED.store(false, Ordering::SeqCst);
    IRQ0_BIND_HW_SMOKE_PERFORMED.store(false, Ordering::SeqCst);
    IRQ0_BIND_HW_SMOKE_HANDLER_REACHED.store(false, Ordering::SeqCst);
    IRQ0_BIND_HW_SMOKE_ARMED.store(true, Ordering::SeqCst);
    irq0_bind_hw_smoke_from_state(
        true,
        false,
        IRQ0_BIND_HW_SMOKE_BINDS_ZERO,
        IDT_BIND_HW_SMOKE_NO,
        IRQ0_BIND_HW_SMOKE_RESULT_ARMED,
    )
}

pub fn irq0_bind_hw_smoke_clear() -> Irq0BindHwSmokeStatus {
    IRQ0_BIND_HW_SMOKE_ARMED.store(false, Ordering::SeqCst);
    IRQ0_BIND_HW_SMOKE_CONSUMED.store(false, Ordering::SeqCst);
    IRQ0_BIND_HW_SMOKE_PERFORMED.store(false, Ordering::SeqCst);
    IRQ0_BIND_HW_SMOKE_HANDLER_REACHED.store(false, Ordering::SeqCst);
    irq0_bind_hw_smoke_from_state(
        false,
        false,
        IRQ0_BIND_HW_SMOKE_BINDS_ZERO,
        IDT_BIND_HW_SMOKE_NO,
        IRQ0_BIND_HW_SMOKE_RESULT_CLEARED,
    )
}

pub fn irq0_bind_hw_smoke_fire() -> Irq0BindHwSmokeStatus {
    match IRQ0_BIND_HW_SMOKE_ARMED.compare_exchange(
        true,
        false,
        Ordering::SeqCst,
        Ordering::SeqCst,
    ) {
        Ok(_) => {
            unsafe {
                IDT.entries[pic::ICW2_MASTER_OFFSET as usize]
                    .set_handler(interrupts::irq0_timer_gate_smoke_asm as *const ());
            }
            IRQ0_BIND_HW_SMOKE_CONSUMED.store(true, Ordering::SeqCst);
            IRQ0_BIND_HW_SMOKE_PERFORMED.store(true, Ordering::SeqCst);
            IRQ0_BIND_HW_SMOKE_PROVEN_THIS_BOOT.store(true, Ordering::SeqCst);
            irq0_bind_hw_smoke_from_state(
                false,
                true,
                IRQ0_BIND_HW_SMOKE_BINDS_ONE,
                IDT_BIND_HW_SMOKE_YES,
                IRQ0_BIND_HW_SMOKE_RESULT_PERFORMED,
            )
        }
        Err(_) => {
            let consumed = IRQ0_BIND_HW_SMOKE_CONSUMED.load(Ordering::SeqCst);
            irq0_bind_hw_smoke_from_state(
                false,
                consumed,
                IRQ0_BIND_HW_SMOKE_BINDS_ZERO,
                IDT_BIND_HW_SMOKE_NO,
                IRQ0_BIND_HW_SMOKE_RESULT_BLOCKED,
            )
        }
    }
}

fn idt_bind_hw_smoke_from_state(
    armed: bool,
    consumed: bool,
    idt_descriptor_binds_this_command: &'static str,
    hardware_mutation: &'static str,
    fire_result: &'static str,
) -> IdtBindHwSmokeStatus {
    IdtBindHwSmokeStatus {
        scope: IDT_BIND_HW_SMOKE_SCOPE,
        mode: IDT_BIND_HW_SMOKE_MODE,
        armed: yes_no(armed),
        consumed: yes_no(consumed),
        target_vector: IDT_BIND_HW_SMOKE_VECTOR_LABEL,
        target_handler: IDT_BIND_HW_SMOKE_TARGET_HANDLER,
        live_irq_bind: IDT_BIND_HW_SMOKE_LIVE_IRQ_BIND_NO,
        irq0_bind: IDT_BIND_HW_SMOKE_IRQ0_BIND_NO,
        irq1_bind: IDT_BIND_HW_SMOKE_IRQ1_BIND_NO,
        interrupt_invocation: IDT_BIND_HW_SMOKE_INTERRUPT_INVOCATION_NO,
        hardware_mutation_allowed: IDT_BIND_HW_SMOKE_MUTATION_ALLOWED,
        idt_descriptor_binds_this_command,
        first_idt_bind_performed: yes_no(IDT_BIND_HW_SMOKE_PERFORMED.load(Ordering::SeqCst)),
        manual_idt_bind_smoke_proven_this_boot: yes_no(
            IDT_BIND_HW_SMOKE_PROVEN_THIS_BOOT.load(Ordering::SeqCst),
        ),
        hardware_mutation,
        runtime_irq_active: IDT_BIND_HW_SMOKE_RUNTIME_IRQ_ACTIVE_NO,
        sti: IDT_BIND_HW_SMOKE_STI_DISABLED,
        pic_unmask: IDT_BIND_HW_SMOKE_PIC_UNMASK_DISABLED,
        keyboard_mode: IDT_BIND_HW_SMOKE_KEYBOARD_POLLING,
        fire_result,
        blocker_manual_only: IDT_BIND_HW_SMOKE_BLOCKER_MANUAL_ONLY,
        blocker_test_vector: IDT_BIND_HW_SMOKE_BLOCKER_TEST_VECTOR,
        blocker_inert_stub: IDT_BIND_HW_SMOKE_BLOCKER_INERT_STUB,
        blocker_no_invocation: IDT_BIND_HW_SMOKE_BLOCKER_NO_INVOCATION,
        blocker_no_live_irq: IDT_BIND_HW_SMOKE_BLOCKER_NO_LIVE_IRQ,
        blocker_runtime: IDT_BIND_HW_SMOKE_BLOCKER_RUNTIME,
    }
}

pub fn idt_bind_hw_smoke_status() -> IdtBindHwSmokeStatus {
    let armed = IDT_BIND_HW_SMOKE_ARMED.load(Ordering::SeqCst);
    let consumed = IDT_BIND_HW_SMOKE_CONSUMED.load(Ordering::SeqCst);
    idt_bind_hw_smoke_from_state(
        armed,
        consumed,
        IDT_BIND_HW_SMOKE_BINDS_ZERO,
        IDT_BIND_HW_SMOKE_NO,
        IDT_BIND_HW_SMOKE_RESULT_IDLE,
    )
}

pub fn idt_bind_hw_smoke_arm() -> IdtBindHwSmokeStatus {
    IDT_BIND_HW_SMOKE_CONSUMED.store(false, Ordering::SeqCst);
    IDT_BIND_HW_SMOKE_PERFORMED.store(false, Ordering::SeqCst);
    IDT_BIND_HW_SMOKE_ARMED.store(true, Ordering::SeqCst);
    idt_bind_hw_smoke_from_state(
        true,
        false,
        IDT_BIND_HW_SMOKE_BINDS_ZERO,
        IDT_BIND_HW_SMOKE_NO,
        IDT_BIND_HW_SMOKE_RESULT_ARMED,
    )
}

pub fn idt_bind_hw_smoke_clear() -> IdtBindHwSmokeStatus {
    IDT_BIND_HW_SMOKE_ARMED.store(false, Ordering::SeqCst);
    IDT_BIND_HW_SMOKE_CONSUMED.store(false, Ordering::SeqCst);
    IDT_BIND_HW_SMOKE_PERFORMED.store(false, Ordering::SeqCst);
    idt_bind_hw_smoke_from_state(
        false,
        false,
        IDT_BIND_HW_SMOKE_BINDS_ZERO,
        IDT_BIND_HW_SMOKE_NO,
        IDT_BIND_HW_SMOKE_RESULT_CLEARED,
    )
}

pub fn idt_bind_hw_smoke_fire() -> IdtBindHwSmokeStatus {
    match IDT_BIND_HW_SMOKE_ARMED.compare_exchange(
        true,
        false,
        Ordering::SeqCst,
        Ordering::SeqCst,
    ) {
        Ok(_) => {
            unsafe {
                IDT.entries[0x81].set_handler(interrupts::idt_bind_hw_smoke_test_asm as *const ());
            }
            IDT_BIND_HW_SMOKE_CONSUMED.store(true, Ordering::SeqCst);
            IDT_BIND_HW_SMOKE_PERFORMED.store(true, Ordering::SeqCst);
            IDT_BIND_HW_SMOKE_PROVEN_THIS_BOOT.store(true, Ordering::SeqCst);
            idt_bind_hw_smoke_from_state(
                false,
                true,
                IDT_BIND_HW_SMOKE_BINDS_ONE,
                IDT_BIND_HW_SMOKE_YES,
                IDT_BIND_HW_SMOKE_RESULT_PERFORMED,
            )
        }
        Err(_) => {
            let consumed = IDT_BIND_HW_SMOKE_CONSUMED.load(Ordering::SeqCst);
            idt_bind_hw_smoke_from_state(
                false,
                consumed,
                IDT_BIND_HW_SMOKE_BINDS_ZERO,
                IDT_BIND_HW_SMOKE_NO,
                IDT_BIND_HW_SMOKE_RESULT_BLOCKED,
            )
        }
    }
}

fn idt_invoke_hw_smoke_from_state(
    bind_proven: bool,
    armed: bool,
    consumed: bool,
    interrupt_invocations_this_command: &'static str,
    hardware_mutation: &'static str,
    fire_result: &'static str,
) -> IdtInvokeHwSmokeStatus {
    IdtInvokeHwSmokeStatus {
        scope: IDT_INVOKE_HW_SMOKE_SCOPE,
        bind_proven_this_boot: yes_no(bind_proven),
        armed: yes_no(armed),
        consumed: yes_no(consumed),
        target_vector: IDT_BIND_HW_SMOKE_VECTOR_LABEL,
        target_handler: IDT_INVOKE_HW_SMOKE_TARGET_HANDLER,
        interrupt_invocations_this_command,
        inert_stub_reached: yes_no(IDT_INVOKE_HW_SMOKE_STUB_REACHED.load(Ordering::SeqCst)),
        first_idt_invocation_performed: yes_no(
            IDT_INVOKE_HW_SMOKE_PERFORMED.load(Ordering::SeqCst),
        ),
        manual_idt_invocation_smoke_proven_this_boot: yes_no(
            IDT_INVOKE_HW_SMOKE_PROVEN_THIS_BOOT.load(Ordering::SeqCst),
        ),
        hardware_mutation,
        runtime_irq_active: IDT_INVOKE_HW_SMOKE_RUNTIME_IRQ_ACTIVE_NO,
        sti: IDT_INVOKE_HW_SMOKE_STI_DISABLED,
        pic_unmask: IDT_INVOKE_HW_SMOKE_PIC_UNMASK_DISABLED,
        keyboard_mode: IDT_INVOKE_HW_SMOKE_KEYBOARD_POLLING,
        fire_result,
        blocker_bind_proof: IDT_INVOKE_HW_SMOKE_BLOCKER_BIND_PROOF,
        blocker_manual_only: IDT_INVOKE_HW_SMOKE_BLOCKER_MANUAL_ONLY,
        blocker_vector: IDT_INVOKE_HW_SMOKE_BLOCKER_VECTOR,
        blocker_no_irq: IDT_INVOKE_HW_SMOKE_BLOCKER_NO_IRQ,
        blocker_runtime: IDT_INVOKE_HW_SMOKE_BLOCKER_RUNTIME,
    }
}

pub fn idt_invoke_hw_smoke_record_stub_reached() {
    IDT_INVOKE_HW_SMOKE_STUB_REACHED.store(true, Ordering::SeqCst);
}

pub fn idt_invoke_hw_smoke_status() -> IdtInvokeHwSmokeStatus {
    let bind_proven = IDT_BIND_HW_SMOKE_PROVEN_THIS_BOOT.load(Ordering::SeqCst);
    let armed = IDT_INVOKE_HW_SMOKE_ARMED.load(Ordering::SeqCst);
    let consumed = IDT_INVOKE_HW_SMOKE_CONSUMED.load(Ordering::SeqCst);
    idt_invoke_hw_smoke_from_state(
        bind_proven,
        armed,
        consumed,
        IDT_INVOKE_HW_SMOKE_INVOCATIONS_ZERO,
        IDT_INVOKE_HW_SMOKE_NO,
        IDT_INVOKE_HW_SMOKE_RESULT_IDLE,
    )
}

pub fn idt_invoke_hw_smoke_arm() -> IdtInvokeHwSmokeStatus {
    let bind_proven = IDT_BIND_HW_SMOKE_PROVEN_THIS_BOOT.load(Ordering::SeqCst);
    if !bind_proven {
        return idt_invoke_hw_smoke_from_state(
            false,
            false,
            IDT_INVOKE_HW_SMOKE_CONSUMED.load(Ordering::SeqCst),
            IDT_INVOKE_HW_SMOKE_INVOCATIONS_ZERO,
            IDT_INVOKE_HW_SMOKE_NO,
            IDT_INVOKE_HW_SMOKE_RESULT_BLOCKED_BIND,
        );
    }

    IDT_INVOKE_HW_SMOKE_CONSUMED.store(false, Ordering::SeqCst);
    IDT_INVOKE_HW_SMOKE_PERFORMED.store(false, Ordering::SeqCst);
    IDT_INVOKE_HW_SMOKE_STUB_REACHED.store(false, Ordering::SeqCst);
    IDT_INVOKE_HW_SMOKE_ARMED.store(true, Ordering::SeqCst);
    idt_invoke_hw_smoke_from_state(
        true,
        true,
        false,
        IDT_INVOKE_HW_SMOKE_INVOCATIONS_ZERO,
        IDT_INVOKE_HW_SMOKE_NO,
        IDT_INVOKE_HW_SMOKE_RESULT_ARMED,
    )
}

pub fn idt_invoke_hw_smoke_clear() -> IdtInvokeHwSmokeStatus {
    let bind_proven = IDT_BIND_HW_SMOKE_PROVEN_THIS_BOOT.load(Ordering::SeqCst);
    IDT_INVOKE_HW_SMOKE_ARMED.store(false, Ordering::SeqCst);
    IDT_INVOKE_HW_SMOKE_CONSUMED.store(false, Ordering::SeqCst);
    IDT_INVOKE_HW_SMOKE_PERFORMED.store(false, Ordering::SeqCst);
    IDT_INVOKE_HW_SMOKE_STUB_REACHED.store(false, Ordering::SeqCst);
    idt_invoke_hw_smoke_from_state(
        bind_proven,
        false,
        false,
        IDT_INVOKE_HW_SMOKE_INVOCATIONS_ZERO,
        IDT_INVOKE_HW_SMOKE_NO,
        IDT_INVOKE_HW_SMOKE_RESULT_CLEARED,
    )
}

pub fn idt_invoke_hw_smoke_fire() -> IdtInvokeHwSmokeStatus {
    let bind_proven = IDT_BIND_HW_SMOKE_PROVEN_THIS_BOOT.load(Ordering::SeqCst);
    if !bind_proven {
        return idt_invoke_hw_smoke_from_state(
            false,
            false,
            IDT_INVOKE_HW_SMOKE_CONSUMED.load(Ordering::SeqCst),
            IDT_INVOKE_HW_SMOKE_INVOCATIONS_ZERO,
            IDT_INVOKE_HW_SMOKE_NO,
            IDT_INVOKE_HW_SMOKE_RESULT_BLOCKED_BIND,
        );
    }

    match IDT_INVOKE_HW_SMOKE_ARMED.compare_exchange(
        true,
        false,
        Ordering::SeqCst,
        Ordering::SeqCst,
    ) {
        Ok(_) => {
            IDT_INVOKE_HW_SMOKE_STUB_REACHED.store(false, Ordering::SeqCst);
            unsafe {
                core::arch::asm!("int 0x81");
            }
            IDT_INVOKE_HW_SMOKE_CONSUMED.store(true, Ordering::SeqCst);
            IDT_INVOKE_HW_SMOKE_PERFORMED.store(true, Ordering::SeqCst);
            IDT_INVOKE_HW_SMOKE_PROVEN_THIS_BOOT.store(true, Ordering::SeqCst);
            idt_invoke_hw_smoke_from_state(
                true,
                false,
                true,
                IDT_INVOKE_HW_SMOKE_INVOCATIONS_ONE,
                IDT_INVOKE_HW_SMOKE_YES,
                IDT_INVOKE_HW_SMOKE_RESULT_PERFORMED,
            )
        }
        Err(_) => {
            let consumed = IDT_INVOKE_HW_SMOKE_CONSUMED.load(Ordering::SeqCst);
            idt_invoke_hw_smoke_from_state(
                true,
                false,
                consumed,
                IDT_INVOKE_HW_SMOKE_INVOCATIONS_ZERO,
                IDT_INVOKE_HW_SMOKE_NO,
                IDT_INVOKE_HW_SMOKE_RESULT_BLOCKED_UNARMED,
            )
        }
    }
}

/// A standard packed 8-byte x86 Gate Descriptor representing an IDT entry.
///
/// Layout constraints (8 bytes, packed):
/// - Bytes 0..1: Offset low bits (0..15 of target handler address)
/// - Bytes 2..3: GDT segment selector (typically code selector 0x08)
/// - Byte 4: Reserved/Zero (always 0x00)
/// - Byte 5: Type attributes (Present flag, DPL privilege levels, Gate type details)
/// - Bytes 6..7: Offset high bits (16..31 of target handler address)
#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub struct IdtEntry {
    /// Low 16 bits of the interrupt service routine (ISR) address.
    pub offset_low: u16,
    /// Code segment selector in the Global Descriptor Table (GDT).
    pub selector: u16,
    /// Reserved byte, always 0.
    pub zero: u8,
    /// Gate type and attributes (e.g. Present flag, Privilege level).
    pub type_attr: u8,
    /// High 16 bits of the interrupt service routine (ISR) address.
    pub offset_high: u16,
}

impl IdtEntry {
    /// Create a zero-initialized gate descriptor.
    pub const fn new() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            zero: 0,
            type_attr: 0,
            offset_high: 0,
        }
    }

    /// Create a dummy / non-present missing entry.
    pub const fn missing() -> Self {
        Self::new()
    }

    /// Configure this gate descriptor to point to a specific handler function.
    pub fn set_handler(&mut self, handler: *const ()) {
        let addr = handler as u32;
        self.offset_low = (addr & 0xFFFF) as u16;
        self.selector = 8; // GDT kernel code segment selector (0x08)
        self.zero = 0;
        self.type_attr = 0x8E; // Present, Ring 0, 32-bit Interrupt Gate
        self.offset_high = ((addr >> 16) & 0xFFFF) as u16;
    }
}

/// The IDT Pointer structure loaded into the processor register via the `lidt` assembly instruction.
///
/// Layout constraints (6 bytes, packed):
/// - Bytes 0..1 (limit): Size of the IDT table in bytes minus 1 (typically 0x7FF for 256 entries).
/// - Bytes 2..5 (base): Linear 32-bit memory address pointing to the start of the table array.
#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub struct IdtPtr {
    /// Size of the IDT in bytes minus 1.
    pub limit: u16,
    /// Linear base address of the IDT.
    pub base: u32,
}

impl IdtPtr {
    /// Create a zero-initialized pointer descriptor.
    pub const fn new() -> Self {
        Self { limit: 0, base: 0 }
    }
}

/// The main IDT table structure containing gate descriptors.
/// For standard x86, we allocate 256 entry gates.
pub struct InterruptDescriptorTable {
    pub entries: [IdtEntry; 256],
}

impl InterruptDescriptorTable {
    /// Create a new zeroed out Interrupt Descriptor Table.
    pub const fn new() -> Self {
        Self {
            entries: [IdtEntry::new(); 256],
        }
    }

    /// Load the Interrupt Descriptor Table into the CPU's IDTR register using standard `lidt` assembly.
    pub unsafe fn load(&self) {
        let ptr = IdtPtr {
            limit: (core::mem::size_of::<Self>() - 1) as u16,
            base: self as *const _ as u32,
        };
        core::arch::asm!(
            "lidt [{}]",
            in(reg) &ptr,
            options(readonly, nostack, preserves_flags)
        );
    }
}

/// Global static instance representing the active CPU Interrupt Descriptor Table.
pub static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();
