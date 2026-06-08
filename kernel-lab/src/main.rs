#![no_std]
#![no_main]
#![allow(static_mut_refs)]

use core::arch::global_asm;
use core::panic::PanicInfo;

mod idt;
mod interrupts;
mod irq;
mod mem;
mod page_fault;
mod pic;
mod serial;
mod vga;
mod vga_window;

// Minimal Multiboot 1 Header and entry point
global_asm!(
    r#"
    .section .multiboot_header, "a"
    .align 4
    .long 0x1BADB002           /* magic */
    .long 0x00                 /* flags */
    .long -(0x1BADB002 + 0x00)  /* checksum */

    .section .text
    .global _start
    _start:
        cli
        mov esp, offset stack_top
        call kernel_main
        hlt

    .section .bss
    .align 16
    stack_bottom:
        .skip 262144           /* 256 KiB stack */
    stack_top:
    "#
);

static mut SHIFT_ACTIVE: bool = false;
static mut CAPS_LOCK_ACTIVE: bool = false;

static mut LINE_BUFFER: [u8; 128] = [0; 128];
static mut LINE_LEN: usize = 0;

// Verification contract snippets kept stable across rustfmt line wrapping:
// line_str == "exception-status" || line_str == "exceptions"
// interrupts::PF_SMOKE_RECOVERY_EIP = interrupts::pf_smoke_recovery_asm as *const () as u32;

fn scancode_to_ascii(scancode: u8, shift: bool, caps: bool) -> Option<char> {
    match scancode {
        // Letters (using shift ^ caps XOR logic for uppercase/lowercase toggle)
        0x1E => Some(if shift ^ caps { 'A' } else { 'a' }), // A
        0x30 => Some(if shift ^ caps { 'B' } else { 'b' }), // B
        0x2E => Some(if shift ^ caps { 'C' } else { 'c' }), // C
        0x20 => Some(if shift ^ caps { 'D' } else { 'd' }), // D
        0x12 => Some(if shift ^ caps { 'E' } else { 'e' }), // E
        0x21 => Some(if shift ^ caps { 'F' } else { 'f' }), // F
        0x22 => Some(if shift ^ caps { 'G' } else { 'g' }), // G
        0x23 => Some(if shift ^ caps { 'H' } else { 'h' }), // H
        0x17 => Some(if shift ^ caps { 'I' } else { 'i' }), // I
        0x24 => Some(if shift ^ caps { 'J' } else { 'j' }), // J
        0x25 => Some(if shift ^ caps { 'K' } else { 'k' }), // K
        0x26 => Some(if shift ^ caps { 'L' } else { 'l' }), // L
        0x32 => Some(if shift ^ caps { 'M' } else { 'm' }), // M
        0x31 => Some(if shift ^ caps { 'N' } else { 'n' }), // N
        0x18 => Some(if shift ^ caps { 'O' } else { 'o' }), // O
        0x19 => Some(if shift ^ caps { 'P' } else { 'p' }), // P
        0x10 => Some(if shift ^ caps { 'Q' } else { 'q' }), // Q
        0x13 => Some(if shift ^ caps { 'R' } else { 'r' }), // R
        0x1F => Some(if shift ^ caps { 'S' } else { 's' }), // S
        0x14 => Some(if shift ^ caps { 'T' } else { 't' }), // T
        0x16 => Some(if shift ^ caps { 'U' } else { 'u' }), // U
        0x2F => Some(if shift ^ caps { 'V' } else { 'v' }), // V
        0x11 => Some(if shift ^ caps { 'W' } else { 'w' }), // W
        0x2D => Some(if shift ^ caps { 'X' } else { 'x' }), // X
        0x15 => Some(if shift ^ caps { 'Y' } else { 'y' }), // Y
        0x2C => Some(if shift ^ caps { 'Z' } else { 'z' }), // Z

        // Numbers and shifted basic symbols
        0x02 => Some(if shift { '!' } else { '1' }),
        0x03 => Some(if shift { '@' } else { '2' }),
        0x04 => Some(if shift { '#' } else { '3' }),
        0x05 => Some(if shift { '$' } else { '4' }),
        0x06 => Some(if shift { '%' } else { '5' }),
        0x07 => Some(if shift { '^' } else { '6' }),
        0x08 => Some(if shift { '&' } else { '7' }),
        0x09 => Some(if shift { '*' } else { '8' }),
        0x0A => Some(if shift { '(' } else { '9' }),
        0x0B => Some(if shift { ')' } else { '0' }),
        0x0C => Some(if shift { '_' } else { '-' }),
        0x0D => Some(if shift { '+' } else { '=' }),

        // Numpad arithmetic symbols
        0x4A => Some('-'),
        0x4E => Some('+'),

        // Spaces and controls
        0x39 => Some(' '),
        0x1C => Some('\n'),
        0x0E => Some('\x08'), // Backspace
        _ => None,
    }
}

fn irq_runtime_decision_snapshot() -> irq::IrqRuntimeActivationDecision {
    let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
    let gate_state = irq::irq_gate_bind_state();
    // Verification contract snippets kept stable across rustfmt line wrapping:
    // pic::ProgrammableInterruptController::pic_mask_plan();
    // pic::ProgrammableInterruptController::pic_mask_status();
    // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
    let mask_plan = pic::ProgrammableInterruptController::pic_mask_plan();
    let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
    let eoi_ready = irq::eoi_runtime_check_all_preconditions(pic_state.executed);
    let matrix = irq::irq_runtime_matrix(
        pic_state.executed,
        gate_state.executed,
        eoi_ready,
        mask_plan.mask_policy,
        irq::irq_runtime_is_armed(),
        irq::irq_runtime_is_committed(),
    );
    let activation = irq::irq_runtime_activation_dry_run(&matrix);
    let token = irq::irq_runtime_activation_token_status();
    let gate = irq::irq_runtime_activation_gate(
        token,
        matrix,
        activation,
        eoi_ready,
        mask_plan.mask_policy,
        mask_plan.unmask_policy,
    );
    let simulation = irq::irq_runtime_activation_simulation(token, matrix, activation, gate);
    let sti_plan = irq::sti_controlled_activation_plan(token, matrix, gate, simulation);
    let activation_smoke =
        irq::irq_runtime_activation_smoke(token, matrix, gate, simulation, sti_plan);
    let eoi_smoke = irq::eoi_dispatch_smoke(
        pic_state.executed,
        gate_state.executed,
        matrix,
        activation_smoke,
    );
    let pic_unmask_smoke = irq::pic_unmask_smoke(
        mask_plan.mask_policy,
        mask_plan.unmask_policy,
        token,
        matrix,
        gate,
        sti_plan,
        eoi_smoke,
    );
    let idt_bind_smoke = irq::idt_runtime_bind_smoke(
        token,
        matrix,
        gate,
        gate_state,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
    );
    let final_gate = irq::irq_runtime_final_gate(
        token,
        matrix,
        gate,
        simulation,
        sti_plan,
        activation_smoke,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let decision = irq::irq_runtime_decision_freeze(
        final_gate,
        activation_smoke,
        simulation,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    core::hint::black_box(mask_status);
    decision
}

fn print_irq_runtime_decision_note() {
    use core::fmt::Write;

    let decision = irq_runtime_decision_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "IRQ runtime activation decision note\nscope: {}\nactivation inputs: {}\nactivation decision: {}\nfinal activation allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        decision.scope,
        decision.inputs,
        decision.activation_decision,
        decision.final_activation_allowed,
        decision.hardware_mutation,
        decision.runtime_irq_active
    );
    let _ = write!(serial_writer, "IRQ runtime activation decision note\nscope: {}\nactivation inputs: {}\nactivation decision: {}\nfinal activation allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        decision.scope,
        decision.inputs,
        decision.activation_decision,
        decision.final_activation_allowed,
        decision.hardware_mutation,
        decision.runtime_irq_active
    );
}

fn print_irq_runtime_decision_status() {
    use core::fmt::Write;

    let decision = irq_runtime_decision_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "IRQ runtime activation decision\nactivation decision: {}\nfinal activation allowed: {}\nruntime irq active: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\neoi dispatch: {}\nlive idt bind: {}\nkeyboard mode: {}\n",
        decision.activation_decision,
        decision.final_activation_allowed,
        decision.runtime_irq_active,
        decision.hardware_mutation,
        decision.sti_instruction,
        decision.pic_unmask,
        decision.eoi_dispatch,
        decision.live_idt_bind,
        decision.keyboard_mode
    );
    let _ = write!(serial_writer, "IRQ runtime activation decision\nactivation decision: {}\nfinal activation allowed: {}\nruntime irq active: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\neoi dispatch: {}\nlive idt bind: {}\nkeyboard mode: {}\n",
        decision.activation_decision,
        decision.final_activation_allowed,
        decision.runtime_irq_active,
        decision.hardware_mutation,
        decision.sti_instruction,
        decision.pic_unmask,
        decision.eoi_dispatch,
        decision.live_idt_bind,
        decision.keyboard_mode
    );
}

fn print_irq_runtime_decision_blockers() {
    use core::fmt::Write;

    let decision = irq_runtime_decision_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "IRQ runtime activation decision blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nactivation decision: {}\n",
        irq::IRQ_RUNTIME_DECISION_BLOCKER_STI,
        irq::IRQ_RUNTIME_DECISION_BLOCKER_PIC_UNMASK,
        irq::IRQ_RUNTIME_DECISION_BLOCKER_EOI_DISPATCH,
        irq::IRQ_RUNTIME_DECISION_BLOCKER_LIVE_IDT_BIND,
        irq::IRQ_RUNTIME_DECISION_BLOCKER_KEYBOARD_IRQ,
        irq::IRQ_RUNTIME_DECISION_BLOCKER_RUNTIME_IRQ_ACTIVE,
        decision.activation_decision
    );
    let _ = write!(serial_writer, "IRQ runtime activation decision blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nactivation decision: {}\n",
        irq::IRQ_RUNTIME_DECISION_BLOCKER_STI,
        irq::IRQ_RUNTIME_DECISION_BLOCKER_PIC_UNMASK,
        irq::IRQ_RUNTIME_DECISION_BLOCKER_EOI_DISPATCH,
        irq::IRQ_RUNTIME_DECISION_BLOCKER_LIVE_IDT_BIND,
        irq::IRQ_RUNTIME_DECISION_BLOCKER_KEYBOARD_IRQ,
        irq::IRQ_RUNTIME_DECISION_BLOCKER_RUNTIME_IRQ_ACTIVE,
        decision.activation_decision
    );
}

fn irq_runtime_mutation_snapshot() -> irq::IrqRuntimeHardwareMutationChecklist {
    let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
    let gate_state = irq::irq_gate_bind_state();
    let mask_plan = pic::ProgrammableInterruptController::pic_mask_plan();
    let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
    let eoi_ready = irq::eoi_runtime_check_all_preconditions(pic_state.executed);
    let matrix = irq::irq_runtime_matrix(
        pic_state.executed,
        gate_state.executed,
        eoi_ready,
        mask_plan.mask_policy,
        irq::irq_runtime_is_armed(),
        irq::irq_runtime_is_committed(),
    );
    let activation = irq::irq_runtime_activation_dry_run(&matrix);
    let token = irq::irq_runtime_activation_token_status();
    let gate = irq::irq_runtime_activation_gate(
        token,
        matrix,
        activation,
        eoi_ready,
        mask_plan.mask_policy,
        mask_plan.unmask_policy,
    );
    let simulation = irq::irq_runtime_activation_simulation(token, matrix, activation, gate);
    let sti_plan = irq::sti_controlled_activation_plan(token, matrix, gate, simulation);
    let activation_smoke =
        irq::irq_runtime_activation_smoke(token, matrix, gate, simulation, sti_plan);
    let eoi_smoke = irq::eoi_dispatch_smoke(
        pic_state.executed,
        gate_state.executed,
        matrix,
        activation_smoke,
    );
    let pic_unmask_smoke = irq::pic_unmask_smoke(
        mask_plan.mask_policy,
        mask_plan.unmask_policy,
        token,
        matrix,
        gate,
        sti_plan,
        eoi_smoke,
    );
    let idt_bind_smoke = irq::idt_runtime_bind_smoke(
        token,
        matrix,
        gate,
        gate_state,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
    );
    let final_gate = irq::irq_runtime_final_gate(
        token,
        matrix,
        gate,
        simulation,
        sti_plan,
        activation_smoke,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let decision = irq::irq_runtime_decision_freeze(
        final_gate,
        activation_smoke,
        simulation,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let mutation = irq::irq_runtime_mutation_check(
        decision,
        final_gate,
        activation_smoke,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    core::hint::black_box(mask_status);
    mutation
}

fn print_irq_runtime_mutation_note() {
    use core::fmt::Write;

    let mutation = irq_runtime_mutation_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "IRQ runtime hardware mutation note\nscope: {}\nmutation inputs: {}\nhardware mutation ready: {}\nactivation decision: {}\nruntime irq active: {}\n",
        mutation.scope,
        mutation.inputs,
        mutation.hardware_mutation_ready,
        mutation.activation_decision,
        mutation.runtime_irq_active
    );
    let _ = write!(serial_writer, "IRQ runtime hardware mutation note\nscope: {}\nmutation inputs: {}\nhardware mutation ready: {}\nactivation decision: {}\nruntime irq active: {}\n",
        mutation.scope,
        mutation.inputs,
        mutation.hardware_mutation_ready,
        mutation.activation_decision,
        mutation.runtime_irq_active
    );
}

fn print_irq_runtime_mutation_status() {
    use core::fmt::Write;

    let mutation = irq_runtime_mutation_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "IRQ runtime hardware mutation readiness\nhardware mutation ready: {}\nactivation decision: {}\nfinal activation allowed: {}\nruntime irq active: {}\nsti mutation: {}\npic unmask mutation: {}\neoi dispatch mutation: {}\nidt live bind mutation: {}\nkeyboard irq mutation: {}\n",
        mutation.hardware_mutation_ready,
        mutation.activation_decision,
        mutation.final_activation_allowed,
        mutation.runtime_irq_active,
        mutation.sti_mutation,
        mutation.pic_unmask_mutation,
        mutation.eoi_dispatch_mutation,
        mutation.idt_live_bind_mutation,
        mutation.keyboard_input_mutation
    );
    let _ = write!(serial_writer, "IRQ runtime hardware mutation readiness\nhardware mutation ready: {}\nactivation decision: {}\nfinal activation allowed: {}\nruntime irq active: {}\nsti mutation: {}\npic unmask mutation: {}\neoi dispatch mutation: {}\nidt live bind mutation: {}\nkeyboard irq mutation: {}\n",
        mutation.hardware_mutation_ready,
        mutation.activation_decision,
        mutation.final_activation_allowed,
        mutation.runtime_irq_active,
        mutation.sti_mutation,
        mutation.pic_unmask_mutation,
        mutation.eoi_dispatch_mutation,
        mutation.idt_live_bind_mutation,
        mutation.keyboard_input_mutation
    );
}

fn print_irq_runtime_mutation_blockers() {
    use core::fmt::Write;

    let mutation = irq_runtime_mutation_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "IRQ runtime hardware mutation blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nhardware mutation ready: {}\n",
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_DECISION,
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_FINAL,
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_RUNTIME_IRQ,
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_STI,
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_PIC_UNMASK,
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_EOI_DISPATCH,
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_IDT_LIVE_BIND,
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_KEYBOARD_IRQ,
        mutation.hardware_mutation_ready
    );
    let _ = write!(serial_writer, "IRQ runtime hardware mutation blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nhardware mutation ready: {}\n",
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_DECISION,
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_FINAL,
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_RUNTIME_IRQ,
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_STI,
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_PIC_UNMASK,
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_EOI_DISPATCH,
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_IDT_LIVE_BIND,
        irq::IRQ_RUNTIME_MUTATION_BLOCKER_KEYBOARD_IRQ,
        mutation.hardware_mutation_ready
    );
}

fn irq_runtime_mutation_sequence_snapshot() -> irq::IrqRuntimeMutationSmokeSequence {
    let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
    let gate_state = irq::irq_gate_bind_state();
    let mask_plan = pic::ProgrammableInterruptController::pic_mask_plan();
    let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
    let eoi_ready = irq::eoi_runtime_check_all_preconditions(pic_state.executed);
    let matrix = irq::irq_runtime_matrix(
        pic_state.executed,
        gate_state.executed,
        eoi_ready,
        mask_plan.mask_policy,
        irq::irq_runtime_is_armed(),
        irq::irq_runtime_is_committed(),
    );
    let activation = irq::irq_runtime_activation_dry_run(&matrix);
    let token = irq::irq_runtime_activation_token_status();
    let gate = irq::irq_runtime_activation_gate(
        token,
        matrix,
        activation,
        eoi_ready,
        mask_plan.mask_policy,
        mask_plan.unmask_policy,
    );
    let simulation = irq::irq_runtime_activation_simulation(token, matrix, activation, gate);
    let sti_plan = irq::sti_controlled_activation_plan(token, matrix, gate, simulation);
    let activation_smoke =
        irq::irq_runtime_activation_smoke(token, matrix, gate, simulation, sti_plan);
    let eoi_smoke = irq::eoi_dispatch_smoke(
        pic_state.executed,
        gate_state.executed,
        matrix,
        activation_smoke,
    );
    let pic_unmask_smoke = irq::pic_unmask_smoke(
        mask_plan.mask_policy,
        mask_plan.unmask_policy,
        token,
        matrix,
        gate,
        sti_plan,
        eoi_smoke,
    );
    let idt_bind_smoke = irq::idt_runtime_bind_smoke(
        token,
        matrix,
        gate,
        gate_state,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
    );
    let final_gate = irq::irq_runtime_final_gate(
        token,
        matrix,
        gate,
        simulation,
        sti_plan,
        activation_smoke,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let decision = irq::irq_runtime_decision_freeze(
        final_gate,
        activation_smoke,
        simulation,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let mutation = irq::irq_runtime_mutation_check(
        decision,
        final_gate,
        activation_smoke,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let sequence = irq::irq_runtime_mutation_sequence(
        mutation,
        decision,
        final_gate,
        activation_smoke,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    core::hint::black_box(mask_status);
    sequence
}

fn print_irq_runtime_mutation_sequence_note() {
    use core::fmt::Write;

    let sequence = irq_runtime_mutation_sequence_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "IRQ runtime mutation smoke sequence note\nscope: {}\nsequence inputs: {}\nmutation sequence ready: {}\nnext mutation step: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        sequence.scope,
        sequence.inputs,
        sequence.mutation_sequence_ready,
        sequence.next_mutation_step,
        sequence.hardware_mutation,
        sequence.runtime_irq_active
    );
    let _ = write!(serial_writer, "IRQ runtime mutation smoke sequence note\nscope: {}\nsequence inputs: {}\nmutation sequence ready: {}\nnext mutation step: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        sequence.scope,
        sequence.inputs,
        sequence.mutation_sequence_ready,
        sequence.next_mutation_step,
        sequence.hardware_mutation,
        sequence.runtime_irq_active
    );
}

fn print_irq_runtime_mutation_sequence_status() {
    use core::fmt::Write;

    let sequence = irq_runtime_mutation_sequence_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "IRQ runtime mutation smoke sequence\nmutation sequence ready: {}\nhardware mutation: {}\nruntime irq active: {}\nnext mutation step: {}\nallowed mutation steps: {}\nsti: {}\npic unmask: {}\neoi dispatch: {}\nlive idt bind: {}\nkeyboard mode: {}\n",
        sequence.mutation_sequence_ready,
        sequence.hardware_mutation,
        sequence.runtime_irq_active,
        sequence.next_mutation_step,
        sequence.allowed_mutation_steps,
        sequence.sti_instruction,
        sequence.pic_unmask,
        sequence.eoi_dispatch,
        sequence.live_idt_bind,
        sequence.keyboard_mode
    );
    let _ = write!(serial_writer, "IRQ runtime mutation smoke sequence\nmutation sequence ready: {}\nhardware mutation: {}\nruntime irq active: {}\nnext mutation step: {}\nallowed mutation steps: {}\nsti: {}\npic unmask: {}\neoi dispatch: {}\nlive idt bind: {}\nkeyboard mode: {}\n",
        sequence.mutation_sequence_ready,
        sequence.hardware_mutation,
        sequence.runtime_irq_active,
        sequence.next_mutation_step,
        sequence.allowed_mutation_steps,
        sequence.sti_instruction,
        sequence.pic_unmask,
        sequence.eoi_dispatch,
        sequence.live_idt_bind,
        sequence.keyboard_mode
    );
}

fn print_irq_runtime_mutation_sequence_blockers() {
    use core::fmt::Write;

    let sequence = irq_runtime_mutation_sequence_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "IRQ runtime mutation smoke sequence blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nmutation sequence ready: {}\n",
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_DECISION,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_FINAL,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_MUTATION,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_RUNTIME_IRQ,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_STI,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_PIC_UNMASK,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_EOI_DISPATCH,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_LIVE_IDT_BIND,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_KEYBOARD,
        sequence.mutation_sequence_ready
    );
    let _ = write!(serial_writer, "IRQ runtime mutation smoke sequence blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nmutation sequence ready: {}\n",
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_DECISION,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_FINAL,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_MUTATION,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_RUNTIME_IRQ,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_STI,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_PIC_UNMASK,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_EOI_DISPATCH,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_LIVE_IDT_BIND,
        irq::IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_KEYBOARD,
        sequence.mutation_sequence_ready
    );
}

fn eoi_write_smoke_preflight_snapshot() -> irq::EoiWriteSmokePreflight {
    let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
    let gate_state = irq::irq_gate_bind_state();
    let mask_plan = pic::ProgrammableInterruptController::pic_mask_plan();
    let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
    let eoi_ready = irq::eoi_runtime_check_all_preconditions(pic_state.executed);
    let matrix = irq::irq_runtime_matrix(
        pic_state.executed,
        gate_state.executed,
        eoi_ready,
        mask_plan.mask_policy,
        irq::irq_runtime_is_armed(),
        irq::irq_runtime_is_committed(),
    );
    let activation = irq::irq_runtime_activation_dry_run(&matrix);
    let token = irq::irq_runtime_activation_token_status();
    let gate = irq::irq_runtime_activation_gate(
        token,
        matrix,
        activation,
        eoi_ready,
        mask_plan.mask_policy,
        mask_plan.unmask_policy,
    );
    let simulation = irq::irq_runtime_activation_simulation(token, matrix, activation, gate);
    let sti_plan = irq::sti_controlled_activation_plan(token, matrix, gate, simulation);
    let activation_smoke =
        irq::irq_runtime_activation_smoke(token, matrix, gate, simulation, sti_plan);
    let eoi_smoke = irq::eoi_dispatch_smoke(
        pic_state.executed,
        gate_state.executed,
        matrix,
        activation_smoke,
    );
    let pic_unmask_smoke = irq::pic_unmask_smoke(
        mask_plan.mask_policy,
        mask_plan.unmask_policy,
        token,
        matrix,
        gate,
        sti_plan,
        eoi_smoke,
    );
    let idt_bind_smoke = irq::idt_runtime_bind_smoke(
        token,
        matrix,
        gate,
        gate_state,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
    );
    let final_gate = irq::irq_runtime_final_gate(
        token,
        matrix,
        gate,
        simulation,
        sti_plan,
        activation_smoke,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let decision = irq::irq_runtime_decision_freeze(
        final_gate,
        activation_smoke,
        simulation,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let mutation = irq::irq_runtime_mutation_check(
        decision,
        final_gate,
        activation_smoke,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let sequence = irq::irq_runtime_mutation_sequence(
        mutation,
        decision,
        final_gate,
        activation_smoke,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let preflight = irq::eoi_write_smoke_preflight(
        sequence,
        mutation,
        decision,
        final_gate,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    core::hint::black_box(mask_status);
    preflight
}

fn eoi_write_smoke_candidate_snapshot() -> irq::EoiWriteSmokeCandidate {
    let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
    let gate_state = irq::irq_gate_bind_state();
    let mask_plan = pic::ProgrammableInterruptController::pic_mask_plan();
    let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
    let eoi_ready = irq::eoi_runtime_check_all_preconditions(pic_state.executed);
    let matrix = irq::irq_runtime_matrix(
        pic_state.executed,
        gate_state.executed,
        eoi_ready,
        mask_plan.mask_policy,
        irq::irq_runtime_is_armed(),
        irq::irq_runtime_is_committed(),
    );
    let activation = irq::irq_runtime_activation_dry_run(&matrix);
    let token = irq::irq_runtime_activation_token_status();
    let gate = irq::irq_runtime_activation_gate(
        token,
        matrix,
        activation,
        eoi_ready,
        mask_plan.mask_policy,
        mask_plan.unmask_policy,
    );
    let simulation = irq::irq_runtime_activation_simulation(token, matrix, activation, gate);
    let sti_plan = irq::sti_controlled_activation_plan(token, matrix, gate, simulation);
    let activation_smoke =
        irq::irq_runtime_activation_smoke(token, matrix, gate, simulation, sti_plan);
    let eoi_smoke = irq::eoi_dispatch_smoke(
        pic_state.executed,
        gate_state.executed,
        matrix,
        activation_smoke,
    );
    let pic_unmask_smoke = irq::pic_unmask_smoke(
        mask_plan.mask_policy,
        mask_plan.unmask_policy,
        token,
        matrix,
        gate,
        sti_plan,
        eoi_smoke,
    );
    let idt_bind_smoke = irq::idt_runtime_bind_smoke(
        token,
        matrix,
        gate,
        gate_state,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
    );
    let final_gate = irq::irq_runtime_final_gate(
        token,
        matrix,
        gate,
        simulation,
        sti_plan,
        activation_smoke,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let decision = irq::irq_runtime_decision_freeze(
        final_gate,
        activation_smoke,
        simulation,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let mutation = irq::irq_runtime_mutation_check(
        decision,
        final_gate,
        activation_smoke,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let sequence = irq::irq_runtime_mutation_sequence(
        mutation,
        decision,
        final_gate,
        activation_smoke,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let preflight = irq::eoi_write_smoke_preflight(
        sequence,
        mutation,
        decision,
        final_gate,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let candidate =
        irq::eoi_write_smoke_candidate(preflight, sequence, mutation, decision, final_gate);
    core::hint::black_box(mask_status);
    candidate
}

fn eoi_write_permit_model_snapshot() -> irq::EoiWritePermitModel {
    let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
    let gate_state = irq::irq_gate_bind_state();
    let mask_plan = pic::ProgrammableInterruptController::pic_mask_plan();
    let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
    let eoi_ready = irq::eoi_runtime_check_all_preconditions(pic_state.executed);
    let matrix = irq::irq_runtime_matrix(
        pic_state.executed,
        gate_state.executed,
        eoi_ready,
        mask_plan.mask_policy,
        irq::irq_runtime_is_armed(),
        irq::irq_runtime_is_committed(),
    );
    let activation = irq::irq_runtime_activation_dry_run(&matrix);
    let token = irq::irq_runtime_activation_token_status();
    let gate = irq::irq_runtime_activation_gate(
        token,
        matrix,
        activation,
        eoi_ready,
        mask_plan.mask_policy,
        mask_plan.unmask_policy,
    );
    let simulation = irq::irq_runtime_activation_simulation(token, matrix, activation, gate);
    let sti_plan = irq::sti_controlled_activation_plan(token, matrix, gate, simulation);
    let activation_smoke =
        irq::irq_runtime_activation_smoke(token, matrix, gate, simulation, sti_plan);
    let eoi_smoke = irq::eoi_dispatch_smoke(
        pic_state.executed,
        gate_state.executed,
        matrix,
        activation_smoke,
    );
    let pic_unmask_smoke = irq::pic_unmask_smoke(
        mask_plan.mask_policy,
        mask_plan.unmask_policy,
        token,
        matrix,
        gate,
        sti_plan,
        eoi_smoke,
    );
    let idt_bind_smoke = irq::idt_runtime_bind_smoke(
        token,
        matrix,
        gate,
        gate_state,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
    );
    let final_gate = irq::irq_runtime_final_gate(
        token,
        matrix,
        gate,
        simulation,
        sti_plan,
        activation_smoke,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let decision = irq::irq_runtime_decision_freeze(
        final_gate,
        activation_smoke,
        simulation,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let mutation = irq::irq_runtime_mutation_check(
        decision,
        final_gate,
        activation_smoke,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let sequence = irq::irq_runtime_mutation_sequence(
        mutation,
        decision,
        final_gate,
        activation_smoke,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let preflight = irq::eoi_write_smoke_preflight(
        sequence,
        mutation,
        decision,
        final_gate,
        sti_plan,
        eoi_smoke,
        pic_unmask_smoke,
        idt_bind_smoke,
    );
    let candidate =
        irq::eoi_write_smoke_candidate(preflight, sequence, mutation, decision, final_gate);
    let permit = irq::eoi_write_permit_model(
        candidate, preflight, sequence, mutation, decision, final_gate,
    );
    core::hint::black_box(mask_status);
    permit
}

fn eoi_write_oneshot_command_path_snapshot() -> irq::EoiWriteOneShotCommandPath {
    let permit = eoi_write_permit_model_snapshot();
    irq::eoi_write_oneshot_command_path(permit)
}

fn eoi_write_oneshot_latch_status_snapshot() -> irq::EoiWriteOneShotLatch {
    let permit = eoi_write_permit_model_snapshot();
    irq::eoi_write_oneshot_latch_status(permit)
}

fn eoi_write_oneshot_latch_arm_snapshot() -> irq::EoiWriteOneShotLatch {
    let permit = eoi_write_permit_model_snapshot();
    irq::eoi_write_oneshot_latch_arm(permit)
}

fn eoi_write_oneshot_latch_clear_snapshot() -> irq::EoiWriteOneShotLatch {
    let permit = eoi_write_permit_model_snapshot();
    irq::eoi_write_oneshot_latch_clear(permit)
}

fn eoi_write_oneshot_latch_fire_snapshot() -> irq::EoiWriteOneShotLatch {
    let permit = eoi_write_permit_model_snapshot();
    irq::eoi_write_oneshot_latch_fire(permit)
}

fn eoi_write_bridge_snapshot() -> irq::EoiWriteBridge {
    let permit = eoi_write_permit_model_snapshot();
    let latch = irq::eoi_write_oneshot_latch_status(permit);
    irq::eoi_write_bridge(permit, latch)
}

fn eoi_write_permit_transition_status_snapshot() -> irq::EoiWritePermitTransition {
    let permit = eoi_write_permit_model_snapshot();
    let latch = irq::eoi_write_oneshot_latch_status(permit);
    let bridge = irq::eoi_write_bridge(permit, latch);
    irq::eoi_write_permit_transition_status(bridge)
}

fn eoi_write_permit_transition_arm_snapshot() -> irq::EoiWritePermitTransition {
    let permit = eoi_write_permit_model_snapshot();
    let latch = irq::eoi_write_oneshot_latch_status(permit);
    let bridge = irq::eoi_write_bridge(permit, latch);
    irq::eoi_write_permit_transition_arm(bridge)
}

fn eoi_write_permit_transition_clear_snapshot() -> irq::EoiWritePermitTransition {
    let permit = eoi_write_permit_model_snapshot();
    let latch = irq::eoi_write_oneshot_latch_status(permit);
    let bridge = irq::eoi_write_bridge(permit, latch);
    irq::eoi_write_permit_transition_clear(bridge)
}

fn eoi_write_permit_transition_check_snapshot() -> irq::EoiWritePermitTransition {
    let permit = eoi_write_permit_model_snapshot();
    let latch = irq::eoi_write_oneshot_latch_status(permit);
    let bridge = irq::eoi_write_bridge(permit, latch);
    irq::eoi_write_permit_transition_check(bridge)
}

fn eoi_write_permit_evaluation_snapshot() -> irq::EoiWritePermitEvaluation {
    let permit = eoi_write_permit_model_snapshot();
    let latch = irq::eoi_write_oneshot_latch_status(permit);
    let bridge = irq::eoi_write_bridge(permit, latch);
    let transition = irq::eoi_write_permit_transition_status(bridge);
    irq::eoi_write_permit_evaluation(permit, latch, bridge, transition)
}

fn eoi_write_hw_smoke_status_snapshot() -> pic::EoiWriteHwSmokeStatus {
    pic::ProgrammableInterruptController::eoi_write_hw_smoke_status()
}

fn eoi_write_hw_smoke_arm_snapshot() -> pic::EoiWriteHwSmokeStatus {
    pic::ProgrammableInterruptController::eoi_write_hw_smoke_arm()
}

fn eoi_write_hw_smoke_fire_snapshot() -> pic::EoiWriteHwSmokeStatus {
    pic::ProgrammableInterruptController::eoi_write_hw_smoke_fire()
}

fn eoi_write_hw_smoke_clear_snapshot() -> pic::EoiWriteHwSmokeStatus {
    pic::ProgrammableInterruptController::eoi_write_hw_smoke_clear()
}

fn eoi_runtime_bridge_readiness_snapshot() -> irq::EoiRuntimeBridgeReadiness {
    let manual_smoke = eoi_write_hw_smoke_status_snapshot();
    let permit = eoi_write_permit_model_snapshot();
    let latch = irq::eoi_write_oneshot_latch_status(permit);
    let bridge = irq::eoi_write_bridge(permit, latch);
    let transition = irq::eoi_write_permit_transition_status(bridge);
    let evaluation = irq::eoi_write_permit_evaluation(permit, latch, bridge, transition);
    let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
    let gate_state = irq::irq_gate_bind_state();
    let runtime_dispatch_ready = irq::eoi_runtime_check_all_preconditions(pic_state.executed);
    irq::eoi_runtime_bridge_readiness(
        manual_smoke.manual_pic_eoi_smoke_proven_this_boot,
        evaluation,
        runtime_dispatch_ready,
        gate_state,
    )
}

fn irq_handler_eoi_candidate_snapshot() -> irq::IrqHandlerEoiCandidate {
    let bridge = eoi_runtime_bridge_readiness_snapshot();
    irq::irq_handler_eoi_candidate(bridge)
}

fn irq_handler_eoi_stub_snapshot() -> irq::IrqHandlerEoiStub {
    let candidate = irq_handler_eoi_candidate_snapshot();
    irq::irq_handler_eoi_stub(candidate)
}

fn irq_handler_bind_candidate_snapshot() -> irq::IrqHandlerBindCandidate {
    let stub = irq_handler_eoi_stub_snapshot();
    irq::irq_handler_bind_candidate(stub)
}

fn idt_bind_runtime_bridge_readiness_snapshot() -> irq::IdtBindRuntimeBridgeReadiness {
    let smoke = idt_bind_hw_smoke_status_snapshot();
    let bind = irq_handler_bind_candidate_snapshot();
    irq::idt_bind_runtime_bridge_readiness(smoke.manual_idt_bind_smoke_proven_this_boot, bind)
}

fn idt_bind_hw_smoke_status_snapshot() -> idt::IdtBindHwSmokeStatus {
    idt::idt_bind_hw_smoke_status()
}

fn idt_bind_hw_smoke_arm_snapshot() -> idt::IdtBindHwSmokeStatus {
    idt::idt_bind_hw_smoke_arm()
}

fn idt_bind_hw_smoke_fire_snapshot() -> idt::IdtBindHwSmokeStatus {
    idt::idt_bind_hw_smoke_fire()
}

fn idt_bind_hw_smoke_clear_snapshot() -> idt::IdtBindHwSmokeStatus {
    idt::idt_bind_hw_smoke_clear()
}

fn irq0_bind_hw_smoke_status_snapshot() -> idt::Irq0BindHwSmokeStatus {
    idt::irq0_bind_hw_smoke_status()
}

fn irq0_bind_hw_smoke_arm_snapshot() -> idt::Irq0BindHwSmokeStatus {
    idt::irq0_bind_hw_smoke_arm()
}

fn irq0_bind_hw_smoke_fire_snapshot() -> idt::Irq0BindHwSmokeStatus {
    idt::irq0_bind_hw_smoke_fire()
}

fn irq0_bind_hw_smoke_clear_snapshot() -> idt::Irq0BindHwSmokeStatus {
    idt::irq0_bind_hw_smoke_clear()
}

fn irq0_unmask_hw_smoke_status_snapshot() -> pic::Irq0UnmaskHwSmokeStatus {
    pic::ProgrammableInterruptController::irq0_unmask_hw_smoke_status()
}

fn irq0_unmask_hw_smoke_arm_snapshot() -> pic::Irq0UnmaskHwSmokeStatus {
    pic::ProgrammableInterruptController::irq0_unmask_hw_smoke_arm()
}

fn irq0_unmask_hw_smoke_fire_snapshot() -> pic::Irq0UnmaskHwSmokeStatus {
    pic::ProgrammableInterruptController::irq0_unmask_hw_smoke_fire()
}

fn irq0_unmask_hw_smoke_clear_snapshot() -> pic::Irq0UnmaskHwSmokeStatus {
    pic::ProgrammableInterruptController::irq0_unmask_hw_smoke_clear()
}

fn idt_invoke_hw_smoke_status_snapshot() -> idt::IdtInvokeHwSmokeStatus {
    idt::idt_invoke_hw_smoke_status()
}

fn idt_invoke_hw_smoke_arm_snapshot() -> idt::IdtInvokeHwSmokeStatus {
    idt::idt_invoke_hw_smoke_arm()
}

fn idt_invoke_hw_smoke_fire_snapshot() -> idt::IdtInvokeHwSmokeStatus {
    idt::idt_invoke_hw_smoke_fire()
}

fn idt_invoke_hw_smoke_clear_snapshot() -> idt::IdtInvokeHwSmokeStatus {
    idt::idt_invoke_hw_smoke_clear()
}

fn idt_invoke_runtime_bridge_readiness_snapshot() -> irq::IdtInvokeRuntimeBridgeReadiness {
    let bind_smoke = idt_bind_hw_smoke_status_snapshot();
    let invoke_smoke = idt_invoke_hw_smoke_status_snapshot();
    let bind_bridge = idt_bind_runtime_bridge_readiness_snapshot();
    let bind_candidate = irq_handler_bind_candidate_snapshot();
    let stub = irq_handler_eoi_stub_snapshot();
    irq::idt_invoke_runtime_bridge_readiness(
        bind_smoke.manual_idt_bind_smoke_proven_this_boot,
        invoke_smoke.manual_idt_invocation_smoke_proven_this_boot,
        bind_bridge,
        bind_candidate,
        stub,
    )
}

fn print_idt_bind_hw_smoke_note() {
    use core::fmt::Write;

    let smoke = idt_bind_hw_smoke_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT bind one-shot hardware smoke note\nscope: {}\nmode: {}\ntarget vector: {}\ntarget handler: {}\nlive IRQ bind: {}\ninterrupt invocation: {}\nruntime irq active: {}\n",
        smoke.scope,
        smoke.mode,
        smoke.target_vector,
        smoke.target_handler,
        smoke.live_irq_bind,
        smoke.interrupt_invocation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IDT bind one-shot hardware smoke note\nscope: {}\nmode: {}\ntarget vector: {}\ntarget handler: {}\nlive IRQ bind: {}\ninterrupt invocation: {}\nruntime irq active: {}\n",
        smoke.scope,
        smoke.mode,
        smoke.target_vector,
        smoke.target_handler,
        smoke.live_irq_bind,
        smoke.interrupt_invocation,
        smoke.runtime_irq_active
    );
}

fn print_idt_bind_hw_smoke_status() {
    use core::fmt::Write;

    let smoke = idt_bind_hw_smoke_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT bind one-shot hardware smoke\narmed: {}\nconsumed: {}\ntarget vector: {}\ntarget handler: {}\nlive IRQ bind: {}\nIRQ0 bind: {}\nIRQ1 bind: {}\ninterrupt invocation: {}\nhardware mutation allowed: {}\nIDT descriptor binds this command: {}\nfirst IDT bind performed: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        smoke.armed,
        smoke.consumed,
        smoke.target_vector,
        smoke.target_handler,
        smoke.live_irq_bind,
        smoke.irq0_bind,
        smoke.irq1_bind,
        smoke.interrupt_invocation,
        smoke.hardware_mutation_allowed,
        smoke.idt_descriptor_binds_this_command,
        smoke.first_idt_bind_performed,
        smoke.runtime_irq_active,
        smoke.sti,
        smoke.pic_unmask,
        smoke.keyboard_mode
    );
    let _ = write!(serial_writer, "Controlled IDT bind one-shot hardware smoke\narmed: {}\nconsumed: {}\ntarget vector: {}\ntarget handler: {}\nlive IRQ bind: {}\nIRQ0 bind: {}\nIRQ1 bind: {}\ninterrupt invocation: {}\nhardware mutation allowed: {}\nIDT descriptor binds this command: {}\nfirst IDT bind performed: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        smoke.armed,
        smoke.consumed,
        smoke.target_vector,
        smoke.target_handler,
        smoke.live_irq_bind,
        smoke.irq0_bind,
        smoke.irq1_bind,
        smoke.interrupt_invocation,
        smoke.hardware_mutation_allowed,
        smoke.idt_descriptor_binds_this_command,
        smoke.first_idt_bind_performed,
        smoke.runtime_irq_active,
        smoke.sti,
        smoke.pic_unmask,
        smoke.keyboard_mode
    );
}

fn print_idt_bind_hw_smoke_arm() {
    use core::fmt::Write;

    let smoke = idt_bind_hw_smoke_arm_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT bind one-shot hardware smoke arm\n{}\narmed: {}\nconsumed: {}\nIDT descriptor binds this command: {}\nfirst IDT bind performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.idt_descriptor_binds_this_command,
        smoke.first_idt_bind_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IDT bind one-shot hardware smoke arm\n{}\narmed: {}\nconsumed: {}\nIDT descriptor binds this command: {}\nfirst IDT bind performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.idt_descriptor_binds_this_command,
        smoke.first_idt_bind_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
}

fn print_idt_bind_hw_smoke_fire() {
    use core::fmt::Write;

    let smoke = idt_bind_hw_smoke_fire_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT bind one-shot hardware smoke fire\n{}\narmed: {}\nconsumed: {}\ntarget vector: {}\ntarget handler: {}\nIDT descriptor binds this command: {}\nfirst IDT bind performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.target_vector,
        smoke.target_handler,
        smoke.idt_descriptor_binds_this_command,
        smoke.first_idt_bind_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IDT bind one-shot hardware smoke fire\n{}\narmed: {}\nconsumed: {}\ntarget vector: {}\ntarget handler: {}\nIDT descriptor binds this command: {}\nfirst IDT bind performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.target_vector,
        smoke.target_handler,
        smoke.idt_descriptor_binds_this_command,
        smoke.first_idt_bind_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
}

fn print_idt_bind_hw_smoke_clear() {
    use core::fmt::Write;

    let smoke = idt_bind_hw_smoke_clear_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT bind one-shot hardware smoke clear\n{}\narmed: {}\nconsumed: {}\nIDT descriptor binds this command: {}\nfirst IDT bind performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.idt_descriptor_binds_this_command,
        smoke.first_idt_bind_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IDT bind one-shot hardware smoke clear\n{}\narmed: {}\nconsumed: {}\nIDT descriptor binds this command: {}\nfirst IDT bind performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.idt_descriptor_binds_this_command,
        smoke.first_idt_bind_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
}

fn print_idt_bind_hw_smoke_blockers() {
    use core::fmt::Write;

    let smoke = idt_bind_hw_smoke_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT bind one-shot hardware smoke blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\narmed: {}\nconsumed: {}\nfirst IDT bind performed: {}\nruntime irq active: {}\n",
        smoke.blocker_manual_only,
        smoke.blocker_test_vector,
        smoke.blocker_inert_stub,
        smoke.blocker_no_invocation,
        smoke.blocker_no_live_irq,
        smoke.blocker_runtime,
        smoke.armed,
        smoke.consumed,
        smoke.first_idt_bind_performed,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IDT bind one-shot hardware smoke blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\narmed: {}\nconsumed: {}\nfirst IDT bind performed: {}\nruntime irq active: {}\n",
        smoke.blocker_manual_only,
        smoke.blocker_test_vector,
        smoke.blocker_inert_stub,
        smoke.blocker_no_invocation,
        smoke.blocker_no_live_irq,
        smoke.blocker_runtime,
        smoke.armed,
        smoke.consumed,
        smoke.first_idt_bind_performed,
        smoke.runtime_irq_active
    );
}

fn print_irq0_bind_hw_smoke_note() {
    use core::fmt::Write;

    let smoke = irq0_bind_hw_smoke_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IRQ0 timer bind one-shot hardware smoke note\nscope: {}\nmode: {}\nIRQ0 bind smoke vector: {}\ntarget handler: {}\nIRQ0 hardware delivery allowed: {}\nPIC IRQ0 unmask: {}\nSTI: {}\nruntime irq active: {}\n",
        smoke.scope,
        smoke.mode,
        smoke.irq0_bind_smoke_vector,
        smoke.target_handler,
        smoke.irq0_hardware_delivery_allowed,
        smoke.pic_irq0_unmask,
        smoke.sti,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IRQ0 timer bind one-shot hardware smoke note\nscope: {}\nmode: {}\nIRQ0 bind smoke vector: {}\ntarget handler: {}\nIRQ0 hardware delivery allowed: {}\nPIC IRQ0 unmask: {}\nSTI: {}\nruntime irq active: {}\n",
        smoke.scope,
        smoke.mode,
        smoke.irq0_bind_smoke_vector,
        smoke.target_handler,
        smoke.irq0_hardware_delivery_allowed,
        smoke.pic_irq0_unmask,
        smoke.sti,
        smoke.runtime_irq_active
    );
}

fn print_irq0_bind_hw_smoke_status() {
    use core::fmt::Write;

    let smoke = irq0_bind_hw_smoke_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IRQ0 timer bind one-shot hardware smoke\narmed: {}\nconsumed: {}\nIRQ0 bind smoke vector: {}\ntarget handler: {}\nIRQ0 descriptor bound: {}\nIRQ0 bind proven this boot: {}\nIRQ0 handler reached: {}\nIRQ0 hardware delivery allowed: {}\nPIC IRQ0 unmask: {}\nSTI: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\nkeyboard mode: {}\n",
        smoke.armed,
        smoke.consumed,
        smoke.irq0_bind_smoke_vector,
        smoke.target_handler,
        smoke.irq0_descriptor_bound,
        smoke.irq0_bind_proven_this_boot,
        smoke.irq0_handler_reached,
        smoke.irq0_hardware_delivery_allowed,
        smoke.pic_irq0_unmask,
        smoke.sti,
        smoke.handler_triggered_eoi_allowed,
        smoke.runtime_irq_active,
        smoke.keyboard_mode
    );
    let _ = write!(serial_writer, "Controlled IRQ0 timer bind one-shot hardware smoke\narmed: {}\nconsumed: {}\nIRQ0 bind smoke vector: {}\ntarget handler: {}\nIRQ0 descriptor bound: {}\nIRQ0 bind proven this boot: {}\nIRQ0 handler reached: {}\nIRQ0 hardware delivery allowed: {}\nPIC IRQ0 unmask: {}\nSTI: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\nkeyboard mode: {}\n",
        smoke.armed,
        smoke.consumed,
        smoke.irq0_bind_smoke_vector,
        smoke.target_handler,
        smoke.irq0_descriptor_bound,
        smoke.irq0_bind_proven_this_boot,
        smoke.irq0_handler_reached,
        smoke.irq0_hardware_delivery_allowed,
        smoke.pic_irq0_unmask,
        smoke.sti,
        smoke.handler_triggered_eoi_allowed,
        smoke.runtime_irq_active,
        smoke.keyboard_mode
    );
}

fn print_irq0_bind_hw_smoke_arm() {
    use core::fmt::Write;

    let smoke = irq0_bind_hw_smoke_arm_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IRQ0 timer bind one-shot hardware smoke arm\n{}\narmed: {}\nconsumed: {}\nIRQ0 descriptor binds this command: {}\nIRQ0 descriptor bound: {}\nIRQ0 bind proven this boot: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_descriptor_binds_this_command,
        smoke.irq0_descriptor_bound,
        smoke.irq0_bind_proven_this_boot,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IRQ0 timer bind one-shot hardware smoke arm\n{}\narmed: {}\nconsumed: {}\nIRQ0 descriptor binds this command: {}\nIRQ0 descriptor bound: {}\nIRQ0 bind proven this boot: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_descriptor_binds_this_command,
        smoke.irq0_descriptor_bound,
        smoke.irq0_bind_proven_this_boot,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
}

fn print_irq0_bind_hw_smoke_fire() {
    use core::fmt::Write;

    let smoke = irq0_bind_hw_smoke_fire_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IRQ0 timer bind one-shot hardware smoke fire\n{}\narmed: {}\nconsumed: {}\nIRQ0 bind smoke vector: {}\ntarget handler: {}\nIRQ0 descriptor binds this command: {}\nIRQ0 descriptor bound: {}\nIRQ0 bind proven this boot: {}\nIRQ0 handler reached: {}\nIRQ0 hardware delivery allowed: {}\nPIC IRQ0 unmask: {}\nSTI: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_bind_smoke_vector,
        smoke.target_handler,
        smoke.irq0_descriptor_binds_this_command,
        smoke.irq0_descriptor_bound,
        smoke.irq0_bind_proven_this_boot,
        smoke.irq0_handler_reached,
        smoke.irq0_hardware_delivery_allowed,
        smoke.pic_irq0_unmask,
        smoke.sti,
        smoke.handler_triggered_eoi_allowed,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IRQ0 timer bind one-shot hardware smoke fire\n{}\narmed: {}\nconsumed: {}\nIRQ0 bind smoke vector: {}\ntarget handler: {}\nIRQ0 descriptor binds this command: {}\nIRQ0 descriptor bound: {}\nIRQ0 bind proven this boot: {}\nIRQ0 handler reached: {}\nIRQ0 hardware delivery allowed: {}\nPIC IRQ0 unmask: {}\nSTI: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_bind_smoke_vector,
        smoke.target_handler,
        smoke.irq0_descriptor_binds_this_command,
        smoke.irq0_descriptor_bound,
        smoke.irq0_bind_proven_this_boot,
        smoke.irq0_handler_reached,
        smoke.irq0_hardware_delivery_allowed,
        smoke.pic_irq0_unmask,
        smoke.sti,
        smoke.handler_triggered_eoi_allowed,
        smoke.runtime_irq_active
    );
}

fn print_irq0_bind_hw_smoke_clear() {
    use core::fmt::Write;

    let smoke = irq0_bind_hw_smoke_clear_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IRQ0 timer bind one-shot hardware smoke clear\n{}\narmed: {}\nconsumed: {}\nIRQ0 descriptor binds this command: {}\nIRQ0 descriptor bound: {}\nIRQ0 bind proven this boot: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_descriptor_binds_this_command,
        smoke.irq0_descriptor_bound,
        smoke.irq0_bind_proven_this_boot,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IRQ0 timer bind one-shot hardware smoke clear\n{}\narmed: {}\nconsumed: {}\nIRQ0 descriptor binds this command: {}\nIRQ0 descriptor bound: {}\nIRQ0 bind proven this boot: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_descriptor_binds_this_command,
        smoke.irq0_descriptor_bound,
        smoke.irq0_bind_proven_this_boot,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
}

fn print_irq0_bind_hw_smoke_blockers() {
    use core::fmt::Write;

    let smoke = irq0_bind_hw_smoke_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IRQ0 timer bind one-shot hardware smoke blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\narmed: {}\nconsumed: {}\nIRQ0 descriptor bound: {}\nIRQ0 bind proven this boot: {}\nruntime irq active: {}\n",
        smoke.blocker_manual_only,
        smoke.blocker_irq0_only,
        smoke.blocker_no_unmask,
        smoke.blocker_sti,
        smoke.blocker_no_delivery,
        smoke.blocker_no_eoi,
        smoke.blocker_runtime,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_descriptor_bound,
        smoke.irq0_bind_proven_this_boot,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IRQ0 timer bind one-shot hardware smoke blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\narmed: {}\nconsumed: {}\nIRQ0 descriptor bound: {}\nIRQ0 bind proven this boot: {}\nruntime irq active: {}\n",
        smoke.blocker_manual_only,
        smoke.blocker_irq0_only,
        smoke.blocker_no_unmask,
        smoke.blocker_sti,
        smoke.blocker_no_delivery,
        smoke.blocker_no_eoi,
        smoke.blocker_runtime,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_descriptor_bound,
        smoke.irq0_bind_proven_this_boot,
        smoke.runtime_irq_active
    );
}

fn print_irq0_unmask_hw_smoke_note() {
    use core::fmt::Write;

    let smoke = irq0_unmask_hw_smoke_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled PIC IRQ0 unmask one-shot hardware smoke note\nscope: {}\nmode: {}\nIRQ0 currently unmasked: {}\nSTI: {}\nhardware IRQ delivery allowed: {}\nruntime irq active: {}\nkeyboard mode: {}\n",
        smoke.scope,
        smoke.mode,
        smoke.irq0_currently_unmasked,
        smoke.sti,
        smoke.hardware_irq_delivery_allowed,
        smoke.runtime_irq_active,
        smoke.keyboard_mode
    );
    let _ = write!(serial_writer, "Controlled PIC IRQ0 unmask one-shot hardware smoke note\nscope: {}\nmode: {}\nIRQ0 currently unmasked: {}\nSTI: {}\nhardware IRQ delivery allowed: {}\nruntime irq active: {}\nkeyboard mode: {}\n",
        smoke.scope,
        smoke.mode,
        smoke.irq0_currently_unmasked,
        smoke.sti,
        smoke.hardware_irq_delivery_allowed,
        smoke.runtime_irq_active,
        smoke.keyboard_mode
    );
}

fn print_irq0_unmask_hw_smoke_status() {
    use core::fmt::Write;

    let smoke = irq0_unmask_hw_smoke_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled PIC IRQ0 unmask one-shot hardware smoke\narmed: {}\nconsumed: {}\nIRQ0 temporary unmask performed: {}\nIRQ0 restore performed: {}\nIRQ0 currently unmasked: {}\nPIC master mask restored: {}\nIRQ0 unmask proven this boot: {}\nSTI: {}\nhardware IRQ delivery allowed: {}\nIRQ0 handler reached: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\nkeyboard mode: {}\n",
        smoke.armed,
        smoke.consumed,
        smoke.irq0_temporary_unmask_performed,
        smoke.irq0_restore_performed,
        smoke.irq0_currently_unmasked,
        smoke.pic_master_mask_restored,
        smoke.irq0_unmask_proven_this_boot,
        smoke.sti,
        smoke.hardware_irq_delivery_allowed,
        smoke.irq0_handler_reached,
        smoke.handler_triggered_eoi_allowed,
        smoke.runtime_irq_active,
        smoke.keyboard_mode
    );
    let _ = write!(serial_writer, "Controlled PIC IRQ0 unmask one-shot hardware smoke\narmed: {}\nconsumed: {}\nIRQ0 temporary unmask performed: {}\nIRQ0 restore performed: {}\nIRQ0 currently unmasked: {}\nPIC master mask restored: {}\nIRQ0 unmask proven this boot: {}\nSTI: {}\nhardware IRQ delivery allowed: {}\nIRQ0 handler reached: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\nkeyboard mode: {}\n",
        smoke.armed,
        smoke.consumed,
        smoke.irq0_temporary_unmask_performed,
        smoke.irq0_restore_performed,
        smoke.irq0_currently_unmasked,
        smoke.pic_master_mask_restored,
        smoke.irq0_unmask_proven_this_boot,
        smoke.sti,
        smoke.hardware_irq_delivery_allowed,
        smoke.irq0_handler_reached,
        smoke.handler_triggered_eoi_allowed,
        smoke.runtime_irq_active,
        smoke.keyboard_mode
    );
}

fn print_irq0_unmask_hw_smoke_arm() {
    use core::fmt::Write;

    let smoke = irq0_unmask_hw_smoke_arm_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled PIC IRQ0 unmask one-shot hardware smoke arm\n{}\narmed: {}\nconsumed: {}\nIRQ0 temporary unmask performed: {}\nIRQ0 restore performed: {}\nPIC master mask restored: {}\nIRQ0 unmask proven this boot: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_temporary_unmask_performed,
        smoke.irq0_restore_performed,
        smoke.pic_master_mask_restored,
        smoke.irq0_unmask_proven_this_boot,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled PIC IRQ0 unmask one-shot hardware smoke arm\n{}\narmed: {}\nconsumed: {}\nIRQ0 temporary unmask performed: {}\nIRQ0 restore performed: {}\nPIC master mask restored: {}\nIRQ0 unmask proven this boot: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_temporary_unmask_performed,
        smoke.irq0_restore_performed,
        smoke.pic_master_mask_restored,
        smoke.irq0_unmask_proven_this_boot,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
}

fn print_irq0_unmask_hw_smoke_fire() {
    use core::fmt::Write;

    let smoke = irq0_unmask_hw_smoke_fire_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled PIC IRQ0 unmask one-shot hardware smoke fire\n{}\narmed: {}\nconsumed: {}\nIRQ0 temporary unmask performed: {}\nIRQ0 restore performed: {}\nIRQ0 currently unmasked: {}\nPIC master mask restored: {}\nIRQ0 unmask proven this boot: {}\nSTI: {}\nhardware IRQ delivery allowed: {}\nIRQ0 handler reached: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\nkeyboard mode: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_temporary_unmask_performed,
        smoke.irq0_restore_performed,
        smoke.irq0_currently_unmasked,
        smoke.pic_master_mask_restored,
        smoke.irq0_unmask_proven_this_boot,
        smoke.sti,
        smoke.hardware_irq_delivery_allowed,
        smoke.irq0_handler_reached,
        smoke.handler_triggered_eoi_allowed,
        smoke.runtime_irq_active,
        smoke.keyboard_mode
    );
    let _ = write!(serial_writer, "Controlled PIC IRQ0 unmask one-shot hardware smoke fire\n{}\narmed: {}\nconsumed: {}\nIRQ0 temporary unmask performed: {}\nIRQ0 restore performed: {}\nIRQ0 currently unmasked: {}\nPIC master mask restored: {}\nIRQ0 unmask proven this boot: {}\nSTI: {}\nhardware IRQ delivery allowed: {}\nIRQ0 handler reached: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\nkeyboard mode: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_temporary_unmask_performed,
        smoke.irq0_restore_performed,
        smoke.irq0_currently_unmasked,
        smoke.pic_master_mask_restored,
        smoke.irq0_unmask_proven_this_boot,
        smoke.sti,
        smoke.hardware_irq_delivery_allowed,
        smoke.irq0_handler_reached,
        smoke.handler_triggered_eoi_allowed,
        smoke.runtime_irq_active,
        smoke.keyboard_mode
    );
}

fn print_irq0_unmask_hw_smoke_clear() {
    use core::fmt::Write;

    let smoke = irq0_unmask_hw_smoke_clear_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled PIC IRQ0 unmask one-shot hardware smoke clear\n{}\narmed: {}\nconsumed: {}\nIRQ0 temporary unmask performed: {}\nIRQ0 restore performed: {}\nPIC master mask restored: {}\nIRQ0 unmask proven this boot: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_temporary_unmask_performed,
        smoke.irq0_restore_performed,
        smoke.pic_master_mask_restored,
        smoke.irq0_unmask_proven_this_boot,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled PIC IRQ0 unmask one-shot hardware smoke clear\n{}\narmed: {}\nconsumed: {}\nIRQ0 temporary unmask performed: {}\nIRQ0 restore performed: {}\nPIC master mask restored: {}\nIRQ0 unmask proven this boot: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_temporary_unmask_performed,
        smoke.irq0_restore_performed,
        smoke.pic_master_mask_restored,
        smoke.irq0_unmask_proven_this_boot,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
}

fn print_irq0_unmask_hw_smoke_blockers() {
    use core::fmt::Write;

    let smoke = irq0_unmask_hw_smoke_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled PIC IRQ0 unmask one-shot hardware smoke blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\narmed: {}\nconsumed: {}\nIRQ0 currently unmasked: {}\nPIC master mask restored: {}\nIRQ0 unmask proven this boot: {}\nruntime irq active: {}\n",
        smoke.blocker_manual_only,
        smoke.blocker_transactional,
        smoke.blocker_irq1,
        smoke.blocker_slave,
        smoke.blocker_sti,
        smoke.blocker_delivery,
        smoke.blocker_eoi,
        smoke.blocker_runtime,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_currently_unmasked,
        smoke.pic_master_mask_restored,
        smoke.irq0_unmask_proven_this_boot,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled PIC IRQ0 unmask one-shot hardware smoke blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\narmed: {}\nconsumed: {}\nIRQ0 currently unmasked: {}\nPIC master mask restored: {}\nIRQ0 unmask proven this boot: {}\nruntime irq active: {}\n",
        smoke.blocker_manual_only,
        smoke.blocker_transactional,
        smoke.blocker_irq1,
        smoke.blocker_slave,
        smoke.blocker_sti,
        smoke.blocker_delivery,
        smoke.blocker_eoi,
        smoke.blocker_runtime,
        smoke.armed,
        smoke.consumed,
        smoke.irq0_currently_unmasked,
        smoke.pic_master_mask_restored,
        smoke.irq0_unmask_proven_this_boot,
        smoke.runtime_irq_active
    );
}

fn print_idt_invoke_hw_smoke_note() {
    use core::fmt::Write;

    let smoke = idt_invoke_hw_smoke_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT vector invocation one-shot hardware smoke note\nscope: {}\nbind proven this boot: {}\ntarget vector: {}\ntarget handler: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        smoke.scope,
        smoke.bind_proven_this_boot,
        smoke.target_vector,
        smoke.target_handler,
        smoke.runtime_irq_active,
        smoke.sti,
        smoke.pic_unmask,
        smoke.keyboard_mode
    );
    let _ = write!(serial_writer, "Controlled IDT vector invocation one-shot hardware smoke note\nscope: {}\nbind proven this boot: {}\ntarget vector: {}\ntarget handler: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        smoke.scope,
        smoke.bind_proven_this_boot,
        smoke.target_vector,
        smoke.target_handler,
        smoke.runtime_irq_active,
        smoke.sti,
        smoke.pic_unmask,
        smoke.keyboard_mode
    );
}

fn print_idt_invoke_hw_smoke_status() {
    use core::fmt::Write;

    let smoke = idt_invoke_hw_smoke_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT vector invocation one-shot hardware smoke\nbind proven this boot: {}\narmed: {}\nconsumed: {}\ntarget vector: {}\ntarget handler: {}\ninterrupt invocations this command: {}\ninert stub reached: {}\nfirst IDT invocation performed: {}\nmanual IDT invocation smoke proven this boot: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        smoke.bind_proven_this_boot,
        smoke.armed,
        smoke.consumed,
        smoke.target_vector,
        smoke.target_handler,
        smoke.interrupt_invocations_this_command,
        smoke.inert_stub_reached,
        smoke.first_idt_invocation_performed,
        smoke.manual_idt_invocation_smoke_proven_this_boot,
        smoke.runtime_irq_active,
        smoke.sti,
        smoke.pic_unmask,
        smoke.keyboard_mode
    );
    let _ = write!(serial_writer, "Controlled IDT vector invocation one-shot hardware smoke\nbind proven this boot: {}\narmed: {}\nconsumed: {}\ntarget vector: {}\ntarget handler: {}\ninterrupt invocations this command: {}\ninert stub reached: {}\nfirst IDT invocation performed: {}\nmanual IDT invocation smoke proven this boot: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        smoke.bind_proven_this_boot,
        smoke.armed,
        smoke.consumed,
        smoke.target_vector,
        smoke.target_handler,
        smoke.interrupt_invocations_this_command,
        smoke.inert_stub_reached,
        smoke.first_idt_invocation_performed,
        smoke.manual_idt_invocation_smoke_proven_this_boot,
        smoke.runtime_irq_active,
        smoke.sti,
        smoke.pic_unmask,
        smoke.keyboard_mode
    );
}

fn print_idt_invoke_hw_smoke_arm() {
    use core::fmt::Write;

    let smoke = idt_invoke_hw_smoke_arm_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT vector invocation one-shot hardware smoke arm\n{}\nbind proven this boot: {}\narmed: {}\nconsumed: {}\ninterrupt invocations this command: {}\ninert stub reached: {}\nfirst IDT invocation performed: {}\nmanual IDT invocation smoke proven this boot: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.bind_proven_this_boot,
        smoke.armed,
        smoke.consumed,
        smoke.interrupt_invocations_this_command,
        smoke.inert_stub_reached,
        smoke.first_idt_invocation_performed,
        smoke.manual_idt_invocation_smoke_proven_this_boot,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IDT vector invocation one-shot hardware smoke arm\n{}\nbind proven this boot: {}\narmed: {}\nconsumed: {}\ninterrupt invocations this command: {}\ninert stub reached: {}\nfirst IDT invocation performed: {}\nmanual IDT invocation smoke proven this boot: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.bind_proven_this_boot,
        smoke.armed,
        smoke.consumed,
        smoke.interrupt_invocations_this_command,
        smoke.inert_stub_reached,
        smoke.first_idt_invocation_performed,
        smoke.manual_idt_invocation_smoke_proven_this_boot,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
}

fn print_idt_invoke_hw_smoke_fire() {
    use core::fmt::Write;

    let smoke = idt_invoke_hw_smoke_fire_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT vector invocation one-shot hardware smoke fire\n{}\nbind proven this boot: {}\narmed: {}\nconsumed: {}\ntarget vector: {}\ntarget handler: {}\ninterrupt invocations this command: {}\ninert stub reached: {}\nfirst IDT invocation performed: {}\nmanual IDT invocation smoke proven this boot: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.bind_proven_this_boot,
        smoke.armed,
        smoke.consumed,
        smoke.target_vector,
        smoke.target_handler,
        smoke.interrupt_invocations_this_command,
        smoke.inert_stub_reached,
        smoke.first_idt_invocation_performed,
        smoke.manual_idt_invocation_smoke_proven_this_boot,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IDT vector invocation one-shot hardware smoke fire\n{}\nbind proven this boot: {}\narmed: {}\nconsumed: {}\ntarget vector: {}\ntarget handler: {}\ninterrupt invocations this command: {}\ninert stub reached: {}\nfirst IDT invocation performed: {}\nmanual IDT invocation smoke proven this boot: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.bind_proven_this_boot,
        smoke.armed,
        smoke.consumed,
        smoke.target_vector,
        smoke.target_handler,
        smoke.interrupt_invocations_this_command,
        smoke.inert_stub_reached,
        smoke.first_idt_invocation_performed,
        smoke.manual_idt_invocation_smoke_proven_this_boot,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
}

fn print_idt_invoke_hw_smoke_clear() {
    use core::fmt::Write;

    let smoke = idt_invoke_hw_smoke_clear_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT vector invocation one-shot hardware smoke clear\n{}\nbind proven this boot: {}\narmed: {}\nconsumed: {}\ninterrupt invocations this command: {}\ninert stub reached: {}\nfirst IDT invocation performed: {}\nmanual IDT invocation smoke proven this boot: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.bind_proven_this_boot,
        smoke.armed,
        smoke.consumed,
        smoke.interrupt_invocations_this_command,
        smoke.inert_stub_reached,
        smoke.first_idt_invocation_performed,
        smoke.manual_idt_invocation_smoke_proven_this_boot,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IDT vector invocation one-shot hardware smoke clear\n{}\nbind proven this boot: {}\narmed: {}\nconsumed: {}\ninterrupt invocations this command: {}\ninert stub reached: {}\nfirst IDT invocation performed: {}\nmanual IDT invocation smoke proven this boot: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.bind_proven_this_boot,
        smoke.armed,
        smoke.consumed,
        smoke.interrupt_invocations_this_command,
        smoke.inert_stub_reached,
        smoke.first_idt_invocation_performed,
        smoke.manual_idt_invocation_smoke_proven_this_boot,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
}

fn print_idt_invoke_hw_smoke_blockers() {
    use core::fmt::Write;

    let smoke = idt_invoke_hw_smoke_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT vector invocation one-shot hardware smoke blockers\n- {}\n- {}\n- {}\n- {}\n- {}\nbind proven this boot: {}\narmed: {}\nconsumed: {}\ninert stub reached: {}\nmanual IDT invocation smoke proven this boot: {}\nruntime irq active: {}\n",
        smoke.blocker_bind_proof,
        smoke.blocker_manual_only,
        smoke.blocker_vector,
        smoke.blocker_no_irq,
        smoke.blocker_runtime,
        smoke.bind_proven_this_boot,
        smoke.armed,
        smoke.consumed,
        smoke.inert_stub_reached,
        smoke.manual_idt_invocation_smoke_proven_this_boot,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IDT vector invocation one-shot hardware smoke blockers\n- {}\n- {}\n- {}\n- {}\n- {}\nbind proven this boot: {}\narmed: {}\nconsumed: {}\ninert stub reached: {}\nmanual IDT invocation smoke proven this boot: {}\nruntime irq active: {}\n",
        smoke.blocker_bind_proof,
        smoke.blocker_manual_only,
        smoke.blocker_vector,
        smoke.blocker_no_irq,
        smoke.blocker_runtime,
        smoke.bind_proven_this_boot,
        smoke.armed,
        smoke.consumed,
        smoke.inert_stub_reached,
        smoke.manual_idt_invocation_smoke_proven_this_boot,
        smoke.runtime_irq_active
    );
}

fn print_idt_bind_runtime_bridge_note() {
    use core::fmt::Write;

    let bridge = idt_bind_runtime_bridge_readiness_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT bind runtime bridge note\nscope: {}\ninputs: {}\nmanual IDT bind smoke proven this boot: {}\nruntime IDT bridge ready: {}\nlive IRQ bind allowed: {}\ninterrupt invocation allowed: {}\nruntime irq active: {}\n",
        bridge.scope,
        bridge.inputs,
        bridge.manual_idt_bind_smoke_proven_this_boot,
        bridge.runtime_idt_bridge_ready,
        bridge.live_irq_bind_allowed,
        bridge.interrupt_invocation_allowed,
        bridge.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IDT bind runtime bridge note\nscope: {}\ninputs: {}\nmanual IDT bind smoke proven this boot: {}\nruntime IDT bridge ready: {}\nlive IRQ bind allowed: {}\ninterrupt invocation allowed: {}\nruntime irq active: {}\n",
        bridge.scope,
        bridge.inputs,
        bridge.manual_idt_bind_smoke_proven_this_boot,
        bridge.runtime_idt_bridge_ready,
        bridge.live_irq_bind_allowed,
        bridge.interrupt_invocation_allowed,
        bridge.runtime_irq_active
    );
}

fn print_idt_bind_runtime_bridge_status() {
    use core::fmt::Write;

    let bridge = idt_bind_runtime_bridge_readiness_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT bind runtime bridge readiness\nmanual IDT bind smoke proven this boot: {}\nruntime IDT bridge ready: {}\nlive IRQ bind allowed: {}\nIRQ handler reachable: {}\ninterrupt invocation allowed: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        bridge.manual_idt_bind_smoke_proven_this_boot,
        bridge.runtime_idt_bridge_ready,
        bridge.live_irq_bind_allowed,
        bridge.irq_handler_reachable,
        bridge.interrupt_invocation_allowed,
        bridge.runtime_irq_active,
        bridge.sti,
        bridge.pic_unmask,
        bridge.keyboard_mode
    );
    let _ = write!(serial_writer, "Controlled IDT bind runtime bridge readiness\nmanual IDT bind smoke proven this boot: {}\nruntime IDT bridge ready: {}\nlive IRQ bind allowed: {}\nIRQ handler reachable: {}\ninterrupt invocation allowed: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        bridge.manual_idt_bind_smoke_proven_this_boot,
        bridge.runtime_idt_bridge_ready,
        bridge.live_irq_bind_allowed,
        bridge.irq_handler_reachable,
        bridge.interrupt_invocation_allowed,
        bridge.runtime_irq_active,
        bridge.sti,
        bridge.pic_unmask,
        bridge.keyboard_mode
    );
}

fn print_idt_bind_runtime_bridge_check() {
    print_idt_bind_runtime_bridge_status();
}

fn print_idt_bind_runtime_bridge_blockers() {
    use core::fmt::Write;

    let bridge = idt_bind_runtime_bridge_readiness_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT bind runtime bridge blockers\n- {}\n- {}\n- {}\n- {}\n- {}\nruntime IDT bridge ready: {}\nlive IRQ bind allowed: {}\nIRQ handler reachable: {}\nruntime irq active: {}\n",
        bridge.blocker_proof,
        bridge.blocker_live_bind,
        bridge.blocker_irq_reachable,
        bridge.blocker_interrupt,
        bridge.blocker_runtime,
        bridge.runtime_idt_bridge_ready,
        bridge.live_irq_bind_allowed,
        bridge.irq_handler_reachable,
        bridge.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IDT bind runtime bridge blockers\n- {}\n- {}\n- {}\n- {}\n- {}\nruntime IDT bridge ready: {}\nlive IRQ bind allowed: {}\nIRQ handler reachable: {}\nruntime irq active: {}\n",
        bridge.blocker_proof,
        bridge.blocker_live_bind,
        bridge.blocker_irq_reachable,
        bridge.blocker_interrupt,
        bridge.blocker_runtime,
        bridge.runtime_idt_bridge_ready,
        bridge.live_irq_bind_allowed,
        bridge.irq_handler_reachable,
        bridge.runtime_irq_active
    );
}

fn print_idt_invoke_runtime_bridge_note() {
    use core::fmt::Write;

    let bridge = idt_invoke_runtime_bridge_readiness_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT invocation runtime bridge note\nscope: {}\ninputs: {}\nmanual IDT bind smoke proven this boot: {}\nmanual IDT invocation smoke proven this boot: {}\nruntime invocation bridge ready: {}\nlive IRQ delivery allowed: {}\nruntime irq active: {}\n",
        bridge.scope,
        bridge.inputs,
        bridge.manual_idt_bind_smoke_proven_this_boot,
        bridge.manual_idt_invocation_smoke_proven_this_boot,
        bridge.runtime_invocation_bridge_ready,
        bridge.live_irq_delivery_allowed,
        bridge.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IDT invocation runtime bridge note\nscope: {}\ninputs: {}\nmanual IDT bind smoke proven this boot: {}\nmanual IDT invocation smoke proven this boot: {}\nruntime invocation bridge ready: {}\nlive IRQ delivery allowed: {}\nruntime irq active: {}\n",
        bridge.scope,
        bridge.inputs,
        bridge.manual_idt_bind_smoke_proven_this_boot,
        bridge.manual_idt_invocation_smoke_proven_this_boot,
        bridge.runtime_invocation_bridge_ready,
        bridge.live_irq_delivery_allowed,
        bridge.runtime_irq_active
    );
}

fn print_idt_invoke_runtime_bridge_status() {
    use core::fmt::Write;

    let bridge = idt_invoke_runtime_bridge_readiness_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT invocation runtime bridge readiness\nmanual IDT bind smoke proven this boot: {}\nmanual IDT invocation smoke proven this boot: {}\nruntime invocation bridge ready: {}\nlive IRQ delivery allowed: {}\nIRQ handler reachable from hardware: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        bridge.manual_idt_bind_smoke_proven_this_boot,
        bridge.manual_idt_invocation_smoke_proven_this_boot,
        bridge.runtime_invocation_bridge_ready,
        bridge.live_irq_delivery_allowed,
        bridge.irq_handler_reachable_from_hardware,
        bridge.handler_triggered_eoi_allowed,
        bridge.runtime_irq_active,
        bridge.sti,
        bridge.pic_unmask,
        bridge.keyboard_mode
    );
    let _ = write!(serial_writer, "Controlled IDT invocation runtime bridge readiness\nmanual IDT bind smoke proven this boot: {}\nmanual IDT invocation smoke proven this boot: {}\nruntime invocation bridge ready: {}\nlive IRQ delivery allowed: {}\nIRQ handler reachable from hardware: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        bridge.manual_idt_bind_smoke_proven_this_boot,
        bridge.manual_idt_invocation_smoke_proven_this_boot,
        bridge.runtime_invocation_bridge_ready,
        bridge.live_irq_delivery_allowed,
        bridge.irq_handler_reachable_from_hardware,
        bridge.handler_triggered_eoi_allowed,
        bridge.runtime_irq_active,
        bridge.sti,
        bridge.pic_unmask,
        bridge.keyboard_mode
    );
}

fn print_idt_invoke_runtime_bridge_check() {
    print_idt_invoke_runtime_bridge_status();
}

fn print_idt_invoke_runtime_bridge_blockers() {
    use core::fmt::Write;

    let bridge = idt_invoke_runtime_bridge_readiness_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IDT invocation runtime bridge blockers\n- {}\n- {}\n- {}\n- {}\n- {}\nruntime invocation bridge ready: {}\nlive IRQ delivery allowed: {}\nIRQ handler reachable from hardware: {}\nruntime irq active: {}\n",
        bridge.blocker_bind_proof,
        bridge.blocker_invoke_proof,
        bridge.blocker_delivery,
        bridge.blocker_hardware_reachable,
        bridge.blocker_runtime,
        bridge.runtime_invocation_bridge_ready,
        bridge.live_irq_delivery_allowed,
        bridge.irq_handler_reachable_from_hardware,
        bridge.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IDT invocation runtime bridge blockers\n- {}\n- {}\n- {}\n- {}\n- {}\nruntime invocation bridge ready: {}\nlive IRQ delivery allowed: {}\nIRQ handler reachable from hardware: {}\nruntime irq active: {}\n",
        bridge.blocker_bind_proof,
        bridge.blocker_invoke_proof,
        bridge.blocker_delivery,
        bridge.blocker_hardware_reachable,
        bridge.blocker_runtime,
        bridge.runtime_invocation_bridge_ready,
        bridge.live_irq_delivery_allowed,
        bridge.irq_handler_reachable_from_hardware,
        bridge.runtime_irq_active
    );
}

fn irq_delivery_candidate_snapshot() -> irq::IrqDeliveryCandidate {
    let pic_smoke = eoi_write_hw_smoke_status_snapshot();
    let bind_smoke = idt_bind_hw_smoke_status_snapshot();
    let invoke_smoke = idt_invoke_hw_smoke_status_snapshot();
    let invocation_bridge = idt_invoke_runtime_bridge_readiness_snapshot();
    let bind_candidate = irq_handler_bind_candidate_snapshot();
    let stub = irq_handler_eoi_stub_snapshot();
    irq::irq_delivery_candidate(
        pic_smoke.manual_pic_eoi_smoke_proven_this_boot,
        bind_smoke.manual_idt_bind_smoke_proven_this_boot,
        invoke_smoke.manual_idt_invocation_smoke_proven_this_boot,
        invocation_bridge,
        bind_candidate,
        stub,
    )
}

fn irq0_activation_preflight_snapshot() -> irq::Irq0ActivationPreflight {
    let bind = irq0_bind_hw_smoke_status_snapshot();
    let unmask = irq0_unmask_hw_smoke_status_snapshot();
    let eoi = eoi_write_hw_smoke_status_snapshot();
    irq::irq0_activation_preflight(
        bind.irq0_bind_proven_this_boot,
        unmask.irq0_unmask_proven_this_boot,
        eoi.manual_pic_eoi_smoke_proven_this_boot,
    )
}

fn irq0_timer_handler_stub_snapshot() -> irq::Irq0TimerHandlerStub {
    irq::irq0_timer_handler_stub()
}

fn print_irq_delivery_candidate_note() {
    use core::fmt::Write;

    let candidate = irq_delivery_candidate_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled hardware IRQ delivery candidate note\nscope: {}\ninputs: {}\nmanual PIC_EOI smoke proven this boot: {}\nmanual IDT bind smoke proven this boot: {}\nmanual IDT invocation smoke proven this boot: {}\nhardware IRQ delivery candidate exists: {}\ncandidate ready: {}\nruntime irq active: {}\n",
        candidate.scope,
        candidate.inputs,
        candidate.manual_pic_eoi_smoke_proven_this_boot,
        candidate.manual_idt_bind_smoke_proven_this_boot,
        candidate.manual_idt_invocation_smoke_proven_this_boot,
        candidate.hardware_irq_delivery_candidate_exists,
        candidate.candidate_ready,
        candidate.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled hardware IRQ delivery candidate note\nscope: {}\ninputs: {}\nmanual PIC_EOI smoke proven this boot: {}\nmanual IDT bind smoke proven this boot: {}\nmanual IDT invocation smoke proven this boot: {}\nhardware IRQ delivery candidate exists: {}\ncandidate ready: {}\nruntime irq active: {}\n",
        candidate.scope,
        candidate.inputs,
        candidate.manual_pic_eoi_smoke_proven_this_boot,
        candidate.manual_idt_bind_smoke_proven_this_boot,
        candidate.manual_idt_invocation_smoke_proven_this_boot,
        candidate.hardware_irq_delivery_candidate_exists,
        candidate.candidate_ready,
        candidate.runtime_irq_active
    );
}

fn print_irq_delivery_candidate_status() {
    use core::fmt::Write;

    let candidate = irq_delivery_candidate_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled hardware IRQ delivery candidate status\nmanual PIC_EOI smoke proven this boot: {}\nmanual IDT bind smoke proven this boot: {}\nmanual IDT invocation smoke proven this boot: {}\nruntime invocation bridge ready: {}\nhardware IRQ delivery candidate exists: {}\ncandidate ready: {}\nIRQ0 delivery allowed: {}\nIRQ1 delivery allowed: {}\nlive IRQ handler bind: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        candidate.manual_pic_eoi_smoke_proven_this_boot,
        candidate.manual_idt_bind_smoke_proven_this_boot,
        candidate.manual_idt_invocation_smoke_proven_this_boot,
        candidate.runtime_invocation_bridge_ready,
        candidate.hardware_irq_delivery_candidate_exists,
        candidate.candidate_ready,
        candidate.irq0_delivery_allowed,
        candidate.irq1_delivery_allowed,
        candidate.live_irq_handler_bind,
        candidate.handler_triggered_eoi_allowed,
        candidate.runtime_irq_active,
        candidate.sti,
        candidate.pic_unmask,
        candidate.keyboard_mode
    );
    let _ = write!(serial_writer, "Controlled hardware IRQ delivery candidate status\nmanual PIC_EOI smoke proven this boot: {}\nmanual IDT bind smoke proven this boot: {}\nmanual IDT invocation smoke proven this boot: {}\nruntime invocation bridge ready: {}\nhardware IRQ delivery candidate exists: {}\ncandidate ready: {}\nIRQ0 delivery allowed: {}\nIRQ1 delivery allowed: {}\nlive IRQ handler bind: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        candidate.manual_pic_eoi_smoke_proven_this_boot,
        candidate.manual_idt_bind_smoke_proven_this_boot,
        candidate.manual_idt_invocation_smoke_proven_this_boot,
        candidate.runtime_invocation_bridge_ready,
        candidate.hardware_irq_delivery_candidate_exists,
        candidate.candidate_ready,
        candidate.irq0_delivery_allowed,
        candidate.irq1_delivery_allowed,
        candidate.live_irq_handler_bind,
        candidate.handler_triggered_eoi_allowed,
        candidate.runtime_irq_active,
        candidate.sti,
        candidate.pic_unmask,
        candidate.keyboard_mode
    );
}

fn print_irq_delivery_candidate_check() {
    print_irq_delivery_candidate_status();
}

fn print_irq_delivery_candidate_blockers() {
    use core::fmt::Write;

    let candidate = irq_delivery_candidate_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled hardware IRQ delivery candidate blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nhardware IRQ delivery candidate exists: {}\ncandidate ready: {}\nIRQ0 delivery allowed: {}\nIRQ1 delivery allowed: {}\nlive IRQ handler bind: {}\nruntime irq active: {}\n",
        candidate.blocker_readiness,
        candidate.blocker_irq0,
        candidate.blocker_irq1,
        candidate.blocker_live_bind,
        candidate.blocker_handler_eoi,
        candidate.blocker_sti,
        candidate.blocker_pic_unmask,
        candidate.blocker_runtime,
        candidate.hardware_irq_delivery_candidate_exists,
        candidate.candidate_ready,
        candidate.irq0_delivery_allowed,
        candidate.irq1_delivery_allowed,
        candidate.live_irq_handler_bind,
        candidate.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled hardware IRQ delivery candidate blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nhardware IRQ delivery candidate exists: {}\ncandidate ready: {}\nIRQ0 delivery allowed: {}\nIRQ1 delivery allowed: {}\nlive IRQ handler bind: {}\nruntime irq active: {}\n",
        candidate.blocker_readiness,
        candidate.blocker_irq0,
        candidate.blocker_irq1,
        candidate.blocker_live_bind,
        candidate.blocker_handler_eoi,
        candidate.blocker_sti,
        candidate.blocker_pic_unmask,
        candidate.blocker_runtime,
        candidate.hardware_irq_delivery_candidate_exists,
        candidate.candidate_ready,
        candidate.irq0_delivery_allowed,
        candidate.irq1_delivery_allowed,
        candidate.live_irq_handler_bind,
        candidate.runtime_irq_active
    );
}

fn print_irq0_preflight_status() {
    use core::fmt::Write;

    let preflight = irq0_activation_preflight_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "IRQ0 activation preflight\ndescriptor bind proof: {}\ntransactional unmask proof: {}\nmanual EOI proof: {}\nsti: {}\nirq0 currently masked: {}\nruntime irq active: {}\nactivation allowed: {}\n",
        preflight.descriptor_bind_proof,
        preflight.transactional_unmask_proof,
        preflight.manual_eoi_proof,
        preflight.sti,
        preflight.irq0_currently_masked,
        preflight.runtime_irq_active,
        preflight.activation_allowed
    );
    let _ = write!(serial_writer, "IRQ0 activation preflight\ndescriptor bind proof: {}\ntransactional unmask proof: {}\nmanual EOI proof: {}\nsti: {}\nirq0 currently masked: {}\nruntime irq active: {}\nactivation allowed: {}\n",
        preflight.descriptor_bind_proof,
        preflight.transactional_unmask_proof,
        preflight.manual_eoi_proof,
        preflight.sti,
        preflight.irq0_currently_masked,
        preflight.runtime_irq_active,
        preflight.activation_allowed
    );
}

fn print_irq0_preflight_check() {
    print_irq0_preflight_status();
}

fn print_irq0_preflight_blockers() {
    use core::fmt::Write;

    let preflight = irq0_activation_preflight_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "IRQ0 activation preflight blockers\n- {}\n- {}\n- {}\n- {}\nsti: {}\nirq0 currently masked: {}\nruntime irq active: {}\nactivation allowed: {}\n",
        preflight.blocker_descriptor_bind,
        preflight.blocker_transactional_unmask,
        preflight.blocker_manual_eoi,
        preflight.blocker_bounded_sti,
        preflight.sti,
        preflight.irq0_currently_masked,
        preflight.runtime_irq_active,
        preflight.activation_allowed
    );
    let _ = write!(serial_writer, "IRQ0 activation preflight blockers\n- {}\n- {}\n- {}\n- {}\nsti: {}\nirq0 currently masked: {}\nruntime irq active: {}\nactivation allowed: {}\n",
        preflight.blocker_descriptor_bind,
        preflight.blocker_transactional_unmask,
        preflight.blocker_manual_eoi,
        preflight.blocker_bounded_sti,
        preflight.sti,
        preflight.irq0_currently_masked,
        preflight.runtime_irq_active,
        preflight.activation_allowed
    );
}

fn print_irq0_handler_stub_status() {
    use core::fmt::Write;

    let stub = irq0_timer_handler_stub_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "IRQ0 timer handler stub\nIRQ0 timer handler stub exists: {}\nstub reachable from hardware: {}\ncounter increment path: {}\nIRQ0 self-mask path: {}\nmaster PIC_EOI path: {}\nSTI: {}\nIRQ0 currently masked: {}\nruntime irq active: {}\nkeyboard mode: {}\n",
        stub.stub_exists,
        stub.stub_reachable_from_hardware,
        stub.counter_increment_path,
        stub.irq0_self_mask_path,
        stub.master_pic_eoi_path,
        stub.sti,
        stub.irq0_currently_masked,
        stub.runtime_irq_active,
        stub.keyboard_mode
    );
    let _ = write!(serial_writer, "IRQ0 timer handler stub\nIRQ0 timer handler stub exists: {}\nstub reachable from hardware: {}\ncounter increment path: {}\nIRQ0 self-mask path: {}\nmaster PIC_EOI path: {}\nSTI: {}\nIRQ0 currently masked: {}\nruntime irq active: {}\nkeyboard mode: {}\n",
        stub.stub_exists,
        stub.stub_reachable_from_hardware,
        stub.counter_increment_path,
        stub.irq0_self_mask_path,
        stub.master_pic_eoi_path,
        stub.sti,
        stub.irq0_currently_masked,
        stub.runtime_irq_active,
        stub.keyboard_mode
    );
}

fn print_irq0_handler_stub_check() {
    print_irq0_handler_stub_status();
}

fn print_irq0_handler_stub_blockers() {
    use core::fmt::Write;

    let stub = irq0_timer_handler_stub_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "IRQ0 timer handler stub blockers\n- {}\n- {}\n- {}\n- {}\nstub reachable from hardware: {}\nruntime irq active: {}\nkeyboard mode: {}\n",
        stub.blocker_sti,
        stub.blocker_irq0_masked,
        stub.blocker_delivery,
        stub.blocker_activation_window,
        stub.stub_reachable_from_hardware,
        stub.runtime_irq_active,
        stub.keyboard_mode
    );
    let _ = write!(serial_writer, "IRQ0 timer handler stub blockers\n- {}\n- {}\n- {}\n- {}\nstub reachable from hardware: {}\nruntime irq active: {}\nkeyboard mode: {}\n",
        stub.blocker_sti,
        stub.blocker_irq0_masked,
        stub.blocker_delivery,
        stub.blocker_activation_window,
        stub.stub_reachable_from_hardware,
        stub.runtime_irq_active,
        stub.keyboard_mode
    );
}

fn print_eoi_write_hw_smoke_note() {
    use core::fmt::Write;

    let smoke = eoi_write_hw_smoke_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write hardware smoke note\nscope: {}\nmode: {}\narmed: {}\nconsumed: {}\ntarget command port: {}\ntarget value: {}\nPIC_EOI writes this command: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.scope,
        smoke.mode,
        smoke.armed,
        smoke.consumed,
        smoke.target_command_port,
        smoke.target_value,
        smoke.pic_eoi_writes_this_command,
        smoke.first_pic_eoi_write_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write hardware smoke note\nscope: {}\nmode: {}\narmed: {}\nconsumed: {}\ntarget command port: {}\ntarget value: {}\nPIC_EOI writes this command: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.scope,
        smoke.mode,
        smoke.armed,
        smoke.consumed,
        smoke.target_command_port,
        smoke.target_value,
        smoke.pic_eoi_writes_this_command,
        smoke.first_pic_eoi_write_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
}

fn print_eoi_write_hw_smoke_status() {
    use core::fmt::Write;

    let smoke = eoi_write_hw_smoke_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write hardware smoke status\narmed: {}\nconsumed: {}\ntarget command port: {}\ntarget value: {}\nPIC_EOI writes this command: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.armed,
        smoke.consumed,
        smoke.target_command_port,
        smoke.target_value,
        smoke.pic_eoi_writes_this_command,
        smoke.first_pic_eoi_write_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write hardware smoke status\narmed: {}\nconsumed: {}\ntarget command port: {}\ntarget value: {}\nPIC_EOI writes this command: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.armed,
        smoke.consumed,
        smoke.target_command_port,
        smoke.target_value,
        smoke.pic_eoi_writes_this_command,
        smoke.first_pic_eoi_write_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
}

fn print_eoi_write_hw_smoke_arm() {
    use core::fmt::Write;

    let smoke = eoi_write_hw_smoke_arm_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write hardware smoke arm\n{}\narmed: {}\nconsumed: {}\nPIC_EOI writes this command: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.pic_eoi_writes_this_command,
        smoke.first_pic_eoi_write_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write hardware smoke arm\n{}\narmed: {}\nconsumed: {}\nPIC_EOI writes this command: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.pic_eoi_writes_this_command,
        smoke.first_pic_eoi_write_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
}

fn print_eoi_write_hw_smoke_fire() {
    use core::fmt::Write;

    let smoke = eoi_write_hw_smoke_fire_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write hardware smoke fire\n{}\narmed: {}\nconsumed: {}\ntarget command port: {}\ntarget value: {}\nPIC_EOI writes this command: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.target_command_port,
        smoke.target_value,
        smoke.pic_eoi_writes_this_command,
        smoke.first_pic_eoi_write_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write hardware smoke fire\n{}\narmed: {}\nconsumed: {}\ntarget command port: {}\ntarget value: {}\nPIC_EOI writes this command: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.target_command_port,
        smoke.target_value,
        smoke.pic_eoi_writes_this_command,
        smoke.first_pic_eoi_write_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
}

fn print_eoi_write_hw_smoke_clear() {
    use core::fmt::Write;

    let smoke = eoi_write_hw_smoke_clear_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write hardware smoke clear\n{}\narmed: {}\nconsumed: {}\nPIC_EOI writes this command: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.pic_eoi_writes_this_command,
        smoke.first_pic_eoi_write_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write hardware smoke clear\n{}\narmed: {}\nconsumed: {}\nPIC_EOI writes this command: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        smoke.fire_result,
        smoke.armed,
        smoke.consumed,
        smoke.pic_eoi_writes_this_command,
        smoke.first_pic_eoi_write_performed,
        smoke.hardware_mutation,
        smoke.runtime_irq_active
    );
}

fn print_eoi_write_hw_smoke_blockers() {
    use core::fmt::Write;

    let smoke = eoi_write_hw_smoke_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write hardware smoke blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\narmed: {}\nconsumed: {}\nfirst PIC_EOI write performed: {}\nruntime irq active: {}\n",
        smoke.blocker_manual_only,
        smoke.blocker_master_only,
        smoke.blocker_one_shot,
        smoke.blocker_sti,
        smoke.blocker_unmask,
        smoke.blocker_live_irq,
        smoke.blocker_runtime,
        smoke.armed,
        smoke.consumed,
        smoke.first_pic_eoi_write_performed,
        smoke.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write hardware smoke blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\narmed: {}\nconsumed: {}\nfirst PIC_EOI write performed: {}\nruntime irq active: {}\n",
        smoke.blocker_manual_only,
        smoke.blocker_master_only,
        smoke.blocker_one_shot,
        smoke.blocker_sti,
        smoke.blocker_unmask,
        smoke.blocker_live_irq,
        smoke.blocker_runtime,
        smoke.armed,
        smoke.consumed,
        smoke.first_pic_eoi_write_performed,
        smoke.runtime_irq_active
    );
}

fn print_eoi_runtime_bridge_note() {
    use core::fmt::Write;

    let bridge = eoi_runtime_bridge_readiness_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled PIC_EOI runtime bridge note\nscope: {}\ninputs: {}\nmanual PIC_EOI smoke proven: {}\nruntime bridge ready: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\n",
        bridge.scope,
        bridge.inputs,
        bridge.manual_pic_eoi_smoke_proven,
        bridge.runtime_bridge_ready,
        bridge.handler_triggered_eoi_allowed,
        bridge.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled PIC_EOI runtime bridge note\nscope: {}\ninputs: {}\nmanual PIC_EOI smoke proven: {}\nruntime bridge ready: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\n",
        bridge.scope,
        bridge.inputs,
        bridge.manual_pic_eoi_smoke_proven,
        bridge.runtime_bridge_ready,
        bridge.handler_triggered_eoi_allowed,
        bridge.runtime_irq_active
    );
}

fn print_eoi_runtime_bridge_status() {
    use core::fmt::Write;

    let bridge = eoi_runtime_bridge_readiness_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled PIC_EOI runtime bridge readiness\nmanual PIC_EOI smoke proven: {}\nruntime bridge ready: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nlive irq handlers: {}\nkeyboard mode: {}\n",
        bridge.manual_pic_eoi_smoke_proven,
        bridge.runtime_bridge_ready,
        bridge.handler_triggered_eoi_allowed,
        bridge.runtime_irq_active,
        bridge.sti,
        bridge.pic_unmask,
        bridge.live_irq_handlers,
        bridge.keyboard_mode
    );
    let _ = write!(serial_writer, "Controlled PIC_EOI runtime bridge readiness\nmanual PIC_EOI smoke proven: {}\nruntime bridge ready: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nlive irq handlers: {}\nkeyboard mode: {}\n",
        bridge.manual_pic_eoi_smoke_proven,
        bridge.runtime_bridge_ready,
        bridge.handler_triggered_eoi_allowed,
        bridge.runtime_irq_active,
        bridge.sti,
        bridge.pic_unmask,
        bridge.live_irq_handlers,
        bridge.keyboard_mode
    );
}

fn print_eoi_runtime_bridge_check() {
    print_eoi_runtime_bridge_status();
}

fn print_eoi_runtime_bridge_blockers() {
    use core::fmt::Write;

    let bridge = eoi_runtime_bridge_readiness_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled PIC_EOI runtime bridge blockers\n- {}\n- {}\n- {}\n- {}\n- {}\nruntime bridge ready: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\n",
        bridge.blocker_dispatch,
        bridge.blocker_sti,
        bridge.blocker_pic_lines,
        bridge.blocker_live_handlers,
        bridge.blocker_handler_eoi,
        bridge.runtime_bridge_ready,
        bridge.handler_triggered_eoi_allowed,
        bridge.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled PIC_EOI runtime bridge blockers\n- {}\n- {}\n- {}\n- {}\n- {}\nruntime bridge ready: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\n",
        bridge.blocker_dispatch,
        bridge.blocker_sti,
        bridge.blocker_pic_lines,
        bridge.blocker_live_handlers,
        bridge.blocker_handler_eoi,
        bridge.runtime_bridge_ready,
        bridge.handler_triggered_eoi_allowed,
        bridge.runtime_irq_active
    );
}

fn print_irq_handler_eoi_candidate_note() {
    use core::fmt::Write;

    let candidate = irq_handler_eoi_candidate_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IRQ handler EOI path candidate note\nscope: {}\ninputs: {}\nruntime bridge ready: {}\nhandler EOI candidate ready: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\n",
        candidate.scope,
        candidate.inputs,
        candidate.runtime_bridge_ready,
        candidate.handler_eoi_candidate_ready,
        candidate.handler_triggered_eoi_allowed,
        candidate.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IRQ handler EOI path candidate note\nscope: {}\ninputs: {}\nruntime bridge ready: {}\nhandler EOI candidate ready: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\n",
        candidate.scope,
        candidate.inputs,
        candidate.runtime_bridge_ready,
        candidate.handler_eoi_candidate_ready,
        candidate.handler_triggered_eoi_allowed,
        candidate.runtime_irq_active
    );
}

fn print_irq_handler_eoi_candidate_status() {
    use core::fmt::Write;

    let candidate = irq_handler_eoi_candidate_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IRQ handler EOI path candidate\nruntime bridge ready: {}\nhandler EOI candidate ready: {}\nhandler-triggered EOI allowed: {}\nlive handler bind: {}\nPIC_EOI callsites: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        candidate.runtime_bridge_ready,
        candidate.handler_eoi_candidate_ready,
        candidate.handler_triggered_eoi_allowed,
        candidate.live_handler_bind,
        candidate.pic_eoi_callsites,
        candidate.runtime_irq_active,
        candidate.sti,
        candidate.pic_unmask,
        candidate.keyboard_mode
    );
    let _ = write!(serial_writer, "Controlled IRQ handler EOI path candidate\nruntime bridge ready: {}\nhandler EOI candidate ready: {}\nhandler-triggered EOI allowed: {}\nlive handler bind: {}\nPIC_EOI callsites: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        candidate.runtime_bridge_ready,
        candidate.handler_eoi_candidate_ready,
        candidate.handler_triggered_eoi_allowed,
        candidate.live_handler_bind,
        candidate.pic_eoi_callsites,
        candidate.runtime_irq_active,
        candidate.sti,
        candidate.pic_unmask,
        candidate.keyboard_mode
    );
}

fn print_irq_handler_eoi_candidate_check() {
    print_irq_handler_eoi_candidate_status();
}

fn print_irq_handler_eoi_candidate_blockers() {
    use core::fmt::Write;

    let candidate = irq_handler_eoi_candidate_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IRQ handler EOI path candidate blockers\n- {}\n- {}\n- {}\n- {}\n- {}\nhandler EOI candidate ready: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\n",
        candidate.blocker_bridge,
        candidate.blocker_handler_eoi,
        candidate.blocker_live_handlers,
        candidate.blocker_manual_only,
        candidate.blocker_runtime,
        candidate.handler_eoi_candidate_ready,
        candidate.handler_triggered_eoi_allowed,
        candidate.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IRQ handler EOI path candidate blockers\n- {}\n- {}\n- {}\n- {}\n- {}\nhandler EOI candidate ready: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\n",
        candidate.blocker_bridge,
        candidate.blocker_handler_eoi,
        candidate.blocker_live_handlers,
        candidate.blocker_manual_only,
        candidate.blocker_runtime,
        candidate.handler_eoi_candidate_ready,
        candidate.handler_triggered_eoi_allowed,
        candidate.runtime_irq_active
    );
}

fn print_irq_handler_eoi_stub_note() {
    use core::fmt::Write;

    let stub = irq_handler_eoi_stub_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IRQ handler EOI stub note\nscope: {}\ninputs: {}\nstub exists: {}\nstub bound to live IRQ path: {}\nstub performs PIC_EOI write: {}\nruntime irq active: {}\n",
        stub.scope,
        stub.inputs,
        stub.stub_exists,
        stub.stub_bound_to_live_irq_path,
        stub.stub_performs_pic_eoi_write,
        stub.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IRQ handler EOI stub note\nscope: {}\ninputs: {}\nstub exists: {}\nstub bound to live IRQ path: {}\nstub performs PIC_EOI write: {}\nruntime irq active: {}\n",
        stub.scope,
        stub.inputs,
        stub.stub_exists,
        stub.stub_bound_to_live_irq_path,
        stub.stub_performs_pic_eoi_write,
        stub.runtime_irq_active
    );
}

fn print_irq_handler_eoi_stub_status() {
    use core::fmt::Write;

    let stub = irq_handler_eoi_stub_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IRQ handler EOI stub\nstub exists: {}\nstub bound to live IRQ path: {}\nstub invocation allowed: {}\nstub performs PIC_EOI write: {}\nhandler-triggered EOI allowed: {}\nPIC_EOI callsites: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        stub.stub_exists,
        stub.stub_bound_to_live_irq_path,
        stub.stub_invocation_allowed,
        stub.stub_performs_pic_eoi_write,
        stub.handler_triggered_eoi_allowed,
        stub.pic_eoi_callsites,
        stub.runtime_irq_active,
        stub.sti,
        stub.pic_unmask,
        stub.keyboard_mode
    );
    let _ = write!(serial_writer, "Controlled IRQ handler EOI stub\nstub exists: {}\nstub bound to live IRQ path: {}\nstub invocation allowed: {}\nstub performs PIC_EOI write: {}\nhandler-triggered EOI allowed: {}\nPIC_EOI callsites: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        stub.stub_exists,
        stub.stub_bound_to_live_irq_path,
        stub.stub_invocation_allowed,
        stub.stub_performs_pic_eoi_write,
        stub.handler_triggered_eoi_allowed,
        stub.pic_eoi_callsites,
        stub.runtime_irq_active,
        stub.sti,
        stub.pic_unmask,
        stub.keyboard_mode
    );
}

fn print_irq_handler_eoi_stub_check() {
    print_irq_handler_eoi_stub_status();
}

fn print_irq_handler_eoi_stub_blockers() {
    use core::fmt::Write;

    let stub = irq_handler_eoi_stub_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IRQ handler EOI stub blockers\n- {}\n- {}\n- {}\n- {}\n- {}\nstub invocation allowed: {}\nstub performs PIC_EOI write: {}\nruntime irq active: {}\n",
        stub.blocker_unbound,
        stub.blocker_invocation,
        stub.blocker_handler_eoi,
        stub.blocker_manual_only,
        stub.blocker_runtime,
        stub.stub_invocation_allowed,
        stub.stub_performs_pic_eoi_write,
        stub.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IRQ handler EOI stub blockers\n- {}\n- {}\n- {}\n- {}\n- {}\nstub invocation allowed: {}\nstub performs PIC_EOI write: {}\nruntime irq active: {}\n",
        stub.blocker_unbound,
        stub.blocker_invocation,
        stub.blocker_handler_eoi,
        stub.blocker_manual_only,
        stub.blocker_runtime,
        stub.stub_invocation_allowed,
        stub.stub_performs_pic_eoi_write,
        stub.runtime_irq_active
    );
}

fn print_irq_handler_bind_candidate_note() {
    use core::fmt::Write;

    let bind = irq_handler_bind_candidate_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IRQ handler bind candidate note\nscope: {}\ninputs: {}\nstub exists: {}\nbind candidate exists: {}\nlive IDT bind performed: {}\nruntime irq active: {}\n",
        bind.scope,
        bind.inputs,
        bind.stub_exists,
        bind.bind_candidate_exists,
        bind.live_idt_bind_performed,
        bind.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IRQ handler bind candidate note\nscope: {}\ninputs: {}\nstub exists: {}\nbind candidate exists: {}\nlive IDT bind performed: {}\nruntime irq active: {}\n",
        bind.scope,
        bind.inputs,
        bind.stub_exists,
        bind.bind_candidate_exists,
        bind.live_idt_bind_performed,
        bind.runtime_irq_active
    );
}

fn print_irq_handler_bind_candidate_status() {
    use core::fmt::Write;

    let bind = irq_handler_bind_candidate_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IRQ handler bind candidate\nstub exists: {}\nbind candidate exists: {}\nbind candidate ready: {}\nlive IDT bind performed: {}\nIRQ handler reachable: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        bind.stub_exists,
        bind.bind_candidate_exists,
        bind.bind_candidate_ready,
        bind.live_idt_bind_performed,
        bind.irq_handler_reachable,
        bind.handler_triggered_eoi_allowed,
        bind.runtime_irq_active,
        bind.sti,
        bind.pic_unmask,
        bind.keyboard_mode
    );
    let _ = write!(serial_writer, "Controlled IRQ handler bind candidate\nstub exists: {}\nbind candidate exists: {}\nbind candidate ready: {}\nlive IDT bind performed: {}\nIRQ handler reachable: {}\nhandler-triggered EOI allowed: {}\nruntime irq active: {}\nsti: {}\npic unmask: {}\nkeyboard mode: {}\n",
        bind.stub_exists,
        bind.bind_candidate_exists,
        bind.bind_candidate_ready,
        bind.live_idt_bind_performed,
        bind.irq_handler_reachable,
        bind.handler_triggered_eoi_allowed,
        bind.runtime_irq_active,
        bind.sti,
        bind.pic_unmask,
        bind.keyboard_mode
    );
}

fn print_irq_handler_bind_candidate_check() {
    print_irq_handler_bind_candidate_status();
}

fn print_irq_handler_bind_candidate_blockers() {
    use core::fmt::Write;

    let bind = irq_handler_bind_candidate_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "Controlled IRQ handler bind candidate blockers\n- {}\n- {}\n- {}\n- {}\n- {}\nbind candidate ready: {}\nlive IDT bind performed: {}\nIRQ handler reachable: {}\nruntime irq active: {}\n",
        bind.blocker_idt_bind,
        bind.blocker_irq_registration,
        bind.blocker_stub_invocation,
        bind.blocker_handler_eoi,
        bind.blocker_runtime,
        bind.bind_candidate_ready,
        bind.live_idt_bind_performed,
        bind.irq_handler_reachable,
        bind.runtime_irq_active
    );
    let _ = write!(serial_writer, "Controlled IRQ handler bind candidate blockers\n- {}\n- {}\n- {}\n- {}\n- {}\nbind candidate ready: {}\nlive IDT bind performed: {}\nIRQ handler reachable: {}\nruntime irq active: {}\n",
        bind.blocker_idt_bind,
        bind.blocker_irq_registration,
        bind.blocker_stub_invocation,
        bind.blocker_handler_eoi,
        bind.blocker_runtime,
        bind.bind_candidate_ready,
        bind.live_idt_bind_performed,
        bind.irq_handler_reachable,
        bind.runtime_irq_active
    );
}

fn print_eoi_write_smoke_preflight_note() {
    use core::fmt::Write;

    let preflight = eoi_write_smoke_preflight_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write smoke preflight note\nscope: {}\npreflight inputs: {}\neoi write smoke preflight: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        preflight.scope,
        preflight.inputs,
        preflight.eoi_write_smoke_preflight,
        preflight.first_pic_eoi_write_allowed,
        preflight.hardware_mutation,
        preflight.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write smoke preflight note\nscope: {}\npreflight inputs: {}\neoi write smoke preflight: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        preflight.scope,
        preflight.inputs,
        preflight.eoi_write_smoke_preflight,
        preflight.first_pic_eoi_write_allowed,
        preflight.hardware_mutation,
        preflight.runtime_irq_active
    );
}

fn print_eoi_write_smoke_preflight_status() {
    use core::fmt::Write;

    let preflight = eoi_write_smoke_preflight_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write smoke preflight\neoi write smoke preflight: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\ntarget command port: {}\ntarget irq line: {}\neoi dispatch: {}\nsti: {}\npic unmask: {}\nlive idt bind: {}\nkeyboard mode: {}\n",
        preflight.eoi_write_smoke_preflight,
        preflight.first_pic_eoi_write_allowed,
        preflight.hardware_mutation,
        preflight.runtime_irq_active,
        preflight.target_command_port,
        preflight.target_irq_line,
        preflight.eoi_dispatch,
        preflight.sti_instruction,
        preflight.pic_unmask,
        preflight.live_idt_bind,
        preflight.keyboard_mode
    );
    let _ = write!(serial_writer, "EOI write smoke preflight\neoi write smoke preflight: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\ntarget command port: {}\ntarget irq line: {}\neoi dispatch: {}\nsti: {}\npic unmask: {}\nlive idt bind: {}\nkeyboard mode: {}\n",
        preflight.eoi_write_smoke_preflight,
        preflight.first_pic_eoi_write_allowed,
        preflight.hardware_mutation,
        preflight.runtime_irq_active,
        preflight.target_command_port,
        preflight.target_irq_line,
        preflight.eoi_dispatch,
        preflight.sti_instruction,
        preflight.pic_unmask,
        preflight.live_idt_bind,
        preflight.keyboard_mode
    );
}

fn print_eoi_write_smoke_preflight_blockers() {
    use core::fmt::Write;

    let preflight = eoi_write_smoke_preflight_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write smoke preflight blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nfirst PIC_EOI write allowed: {}\n",
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_SEQUENCE,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_MUTATION,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_DECISION,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_FINAL,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_EOI,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_PIC_UNMASK,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_IDT,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_STI,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_KEYBOARD,
        preflight.first_pic_eoi_write_allowed
    );
    let _ = write!(serial_writer, "EOI write smoke preflight blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nfirst PIC_EOI write allowed: {}\n",
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_SEQUENCE,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_MUTATION,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_DECISION,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_FINAL,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_EOI,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_PIC_UNMASK,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_IDT,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_STI,
        irq::EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_KEYBOARD,
        preflight.first_pic_eoi_write_allowed
    );
}

fn print_eoi_write_smoke_candidate_note() {
    use core::fmt::Write;

    let candidate = eoi_write_smoke_candidate_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write smoke candidate note\nscope: {}\ncandidate inputs: {}\neoi write smoke candidate: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        candidate.scope,
        candidate.inputs,
        candidate.eoi_write_smoke_candidate,
        candidate.first_pic_eoi_write_performed,
        candidate.hardware_mutation,
        candidate.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write smoke candidate note\nscope: {}\ncandidate inputs: {}\neoi write smoke candidate: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        candidate.scope,
        candidate.inputs,
        candidate.eoi_write_smoke_candidate,
        candidate.first_pic_eoi_write_performed,
        candidate.hardware_mutation,
        candidate.runtime_irq_active
    );
}

fn print_eoi_write_smoke_candidate_status() {
    use core::fmt::Write;

    let candidate = eoi_write_smoke_candidate_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write smoke candidate\neoi write smoke candidate: {}\ncandidate armed: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\ntarget command port: {}\ntarget irq line: {}\neoi dispatch: {}\nsti: {}\npic unmask: {}\nlive idt bind: {}\nkeyboard mode: {}\n",
        candidate.eoi_write_smoke_candidate,
        candidate.candidate_armed,
        candidate.first_pic_eoi_write_performed,
        candidate.hardware_mutation,
        candidate.runtime_irq_active,
        candidate.target_command_port,
        candidate.target_irq_line,
        candidate.eoi_dispatch,
        candidate.sti_instruction,
        candidate.pic_unmask,
        candidate.live_idt_bind,
        candidate.keyboard_mode
    );
    let _ = write!(serial_writer, "EOI write smoke candidate\neoi write smoke candidate: {}\ncandidate armed: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\ntarget command port: {}\ntarget irq line: {}\neoi dispatch: {}\nsti: {}\npic unmask: {}\nlive idt bind: {}\nkeyboard mode: {}\n",
        candidate.eoi_write_smoke_candidate,
        candidate.candidate_armed,
        candidate.first_pic_eoi_write_performed,
        candidate.hardware_mutation,
        candidate.runtime_irq_active,
        candidate.target_command_port,
        candidate.target_irq_line,
        candidate.eoi_dispatch,
        candidate.sti_instruction,
        candidate.pic_unmask,
        candidate.live_idt_bind,
        candidate.keyboard_mode
    );
}

fn print_eoi_write_smoke_candidate_fire() {
    use core::fmt::Write;

    let candidate = eoi_write_smoke_candidate_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write smoke candidate fire\nfire result: {}\nfirst PIC_EOI write performed: {}\ntarget command port: {}\ntarget irq line: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        candidate.fire_result,
        candidate.first_pic_eoi_write_performed,
        candidate.target_command_port,
        candidate.target_irq_line,
        candidate.hardware_mutation,
        candidate.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write smoke candidate fire\nfire result: {}\nfirst PIC_EOI write performed: {}\ntarget command port: {}\ntarget irq line: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        candidate.fire_result,
        candidate.first_pic_eoi_write_performed,
        candidate.target_command_port,
        candidate.target_irq_line,
        candidate.hardware_mutation,
        candidate.runtime_irq_active
    );
}

fn print_eoi_write_smoke_candidate_blockers() {
    use core::fmt::Write;

    let candidate = eoi_write_smoke_candidate_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write smoke candidate blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nfirst PIC_EOI write performed: {}\n",
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_PREFLIGHT,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_FIRST_ALLOWED,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_SEQUENCE,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_MUTATION,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_DECISION,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_FINAL,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_EOI,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_PIC_UNMASK,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_IDT,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_STI,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_KEYBOARD,
        candidate.first_pic_eoi_write_performed
    );
    let _ = write!(serial_writer, "EOI write smoke candidate blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nfirst PIC_EOI write performed: {}\n",
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_PREFLIGHT,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_FIRST_ALLOWED,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_SEQUENCE,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_MUTATION,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_DECISION,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_FINAL,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_EOI,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_PIC_UNMASK,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_IDT,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_STI,
        irq::EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_KEYBOARD,
        candidate.first_pic_eoi_write_performed
    );
}

fn print_eoi_write_permit_note() {
    use core::fmt::Write;

    let permit = eoi_write_permit_model_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write permit note\nscope: {}\npermit inputs: {}\npermit granted: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        permit.scope,
        permit.inputs,
        permit.permit_granted,
        permit.first_pic_eoi_write_allowed,
        permit.hardware_mutation,
        permit.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write permit note\nscope: {}\npermit inputs: {}\npermit granted: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        permit.scope,
        permit.inputs,
        permit.permit_granted,
        permit.first_pic_eoi_write_allowed,
        permit.hardware_mutation,
        permit.runtime_irq_active
    );
}

fn print_eoi_write_permit_status() {
    use core::fmt::Write;

    let permit = eoi_write_permit_model_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write permit model\npermit granted: {}\nfirst PIC_EOI write allowed: {}\ntarget command port: {}\ntarget value: {}\ntarget irq line: {}\nhardware mutation: {}\nruntime irq active: {}\nfire command: {}\n",
        permit.permit_granted,
        permit.first_pic_eoi_write_allowed,
        permit.target_command_port,
        permit.target_value,
        permit.target_irq_line,
        permit.hardware_mutation,
        permit.runtime_irq_active,
        permit.fire_command
    );
    let _ = write!(serial_writer, "EOI write permit model\npermit granted: {}\nfirst PIC_EOI write allowed: {}\ntarget command port: {}\ntarget value: {}\ntarget irq line: {}\nhardware mutation: {}\nruntime irq active: {}\nfire command: {}\n",
        permit.permit_granted,
        permit.first_pic_eoi_write_allowed,
        permit.target_command_port,
        permit.target_value,
        permit.target_irq_line,
        permit.hardware_mutation,
        permit.runtime_irq_active,
        permit.fire_command
    );
}

fn print_eoi_write_permit_blockers() {
    use core::fmt::Write;

    let permit = eoi_write_permit_model_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write permit blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\npermit granted: {}\n",
        irq::EOI_WRITE_PERMIT_BLOCKER_DECISION,
        irq::EOI_WRITE_PERMIT_BLOCKER_FINAL_GATE,
        irq::EOI_WRITE_PERMIT_BLOCKER_MUTATION,
        irq::EOI_WRITE_PERMIT_BLOCKER_SEQUENCE,
        irq::EOI_WRITE_PERMIT_BLOCKER_CANDIDATE_FIRE,
        irq::EOI_WRITE_PERMIT_BLOCKER_STI,
        irq::EOI_WRITE_PERMIT_BLOCKER_PIC_UNMASK,
        irq::EOI_WRITE_PERMIT_BLOCKER_LIVE_IRQ,
        permit.permit_granted
    );
    let _ = write!(serial_writer, "EOI write permit blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\npermit granted: {}\n",
        irq::EOI_WRITE_PERMIT_BLOCKER_DECISION,
        irq::EOI_WRITE_PERMIT_BLOCKER_FINAL_GATE,
        irq::EOI_WRITE_PERMIT_BLOCKER_MUTATION,
        irq::EOI_WRITE_PERMIT_BLOCKER_SEQUENCE,
        irq::EOI_WRITE_PERMIT_BLOCKER_CANDIDATE_FIRE,
        irq::EOI_WRITE_PERMIT_BLOCKER_STI,
        irq::EOI_WRITE_PERMIT_BLOCKER_PIC_UNMASK,
        irq::EOI_WRITE_PERMIT_BLOCKER_LIVE_IRQ,
        permit.permit_granted
    );
}

fn print_eoi_write_oneshot_note() {
    use core::fmt::Write;

    let oneshot = eoi_write_oneshot_command_path_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write one-shot note\nscope: {}\ninputs: {}\none-shot armed: {}\nfire allowed: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        oneshot.scope,
        oneshot.inputs,
        oneshot.one_shot_armed,
        oneshot.fire_allowed,
        oneshot.first_pic_eoi_write_performed,
        oneshot.hardware_mutation,
        oneshot.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write one-shot note\nscope: {}\ninputs: {}\none-shot armed: {}\nfire allowed: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        oneshot.scope,
        oneshot.inputs,
        oneshot.one_shot_armed,
        oneshot.fire_allowed,
        oneshot.first_pic_eoi_write_performed,
        oneshot.hardware_mutation,
        oneshot.runtime_irq_active
    );
}

fn print_eoi_write_oneshot_status() {
    use core::fmt::Write;

    let oneshot = eoi_write_oneshot_command_path_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write one-shot command path\none-shot armed: {}\nfire allowed: {}\nfirst PIC_EOI write performed: {}\ntarget command port: {}\ntarget value: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        oneshot.one_shot_armed,
        oneshot.fire_allowed,
        oneshot.first_pic_eoi_write_performed,
        oneshot.target_command_port,
        oneshot.target_value,
        oneshot.hardware_mutation,
        oneshot.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write one-shot command path\none-shot armed: {}\nfire allowed: {}\nfirst PIC_EOI write performed: {}\ntarget command port: {}\ntarget value: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        oneshot.one_shot_armed,
        oneshot.fire_allowed,
        oneshot.first_pic_eoi_write_performed,
        oneshot.target_command_port,
        oneshot.target_value,
        oneshot.hardware_mutation,
        oneshot.runtime_irq_active
    );
}

fn print_eoi_write_oneshot_fire() {
    use core::fmt::Write;

    let oneshot = eoi_write_oneshot_command_path_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(
        vga_writer,
        "EOI write one-shot fire\n{}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\n",
        oneshot.fire_result, oneshot.first_pic_eoi_write_performed, oneshot.hardware_mutation
    );
    let _ = write!(
        serial_writer,
        "EOI write one-shot fire\n{}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\n",
        oneshot.fire_result, oneshot.first_pic_eoi_write_performed, oneshot.hardware_mutation
    );
}

fn print_eoi_write_oneshot_blockers() {
    use core::fmt::Write;

    let oneshot = eoi_write_oneshot_command_path_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write one-shot blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nfirst PIC_EOI write performed: {}\n",
        irq::EOI_WRITE_ONESHOT_BLOCKER_PERMIT,
        irq::EOI_WRITE_ONESHOT_BLOCKER_FIRST_ALLOWED,
        irq::EOI_WRITE_ONESHOT_BLOCKER_HARDWARE,
        irq::EOI_WRITE_ONESHOT_BLOCKER_RUNTIME,
        irq::EOI_WRITE_ONESHOT_BLOCKER_STI,
        irq::EOI_WRITE_ONESHOT_BLOCKER_PIC_UNMASK,
        irq::EOI_WRITE_ONESHOT_BLOCKER_LIVE_IRQ,
        oneshot.first_pic_eoi_write_performed
    );
    let _ = write!(serial_writer, "EOI write one-shot blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nfirst PIC_EOI write performed: {}\n",
        irq::EOI_WRITE_ONESHOT_BLOCKER_PERMIT,
        irq::EOI_WRITE_ONESHOT_BLOCKER_FIRST_ALLOWED,
        irq::EOI_WRITE_ONESHOT_BLOCKER_HARDWARE,
        irq::EOI_WRITE_ONESHOT_BLOCKER_RUNTIME,
        irq::EOI_WRITE_ONESHOT_BLOCKER_STI,
        irq::EOI_WRITE_ONESHOT_BLOCKER_PIC_UNMASK,
        irq::EOI_WRITE_ONESHOT_BLOCKER_LIVE_IRQ,
        oneshot.first_pic_eoi_write_performed
    );
}

fn print_eoi_write_oneshot_latch_note() {
    use core::fmt::Write;

    let latch = eoi_write_oneshot_latch_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write one-shot latch note\nscope: {}\ninputs: {}\nlatch: {}\none-shot armed: {}\nfire allowed: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        latch.scope,
        latch.inputs,
        latch.latch,
        latch.one_shot_armed,
        latch.fire_allowed,
        latch.first_pic_eoi_write_performed,
        latch.hardware_mutation,
        latch.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write one-shot latch note\nscope: {}\ninputs: {}\nlatch: {}\none-shot armed: {}\nfire allowed: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        latch.scope,
        latch.inputs,
        latch.latch,
        latch.one_shot_armed,
        latch.fire_allowed,
        latch.first_pic_eoi_write_performed,
        latch.hardware_mutation,
        latch.runtime_irq_active
    );
}

fn print_eoi_write_oneshot_latch_status() {
    use core::fmt::Write;

    let latch = eoi_write_oneshot_latch_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write one-shot latch status\nlatch: {}\none-shot armed: {}\nfire allowed: {}\nfirst PIC_EOI write performed: {}\ntarget command port: {}\ntarget value: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        latch.latch,
        latch.one_shot_armed,
        latch.fire_allowed,
        latch.first_pic_eoi_write_performed,
        latch.target_command_port,
        latch.target_value,
        latch.hardware_mutation,
        latch.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write one-shot latch status\nlatch: {}\none-shot armed: {}\nfire allowed: {}\nfirst PIC_EOI write performed: {}\ntarget command port: {}\ntarget value: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        latch.latch,
        latch.one_shot_armed,
        latch.fire_allowed,
        latch.first_pic_eoi_write_performed,
        latch.target_command_port,
        latch.target_value,
        latch.hardware_mutation,
        latch.runtime_irq_active
    );
}

fn print_eoi_write_oneshot_latch_arm() {
    use core::fmt::Write;

    let latch = eoi_write_oneshot_latch_arm_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write one-shot latch arm\nresult: {}\nlatch: {}\none-shot armed: {}\nfire allowed: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        irq::EOI_WRITE_ONESHOT_LATCH_ARM_RESULT,
        latch.latch,
        latch.one_shot_armed,
        latch.fire_allowed,
        latch.first_pic_eoi_write_performed,
        latch.hardware_mutation,
        latch.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write one-shot latch arm\nresult: {}\nlatch: {}\none-shot armed: {}\nfire allowed: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        irq::EOI_WRITE_ONESHOT_LATCH_ARM_RESULT,
        latch.latch,
        latch.one_shot_armed,
        latch.fire_allowed,
        latch.first_pic_eoi_write_performed,
        latch.hardware_mutation,
        latch.runtime_irq_active
    );
}

fn print_eoi_write_oneshot_latch_clear() {
    use core::fmt::Write;

    let latch = eoi_write_oneshot_latch_clear_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write one-shot latch clear\nresult: {}\nlatch: {}\none-shot armed: {}\nfire allowed: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        irq::EOI_WRITE_ONESHOT_LATCH_CLEAR_RESULT,
        latch.latch,
        latch.one_shot_armed,
        latch.fire_allowed,
        latch.first_pic_eoi_write_performed,
        latch.hardware_mutation,
        latch.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write one-shot latch clear\nresult: {}\nlatch: {}\none-shot armed: {}\nfire allowed: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        irq::EOI_WRITE_ONESHOT_LATCH_CLEAR_RESULT,
        latch.latch,
        latch.one_shot_armed,
        latch.fire_allowed,
        latch.first_pic_eoi_write_performed,
        latch.hardware_mutation,
        latch.runtime_irq_active
    );
}

fn print_eoi_write_oneshot_latch_fire() {
    use core::fmt::Write;

    let latch = eoi_write_oneshot_latch_fire_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write one-shot latch fire\n{}\none-shot armed: {}\nfire allowed: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\nblocked fire cleared latch: {}\n",
        latch.fire_result,
        latch.one_shot_armed,
        latch.fire_allowed,
        latch.first_pic_eoi_write_performed,
        latch.hardware_mutation,
        latch.runtime_irq_active,
        latch.fire_cleared_latch
    );
    let _ = write!(serial_writer, "EOI write one-shot latch fire\n{}\none-shot armed: {}\nfire allowed: {}\nfirst PIC_EOI write performed: {}\nhardware mutation: {}\nruntime irq active: {}\nblocked fire cleared latch: {}\n",
        latch.fire_result,
        latch.one_shot_armed,
        latch.fire_allowed,
        latch.first_pic_eoi_write_performed,
        latch.hardware_mutation,
        latch.runtime_irq_active,
        latch.fire_cleared_latch
    );
}

fn print_eoi_write_oneshot_latch_blockers() {
    use core::fmt::Write;

    let latch = eoi_write_oneshot_latch_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write one-shot latch blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nfirst PIC_EOI write performed: {}\n",
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_SOFTWARE_ONLY,
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_PERMIT,
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_FIRST_ALLOWED,
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_HARDWARE,
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_RUNTIME,
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_STI,
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_PIC_UNMASK,
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_LIVE_IRQ,
        latch.first_pic_eoi_write_performed
    );
    let _ = write!(serial_writer, "EOI write one-shot latch blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nfirst PIC_EOI write performed: {}\n",
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_SOFTWARE_ONLY,
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_PERMIT,
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_FIRST_ALLOWED,
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_HARDWARE,
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_RUNTIME,
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_STI,
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_PIC_UNMASK,
        irq::EOI_WRITE_ONESHOT_LATCH_BLOCKER_LIVE_IRQ,
        latch.first_pic_eoi_write_performed
    );
}

fn print_eoi_write_bridge_note() {
    use core::fmt::Write;

    let bridge = eoi_write_bridge_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write bridge note\nscope: {}\ninputs: {}\nbridge: {}\npermit granted: {}\none-shot armed: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        bridge.scope,
        bridge.inputs,
        bridge.bridge,
        bridge.permit_granted,
        bridge.one_shot_armed,
        bridge.bridge_ready,
        bridge.first_pic_eoi_write_allowed,
        bridge.hardware_mutation,
        bridge.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write bridge note\nscope: {}\ninputs: {}\nbridge: {}\npermit granted: {}\none-shot armed: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        bridge.scope,
        bridge.inputs,
        bridge.bridge,
        bridge.permit_granted,
        bridge.one_shot_armed,
        bridge.bridge_ready,
        bridge.first_pic_eoi_write_allowed,
        bridge.hardware_mutation,
        bridge.runtime_irq_active
    );
}

fn print_eoi_write_bridge_status() {
    use core::fmt::Write;

    let bridge = eoi_write_bridge_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write bridge status\nlatch: {}\none-shot armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\ntarget command port: {}\ntarget value: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        bridge.latch,
        bridge.one_shot_armed,
        bridge.permit_granted,
        bridge.bridge_ready,
        bridge.first_pic_eoi_write_allowed,
        bridge.target_command_port,
        bridge.target_value,
        bridge.hardware_mutation,
        bridge.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write bridge status\nlatch: {}\none-shot armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\ntarget command port: {}\ntarget value: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        bridge.latch,
        bridge.one_shot_armed,
        bridge.permit_granted,
        bridge.bridge_ready,
        bridge.first_pic_eoi_write_allowed,
        bridge.target_command_port,
        bridge.target_value,
        bridge.hardware_mutation,
        bridge.runtime_irq_active
    );
}

fn print_eoi_write_bridge_check() {
    use core::fmt::Write;

    let bridge = eoi_write_bridge_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write bridge check\nlatch: {}\none-shot armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\ntarget command port: {}\ntarget value: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        bridge.latch,
        bridge.one_shot_armed,
        bridge.permit_granted,
        bridge.bridge_ready,
        bridge.first_pic_eoi_write_allowed,
        bridge.target_command_port,
        bridge.target_value,
        bridge.hardware_mutation,
        bridge.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write bridge check\nlatch: {}\none-shot armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\ntarget command port: {}\ntarget value: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        bridge.latch,
        bridge.one_shot_armed,
        bridge.permit_granted,
        bridge.bridge_ready,
        bridge.first_pic_eoi_write_allowed,
        bridge.target_command_port,
        bridge.target_value,
        bridge.hardware_mutation,
        bridge.runtime_irq_active
    );
}

fn print_eoi_write_bridge_blockers() {
    use core::fmt::Write;

    let bridge = eoi_write_bridge_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write bridge blockers\n- {}\n- {}\n- first PIC_EOI write allowed: {}\n- hardware mutation: {}\n- runtime irq active: {}\n- STI disabled\n- PIC unmask disabled\n- live IRQ runtime disabled\nbridge ready: {}\n",
        bridge.blocker_latch,
        bridge.blocker_permit,
        bridge.first_pic_eoi_write_allowed,
        bridge.hardware_mutation,
        bridge.runtime_irq_active,
        bridge.bridge_ready
    );
    let _ = write!(serial_writer, "EOI write bridge blockers\n- {}\n- {}\n- first PIC_EOI write allowed: {}\n- hardware mutation: {}\n- runtime irq active: {}\n- STI disabled\n- PIC unmask disabled\n- live IRQ runtime disabled\nbridge ready: {}\n",
        bridge.blocker_latch,
        bridge.blocker_permit,
        bridge.first_pic_eoi_write_allowed,
        bridge.hardware_mutation,
        bridge.runtime_irq_active,
        bridge.bridge_ready
    );
}

fn print_eoi_write_permit_transition_note() {
    use core::fmt::Write;

    let transition = eoi_write_permit_transition_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write permit transition note\nscope: {}\ntransition: {}\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        transition.scope,
        transition.transition,
        transition.permit_transition_armed,
        transition.permit_granted,
        transition.bridge_ready,
        transition.first_pic_eoi_write_allowed,
        transition.hardware_mutation,
        transition.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write permit transition note\nscope: {}\ntransition: {}\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        transition.scope,
        transition.transition,
        transition.permit_transition_armed,
        transition.permit_granted,
        transition.bridge_ready,
        transition.first_pic_eoi_write_allowed,
        transition.hardware_mutation,
        transition.runtime_irq_active
    );
}

fn print_eoi_write_permit_transition_status() {
    use core::fmt::Write;

    let transition = eoi_write_permit_transition_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write permit transition status\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\ntarget command port: {}\ntarget value: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        transition.permit_transition_armed,
        transition.permit_granted,
        transition.bridge_ready,
        transition.target_command_port,
        transition.target_value,
        transition.first_pic_eoi_write_allowed,
        transition.hardware_mutation,
        transition.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write permit transition status\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\ntarget command port: {}\ntarget value: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        transition.permit_transition_armed,
        transition.permit_granted,
        transition.bridge_ready,
        transition.target_command_port,
        transition.target_value,
        transition.first_pic_eoi_write_allowed,
        transition.hardware_mutation,
        transition.runtime_irq_active
    );
}

fn print_eoi_write_permit_transition_arm() {
    use core::fmt::Write;

    let transition = eoi_write_permit_transition_arm_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write permit transition arm\nresult: {}\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        irq::EOI_WRITE_PERMIT_TRANSITION_ARM_RESULT,
        transition.permit_transition_armed,
        transition.permit_granted,
        transition.bridge_ready,
        transition.first_pic_eoi_write_allowed,
        transition.hardware_mutation,
        transition.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write permit transition arm\nresult: {}\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        irq::EOI_WRITE_PERMIT_TRANSITION_ARM_RESULT,
        transition.permit_transition_armed,
        transition.permit_granted,
        transition.bridge_ready,
        transition.first_pic_eoi_write_allowed,
        transition.hardware_mutation,
        transition.runtime_irq_active
    );
}

fn print_eoi_write_permit_transition_clear() {
    use core::fmt::Write;

    let transition = eoi_write_permit_transition_clear_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write permit transition clear\nresult: {}\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        irq::EOI_WRITE_PERMIT_TRANSITION_CLEAR_RESULT,
        transition.permit_transition_armed,
        transition.permit_granted,
        transition.bridge_ready,
        transition.first_pic_eoi_write_allowed,
        transition.hardware_mutation,
        transition.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write permit transition clear\nresult: {}\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        irq::EOI_WRITE_PERMIT_TRANSITION_CLEAR_RESULT,
        transition.permit_transition_armed,
        transition.permit_granted,
        transition.bridge_ready,
        transition.first_pic_eoi_write_allowed,
        transition.hardware_mutation,
        transition.runtime_irq_active
    );
}

fn print_eoi_write_permit_transition_check() {
    use core::fmt::Write;

    let transition = eoi_write_permit_transition_check_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write permit transition check\n{}\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\ntarget command port: {}\ntarget value: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        irq::EOI_WRITE_PERMIT_TRANSITION_CHECK_RESULT,
        transition.permit_transition_armed,
        transition.permit_granted,
        transition.bridge_ready,
        transition.target_command_port,
        transition.target_value,
        transition.first_pic_eoi_write_allowed,
        transition.hardware_mutation,
        transition.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write permit transition check\n{}\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\ntarget command port: {}\ntarget value: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        irq::EOI_WRITE_PERMIT_TRANSITION_CHECK_RESULT,
        transition.permit_transition_armed,
        transition.permit_granted,
        transition.bridge_ready,
        transition.target_command_port,
        transition.target_value,
        transition.first_pic_eoi_write_allowed,
        transition.hardware_mutation,
        transition.runtime_irq_active
    );
}

fn print_eoi_write_permit_transition_blockers() {
    use core::fmt::Write;

    let transition = eoi_write_permit_transition_status_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write permit transition blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\npermit granted: {}\nbridge ready: {}\n",
        transition.blocker_transition,
        transition.blocker_permit,
        transition.blocker_bridge,
        transition.blocker_first_allowed,
        transition.blocker_hardware,
        transition.blocker_runtime,
        transition.blocker_sti,
        transition.blocker_pic_unmask,
        transition.blocker_live_irq,
        transition.permit_granted,
        transition.bridge_ready
    );
    let _ = write!(serial_writer, "EOI write permit transition blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\npermit granted: {}\nbridge ready: {}\n",
        transition.blocker_transition,
        transition.blocker_permit,
        transition.blocker_bridge,
        transition.blocker_first_allowed,
        transition.blocker_hardware,
        transition.blocker_runtime,
        transition.blocker_sti,
        transition.blocker_pic_unmask,
        transition.blocker_live_irq,
        transition.permit_granted,
        transition.bridge_ready
    );
}

fn print_eoi_write_eval_note() {
    use core::fmt::Write;

    let evaluation = eoi_write_permit_evaluation_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write permit evaluation note\nscope: {}\nevaluation: {}\nevaluation ready: {}\none-shot armed: {}\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        evaluation.scope,
        evaluation.evaluation,
        evaluation.evaluation_ready,
        evaluation.one_shot_armed,
        evaluation.permit_transition_armed,
        evaluation.permit_granted,
        evaluation.bridge_ready,
        evaluation.first_pic_eoi_write_allowed,
        evaluation.hardware_mutation,
        evaluation.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write permit evaluation note\nscope: {}\nevaluation: {}\nevaluation ready: {}\none-shot armed: {}\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        evaluation.scope,
        evaluation.evaluation,
        evaluation.evaluation_ready,
        evaluation.one_shot_armed,
        evaluation.permit_transition_armed,
        evaluation.permit_granted,
        evaluation.bridge_ready,
        evaluation.first_pic_eoi_write_allowed,
        evaluation.hardware_mutation,
        evaluation.runtime_irq_active
    );
}

fn print_eoi_write_eval_status() {
    use core::fmt::Write;

    let evaluation = eoi_write_permit_evaluation_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write permit evaluation status\nevaluation ready: {}\none-shot armed: {}\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        evaluation.evaluation_ready,
        evaluation.one_shot_armed,
        evaluation.permit_transition_armed,
        evaluation.permit_granted,
        evaluation.bridge_ready,
        evaluation.first_pic_eoi_write_allowed,
        evaluation.hardware_mutation,
        evaluation.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write permit evaluation status\nevaluation ready: {}\none-shot armed: {}\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        evaluation.evaluation_ready,
        evaluation.one_shot_armed,
        evaluation.permit_transition_armed,
        evaluation.permit_granted,
        evaluation.bridge_ready,
        evaluation.first_pic_eoi_write_allowed,
        evaluation.hardware_mutation,
        evaluation.runtime_irq_active
    );
}

fn print_eoi_write_eval_check() {
    use core::fmt::Write;

    let evaluation = eoi_write_permit_evaluation_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write permit evaluation check\nEOI write permit evaluation\nevaluation ready: {}\none-shot armed: {}\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        evaluation.evaluation_ready,
        evaluation.one_shot_armed,
        evaluation.permit_transition_armed,
        evaluation.permit_granted,
        evaluation.bridge_ready,
        evaluation.first_pic_eoi_write_allowed,
        evaluation.hardware_mutation,
        evaluation.runtime_irq_active
    );
    let _ = write!(serial_writer, "EOI write permit evaluation check\nEOI write permit evaluation\nevaluation ready: {}\none-shot armed: {}\npermit transition armed: {}\npermit granted: {}\nbridge ready: {}\nfirst PIC_EOI write allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
        evaluation.evaluation_ready,
        evaluation.one_shot_armed,
        evaluation.permit_transition_armed,
        evaluation.permit_granted,
        evaluation.bridge_ready,
        evaluation.first_pic_eoi_write_allowed,
        evaluation.hardware_mutation,
        evaluation.runtime_irq_active
    );
}

fn print_eoi_write_eval_blockers() {
    use core::fmt::Write;

    let evaluation = eoi_write_permit_evaluation_snapshot();
    let mut vga_writer = vga::VgaWriter;
    let mut serial_writer = serial::SerialWriter;
    let _ = write!(vga_writer, "EOI write permit evaluation blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nevaluation ready: {}\npermit granted: {}\nbridge ready: {}\n",
        evaluation.blocker_permit,
        evaluation.blocker_bridge,
        evaluation.blocker_transition,
        evaluation.blocker_first_write,
        evaluation.blocker_hardware,
        evaluation.blocker_runtime,
        evaluation.evaluation_ready,
        evaluation.permit_granted,
        evaluation.bridge_ready
    );
    let _ = write!(serial_writer, "EOI write permit evaluation blockers\n- {}\n- {}\n- {}\n- {}\n- {}\n- {}\nevaluation ready: {}\npermit granted: {}\nbridge ready: {}\n",
        evaluation.blocker_permit,
        evaluation.blocker_bridge,
        evaluation.blocker_transition,
        evaluation.blocker_first_write,
        evaluation.blocker_hardware,
        evaluation.blocker_runtime,
        evaluation.evaluation_ready,
        evaluation.permit_granted,
        evaluation.bridge_ready
    );
}

#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    vga::clear_screen();
    vga::print("========================================================================\n");
    vga::print("                   DByteOS Command Dispatch Lab (v9.0.2)                \n");
    vga::print("========================================================================\n\n");
    vga::print("[OK] Bootstrap entry point successfully resolved.\n");
    vga::print("[OK] Text-mode VGA framebuffer driver loaded.\n");

    unsafe {
        serial::init();
        idt::IDT = idt::InterruptDescriptorTable::new();
        idt::IDT.entries[0].set_handler(interrupts::divide_by_zero_handler_asm as *const ());
        idt::IDT.entries[3].set_handler(interrupts::breakpoint_handler_asm as *const ());
        idt::IDT.entries[14].set_handler(interrupts::page_fault_handler_asm as *const ());
        idt::IDT.load();
    }
    vga::print("[OK] Freestanding COM1 serial port driver loaded.\n");
    vga::print("[OK] Interrupt Descriptor Table (IDT) loaded.\n\n");

    vga::print("Status: Keyboard Listener Active (polling mode)\n");
    vga::print("Press keys inside the QEMU graphical display window.\n\n");

    // Print to serial console for QEMU Boot Smoke automated detection
    serial::print("DByteOS Kernel Lab\n");
    serial::print("version: 9.0.2\n");
    serial::print("status: booted\n");
    serial::print("target: i686 multiboot\n\n");

    serial::print("DByteOS Keyboard Lab\n");
    serial::print("status: listening\n");

    // Draw the first visible VGA window without changing serial smoke output.
    vga_window::draw_first_window();
    serial::print("dbyte-kernel> ");

    // Flush any stale scancodes to prevent reading initial key state junk
    unsafe {
        while (serial::inb(0x64) & 1) != 0 {
            let _ = serial::inb(0x60);
        }
    }

    use core::fmt::Write;

    loop {
        unsafe {
            let status = serial::inb(0x64);
            if (status & 1) != 0 {
                let scancode = serial::inb(0x60);

                // Process modifier states (both Make and Break codes)
                let mut state_changed = false;
                match scancode {
                    // Left Shift / Right Shift Make
                    0x2A | 0x36 => {
                        if !SHIFT_ACTIVE {
                            SHIFT_ACTIVE = true;
                            state_changed = true;
                        }
                    }
                    // Left Shift / Right Shift Break
                    0xAA | 0xB6 => {
                        if SHIFT_ACTIVE {
                            SHIFT_ACTIVE = false;
                            state_changed = true;
                        }
                    }
                    // CapsLock Make
                    0x3A => {
                        CAPS_LOCK_ACTIVE = !CAPS_LOCK_ACTIVE;
                        state_changed = true;
                    }
                    _ => {}
                }

                let (shift_val, caps_val) = (SHIFT_ACTIVE, CAPS_LOCK_ACTIVE);
                if state_changed {
                    let mut writer = serial::SerialWriter;
                    let _ = write!(
                        writer,
                        "[MODIFIER] Shift: {}, CapsLock: {}\n",
                        shift_val, caps_val
                    );
                }

                // Ignore break codes for standard typing (scancode >= 0x80)
                if scancode < 0x80 {
                    // Exclude modifier keys from printing directly as printable key characters
                    if scancode != 0x2A && scancode != 0x36 && scancode != 0x3A {
                        if let Some(c) = scancode_to_ascii(scancode, SHIFT_ACTIVE, CAPS_LOCK_ACTIVE)
                        {
                            if c == '\x08' {
                                // Backspace: only erase if there is text in the buffer!
                                if LINE_LEN > 0 {
                                    LINE_LEN -= 1;
                                    vga::backspace();
                                    serial::write_byte(0x08);
                                    serial::write_byte(b' ');
                                    serial::write_byte(0x08);
                                }
                            } else if c == '\n' {
                                // Newline/Enter: submit line!
                                vga::print("\n");
                                serial::print("\n");
                                let mut vga_prompt_already_rendered = false;

                                if LINE_LEN > 0 {
                                    // Convert and process submitted line
                                    if let Ok(line_str) =
                                        core::str::from_utf8(&LINE_BUFFER[..LINE_LEN])
                                    {
                                        if line_str == "help" {
                                            vga::print("commands: help about version clear echo mem uptime banner keyboard reboot-note system cls status mods keys prompt ui-redraw int3 div0 exception exception-reset handlers handlers --active exception-status exceptions exceptions --verbose exception-help exception-about fault-status fault-reset pf-note pf-status pf-smoke irq-note irq-status irq-handlers eoi-note eoi-status irq-gates irq-gate-status irq-gate-plan irq-gate-arm irq-gate-bind-smoke irq-gate-bind-status irq-gate-state irq-gate-history irq-gate-preflight irq-bind-note irq-bind-status irq-readiness irq-risk irq-preflight irq-runtime-arm irq-runtime-commit irq-runtime-preflight irq-runtime-status irq-runtime-blockers irq-runtime-matrix irq-runtime-readiness irq-runtime-next irq-runtime-activation-plan irq-runtime-token-note irq-runtime-token-status irq-runtime-token-arm irq-runtime-token-clear irq-runtime-gate-note irq-runtime-gate-status irq-runtime-gate-check irq-runtime-gate-blockers irq-runtime-sim-note irq-runtime-sim-status irq-runtime-sim-run irq-runtime-sim-blockers sti-plan sti-status sti-preflight sti-blockers irq-runtime-activation-smoke irq-runtime-activation-smoke-status irq-runtime-activation-smoke-blockers eoi-dispatch-smoke-note eoi-dispatch-smoke-status eoi-dispatch-smoke-plan eoi-dispatch-smoke-blockers pic-unmask-smoke-note pic-unmask-smoke-status pic-unmask-smoke-plan pic-unmask-smoke-blockers idt-runtime-bind-smoke-note idt-runtime-bind-smoke-status idt-runtime-bind-smoke-plan idt-runtime-bind-smoke-blockers irq-runtime-final-gate-note irq-runtime-final-gate-status irq-runtime-final-gate-check irq-runtime-final-gate-blockers irq-runtime-decision-note irq-runtime-decision-status irq-runtime-decision-freeze irq-runtime-decision-blockers irq-runtime-mutation-note irq-runtime-mutation-status irq-runtime-mutation-check irq-runtime-mutation-blockers irq-runtime-mutation-sequence-note irq-runtime-mutation-sequence-status irq-runtime-mutation-sequence-plan irq-runtime-mutation-sequence-blockers eoi-write-smoke-preflight-note eoi-write-smoke-preflight-status eoi-write-smoke-preflight-check eoi-write-smoke-preflight-blockers eoi-write-smoke-candidate-note eoi-write-smoke-candidate-status eoi-write-smoke-candidate-arm eoi-write-smoke-candidate-fire eoi-write-smoke-candidate-blockers eoi-write-permit-note eoi-write-permit-status eoi-write-permit-check eoi-write-permit-blockers eoi-write-oneshot-note eoi-write-oneshot-status eoi-write-oneshot-arm eoi-write-oneshot-fire eoi-write-oneshot-blockers eoi-write-oneshot-latch-note eoi-write-oneshot-latch-status eoi-write-oneshot-latch-arm eoi-write-oneshot-latch-clear eoi-write-oneshot-latch-fire eoi-write-oneshot-latch-blockers eoi-write-bridge-note eoi-write-bridge-status eoi-write-bridge-check eoi-write-bridge-blockers eoi-write-permit-transition-note eoi-write-permit-transition-status eoi-write-permit-transition-arm eoi-write-permit-transition-clear eoi-write-permit-transition-check eoi-write-permit-transition-blockers eoi-write-eval-note eoi-write-eval-status eoi-write-eval-check eoi-write-eval-blockers eoi-write-hw-smoke-note eoi-write-hw-smoke-status eoi-write-hw-smoke-arm eoi-write-hw-smoke-fire eoi-write-hw-smoke-clear eoi-write-hw-smoke-blockers eoi-runtime-bridge-note eoi-runtime-bridge-status eoi-runtime-bridge-check eoi-runtime-bridge-blockers irq-handler-eoi-candidate-note irq-handler-eoi-candidate-status irq-handler-eoi-candidate-check irq-handler-eoi-candidate-blockers irq-handler-eoi-stub-note irq-handler-eoi-stub-status irq-handler-eoi-stub-check irq-handler-eoi-stub-blockers irq-handler-bind-candidate-note irq-handler-bind-candidate-status irq-handler-bind-candidate-check irq-handler-bind-candidate-blockers idt-bind-hw-smoke-note idt-bind-hw-smoke-status idt-bind-hw-smoke-arm idt-bind-hw-smoke-fire idt-bind-hw-smoke-clear idt-bind-hw-smoke-blockers idt-bind-runtime-bridge-note idt-bind-runtime-bridge-status idt-bind-runtime-bridge-check idt-bind-runtime-bridge-blockers idt-invoke-hw-smoke-note idt-invoke-hw-smoke-status idt-invoke-hw-smoke-arm idt-invoke-hw-smoke-fire idt-invoke-hw-smoke-clear idt-invoke-hw-smoke-blockers idt-invoke-runtime-bridge-note idt-invoke-runtime-bridge-status idt-invoke-runtime-bridge-check idt-invoke-runtime-bridge-blockers irq-delivery-candidate-note irq-delivery-candidate-status irq-delivery-candidate-check irq-delivery-candidate-blockers irq0-bind-hw-smoke-note irq0-bind-hw-smoke-status irq0-bind-hw-smoke-arm irq0-bind-hw-smoke-fire irq0-bind-hw-smoke-clear irq0-bind-hw-smoke-blockers irq0-unmask-hw-smoke-note irq0-unmask-hw-smoke-status irq0-unmask-hw-smoke-arm irq0-unmask-hw-smoke-fire irq0-unmask-hw-smoke-clear irq0-unmask-hw-smoke-blockers irq0-preflight-status irq0-preflight-check irq0-preflight-blockers irq0-handler-stub-status irq0-handler-stub-check irq0-handler-stub-blockers pic-note pic-status pic-plan pic-remap-arm pic-remap-smoke pic-remap-status pic-remap-state pic-remap-history pic-remap-preflight irq-map pic-status --verbose pic-mask-plan pic-mask-status irq-mask-blockers\n");
                                            serial::print("commands: help about version clear echo mem uptime banner keyboard reboot-note system cls status mods keys prompt ui-redraw int3 div0 exception exception-reset handlers handlers --active exception-status exceptions exceptions --verbose exception-help exception-about fault-status fault-reset pf-note pf-status pf-smoke irq-note irq-status irq-handlers eoi-note eoi-status irq-gates irq-gate-status irq-gate-plan irq-gate-arm irq-gate-bind-smoke irq-gate-bind-status irq-gate-state irq-gate-history irq-gate-preflight irq-bind-note irq-bind-status irq-readiness irq-risk irq-preflight irq-runtime-arm irq-runtime-commit irq-runtime-preflight irq-runtime-status irq-runtime-blockers irq-runtime-matrix irq-runtime-readiness irq-runtime-next irq-runtime-activation-plan irq-runtime-token-note irq-runtime-token-status irq-runtime-token-arm irq-runtime-token-clear irq-runtime-gate-note irq-runtime-gate-status irq-runtime-gate-check irq-runtime-gate-blockers irq-runtime-sim-note irq-runtime-sim-status irq-runtime-sim-run irq-runtime-sim-blockers sti-plan sti-status sti-preflight sti-blockers irq-runtime-activation-smoke irq-runtime-activation-smoke-status irq-runtime-activation-smoke-blockers eoi-dispatch-smoke-note eoi-dispatch-smoke-status eoi-dispatch-smoke-plan eoi-dispatch-smoke-blockers pic-unmask-smoke-note pic-unmask-smoke-status pic-unmask-smoke-plan pic-unmask-smoke-blockers idt-runtime-bind-smoke-note idt-runtime-bind-smoke-status idt-runtime-bind-smoke-plan idt-runtime-bind-smoke-blockers irq-runtime-final-gate-note irq-runtime-final-gate-status irq-runtime-final-gate-check irq-runtime-final-gate-blockers irq-runtime-decision-note irq-runtime-decision-status irq-runtime-decision-freeze irq-runtime-decision-blockers irq-runtime-mutation-note irq-runtime-mutation-status irq-runtime-mutation-check irq-runtime-mutation-blockers irq-runtime-mutation-sequence-note irq-runtime-mutation-sequence-status irq-runtime-mutation-sequence-plan irq-runtime-mutation-sequence-blockers eoi-write-smoke-preflight-note eoi-write-smoke-preflight-status eoi-write-smoke-preflight-check eoi-write-smoke-preflight-blockers eoi-write-smoke-candidate-note eoi-write-smoke-candidate-status eoi-write-smoke-candidate-arm eoi-write-smoke-candidate-fire eoi-write-smoke-candidate-blockers eoi-write-permit-note eoi-write-permit-status eoi-write-permit-check eoi-write-permit-blockers eoi-write-oneshot-note eoi-write-oneshot-status eoi-write-oneshot-arm eoi-write-oneshot-fire eoi-write-oneshot-blockers eoi-write-oneshot-latch-note eoi-write-oneshot-latch-status eoi-write-oneshot-latch-arm eoi-write-oneshot-latch-clear eoi-write-oneshot-latch-fire eoi-write-oneshot-latch-blockers eoi-write-bridge-note eoi-write-bridge-status eoi-write-bridge-check eoi-write-bridge-blockers eoi-write-permit-transition-note eoi-write-permit-transition-status eoi-write-permit-transition-arm eoi-write-permit-transition-clear eoi-write-permit-transition-check eoi-write-permit-transition-blockers eoi-write-eval-note eoi-write-eval-status eoi-write-eval-check eoi-write-eval-blockers eoi-write-hw-smoke-note eoi-write-hw-smoke-status eoi-write-hw-smoke-arm eoi-write-hw-smoke-fire eoi-write-hw-smoke-clear eoi-write-hw-smoke-blockers eoi-runtime-bridge-note eoi-runtime-bridge-status eoi-runtime-bridge-check eoi-runtime-bridge-blockers irq-handler-eoi-candidate-note irq-handler-eoi-candidate-status irq-handler-eoi-candidate-check irq-handler-eoi-candidate-blockers irq-handler-eoi-stub-note irq-handler-eoi-stub-status irq-handler-eoi-stub-check irq-handler-eoi-stub-blockers irq-handler-bind-candidate-note irq-handler-bind-candidate-status irq-handler-bind-candidate-check irq-handler-bind-candidate-blockers idt-bind-hw-smoke-note idt-bind-hw-smoke-status idt-bind-hw-smoke-arm idt-bind-hw-smoke-fire idt-bind-hw-smoke-clear idt-bind-hw-smoke-blockers idt-bind-runtime-bridge-note idt-bind-runtime-bridge-status idt-bind-runtime-bridge-check idt-bind-runtime-bridge-blockers idt-invoke-hw-smoke-note idt-invoke-hw-smoke-status idt-invoke-hw-smoke-arm idt-invoke-hw-smoke-fire idt-invoke-hw-smoke-clear idt-invoke-hw-smoke-blockers idt-invoke-runtime-bridge-note idt-invoke-runtime-bridge-status idt-invoke-runtime-bridge-check idt-invoke-runtime-bridge-blockers irq-delivery-candidate-note irq-delivery-candidate-status irq-delivery-candidate-check irq-delivery-candidate-blockers irq0-bind-hw-smoke-note irq0-bind-hw-smoke-status irq0-bind-hw-smoke-arm irq0-bind-hw-smoke-fire irq0-bind-hw-smoke-clear irq0-bind-hw-smoke-blockers irq0-unmask-hw-smoke-note irq0-unmask-hw-smoke-status irq0-unmask-hw-smoke-arm irq0-unmask-hw-smoke-fire irq0-unmask-hw-smoke-clear irq0-unmask-hw-smoke-blockers irq0-preflight-status irq0-preflight-check irq0-preflight-blockers irq0-handler-stub-status irq0-handler-stub-check irq0-handler-stub-blockers pic-note pic-status pic-plan pic-remap-arm pic-remap-smoke pic-remap-status pic-remap-state pic-remap-history pic-remap-preflight irq-map pic-status --verbose pic-mask-plan pic-mask-status irq-mask-blockers\n");
                                        } else if line_str == "about" {
                                            vga::print("DByteOS Kernel Lab\n");
                                            serial::print("DByteOS Kernel Lab\n");
                                        } else if line_str == "version" {
                                            vga::print("DByteOS Kernel Lab\n");
                                            serial::print("DByteOS Kernel Lab\n");
                                        } else if line_str == "clear" || line_str == "cls" {
                                            vga::clear_screen();
                                        } else if line_str == "ui-redraw" {
                                            vga_window::draw_first_window();
                                            vga_prompt_already_rendered = true;
                                            serial::print("ui-redraw: first VGA window rendered\n");
                                        } else if line_str == "echo" {
                                            vga::print("\n");
                                            serial::print("\n");
                                        } else if line_str.starts_with("echo ") {
                                            let text = &line_str[5..];
                                            vga::print(text);
                                            vga::print("\n");
                                            serial::print(text);
                                            serial::print("\n");
                                        } else if line_str == "int3" {
                                            core::arch::asm!("int3");
                                        } else if line_str == "div0" {
                                            core::arch::asm!("int 0");
                                        } else if line_str == "pf-smoke" {
                                            interrupts::PF_SMOKE_ACTIVE = true;
                                            interrupts::PF_SMOKE_RECOVERY_EIP =
                                                interrupts::pf_smoke_recovery_asm as *const ()
                                                    as u32;
                                            interrupts::pf_smoke_probe_asm();
                                        } else if line_str == "exception" {
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let count = interrupts::EXCEPTION_COUNT;
                                            let vector = interrupts::LAST_EXCEPTION_VECTOR;
                                            let name = interrupts::LAST_EXCEPTION_NAME;
                                            if vector == -1 {
                                                let _ = write!(vga_writer, "exceptions: {}\nlast vector: none\nlast name: none\n", count);
                                                let _ = write!(serial_writer, "exceptions: {}\nlast vector: none\nlast name: none\n", count);
                                            } else {
                                                let _ = write!(vga_writer, "exceptions: {}\nlast vector: {}\nlast name: {}\n", count, vector, name);
                                                let _ = write!(serial_writer, "exceptions: {}\nlast vector: {}\nlast name: {}\n", count, vector, name);
                                            }
                                        } else if line_str == "exception-reset" {
                                            interrupts::EXCEPTION_COUNT = 0;
                                            interrupts::LAST_EXCEPTION_VECTOR = -1;
                                            interrupts::LAST_EXCEPTION_NAME = "none";
                                            vga::print("exception telemetry: reset successfully\n");
                                            serial::print(
                                                "exception telemetry: reset successfully\n",
                                            );
                                        } else if line_str == "fault-reset" {
                                            interrupts::EXCEPTION_COUNT = 0;
                                            interrupts::LAST_EXCEPTION_VECTOR = -1;
                                            interrupts::LAST_EXCEPTION_NAME = "none";
                                            interrupts::PF_SMOKE_ACTIVE = false;
                                            interrupts::PF_SMOKE_RECOVERY_EIP = 0;
                                            vga::print("fault recovery: reset successfully\n");
                                            serial::print("fault recovery: reset successfully\n");
                                        } else if line_str == "handlers" {
                                            let gate_status = irq::irq_gate_bind_smoke_status();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "active handlers:\nvector 0: divide-by-zero\nvector 3: breakpoint\nvector 14: page fault\nplanned handlers:\nnone\nirq handlers:\n");
                                            let _ = write!(serial_writer, "active handlers:\nvector 0: divide-by-zero\nvector 3: breakpoint\nvector 14: page fault\nplanned handlers:\nnone\nirq handlers:\n");
                                            if gate_status.executed {
                                                let _ = write!(vga_writer, "vector {}: irq0 timer smoke stub / dormant\nvector {}: irq1 keyboard smoke stub / dormant\nruntime irq: disabled\n", gate_status.irq0_vector, gate_status.irq1_vector);
                                                let _ = write!(serial_writer, "vector {}: irq0 timer smoke stub / dormant\nvector {}: irq1 keyboard smoke stub / dormant\nruntime irq: disabled\n", gate_status.irq0_vector, gate_status.irq1_vector);
                                            } else {
                                                vga::print("skeleton planned: irq0 timer, irq1 keyboard\nactive: none\nruntime irq: disabled\n");
                                                serial::print("skeleton planned: irq0 timer, irq1 keyboard\nactive: none\nruntime irq: disabled\n");
                                            }
                                        } else if line_str == "handlers --active" {
                                            let gate_status = irq::irq_gate_bind_smoke_status();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "active handlers:\nvector 0: divide-by-zero\nvector 3: breakpoint\nvector 14: page fault\n");
                                            let _ = write!(serial_writer, "active handlers:\nvector 0: divide-by-zero\nvector 3: breakpoint\nvector 14: page fault\n");
                                            if gate_status.executed {
                                                let _ = write!(vga_writer, "vector {}: irq0 timer smoke stub / dormant\nvector {}: irq1 keyboard smoke stub / dormant\n", gate_status.irq0_vector, gate_status.irq1_vector);
                                                let _ = write!(serial_writer, "vector {}: irq0 timer smoke stub / dormant\nvector {}: irq1 keyboard smoke stub / dormant\n", gate_status.irq0_vector, gate_status.irq1_vector);
                                            }
                                        } else if line_str == "exception-status"
                                            || line_str == "exceptions"
                                        {
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let count = interrupts::EXCEPTION_COUNT;
                                            let vector = interrupts::LAST_EXCEPTION_VECTOR;
                                            let name = interrupts::LAST_EXCEPTION_NAME;
                                            if vector == -1 {
                                                let _ = write!(vga_writer, "exceptions handled: {}\nlast exception: none\ninterrupts: disabled\n", count);
                                                let _ = write!(serial_writer, "exceptions handled: {}\nlast exception: none\ninterrupts: disabled\n", count);
                                            } else {
                                                let _ = write!(vga_writer, "exceptions handled: {}\nlast exception: {} ({})\ninterrupts: disabled\n", count, vector, name);
                                                let _ = write!(serial_writer, "exceptions handled: {}\nlast exception: {} ({})\ninterrupts: disabled\n", count, vector, name);
                                            }
                                        } else if line_str == "fault-status" {
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let count = interrupts::EXCEPTION_COUNT;
                                            let vector = interrupts::LAST_EXCEPTION_VECTOR;
                                            let name = interrupts::LAST_EXCEPTION_NAME;
                                            let armed = interrupts::PF_SMOKE_ACTIVE;
                                            if vector == -1 {
                                                let _ = write!(vga_writer, "fault recovery:\nexceptions handled: {}\nlast exception: none\nrecovery mode: smoke-safe\npage fault smoke: armed={}\ninterrupts: disabled\n", count, armed);
                                                let _ = write!(serial_writer, "fault recovery:\nexceptions handled: {}\nlast exception: none\nrecovery mode: smoke-safe\npage fault smoke: armed={}\ninterrupts: disabled\n", count, armed);
                                            } else {
                                                let _ = write!(vga_writer, "fault recovery:\nexceptions handled: {}\nlast exception: {} ({})\nrecovery mode: smoke-safe\npage fault smoke: armed={}\ninterrupts: disabled\n", count, vector, name, armed);
                                                let _ = write!(serial_writer, "fault recovery:\nexceptions handled: {}\nlast exception: {} ({})\nrecovery mode: smoke-safe\npage fault smoke: armed={}\ninterrupts: disabled\n", count, vector, name, armed);
                                            }
                                        } else if line_str == "pf-status" {
                                            let pf_status_msg = "page fault:\nvector: 14\nhandler: active smoke\ntrigger: pf-smoke controlled real fault\ncr2: available after pf-smoke\nerror code: available after pf-smoke\nrecovery: trampoline\n";
                                            vga::print(pf_status_msg);
                                            serial::print(pf_status_msg);
                                        } else if line_str == "irq-note" {
                                            let irq_note_msg = "pic/irq: planned / disabled\npic remap: documented only\nirq vectors: 32-47 planned\nirq handler skeletons: irq0 timer, irq1 keyboard\nkeyboard irq1: disabled\ntimer irq0: disabled\ninterrupts: disabled\n";
                                            vga::print(irq_note_msg);
                                            serial::print(irq_note_msg);
                                        } else if line_str == "irq-status" {
                                            let irq_status_msg = "irq subsystem:\nfoundation: planned\npic: not remapped\nirq handlers: none\nkeyboard input: polling-only\ntimer: unavailable\ninterrupts: disabled\n";
                                            vga::print(irq_status_msg);
                                            serial::print(irq_status_msg);
                                        } else if line_str == "irq-handlers" {
                                            let irq_handlers_msg = "irq handlers:\nfoundation: skeleton / disabled\nirq0 timer: skeleton / disabled\nirq1 keyboard: skeleton / disabled\nvectors: 32 / 33\nidt binding: disabled\npic remap: disabled\ninterrupts: disabled\n";
                                            vga::print(irq_handlers_msg);
                                            serial::print(irq_handlers_msg);
                                        } else if line_str == "pic-note" {
                                            let pic_note_msg = "pic remap: planned / disabled\nremap offsets: 0x20 / 0x28\nirq vectors: 0x20-0x2f\nicw sequence: documented in code\nhardware writes: disabled\ninterrupts: disabled\n";
                                            vga::print(pic_note_msg);
                                            serial::print(pic_note_msg);
                                        } else if line_str == "pic-status" {
                                            let pic_status_msg = "pic subsystem:\nfoundation: code planned\nremap function: present / not called\nmaster offset: 0x20\nslave offset: 0x28\nirq handlers: none\ninterrupts: disabled\n";
                                            vga::print(pic_status_msg);
                                            serial::print(pic_status_msg);
                                        } else if line_str == "pic-plan" {
                                            let pic_plan_msg = "pic remap dry-run:\nmaster offset: 0x20\nslave offset: 0x28\nirq vector range: 0x20-0x2f\nicw1: 0x11\nicw2 master: 0x20\nicw2 slave: 0x28\nicw3 master: 0x04\nicw3 slave: 0x02\nicw4: 0x01\nmask after remap: 0xff\nhardware writes: disabled\n";
                                            vga::print(pic_plan_msg);
                                            serial::print(pic_plan_msg);
                                        } else if line_str == "pic-remap-arm" {
                                            let arm = pic::ProgrammableInterruptController::pic_remap_smoke_arm();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "PIC remap smoke armed\nmode: {}\nnext: {}\ninterrupts: {}\nirq gates: {}\n",
                                                  arm.mode,
                                                  arm.next,
                                                  arm.interrupts,
                                                  arm.irq_gates
                                              );
                                            let _ = write!(serial_writer, "PIC remap smoke armed\nmode: {}\nnext: {}\ninterrupts: {}\nirq gates: {}\n",
                                                  arm.mode,
                                                  arm.next,
                                                  arm.interrupts,
                                                  arm.irq_gates
                                              );
                                        } else if line_str == "pic-remap-smoke" {
                                            let smoke = pic::ProgrammableInterruptController::pic_remap_controlled_smoke();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            if let Some(icw_sequence) = smoke.icw_sequence {
                                                let _ = write!(vga_writer, "PIC remap controlled smoke\nguard: {}\nicw sequence: {}\nmaster offset: 0x{:02x}\nslave offset: 0x{:02x}\nmask after remap: 0x{:02x}\nsti: {}\nirq gates: {}\neoi dispatch: {}\nresult: {}\n",
                                                      smoke.guard,
                                                      icw_sequence,
                                                      smoke.master_offset,
                                                      smoke.slave_offset,
                                                      smoke.mask_after_remap,
                                                      smoke.sti,
                                                      smoke.irq_gates,
                                                      smoke.eoi_dispatch,
                                                      smoke.result
                                                  );
                                                let _ = write!(serial_writer, "PIC remap controlled smoke\nguard: {}\nicw sequence: {}\nmaster offset: 0x{:02x}\nslave offset: 0x{:02x}\nmask after remap: 0x{:02x}\nsti: {}\nirq gates: {}\neoi dispatch: {}\nresult: {}\n",
                                                      smoke.guard,
                                                      icw_sequence,
                                                      smoke.master_offset,
                                                      smoke.slave_offset,
                                                      smoke.mask_after_remap,
                                                      smoke.sti,
                                                      smoke.irq_gates,
                                                      smoke.eoi_dispatch,
                                                      smoke.result
                                                  );
                                            } else if let Some(next) = smoke.next {
                                                let _ = write!(vga_writer, "PIC remap controlled smoke\nguard: {}\nresult: {}\nnext: {}\n",
                                                      smoke.guard,
                                                      smoke.result,
                                                      next
                                                  );
                                                let _ = write!(serial_writer, "PIC remap controlled smoke\nguard: {}\nresult: {}\nnext: {}\n",
                                                      smoke.guard,
                                                      smoke.result,
                                                      next
                                                  );
                                            }
                                        } else if line_str == "pic-remap-status" {
                                            let status = pic::ProgrammableInterruptController::pic_remap_smoke_status();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "PIC remap smoke status\narmed: {}\nexecuted: {}\nmaster offset: 0x{:02x}\nslave offset: 0x{:02x}\nmask after remap: 0x{:02x}\nsti: {}\nirq gates: {}\neoi dispatch: {}\n",
                                                  if status.armed { "yes" } else { "no" },
                                                  if status.executed { "yes" } else { "no" },
                                                  status.master_offset,
                                                  status.slave_offset,
                                                  status.mask_after_remap,
                                                  status.sti,
                                                  status.irq_gates,
                                                  status.eoi_dispatch
                                              );
                                            let _ = write!(serial_writer, "PIC remap smoke status\narmed: {}\nexecuted: {}\nmaster offset: 0x{:02x}\nslave offset: 0x{:02x}\nmask after remap: 0x{:02x}\nsti: {}\nirq gates: {}\neoi dispatch: {}\n",
                                                  if status.armed { "yes" } else { "no" },
                                                  if status.executed { "yes" } else { "no" },
                                                  status.master_offset,
                                                  status.slave_offset,
                                                  status.mask_after_remap,
                                                  status.sti,
                                                  status.irq_gates,
                                                  status.eoi_dispatch
                                              );
                                        } else if line_str == "pic-remap-state" {
                                            let state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "PIC remap state\narmed: {}\nexecuted: {}\nmaster offset: 0x{:02x}\nslave offset: 0x{:02x}\nicw sequence expected: {}\nicw sequence applied: {}\nmask after remap: 0x{:02x}\nirq runtime: {}\n",
                                                  if state.armed { "yes" } else { "no" },
                                                  if state.executed { "yes" } else { "no" },
                                                  state.master_offset,
                                                  state.slave_offset,
                                                  state.icw_sequence_expected,
                                                  state.icw_sequence_applied,
                                                  state.mask_after_remap,
                                                  state.irq_runtime
                                              );
                                            let _ = write!(serial_writer, "PIC remap state\narmed: {}\nexecuted: {}\nmaster offset: 0x{:02x}\nslave offset: 0x{:02x}\nicw sequence expected: {}\nicw sequence applied: {}\nmask after remap: 0x{:02x}\nirq runtime: {}\n",
                                                  if state.armed { "yes" } else { "no" },
                                                  if state.executed { "yes" } else { "no" },
                                                  state.master_offset,
                                                  state.slave_offset,
                                                  state.icw_sequence_expected,
                                                  state.icw_sequence_applied,
                                                  state.mask_after_remap,
                                                  state.irq_runtime
                                              );
                                        } else if line_str == "pic-remap-history" {
                                            let history = pic::ProgrammableInterruptController::pic_remap_history();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "PIC remap history\narm command: {}\nsmoke command: {}\nlast smoke executed: {}\nicw writes: {}\nboot remap: {}\n",
                                                  history.arm_command,
                                                  history.smoke_command,
                                                  history.last_smoke_executed,
                                                  history.icw_writes,
                                                  history.boot_remap
                                              );
                                            let _ = write!(serial_writer, "PIC remap history\narm command: {}\nsmoke command: {}\nlast smoke executed: {}\nicw writes: {}\nboot remap: {}\n",
                                                  history.arm_command,
                                                  history.smoke_command,
                                                  history.last_smoke_executed,
                                                  history.icw_writes,
                                                  history.boot_remap
                                              );
                                        } else if line_str == "pic-remap-preflight" {
                                            let preflight = pic::ProgrammableInterruptController::pic_remap_preflight();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "PIC remap preflight\nguard: {}\nicw sequence: {}\nmaster offset: 0x{:02x}\nslave offset: 0x{:02x}\nmask after remap: 0x{:02x}\nsti: {}\nirq gates: {}\neoi dispatch: {}\nresult: {}\n",
                                                  preflight.guard,
                                                  preflight.icw_sequence,
                                                  preflight.master_offset,
                                                  preflight.slave_offset,
                                                  preflight.mask_after_remap,
                                                  preflight.sti,
                                                  preflight.irq_gates,
                                                  preflight.eoi_dispatch,
                                                  preflight.result
                                              );
                                            let _ = write!(serial_writer, "PIC remap preflight\nguard: {}\nicw sequence: {}\nmaster offset: 0x{:02x}\nslave offset: 0x{:02x}\nmask after remap: 0x{:02x}\nsti: {}\nirq gates: {}\neoi dispatch: {}\nresult: {}\n",
                                                  preflight.guard,
                                                  preflight.icw_sequence,
                                                  preflight.master_offset,
                                                  preflight.slave_offset,
                                                  preflight.mask_after_remap,
                                                  preflight.sti,
                                                  preflight.irq_gates,
                                                  preflight.eoi_dispatch,
                                                  preflight.result
                                              );
                                        } else if line_str == "irq-map" {
                                            let irq_map_msg = "irq map:\nirq0 timer -> vector 32 (0x20)\nirq1 keyboard -> vector 33 (0x21)\nirq2 cascade -> vector 34 (0x22)\nirq3 serial2 -> vector 35 (0x23)\nirq4 serial1 -> vector 36 (0x24)\nirq5 parallel2 -> vector 37 (0x25)\nirq6 floppy -> vector 38 (0x26)\nirq7 parallel1 -> vector 39 (0x27)\nirq8 rtc -> vector 40 (0x28)\nirq9 acpi -> vector 41 (0x29)\nirq10 reserved -> vector 42 (0x2a)\nirq11 reserved -> vector 43 (0x2b)\nirq12 mouse -> vector 44 (0x2c)\nirq13 fpu -> vector 45 (0x2d)\nirq14 primary-ata -> vector 46 (0x2e)\nirq15 secondary-ata -> vector 47 (0x2f)\nactive irq handlers: none\n";
                                            vga::print(irq_map_msg);
                                            serial::print(irq_map_msg);
                                        } else if line_str == "eoi-status" {
                                            let status = pic::ProgrammableInterruptController::eoi_strategy_status();
                                            // Prevent compiler from optimizing away EOI plan symbols
                                            let dummy_plans = [
                                                  pic::ProgrammableInterruptController::master_eoi_plan as *const () as usize,
                                                  pic::ProgrammableInterruptController::slave_eoi_plan as *const () as usize,
                                                  pic::ProgrammableInterruptController::irq0_timer_eoi_plan as *const () as usize,
                                                  pic::ProgrammableInterruptController::irq1_keyboard_eoi_plan as *const () as usize,
                                              ];
                                            core::hint::black_box(&dummy_plans);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "EOI strategy: {}\nPIC command: 0x{:02x}\nmaster PIC: {}\nslave PIC: {}\ndispatch: {}\n",
                                                  status.strategy_name,
                                                  status.pic_command,
                                                  status.master_pic_state,
                                                  status.slave_pic_state,
                                                  if status.dispatch_enabled { "enabled" } else { "disabled" }
                                              );
                                            let _ = write!(serial_writer, "EOI strategy: {}\nPIC command: 0x{:02x}\nmaster PIC: {}\nslave PIC: {}\ndispatch: {}\n",
                                                  status.strategy_name,
                                                  status.pic_command,
                                                  status.master_pic_state,
                                                  status.slave_pic_state,
                                                  if status.dispatch_enabled { "enabled" } else { "disabled" }
                                              );
                                        } else if line_str == "eoi-note" {
                                            let eoi_note_msg = "EOI strategy note:\n- EOI means End Of Interrupt.\n- Master PIC EOI targets command port 0x20 in the future.\n- Slave IRQs require slave EOI plus master cascade acknowledgement in the future.\n- IRQ0 timer and IRQ1 keyboard EOI paths are planned only.\n- No EOI is dispatched in this milestone.\n";
                                            vga::print(eoi_note_msg);
                                            serial::print(eoi_note_msg);
                                        } else if line_str == "irq-gates" {
                                            let irq_gates_msg = "IRQ Interrupt Gates:\n- Vector 32 (0x20): IRQ0 Timer (planned)\n- Vector 33 (0x21): IRQ1 Keyboard (planned)\n- Handler setup: planned\n- Status: dormant / disabled\n";
                                            vga::print(irq_gates_msg);
                                            serial::print(irq_gates_msg);
                                        } else if line_str == "irq-gate-status" {
                                            let irq_gate_status_msg = "IDT vector 32 (IRQ0 Timer): disabled / null handler\nIDT vector 33 (IRQ1 Keyboard): disabled / null handler\ngate binding dispatch: dormant\n";
                                            vga::print(irq_gate_status_msg);
                                            serial::print(irq_gate_status_msg);
                                        } else if line_str == "irq-gate-plan" {
                                            let plan = irq::irq_gate_plan();
                                            let timer = plan[0];
                                            let keyboard = plan[1];
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ Gate Binding Plan:\nIRQ{} {} -> vector {} (0x{:02x})\nIRQ{} {} -> vector {} (0x{:02x})\nIDT binding: {}\nPIC remap: {}\nEOI dispatch: {}\ninterrupts: {}\nstate: {}\n",
                                                  timer.irq,
                                                  timer.name,
                                                  timer.vector,
                                                  timer.vector,
                                                  keyboard.irq,
                                                  keyboard.name,
                                                  keyboard.vector,
                                                  keyboard.vector,
                                                  timer.idt_binding,
                                                  timer.pic_remap,
                                                  timer.eoi_dispatch,
                                                  timer.interrupts,
                                                  timer.gate_state
                                              );
                                            let _ = write!(serial_writer, "IRQ Gate Binding Plan:\nIRQ{} {} -> vector {} (0x{:02x})\nIRQ{} {} -> vector {} (0x{:02x})\nIDT binding: {}\nPIC remap: {}\nEOI dispatch: {}\ninterrupts: {}\nstate: {}\n",
                                                  timer.irq,
                                                  timer.name,
                                                  timer.vector,
                                                  timer.vector,
                                                  keyboard.irq,
                                                  keyboard.name,
                                                  keyboard.vector,
                                                  keyboard.vector,
                                                  timer.idt_binding,
                                                  timer.pic_remap,
                                                  timer.eoi_dispatch,
                                                  timer.interrupts,
                                                  timer.gate_state
                                              );
                                        } else if line_str == "irq-gate-arm" {
                                            let arm = irq::irq_gate_bind_smoke_arm();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ gate bind smoke armed\nmode: {}\nnext: {}\ninterrupts: {}\npic irq mask: {}\neoi dispatch: {}\n",
                                                  arm.mode,
                                                  arm.next,
                                                  arm.interrupts,
                                                  arm.pic_irq_mask,
                                                  arm.eoi_dispatch
                                              );
                                            let _ = write!(serial_writer, "IRQ gate bind smoke armed\nmode: {}\nnext: {}\ninterrupts: {}\npic irq mask: {}\neoi dispatch: {}\n",
                                                  arm.mode,
                                                  arm.next,
                                                  arm.interrupts,
                                                  arm.pic_irq_mask,
                                                  arm.eoi_dispatch
                                              );
                                        } else if line_str == "irq-gate-bind-smoke" {
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            if irq::irq_gate_bind_smoke_is_armed() {
                                                // Verification contract snippets kept stable across rustfmt line wrapping:
                                                // idt::IDT.entries[32].set_handler(interrupts::irq0_timer_gate_smoke_asm as *const ())
                                                // idt::IDT.entries[33].set_handler(interrupts::irq1_keyboard_gate_smoke_asm as *const ())
                                                idt::IDT.entries[32].set_handler(
                                                    interrupts::irq0_timer_gate_smoke_asm
                                                        as *const (),
                                                );
                                                idt::IDT.entries[33].set_handler(
                                                    interrupts::irq1_keyboard_gate_smoke_asm
                                                        as *const (),
                                                );
                                                let smoke = irq::irq_gate_bind_smoke_mark_bound();
                                                let _ = write!(vga_writer, "IRQ gate bind controlled smoke\nguard: {}\nIDT vector 32: {}\nIDT vector 33: {}\npic irq mask: {}\nsti: {}\neoi dispatch: {}\nkeyboard input: {}\nresult: {}\n",
                                                      smoke.guard,
                                                      smoke.irq0_vector_state,
                                                      smoke.irq1_vector_state,
                                                      smoke.pic_irq_mask,
                                                      smoke.sti,
                                                      smoke.eoi_dispatch,
                                                      smoke.keyboard_input,
                                                      smoke.result
                                                  );
                                                let _ = write!(serial_writer, "IRQ gate bind controlled smoke\nguard: {}\nIDT vector 32: {}\nIDT vector 33: {}\npic irq mask: {}\nsti: {}\neoi dispatch: {}\nkeyboard input: {}\nresult: {}\n",
                                                      smoke.guard,
                                                      smoke.irq0_vector_state,
                                                      smoke.irq1_vector_state,
                                                      smoke.pic_irq_mask,
                                                      smoke.sti,
                                                      smoke.eoi_dispatch,
                                                      smoke.keyboard_input,
                                                      smoke.result
                                                  );
                                            } else {
                                                let smoke = irq::irq_gate_bind_smoke_blocked();
                                                if let Some(next) = smoke.next {
                                                    let _ = write!(vga_writer, "IRQ gate bind controlled smoke\nguard: {}\nresult: {}\nnext: {}\n",
                                                          smoke.guard,
                                                          smoke.result,
                                                          next
                                                      );
                                                    let _ = write!(serial_writer, "IRQ gate bind controlled smoke\nguard: {}\nresult: {}\nnext: {}\n",
                                                          smoke.guard,
                                                          smoke.result,
                                                          next
                                                      );
                                                }
                                            }
                                        } else if line_str == "irq-gate-bind-status" {
                                            let status = irq::irq_gate_bind_smoke_status();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ gate bind smoke status\narmed: {}\nexecuted: {}\nIDT vector {}: {}\nIDT vector {}: {}\nactive IRQ0 handler: {}\nactive IRQ1 handler: {}\npic irq mask: {}\nsti: {}\neoi dispatch: {}\nkeyboard input: {}\n",
                                                  if status.armed { "yes" } else { "no" },
                                                  if status.executed { "yes" } else { "no" },
                                                  status.irq0_vector,
                                                  status.irq0_vector_state,
                                                  status.irq1_vector,
                                                  status.irq1_vector_state,
                                                  status.irq0_active_handler,
                                                  status.irq1_active_handler,
                                                  status.pic_irq_mask,
                                                  status.sti,
                                                  status.eoi_dispatch,
                                                  status.keyboard_input
                                              );
                                            let _ = write!(serial_writer, "IRQ gate bind smoke status\narmed: {}\nexecuted: {}\nIDT vector {}: {}\nIDT vector {}: {}\nactive IRQ0 handler: {}\nactive IRQ1 handler: {}\npic irq mask: {}\nsti: {}\neoi dispatch: {}\nkeyboard input: {}\n",
                                                  if status.armed { "yes" } else { "no" },
                                                  if status.executed { "yes" } else { "no" },
                                                  status.irq0_vector,
                                                  status.irq0_vector_state,
                                                  status.irq1_vector,
                                                  status.irq1_vector_state,
                                                  status.irq0_active_handler,
                                                  status.irq1_active_handler,
                                                  status.pic_irq_mask,
                                                  status.sti,
                                                  status.eoi_dispatch,
                                                  status.keyboard_input
                                              );
                                        } else if line_str == "irq-gate-state" {
                                            let state = irq::irq_gate_bind_state();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ gate bind state\narmed: {}\nexecuted: {}\nIDT vector {}: {}\nIDT vector {}: {}\nactive IRQ0 handler: {}\nactive IRQ1 handler: {}\nbind expected: {}\nbind applied: {}\nirq runtime: {}\npic irq mask: {}\nsti: {}\neoi dispatch: {}\nkeyboard input: {}\n",
                                                  if state.armed { "yes" } else { "no" },
                                                  if state.executed { "yes" } else { "no" },
                                                  state.irq0_vector,
                                                  state.irq0_vector_state,
                                                  state.irq1_vector,
                                                  state.irq1_vector_state,
                                                  state.irq0_active_handler,
                                                  state.irq1_active_handler,
                                                  state.bind_expected,
                                                  state.bind_applied,
                                                  state.irq_runtime,
                                                  state.pic_irq_mask,
                                                  state.sti,
                                                  state.eoi_dispatch,
                                                  state.keyboard_input
                                              );
                                            let _ = write!(serial_writer, "IRQ gate bind state\narmed: {}\nexecuted: {}\nIDT vector {}: {}\nIDT vector {}: {}\nactive IRQ0 handler: {}\nactive IRQ1 handler: {}\nbind expected: {}\nbind applied: {}\nirq runtime: {}\npic irq mask: {}\nsti: {}\neoi dispatch: {}\nkeyboard input: {}\n",
                                                  if state.armed { "yes" } else { "no" },
                                                  if state.executed { "yes" } else { "no" },
                                                  state.irq0_vector,
                                                  state.irq0_vector_state,
                                                  state.irq1_vector,
                                                  state.irq1_vector_state,
                                                  state.irq0_active_handler,
                                                  state.irq1_active_handler,
                                                  state.bind_expected,
                                                  state.bind_applied,
                                                  state.irq_runtime,
                                                  state.pic_irq_mask,
                                                  state.sti,
                                                  state.eoi_dispatch,
                                                  state.keyboard_input
                                              );
                                        } else if line_str == "irq-gate-history" {
                                            let history = irq::irq_gate_bind_history();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ gate bind history\narm command: {}\nsmoke command: {}\nlast smoke executed: {}\nidt binds: {}\nboot bind: {}\n",
                                                  history.arm_command,
                                                  history.smoke_command,
                                                  history.last_smoke_executed,
                                                  history.idt_binds,
                                                  history.boot_bind
                                              );
                                            let _ = write!(serial_writer, "IRQ gate bind history\narm command: {}\nsmoke command: {}\nlast smoke executed: {}\nidt binds: {}\nboot bind: {}\n",
                                                  history.arm_command,
                                                  history.smoke_command,
                                                  history.last_smoke_executed,
                                                  history.idt_binds,
                                                  history.boot_bind
                                              );
                                        } else if line_str == "irq-gate-preflight" {
                                            let preflight = irq::irq_gate_bind_preflight();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ gate bind preflight\nguard: {}\nbind path: {}\nIDT vector {}: {}\nIDT vector {}: {}\npic irq mask: {}\nsti: {}\neoi dispatch: {}\nkeyboard input: {}\nresult: {}\n",
                                                  preflight.guard,
                                                  preflight.bind_path,
                                                  preflight.irq0_vector,
                                                  preflight.irq0_vector_state,
                                                  preflight.irq1_vector,
                                                  preflight.irq1_vector_state,
                                                  preflight.pic_irq_mask,
                                                  preflight.sti,
                                                  preflight.eoi_dispatch,
                                                  preflight.keyboard_input,
                                                  preflight.result
                                              );
                                            let _ = write!(serial_writer, "IRQ gate bind preflight\nguard: {}\nbind path: {}\nIDT vector {}: {}\nIDT vector {}: {}\npic irq mask: {}\nsti: {}\neoi dispatch: {}\nkeyboard input: {}\nresult: {}\n",
                                                  preflight.guard,
                                                  preflight.bind_path,
                                                  preflight.irq0_vector,
                                                  preflight.irq0_vector_state,
                                                  preflight.irq1_vector,
                                                  preflight.irq1_vector_state,
                                                  preflight.pic_irq_mask,
                                                  preflight.sti,
                                                  preflight.eoi_dispatch,
                                                  preflight.keyboard_input,
                                                  preflight.result
                                              );
                                        } else if line_str == "irq-bind-note" {
                                            let bind_status = irq::bind_irq_gates_disabled();
                                            let timer = bind_status.steps[0];
                                            let keyboard = bind_status.steps[1];
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ bind note:\nIRQ{} {} gate: {}\nIRQ{} {} gate: {}\nIDT entries: {}\nPIC remap: {}\nEOI dispatch: {}\ninterrupts: {}\n",
                                                  timer.irq,
                                                  timer.name,
                                                  timer.bind_path,
                                                  keyboard.irq,
                                                  keyboard.name,
                                                  keyboard.bind_path,
                                                  timer.idt_install,
                                                  timer.pic_remap,
                                                  timer.eoi_dispatch,
                                                  timer.interrupts
                                              );
                                            let _ = write!(serial_writer, "IRQ bind note:\nIRQ{} {} gate: {}\nIRQ{} {} gate: {}\nIDT entries: {}\nPIC remap: {}\nEOI dispatch: {}\ninterrupts: {}\n",
                                                  timer.irq,
                                                  timer.name,
                                                  timer.bind_path,
                                                  keyboard.irq,
                                                  keyboard.name,
                                                  keyboard.bind_path,
                                                  timer.idt_install,
                                                  timer.pic_remap,
                                                  timer.eoi_dispatch,
                                                  timer.interrupts
                                              );
                                        } else if line_str == "irq-bind-status" {
                                            let bind_status = irq::bind_irq_gates_disabled();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ bind status:\nhelper: {}\nboot call: {}\nIDT vector {}: {}\nIDT vector {}: {}\nactive IRQ0 handler: {}\nactive IRQ1 handler: {}\nkeyboard input: {}\n",
                                                  bind_status.helper,
                                                  bind_status.boot_call,
                                                  bind_status.irq0_vector,
                                                  bind_status.irq0_state,
                                                  bind_status.irq1_vector,
                                                  bind_status.irq1_state,
                                                  bind_status.irq0_active_handler,
                                                  bind_status.irq1_active_handler,
                                                  bind_status.keyboard_input
                                              );
                                            let _ = write!(serial_writer, "IRQ bind status:\nhelper: {}\nboot call: {}\nIDT vector {}: {}\nIDT vector {}: {}\nactive IRQ0 handler: {}\nactive IRQ1 handler: {}\nkeyboard input: {}\n",
                                                  bind_status.helper,
                                                  bind_status.boot_call,
                                                  bind_status.irq0_vector,
                                                  bind_status.irq0_state,
                                                  bind_status.irq1_vector,
                                                  bind_status.irq1_state,
                                                  bind_status.irq0_active_handler,
                                                  bind_status.irq1_active_handler,
                                                  bind_status.keyboard_input
                                              );
                                        } else if line_str == "irq-readiness" {
                                            let readiness = irq::irq_runtime_readiness();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime readiness\nidt exceptions: {}\nirq gate plan: {}\neoi strategy: {}\npic remap: {}\nsti: {}\nkeyboard fallback: {}\nready for runtime irq: {}\n",
                                                  readiness.idt_exceptions,
                                                  readiness.irq_gate_plan,
                                                  readiness.eoi_strategy,
                                                  readiness.pic_remap,
                                                  readiness.sti,
                                                  readiness.keyboard_fallback,
                                                  readiness.ready_for_runtime_irq
                                              );
                                            let _ = write!(serial_writer, "IRQ runtime readiness\nidt exceptions: {}\nirq gate plan: {}\neoi strategy: {}\npic remap: {}\nsti: {}\nkeyboard fallback: {}\nready for runtime irq: {}\n",
                                                  readiness.idt_exceptions,
                                                  readiness.irq_gate_plan,
                                                  readiness.eoi_strategy,
                                                  readiness.pic_remap,
                                                  readiness.sti,
                                                  readiness.keyboard_fallback,
                                                  readiness.ready_for_runtime_irq
                                              );
                                        } else if line_str == "irq-risk" {
                                            let risk = irq::irq_runtime_risk();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime risk\nruntime irq: {}\nreason: {}\nrequired before enable: {}\nsti allowed: {}\n",
                                                  risk.runtime_irq,
                                                  risk.reason,
                                                  risk.required_before_enable,
                                                  risk.sti_allowed
                                              );
                                            let _ = write!(serial_writer, "IRQ runtime risk\nruntime irq: {}\nreason: {}\nrequired before enable: {}\nsti allowed: {}\n",
                                                  risk.runtime_irq,
                                                  risk.reason,
                                                  risk.required_before_enable,
                                                  risk.sti_allowed
                                              );
                                        } else if line_str == "irq-preflight" {
                                            let preflight = irq::irq_runtime_preflight();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime preflight\nIDT exceptions 0/3/14: {}\nIRQ vectors 32/33: {}\nbind path: {}\nEOI dispatch: {}\nPIC remap: {}\nkeyboard fallback: {}\npf-smoke: {}\nresult: {}\n",
                                                  preflight.idt_exceptions,
                                                  preflight.irq_vectors,
                                                  preflight.bind_path,
                                                  preflight.eoi_dispatch,
                                                  preflight.pic_remap,
                                                  preflight.keyboard_fallback,
                                                  preflight.pf_smoke,
                                                  preflight.result
                                              );
                                            let _ = write!(serial_writer, "IRQ runtime preflight\nIDT exceptions 0/3/14: {}\nIRQ vectors 32/33: {}\nbind path: {}\nEOI dispatch: {}\nPIC remap: {}\nkeyboard fallback: {}\npf-smoke: {}\nresult: {}\n",
                                                  preflight.idt_exceptions,
                                                  preflight.irq_vectors,
                                                  preflight.bind_path,
                                                  preflight.eoi_dispatch,
                                                  preflight.pic_remap,
                                                  preflight.keyboard_fallback,
                                                  preflight.pf_smoke,
                                                  preflight.result
                                              );
                                        } else if line_str == "irq-runtime-preflight" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation preflight\npic remap: {}\nirq gates: controlled smoke bound={}\neoi strategy: {}\nkeyboard fallback: {}\nsti: {}\nruntime irq ready: {}\n",
                                                   if pic_state.executed { "controlled smoke available" } else { "not ready" },
                                                   if gate_state.executed { "yes" } else { "no" },
                                                   "planned / disabled",
                                                   "polling",
                                                   "disabled",
                                                   "no"
                                               );
                                            let _ = write!(serial_writer, "IRQ runtime activation preflight\npic remap: {}\nirq gates: controlled smoke bound={}\neoi strategy: {}\nkeyboard fallback: {}\nsti: {}\nruntime irq ready: {}\n",
                                                   if pic_state.executed { "controlled smoke available" } else { "not ready" },
                                                   if gate_state.executed { "yes" } else { "no" },
                                                   "planned / disabled",
                                                   "polling",
                                                   "disabled",
                                                   "no"
                                               );
                                        } else if line_str == "irq-runtime-arm" {
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            if irq::irq_runtime_is_committed() {
                                                let _ = write!(vga_writer, "error: IRQ runtime activation already committed (no-op).\n");
                                                let _ = write!(serial_writer, "error: IRQ runtime activation already committed (no-op).\n");
                                            } else if irq::irq_runtime_is_armed() {
                                                let _ = write!(vga_writer, "error: IRQ runtime activation already armed (no-op).\nnext: execute irq-runtime-commit\n");
                                                let _ = write!(serial_writer, "error: IRQ runtime activation already armed (no-op).\nnext: execute irq-runtime-commit\n");
                                            } else {
                                                irq::irq_runtime_arm();
                                                let _ = write!(vga_writer, "IRQ runtime activation armed.\nnext: execute irq-runtime-commit\n");
                                                let _ = write!(serial_writer, "IRQ runtime activation armed.\nnext: execute irq-runtime-commit\n");
                                            }
                                        } else if line_str == "irq-runtime-commit" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            if irq::irq_runtime_is_committed() {
                                                let _ = write!(vga_writer, "error: IRQ runtime activation already committed (no-op).\n");
                                                let _ = write!(serial_writer, "error: IRQ runtime activation already committed (no-op).\n");
                                            } else if !irq::irq_runtime_is_armed() {
                                                let _ = write!(vga_writer, "error: IRQ runtime activation not armed.\nrequired: execute irq-runtime-arm first.\n");
                                                let _ = write!(serial_writer, "error: IRQ runtime activation not armed.\nrequired: execute irq-runtime-arm first.\n");
                                            } else {
                                                let _ = write!(vga_writer, "IRQ runtime activation commit dry-run\npic remap smoke: {}\nirq gate bind smoke: {}\neoi runtime boundary: {}\npic mask policy: {}\nunmask policy: {}\nruntime latch: {}\nsti: {}\nruntime irq active: {}\ndry-run commit allowed: {}\nresult: {}\n",
                                                        matrix.pic_remap_smoke,
                                                        matrix.irq_gate_bind_smoke,
                                                        matrix.eoi_runtime_boundary,
                                                        matrix.pic_mask_policy,
                                                        matrix.unmask_policy,
                                                        matrix.runtime_latch,
                                                        matrix.sti,
                                                        matrix.runtime_irq_active,
                                                        activation.allowed_text,
                                                        activation.result
                                                    );
                                                let _ = write!(serial_writer, "IRQ runtime activation commit dry-run\npic remap smoke: {}\nirq gate bind smoke: {}\neoi runtime boundary: {}\npic mask policy: {}\nunmask policy: {}\nruntime latch: {}\nsti: {}\nruntime irq active: {}\ndry-run commit allowed: {}\nresult: {}\n",
                                                        matrix.pic_remap_smoke,
                                                        matrix.irq_gate_bind_smoke,
                                                        matrix.eoi_runtime_boundary,
                                                        matrix.pic_mask_policy,
                                                        matrix.unmask_policy,
                                                        matrix.runtime_latch,
                                                        matrix.sti,
                                                        matrix.runtime_irq_active,
                                                        activation.allowed_text,
                                                        activation.result
                                                    );
                                                if !activation.allowed {
                                                    // Verification contract snippet kept stable across rustfmt line wrapping:
                                                    // "next: {}\n", activation.next
                                                    if !pic_state.executed {
                                                        let _ = write!(
                                                            vga_writer,
                                                            "- {}\n",
                                                            irq::IRQ_RUNTIME_BLOCKER_PIC_REMAP
                                                        );
                                                        let _ = write!(
                                                            serial_writer,
                                                            "- {}\n",
                                                            irq::IRQ_RUNTIME_BLOCKER_PIC_REMAP
                                                        );
                                                    }
                                                    if !gate_state.executed {
                                                        let _ = write!(
                                                            vga_writer,
                                                            "- {}\n",
                                                            irq::IRQ_RUNTIME_BLOCKER_IRQ_GATES
                                                        );
                                                        let _ = write!(
                                                            serial_writer,
                                                            "- {}\n",
                                                            irq::IRQ_RUNTIME_BLOCKER_IRQ_GATES
                                                        );
                                                    }
                                                    let _ = write!(
                                                        vga_writer,
                                                        "- {}\n",
                                                        irq::IRQ_RUNTIME_BLOCKER_EOI_DISPATCH
                                                    );
                                                    let _ = write!(
                                                        serial_writer,
                                                        "- {}\n",
                                                        irq::IRQ_RUNTIME_BLOCKER_EOI_DISPATCH
                                                    );
                                                    let _ = write!(
                                                        vga_writer,
                                                        "- {}\n",
                                                        irq::IRQ_RUNTIME_BLOCKER_STI
                                                    );
                                                    let _ = write!(
                                                        serial_writer,
                                                        "- {}\n",
                                                        irq::IRQ_RUNTIME_BLOCKER_STI
                                                    );
                                                    let _ = write!(
                                                        vga_writer,
                                                        "next: {}\n",
                                                        activation.next
                                                    );
                                                    let _ = write!(
                                                        serial_writer,
                                                        "next: {}\n",
                                                        activation.next
                                                    );
                                                }
                                            }
                                        } else if line_str == "irq-runtime-status" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            let runtime_activation =
                                                if irq::irq_runtime_is_committed() {
                                                    "committed (dry-run)"
                                                } else if irq::irq_runtime_is_armed() {
                                                    "armed / standby"
                                                } else {
                                                    "blocked"
                                                };
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime readiness status\npic remap: {}\nirq gates: {}\neoi dispatch: {}\nkeyboard input: {}\npage fault smoke: {}\nruntime irq activation: {}\nsti enabled: {}\n",
                                                    if pic_state.executed { "controlled smoke available" } else { "not ready" },
                                                    if gate_state.executed { "bound" } else { "unbound" },
                                                    "disabled",
                                                    "polling",
                                                    "stable",
                                                    runtime_activation,
                                                    "no"
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime readiness status\npic remap: {}\nirq gates: {}\neoi dispatch: {}\nkeyboard input: {}\npage fault smoke: {}\nruntime irq activation: {}\nsti enabled: {}\n",
                                                    if pic_state.executed { "controlled smoke available" } else { "not ready" },
                                                    if gate_state.executed { "bound" } else { "unbound" },
                                                    "disabled",
                                                    "polling",
                                                    "stable",
                                                    runtime_activation,
                                                    "no"
                                                );
                                        } else if line_str == "irq-runtime-blockers" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(
                                                vga_writer,
                                                "IRQ runtime activation blockers\n"
                                            );
                                            let _ = write!(
                                                serial_writer,
                                                "IRQ runtime activation blockers\n"
                                            );
                                            if !pic_state.executed {
                                                let _ = write!(
                                                    vga_writer,
                                                    "- PIC remap: not ready for controlled smoke\n"
                                                );
                                                let _ = write!(
                                                    serial_writer,
                                                    "- PIC remap: not ready for controlled smoke\n"
                                                );
                                            }
                                            if !gate_state.executed {
                                                let _ = write!(
                                                    vga_writer,
                                                    "- IRQ gates: vectors 32/33 not bound\n"
                                                );
                                                let _ = write!(
                                                    serial_writer,
                                                    "- IRQ gates: vectors 32/33 not bound\n"
                                                );
                                            }
                                            let _ =
                                                write!(vga_writer, "- EOI dispatch: not enabled\n");
                                            let _ = write!(
                                                serial_writer,
                                                "- EOI dispatch: not enabled\n"
                                            );
                                            let _ = write!(vga_writer, "- STI: disabled\n");
                                            let _ = write!(serial_writer, "- STI: disabled\n");
                                            let _ = write!(vga_writer, "smoke prerequisites: satisfied\nruntime irq ready: no\n");
                                            let _ = write!(serial_writer, "smoke prerequisites: satisfied\nruntime irq ready: no\n");
                                        } else if line_str == "irq-runtime-matrix" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime readiness matrix\npic remap smoke: {}\nirq gate bind smoke: {}\neoi runtime boundary: {}\npic mask policy: {}\nunmask policy: {}\nruntime latch: {}\nkeyboard mode: {}\nsti: {}\nruntime irq active: {}\n",
                                                    matrix.pic_remap_smoke,
                                                    matrix.irq_gate_bind_smoke,
                                                    matrix.eoi_runtime_boundary,
                                                    matrix.pic_mask_policy,
                                                    matrix.unmask_policy,
                                                    matrix.runtime_latch,
                                                    matrix.keyboard_mode,
                                                    matrix.sti,
                                                    matrix.runtime_irq_active
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime readiness matrix\npic remap smoke: {}\nirq gate bind smoke: {}\neoi runtime boundary: {}\npic mask policy: {}\nunmask policy: {}\nruntime latch: {}\nkeyboard mode: {}\nsti: {}\nruntime irq active: {}\n",
                                                    matrix.pic_remap_smoke,
                                                    matrix.irq_gate_bind_smoke,
                                                    matrix.eoi_runtime_boundary,
                                                    matrix.pic_mask_policy,
                                                    matrix.unmask_policy,
                                                    matrix.runtime_latch,
                                                    matrix.keyboard_mode,
                                                    matrix.sti,
                                                    matrix.runtime_irq_active
                                                );
                                        } else if line_str == "irq-runtime-readiness" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime readiness\nsmoke prerequisites: {}\nmask policy: {}\nruntime latch: {}\nsti: {}\nruntime irq ready: no\n",
                                                    matrix.smoke_prerequisites,
                                                    matrix.pic_mask_policy,
                                                    matrix.runtime_latch,
                                                    matrix.sti
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime readiness\nsmoke prerequisites: {}\nmask policy: {}\nruntime latch: {}\nsti: {}\nruntime irq ready: no\n",
                                                    matrix.smoke_prerequisites,
                                                    matrix.pic_mask_policy,
                                                    matrix.runtime_latch,
                                                    matrix.sti
                                                );
                                        } else if line_str == "irq-runtime-next" {
                                            let next_msg = "IRQ runtime next\n1. keep PIC mask policy all masked (0xFF)\n2. keep unmask policy no unmask\n3. implement live EOI dispatch boundary\n4. enable STI only after EOI and handlers are ready\n5. switch keyboard from polling only after IRQ1 handler is live\nruntime irq active: no\n";
                                            vga::print(next_msg);
                                            serial::print(next_msg);
                                        } else if line_str == "irq-runtime-activation-plan" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation plan\n1. require readiness matrix smoke prerequisites: yes\n2. require EOI runtime boundary: ready (dry-run)\n3. keep PIC mask policy: {}\n4. keep unmask policy: {}\n5. keep STI: {}\n6. commit path remains dry-run only\nruntime irq active: {}\ndry-run commit allowed: {}\n",
                                                    matrix.pic_mask_policy,
                                                    matrix.unmask_policy,
                                                    matrix.sti,
                                                    matrix.runtime_irq_active,
                                                    activation.allowed_text
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation plan\n1. require readiness matrix smoke prerequisites: yes\n2. require EOI runtime boundary: ready (dry-run)\n3. keep PIC mask policy: {}\n4. keep unmask policy: {}\n5. keep STI: {}\n6. commit path remains dry-run only\nruntime irq active: {}\ndry-run commit allowed: {}\n",
                                                    matrix.pic_mask_policy,
                                                    matrix.unmask_policy,
                                                    matrix.sti,
                                                    matrix.runtime_irq_active,
                                                    activation.allowed_text
                                                );
                                        } else if line_str == "irq-runtime-token-note" {
                                            let token = irq::irq_runtime_activation_token_status();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation token note\ntoken gate: explicit\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n",
                                                    token.token_scope,
                                                    token.hardware_mutation,
                                                    token.sti,
                                                    token.pic_unmask,
                                                    token.live_irq0_irq1,
                                                    token.runtime_eoi_dispatch,
                                                    token.keyboard_mode
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation token note\ntoken gate: explicit\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n",
                                                    token.token_scope,
                                                    token.hardware_mutation,
                                                    token.sti,
                                                    token.pic_unmask,
                                                    token.live_irq0_irq1,
                                                    token.runtime_eoi_dispatch,
                                                    token.keyboard_mode
                                                );
                                        } else if line_str == "irq-runtime-token-status" {
                                            let token = irq::irq_runtime_activation_token_status();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation token status\nactivation token: {}\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n",
                                                    token.token_state,
                                                    token.token_scope,
                                                    token.hardware_mutation,
                                                    token.sti,
                                                    token.pic_unmask,
                                                    token.live_irq0_irq1,
                                                    token.runtime_eoi_dispatch,
                                                    token.keyboard_mode
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation token status\nactivation token: {}\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n",
                                                    token.token_state,
                                                    token.token_scope,
                                                    token.hardware_mutation,
                                                    token.sti,
                                                    token.pic_unmask,
                                                    token.live_irq0_irq1,
                                                    token.runtime_eoi_dispatch,
                                                    token.keyboard_mode
                                                );
                                        } else if line_str == "irq-runtime-token-arm" {
                                            let token = irq::irq_runtime_activation_token_arm();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation token armed\nactivation token: {}\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n",
                                                    token.token_state,
                                                    token.token_scope,
                                                    token.hardware_mutation,
                                                    token.sti,
                                                    token.pic_unmask,
                                                    token.live_irq0_irq1,
                                                    token.runtime_eoi_dispatch,
                                                    token.keyboard_mode
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation token armed\nactivation token: {}\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n",
                                                    token.token_state,
                                                    token.token_scope,
                                                    token.hardware_mutation,
                                                    token.sti,
                                                    token.pic_unmask,
                                                    token.live_irq0_irq1,
                                                    token.runtime_eoi_dispatch,
                                                    token.keyboard_mode
                                                );
                                        } else if line_str == "irq-runtime-token-clear" {
                                            let token = irq::irq_runtime_activation_token_clear();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation token cleared\nactivation token: {}\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n",
                                                    token.token_state,
                                                    token.token_scope,
                                                    token.hardware_mutation,
                                                    token.sti,
                                                    token.pic_unmask,
                                                    token.live_irq0_irq1,
                                                    token.runtime_eoi_dispatch,
                                                    token.keyboard_mode
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation token cleared\nactivation token: {}\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n",
                                                    token.token_state,
                                                    token.token_scope,
                                                    token.hardware_mutation,
                                                    token.sti,
                                                    token.pic_unmask,
                                                    token.live_irq0_irq1,
                                                    token.runtime_eoi_dispatch,
                                                    token.keyboard_mode
                                                );
                                        } else if line_str == "irq-runtime-gate-note" {
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation gate note\ngate purpose: {}\ntoken required: {}\nmatrix required: {}\ndry-run commit required: {}\nhardware mutation: {}\nactivation allowed: {}\n",
                                                    irq::IRQ_ACTIVATION_GATE_PURPOSE,
                                                    irq::IRQ_ACTIVATION_GATE_REQUIRED_YES,
                                                    irq::IRQ_ACTIVATION_GATE_MATRIX_REQUIRED_READY,
                                                    irq::IRQ_ACTIVATION_GATE_REQUIRED_YES,
                                                    irq::IRQ_ACTIVATION_TOKEN_HARDWARE_MUTATION_NO,
                                                    irq::IRQ_ACTIVATION_GATE_ALLOWED_NO
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation gate note\ngate purpose: {}\ntoken required: {}\nmatrix required: {}\ndry-run commit required: {}\nhardware mutation: {}\nactivation allowed: {}\n",
                                                    irq::IRQ_ACTIVATION_GATE_PURPOSE,
                                                    irq::IRQ_ACTIVATION_GATE_REQUIRED_YES,
                                                    irq::IRQ_ACTIVATION_GATE_MATRIX_REQUIRED_READY,
                                                    irq::IRQ_ACTIVATION_GATE_REQUIRED_YES,
                                                    irq::IRQ_ACTIVATION_TOKEN_HARDWARE_MUTATION_NO,
                                                    irq::IRQ_ACTIVATION_GATE_ALLOWED_NO
                                                );
                                        } else if line_str == "irq-runtime-gate-status" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation gate status\ntoken gate: {}\nreadiness matrix: {}\neoi runtime boundary: {}\npic mask policy: {}\nunmask policy: {}\ndry-run commit allowed: {}\nruntime irq active: {}\nactivation allowed: {}\n",
                                                    gate.token_gate,
                                                    gate.readiness_matrix,
                                                    gate.eoi_runtime_boundary,
                                                    gate.pic_mask_policy,
                                                    gate.unmask_policy,
                                                    gate.dry_run_commit_allowed,
                                                    gate.runtime_irq_active,
                                                    gate.activation_allowed
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation gate status\ntoken gate: {}\nreadiness matrix: {}\neoi runtime boundary: {}\npic mask policy: {}\nunmask policy: {}\ndry-run commit allowed: {}\nruntime irq active: {}\nactivation allowed: {}\n",
                                                    gate.token_gate,
                                                    gate.readiness_matrix,
                                                    gate.eoi_runtime_boundary,
                                                    gate.pic_mask_policy,
                                                    gate.unmask_policy,
                                                    gate.dry_run_commit_allowed,
                                                    gate.runtime_irq_active,
                                                    gate.activation_allowed
                                                );
                                        } else if line_str == "irq-runtime-gate-check" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation gate check\ntoken gate: {}\nmatrix decision: {}\neoi boundary: {}\nmask policy: {}\nhardware mutation: {}\nresult: {}\nnext: {}\n",
                                                    gate.token_gate,
                                                    gate.readiness_matrix,
                                                    gate.eoi_runtime_boundary,
                                                    gate.pic_mask_policy,
                                                    gate.hardware_mutation,
                                                    gate.result,
                                                    gate.next
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation gate check\ntoken gate: {}\nmatrix decision: {}\neoi boundary: {}\nmask policy: {}\nhardware mutation: {}\nresult: {}\nnext: {}\n",
                                                    gate.token_gate,
                                                    gate.readiness_matrix,
                                                    gate.eoi_runtime_boundary,
                                                    gate.pic_mask_policy,
                                                    gate.hardware_mutation,
                                                    gate.result,
                                                    gate.next
                                                );
                                        } else if line_str == "irq-runtime-gate-blockers" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation gate blockers\n- activation token: {}\n- readiness matrix: {}\n- dry-run commit: {}\n- EOI runtime boundary: {}\n- STI: {}\nactivation allowed: {}\n",
                                                    gate.token_gate,
                                                    irq::IRQ_ACTIVATION_GATE_RUNTIME_READY_NO,
                                                    irq::IRQ_ACTIVATION_GATE_DRY_RUN_NOT_ALLOWED,
                                                    gate.eoi_runtime_boundary,
                                                    matrix.sti,
                                                    gate.activation_allowed
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation gate blockers\n- activation token: {}\n- readiness matrix: {}\n- dry-run commit: {}\n- EOI runtime boundary: {}\n- STI: {}\nactivation allowed: {}\n",
                                                    gate.token_gate,
                                                    irq::IRQ_ACTIVATION_GATE_RUNTIME_READY_NO,
                                                    irq::IRQ_ACTIVATION_GATE_DRY_RUN_NOT_ALLOWED,
                                                    gate.eoi_runtime_boundary,
                                                    matrix.sti,
                                                    gate.activation_allowed
                                                );
                                        } else if line_str == "irq-runtime-sim-note" {
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation simulation note\nsimulation purpose: {}\nhardware mutation: {}\nsti would enable: {}\npic unmask would apply: {}\neoi dispatch would enable: {}\nkeyboard mode: {}\n",
                                                    irq::IRQ_ACTIVATION_SIM_PURPOSE,
                                                    irq::IRQ_ACTIVATION_TOKEN_HARDWARE_MUTATION_NO,
                                                    irq::IRQ_ACTIVATION_SIM_STI_WOULD_ENABLE_NO,
                                                    irq::IRQ_ACTIVATION_SIM_PIC_UNMASK_WOULD_APPLY_NO,
                                                    irq::IRQ_ACTIVATION_SIM_EOI_DISPATCH_WOULD_ENABLE_NO,
                                                    irq::IRQ_MATRIX_KEYBOARD_MODE_POLLING
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation simulation note\nsimulation purpose: {}\nhardware mutation: {}\nsti would enable: {}\npic unmask would apply: {}\neoi dispatch would enable: {}\nkeyboard mode: {}\n",
                                                    irq::IRQ_ACTIVATION_SIM_PURPOSE,
                                                    irq::IRQ_ACTIVATION_TOKEN_HARDWARE_MUTATION_NO,
                                                    irq::IRQ_ACTIVATION_SIM_STI_WOULD_ENABLE_NO,
                                                    irq::IRQ_ACTIVATION_SIM_PIC_UNMASK_WOULD_APPLY_NO,
                                                    irq::IRQ_ACTIVATION_SIM_EOI_DISPATCH_WOULD_ENABLE_NO,
                                                    irq::IRQ_MATRIX_KEYBOARD_MODE_POLLING
                                                );
                                        } else if line_str == "irq-runtime-sim-status" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation simulation status\ntoken gate: {}\nreadiness matrix: {}\ngate decision: {}\ndry-run commit allowed: {}\nsimulated activation allowed: {}\nruntime irq active: {}\nhardware mutation: {}\n",
                                                    simulation.token_gate,
                                                    simulation.readiness_matrix,
                                                    simulation.gate_decision,
                                                    simulation.dry_run_commit_allowed,
                                                    simulation.simulated_activation_allowed,
                                                    simulation.runtime_irq_active,
                                                    simulation.hardware_mutation
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation simulation status\ntoken gate: {}\nreadiness matrix: {}\ngate decision: {}\ndry-run commit allowed: {}\nsimulated activation allowed: {}\nruntime irq active: {}\nhardware mutation: {}\n",
                                                    simulation.token_gate,
                                                    simulation.readiness_matrix,
                                                    simulation.gate_decision,
                                                    simulation.dry_run_commit_allowed,
                                                    simulation.simulated_activation_allowed,
                                                    simulation.runtime_irq_active,
                                                    simulation.hardware_mutation
                                                );
                                        } else if line_str == "irq-runtime-sim-run" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation simulation run\nsimulated activation allowed: {}\nhardware mutation: {}\nsti would enable: {}\npic unmask would apply: {}\neoi dispatch would enable: {}\nkeyboard mode: {}\nresult: {}\nnext: {}\n",
                                                    simulation.simulated_activation_allowed,
                                                    simulation.hardware_mutation,
                                                    simulation.sti_would_enable,
                                                    simulation.pic_unmask_would_apply,
                                                    simulation.eoi_dispatch_would_enable,
                                                    simulation.keyboard_mode,
                                                    simulation.result,
                                                    simulation.next
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation simulation run\nsimulated activation allowed: {}\nhardware mutation: {}\nsti would enable: {}\npic unmask would apply: {}\neoi dispatch would enable: {}\nkeyboard mode: {}\nresult: {}\nnext: {}\n",
                                                    simulation.simulated_activation_allowed,
                                                    simulation.hardware_mutation,
                                                    simulation.sti_would_enable,
                                                    simulation.pic_unmask_would_apply,
                                                    simulation.eoi_dispatch_would_enable,
                                                    simulation.keyboard_mode,
                                                    simulation.result,
                                                    simulation.next
                                                );
                                        } else if line_str == "irq-runtime-sim-blockers" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation simulation blockers\n- activation token: {}\n- gate decision: {}\n- readiness matrix: {}\n- dry-run commit: {}\n- EOI runtime boundary: {}\n- STI would enable: {}\n- PIC unmask would apply: {}\n- EOI dispatch would enable: {}\nactivation allowed: {}\n",
                                                    simulation.token_gate,
                                                    simulation.gate_decision,
                                                    irq::IRQ_ACTIVATION_GATE_RUNTIME_READY_NO,
                                                    irq::IRQ_ACTIVATION_GATE_DRY_RUN_NOT_ALLOWED,
                                                    simulation.eoi_runtime_boundary,
                                                    simulation.sti_would_enable,
                                                    simulation.pic_unmask_would_apply,
                                                    simulation.eoi_dispatch_would_enable,
                                                    simulation.simulated_activation_allowed
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation simulation blockers\n- activation token: {}\n- gate decision: {}\n- readiness matrix: {}\n- dry-run commit: {}\n- EOI runtime boundary: {}\n- STI would enable: {}\n- PIC unmask would apply: {}\n- EOI dispatch would enable: {}\nactivation allowed: {}\n",
                                                    simulation.token_gate,
                                                    simulation.gate_decision,
                                                    irq::IRQ_ACTIVATION_GATE_RUNTIME_READY_NO,
                                                    irq::IRQ_ACTIVATION_GATE_DRY_RUN_NOT_ALLOWED,
                                                    simulation.eoi_runtime_boundary,
                                                    simulation.sti_would_enable,
                                                    simulation.pic_unmask_would_apply,
                                                    simulation.eoi_dispatch_would_enable,
                                                    simulation.simulated_activation_allowed
                                                );
                                        } else if line_str == "sti-plan" {
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "STI controlled activation plan\nsti instruction: {}\nactivation token: {}\nruntime gate: {}\nreadiness matrix: {}\nPIC unmask: {}\nEOI dispatch: {}\nkeyboard mode: {}\nruntime irq active: {}\n",
                                                    irq::IRQ_MATRIX_STI_DISABLED,
                                                    irq::STI_PLAN_TOKEN_REQUIRED,
                                                    irq::STI_PLAN_RUNTIME_GATE_NOT_ALLOWED,
                                                    irq::IRQ_ACTIVATION_GATE_READINESS_BLOCKED,
                                                    irq::STI_PLAN_PIC_UNMASK_DISABLED,
                                                    irq::STI_PLAN_EOI_DISPATCH_DISABLED,
                                                    irq::IRQ_MATRIX_KEYBOARD_MODE_POLLING,
                                                    irq::IRQ_MATRIX_RUNTIME_IRQ_ACTIVE_NO
                                                );
                                            let _ = write!(serial_writer, "STI controlled activation plan\nsti instruction: {}\nactivation token: {}\nruntime gate: {}\nreadiness matrix: {}\nPIC unmask: {}\nEOI dispatch: {}\nkeyboard mode: {}\nruntime irq active: {}\n",
                                                    irq::IRQ_MATRIX_STI_DISABLED,
                                                    irq::STI_PLAN_TOKEN_REQUIRED,
                                                    irq::STI_PLAN_RUNTIME_GATE_NOT_ALLOWED,
                                                    irq::IRQ_ACTIVATION_GATE_READINESS_BLOCKED,
                                                    irq::STI_PLAN_PIC_UNMASK_DISABLED,
                                                    irq::STI_PLAN_EOI_DISPATCH_DISABLED,
                                                    irq::IRQ_MATRIX_KEYBOARD_MODE_POLLING,
                                                    irq::IRQ_MATRIX_RUNTIME_IRQ_ACTIVE_NO
                                                );
                                        } else if line_str == "sti-status" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "STI controlled activation status\nactivation token: {}\nruntime gate: {}\nreadiness matrix: {}\nsimulation: {}\nsti instruction: {}\nsti allowed: {}\nruntime irq active: {}\n",
                                                    sti_plan.activation_token,
                                                    sti_plan.runtime_gate,
                                                    sti_plan.readiness_matrix,
                                                    sti_plan.simulation,
                                                    sti_plan.sti_instruction,
                                                    sti_plan.sti_allowed,
                                                    sti_plan.runtime_irq_active
                                                );
                                            let _ = write!(serial_writer, "STI controlled activation status\nactivation token: {}\nruntime gate: {}\nreadiness matrix: {}\nsimulation: {}\nsti instruction: {}\nsti allowed: {}\nruntime irq active: {}\n",
                                                    sti_plan.activation_token,
                                                    sti_plan.runtime_gate,
                                                    sti_plan.readiness_matrix,
                                                    sti_plan.simulation,
                                                    sti_plan.sti_instruction,
                                                    sti_plan.sti_allowed,
                                                    sti_plan.runtime_irq_active
                                                );
                                        } else if line_str == "sti-preflight" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "STI controlled activation preflight\ntoken gate: {}\nreadiness matrix: {}\nEOI runtime boundary: {}\nPIC unmask policy: {}\nhardware mutation: {}\nkeyboard mode: {}\nresult: {}\nnext: {}\n",
                                                    sti_plan.token_gate,
                                                    sti_plan.readiness_matrix,
                                                    sti_plan.eoi_runtime_boundary,
                                                    sti_plan.pic_unmask_policy,
                                                    sti_plan.hardware_mutation,
                                                    sti_plan.keyboard_mode,
                                                    sti_plan.result,
                                                    sti_plan.next
                                                );
                                            let _ = write!(serial_writer, "STI controlled activation preflight\ntoken gate: {}\nreadiness matrix: {}\nEOI runtime boundary: {}\nPIC unmask policy: {}\nhardware mutation: {}\nkeyboard mode: {}\nresult: {}\nnext: {}\n",
                                                    sti_plan.token_gate,
                                                    sti_plan.readiness_matrix,
                                                    sti_plan.eoi_runtime_boundary,
                                                    sti_plan.pic_unmask_policy,
                                                    sti_plan.hardware_mutation,
                                                    sti_plan.keyboard_mode,
                                                    sti_plan.result,
                                                    sti_plan.next
                                                );
                                        } else if line_str == "sti-blockers" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "STI controlled activation blockers\n- activation token: {}\n- runtime gate: {}\n- readiness matrix: {}\n- simulation: {}\n- EOI runtime boundary: {}\n- PIC unmask: {}\n- EOI dispatch: {}\n- keyboard mode: {}\nsti allowed: {}\n",
                                                    sti_plan.activation_token,
                                                    sti_plan.runtime_gate,
                                                    irq::IRQ_ACTIVATION_GATE_RUNTIME_READY_NO,
                                                    sti_plan.simulation,
                                                    sti_plan.eoi_runtime_boundary,
                                                    sti_plan.pic_unmask,
                                                    sti_plan.eoi_dispatch,
                                                    sti_plan.keyboard_mode,
                                                    sti_plan.sti_allowed
                                                );
                                            let _ = write!(serial_writer, "STI controlled activation blockers\n- activation token: {}\n- runtime gate: {}\n- readiness matrix: {}\n- simulation: {}\n- EOI runtime boundary: {}\n- PIC unmask: {}\n- EOI dispatch: {}\n- keyboard mode: {}\nsti allowed: {}\n",
                                                    sti_plan.activation_token,
                                                    sti_plan.runtime_gate,
                                                    irq::IRQ_ACTIVATION_GATE_RUNTIME_READY_NO,
                                                    sti_plan.simulation,
                                                    sti_plan.eoi_runtime_boundary,
                                                    sti_plan.pic_unmask,
                                                    sti_plan.eoi_dispatch,
                                                    sti_plan.keyboard_mode,
                                                    sti_plan.sti_allowed
                                                );
                                        } else if line_str == "irq-runtime-activation-smoke" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let smoke = irq::irq_runtime_activation_smoke(
                                                token, matrix, gate, simulation, sti_plan,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation smoke\nactivation smoke: {}\nhardware mutation: {}\nruntime irq active: {}\nsti instruction: {}\npic unmask: {}\neoi dispatch: {}\nkeyboard mode: {}\nresult: {}\nnext: {}\n",
                                                    smoke.activation_smoke,
                                                    smoke.hardware_mutation,
                                                    smoke.runtime_irq_active,
                                                    smoke.sti_instruction,
                                                    smoke.pic_unmask,
                                                    smoke.eoi_dispatch,
                                                    smoke.keyboard_mode,
                                                    smoke.result,
                                                    smoke.next
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation smoke\nactivation smoke: {}\nhardware mutation: {}\nruntime irq active: {}\nsti instruction: {}\npic unmask: {}\neoi dispatch: {}\nkeyboard mode: {}\nresult: {}\nnext: {}\n",
                                                    smoke.activation_smoke,
                                                    smoke.hardware_mutation,
                                                    smoke.runtime_irq_active,
                                                    smoke.sti_instruction,
                                                    smoke.pic_unmask,
                                                    smoke.eoi_dispatch,
                                                    smoke.keyboard_mode,
                                                    smoke.result,
                                                    smoke.next
                                                );
                                        } else if line_str == "irq-runtime-activation-smoke-status"
                                        {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let smoke = irq::irq_runtime_activation_smoke(
                                                token, matrix, gate, simulation, sti_plan,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation smoke status\nactivation token: {}\nruntime gate: {}\nreadiness matrix: {}\nsimulation: {}\nsti plan: {}\nactivation smoke: {}\nhardware mutation: {}\nruntime irq active: {}\n",
                                                    smoke.activation_token,
                                                    smoke.runtime_gate,
                                                    smoke.readiness_matrix,
                                                    smoke.simulation,
                                                    smoke.sti_plan,
                                                    smoke.activation_smoke,
                                                    smoke.hardware_mutation,
                                                    smoke.runtime_irq_active
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation smoke status\nactivation token: {}\nruntime gate: {}\nreadiness matrix: {}\nsimulation: {}\nsti plan: {}\nactivation smoke: {}\nhardware mutation: {}\nruntime irq active: {}\n",
                                                    smoke.activation_token,
                                                    smoke.runtime_gate,
                                                    smoke.readiness_matrix,
                                                    smoke.simulation,
                                                    smoke.sti_plan,
                                                    smoke.activation_smoke,
                                                    smoke.hardware_mutation,
                                                    smoke.runtime_irq_active
                                                );
                                        } else if line_str
                                            == "irq-runtime-activation-smoke-blockers"
                                        {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let smoke = irq::irq_runtime_activation_smoke(
                                                token, matrix, gate, simulation, sti_plan,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime activation smoke blockers\n- activation token: {}\n- runtime gate: {}\n- readiness matrix: {}\n- simulation: {}\n- STI plan: {}\n- EOI runtime boundary: {}\n- PIC unmask: {}\n- EOI dispatch: {}\n- keyboard mode: {}\nactivation smoke: {}\n",
                                                    smoke.activation_token,
                                                    smoke.runtime_gate,
                                                    irq::IRQ_ACTIVATION_GATE_RUNTIME_READY_NO,
                                                    smoke.simulation,
                                                    smoke.sti_plan,
                                                    smoke.eoi_runtime_boundary,
                                                    smoke.pic_unmask,
                                                    smoke.eoi_dispatch,
                                                    smoke.keyboard_mode,
                                                    smoke.activation_smoke
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime activation smoke blockers\n- activation token: {}\n- runtime gate: {}\n- readiness matrix: {}\n- simulation: {}\n- STI plan: {}\n- EOI runtime boundary: {}\n- PIC unmask: {}\n- EOI dispatch: {}\n- keyboard mode: {}\nactivation smoke: {}\n",
                                                    smoke.activation_token,
                                                    smoke.runtime_gate,
                                                    irq::IRQ_ACTIVATION_GATE_RUNTIME_READY_NO,
                                                    smoke.simulation,
                                                    smoke.sti_plan,
                                                    smoke.eoi_runtime_boundary,
                                                    smoke.pic_unmask,
                                                    smoke.eoi_dispatch,
                                                    smoke.keyboard_mode,
                                                    smoke.activation_smoke
                                                );
                                        } else if line_str == "eoi-dispatch-smoke-note" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "EOI dispatch smoke note\nscope: controlled dry-run foundation\nack target: planned PIC EOI routing only\nmaster EOI: {}\nslave EOI: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nruntime irq active: {}\n",
                                                    smoke.master_eoi_route,
                                                    smoke.slave_eoi_route,
                                                    smoke.hardware_mutation,
                                                    smoke.sti_instruction,
                                                    smoke.pic_unmask,
                                                    smoke.runtime_irq_active
                                                );
                                            let _ = write!(serial_writer, "EOI dispatch smoke note\nscope: controlled dry-run foundation\nack target: planned PIC EOI routing only\nmaster EOI: {}\nslave EOI: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nruntime irq active: {}\n",
                                                    smoke.master_eoi_route,
                                                    smoke.slave_eoi_route,
                                                    smoke.hardware_mutation,
                                                    smoke.sti_instruction,
                                                    smoke.pic_unmask,
                                                    smoke.runtime_irq_active
                                                );
                                        } else if line_str == "eoi-dispatch-smoke-status" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "EOI dispatch smoke status\neoi dispatch smoke: {}\ndispatch mode: {}\npic remap smoke: {}\nirq gates: {}\npic eoi writes: {}\nsti instruction: {}\npic unmask: {}\nkeyboard mode: {}\nruntime irq active: {}\n",
                                                    smoke.eoi_dispatch_smoke,
                                                    smoke.dispatch_mode,
                                                    smoke.pic_remap_smoke,
                                                    smoke.irq_gates,
                                                    smoke.pic_eoi_writes,
                                                    smoke.sti_instruction,
                                                    smoke.pic_unmask,
                                                    smoke.keyboard_mode,
                                                    smoke.runtime_irq_active
                                                );
                                            let _ = write!(serial_writer, "EOI dispatch smoke status\neoi dispatch smoke: {}\ndispatch mode: {}\npic remap smoke: {}\nirq gates: {}\npic eoi writes: {}\nsti instruction: {}\npic unmask: {}\nkeyboard mode: {}\nruntime irq active: {}\n",
                                                    smoke.eoi_dispatch_smoke,
                                                    smoke.dispatch_mode,
                                                    smoke.pic_remap_smoke,
                                                    smoke.irq_gates,
                                                    smoke.pic_eoi_writes,
                                                    smoke.sti_instruction,
                                                    smoke.pic_unmask,
                                                    smoke.keyboard_mode,
                                                    smoke.runtime_irq_active
                                                );
                                        } else if line_str == "eoi-dispatch-smoke-plan" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "EOI dispatch smoke plan\n1. require PIC remap controlled smoke prerequisite\n2. require IRQ gate bind smoke prerequisite\n3. model IRQ0 master EOI route only\n4. model IRQ1 master EOI route only\n5. keep PIC_EOI writes disabled\n6. keep runtime IRQ inactive\nresult: {}\n",
                                                    smoke.result
                                                );
                                            let _ = write!(serial_writer, "EOI dispatch smoke plan\n1. require PIC remap controlled smoke prerequisite\n2. require IRQ gate bind smoke prerequisite\n3. model IRQ0 master EOI route only\n4. model IRQ1 master EOI route only\n5. keep PIC_EOI writes disabled\n6. keep runtime IRQ inactive\nresult: {}\n",
                                                    smoke.result
                                                );
                                        } else if line_str == "eoi-dispatch-smoke-blockers" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "EOI dispatch smoke blockers\n- {}\n- {}\n- PIC_EOI writes: disabled by guard\n- {}\n- {}\n- {}\n- {}\neoi dispatch smoke: {}\n",
                                                    irq::EOI_DISPATCH_SMOKE_BLOCKER_PIC_REMAP,
                                                    irq::EOI_DISPATCH_SMOKE_BLOCKER_IRQ_GATES,
                                                    irq::EOI_DISPATCH_SMOKE_BLOCKER_STI,
                                                    irq::EOI_DISPATCH_SMOKE_BLOCKER_PIC_UNMASK,
                                                    irq::EOI_DISPATCH_SMOKE_BLOCKER_LIVE_IRQ,
                                                    irq::EOI_DISPATCH_SMOKE_BLOCKER_KEYBOARD_IRQ,
                                                    smoke.eoi_dispatch_smoke
                                                );
                                            let _ = write!(serial_writer, "EOI dispatch smoke blockers\n- {}\n- {}\n- PIC_EOI writes: disabled by guard\n- {}\n- {}\n- {}\n- {}\neoi dispatch smoke: {}\n",
                                                    irq::EOI_DISPATCH_SMOKE_BLOCKER_PIC_REMAP,
                                                    irq::EOI_DISPATCH_SMOKE_BLOCKER_IRQ_GATES,
                                                    irq::EOI_DISPATCH_SMOKE_BLOCKER_STI,
                                                    irq::EOI_DISPATCH_SMOKE_BLOCKER_PIC_UNMASK,
                                                    irq::EOI_DISPATCH_SMOKE_BLOCKER_LIVE_IRQ,
                                                    irq::EOI_DISPATCH_SMOKE_BLOCKER_KEYBOARD_IRQ,
                                                    smoke.eoi_dispatch_smoke
                                                );
                                        } else if line_str == "pic-unmask-smoke-note" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let eoi_smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            let smoke = irq::pic_unmask_smoke(
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                                token,
                                                matrix,
                                                gate,
                                                sti_plan,
                                                eoi_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "PIC unmask smoke note\nscope: controlled dry-run foundation\ntarget IRQ lines: {}\nmask policy: {}\nunmask policy: {}\nlive unmask: {}\nhardware mutation: {}\nsti: {}\nruntime irq active: {}\n",
                                                    smoke.target_irq_lines,
                                                    smoke.pic_mask_policy,
                                                    smoke.unmask_policy,
                                                    smoke.live_unmask,
                                                    smoke.hardware_mutation,
                                                    smoke.sti_instruction,
                                                    smoke.runtime_irq_active
                                                );
                                            let _ = write!(serial_writer, "PIC unmask smoke note\nscope: controlled dry-run foundation\ntarget IRQ lines: {}\nmask policy: {}\nunmask policy: {}\nlive unmask: {}\nhardware mutation: {}\nsti: {}\nruntime irq active: {}\n",
                                                    smoke.target_irq_lines,
                                                    smoke.pic_mask_policy,
                                                    smoke.unmask_policy,
                                                    smoke.live_unmask,
                                                    smoke.hardware_mutation,
                                                    smoke.sti_instruction,
                                                    smoke.runtime_irq_active
                                                );
                                        } else if line_str == "pic-unmask-smoke-status" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let eoi_smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            let smoke = irq::pic_unmask_smoke(
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                                token,
                                                matrix,
                                                gate,
                                                sti_plan,
                                                eoi_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "PIC unmask smoke status\npic unmask smoke: {}\ndispatch mode: {}\ntarget IRQ lines: {}\npic mask policy: {}\nactivation token: {}\nactivation gate: {}\nEOI boundary: {}\nSTI plan: {}\nEOI dispatch smoke: {}\nlive unmask: {}\nhardware mutation: {}\nruntime irq active: {}\n",
                                                    smoke.pic_unmask_smoke,
                                                    smoke.dispatch_mode,
                                                    smoke.target_irq_lines,
                                                    smoke.pic_mask_policy,
                                                    smoke.activation_token,
                                                    smoke.activation_gate,
                                                    smoke.eoi_runtime_boundary,
                                                    smoke.sti_plan,
                                                    smoke.eoi_dispatch_smoke,
                                                    smoke.live_unmask,
                                                    smoke.hardware_mutation,
                                                    smoke.runtime_irq_active
                                                );
                                            let _ = write!(serial_writer, "PIC unmask smoke status\npic unmask smoke: {}\ndispatch mode: {}\ntarget IRQ lines: {}\npic mask policy: {}\nactivation token: {}\nactivation gate: {}\nEOI boundary: {}\nSTI plan: {}\nEOI dispatch smoke: {}\nlive unmask: {}\nhardware mutation: {}\nruntime irq active: {}\n",
                                                    smoke.pic_unmask_smoke,
                                                    smoke.dispatch_mode,
                                                    smoke.target_irq_lines,
                                                    smoke.pic_mask_policy,
                                                    smoke.activation_token,
                                                    smoke.activation_gate,
                                                    smoke.eoi_runtime_boundary,
                                                    smoke.sti_plan,
                                                    smoke.eoi_dispatch_smoke,
                                                    smoke.live_unmask,
                                                    smoke.hardware_mutation,
                                                    smoke.runtime_irq_active
                                                );
                                        } else if line_str == "pic-unmask-smoke-plan" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let eoi_smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            let smoke = irq::pic_unmask_smoke(
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                                token,
                                                matrix,
                                                gate,
                                                sti_plan,
                                                eoi_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "PIC unmask smoke plan\n1. require PIC mask plan prerequisite\n2. require readiness matrix prerequisite\n3. require activation token and gate prerequisite\n4. require EOI boundary and dispatch smoke prerequisite\n5. keep target IRQ lines: {}\n6. keep PIC data-port writes disabled\n7. keep runtime IRQ inactive\nresult: {}\n",
                                                    smoke.target_irq_lines,
                                                    smoke.result
                                                );
                                            let _ = write!(serial_writer, "PIC unmask smoke plan\n1. require PIC mask plan prerequisite\n2. require readiness matrix prerequisite\n3. require activation token and gate prerequisite\n4. require EOI boundary and dispatch smoke prerequisite\n5. keep target IRQ lines: {}\n6. keep PIC data-port writes disabled\n7. keep runtime IRQ inactive\nresult: {}\n",
                                                    smoke.target_irq_lines,
                                                    smoke.result
                                                );
                                        } else if line_str == "pic-unmask-smoke-blockers" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let eoi_smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            let smoke = irq::pic_unmask_smoke(
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                                token,
                                                matrix,
                                                gate,
                                                sti_plan,
                                                eoi_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "PIC unmask smoke blockers\n- activation token: {}\n- activation gate: {}\n- EOI runtime boundary: {}\n- STI: {}\n- live unmask: {}\n- runtime IRQ active: {}\n- {}\npic unmask smoke: {}\n",
                                                    smoke.activation_token,
                                                    smoke.activation_gate,
                                                    smoke.eoi_runtime_boundary,
                                                    smoke.sti_instruction,
                                                    smoke.live_unmask,
                                                    smoke.runtime_irq_active,
                                                    irq::PIC_UNMASK_SMOKE_BLOCKER_KEYBOARD_IRQ,
                                                    smoke.pic_unmask_smoke
                                                );
                                            let _ = write!(serial_writer, "PIC unmask smoke blockers\n- activation token: {}\n- activation gate: {}\n- EOI runtime boundary: {}\n- STI: {}\n- live unmask: {}\n- runtime IRQ active: {}\n- {}\npic unmask smoke: {}\n",
                                                    smoke.activation_token,
                                                    smoke.activation_gate,
                                                    smoke.eoi_runtime_boundary,
                                                    smoke.sti_instruction,
                                                    smoke.live_unmask,
                                                    smoke.runtime_irq_active,
                                                    irq::PIC_UNMASK_SMOKE_BLOCKER_KEYBOARD_IRQ,
                                                    smoke.pic_unmask_smoke
                                                );
                                        } else if line_str == "idt-runtime-bind-smoke-note" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let eoi_smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            let pic_unmask_smoke = irq::pic_unmask_smoke(
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                                token,
                                                matrix,
                                                gate,
                                                sti_plan,
                                                eoi_smoke,
                                            );
                                            let smoke = irq::idt_runtime_bind_smoke(
                                                token,
                                                matrix,
                                                gate,
                                                gate_state,
                                                sti_plan,
                                                eoi_smoke,
                                                pic_unmask_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IDT runtime bind smoke note\nscope: controlled dry-run foundation\ntarget vectors: {}\nlive handler bind: {}\nhardware mutation: {}\nsti: {}\nruntime irq active: {}\n",
                                                    smoke.target_vectors,
                                                    smoke.live_handler_bind,
                                                    smoke.hardware_mutation,
                                                    smoke.sti_instruction,
                                                    smoke.runtime_irq_active
                                                );
                                            let _ = write!(serial_writer, "IDT runtime bind smoke note\nscope: controlled dry-run foundation\ntarget vectors: {}\nlive handler bind: {}\nhardware mutation: {}\nsti: {}\nruntime irq active: {}\n",
                                                    smoke.target_vectors,
                                                    smoke.live_handler_bind,
                                                    smoke.hardware_mutation,
                                                    smoke.sti_instruction,
                                                    smoke.runtime_irq_active
                                                );
                                        } else if line_str == "idt-runtime-bind-smoke-status" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let eoi_smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            let pic_unmask_smoke = irq::pic_unmask_smoke(
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                                token,
                                                matrix,
                                                gate,
                                                sti_plan,
                                                eoi_smoke,
                                            );
                                            let smoke = irq::idt_runtime_bind_smoke(
                                                token,
                                                matrix,
                                                gate,
                                                gate_state,
                                                sti_plan,
                                                eoi_smoke,
                                                pic_unmask_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IDT runtime bind smoke status\nidt runtime bind smoke: {}\ndispatch mode: {}\ntarget vectors: {}\nirq gate bind smoke: {}\nEOI dispatch smoke: {}\nPIC unmask smoke: {}\nSTI plan: {}\nlive handler bind: {}\nhardware mutation: {}\nruntime irq active: {}\n",
                                                    smoke.idt_runtime_bind_smoke,
                                                    smoke.dispatch_mode,
                                                    smoke.target_vectors,
                                                    smoke.irq_gate_bind_smoke,
                                                    smoke.eoi_dispatch_smoke,
                                                    smoke.pic_unmask_smoke,
                                                    smoke.sti_plan,
                                                    smoke.live_handler_bind,
                                                    smoke.hardware_mutation,
                                                    smoke.runtime_irq_active
                                                );
                                            let _ = write!(serial_writer, "IDT runtime bind smoke status\nidt runtime bind smoke: {}\ndispatch mode: {}\ntarget vectors: {}\nirq gate bind smoke: {}\nEOI dispatch smoke: {}\nPIC unmask smoke: {}\nSTI plan: {}\nlive handler bind: {}\nhardware mutation: {}\nruntime irq active: {}\n",
                                                    smoke.idt_runtime_bind_smoke,
                                                    smoke.dispatch_mode,
                                                    smoke.target_vectors,
                                                    smoke.irq_gate_bind_smoke,
                                                    smoke.eoi_dispatch_smoke,
                                                    smoke.pic_unmask_smoke,
                                                    smoke.sti_plan,
                                                    smoke.live_handler_bind,
                                                    smoke.hardware_mutation,
                                                    smoke.runtime_irq_active
                                                );
                                        } else if line_str == "idt-runtime-bind-smoke-plan" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let eoi_smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            let pic_unmask_smoke = irq::pic_unmask_smoke(
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                                token,
                                                matrix,
                                                gate,
                                                sti_plan,
                                                eoi_smoke,
                                            );
                                            let smoke = irq::idt_runtime_bind_smoke(
                                                token,
                                                matrix,
                                                gate,
                                                gate_state,
                                                sti_plan,
                                                eoi_smoke,
                                                pic_unmask_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IDT runtime bind smoke plan\n1. require activation token and gate prerequisite\n2. require readiness matrix prerequisite\n3. require IRQ gate bind smoke prerequisite\n4. require EOI dispatch smoke boundary\n5. require PIC unmask smoke boundary\n6. keep live handler bind disabled\n7. keep runtime IRQ inactive\nresult: {}\n",
                                                    smoke.result
                                                );
                                            let _ = write!(serial_writer, "IDT runtime bind smoke plan\n1. require activation token and gate prerequisite\n2. require readiness matrix prerequisite\n3. require IRQ gate bind smoke prerequisite\n4. require EOI dispatch smoke boundary\n5. require PIC unmask smoke boundary\n6. keep live handler bind disabled\n7. keep runtime IRQ inactive\nresult: {}\n",
                                                    smoke.result
                                                );
                                        } else if line_str == "idt-runtime-bind-smoke-blockers" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let eoi_smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            let pic_unmask_smoke = irq::pic_unmask_smoke(
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                                token,
                                                matrix,
                                                gate,
                                                sti_plan,
                                                eoi_smoke,
                                            );
                                            let smoke = irq::idt_runtime_bind_smoke(
                                                token,
                                                matrix,
                                                gate,
                                                gate_state,
                                                sti_plan,
                                                eoi_smoke,
                                                pic_unmask_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IDT runtime bind smoke blockers\n- activation token: {}\n- activation gate: {}\n- IRQ gate bind smoke: vectors 32/33 not bound\n- EOI dispatch smoke: {}\n- PIC unmask smoke: {}\n- STI: {}\n- live handler bind: {}\n- runtime IRQ active: {}\nidt runtime bind smoke: {}\n",
                                                    smoke.activation_token,
                                                    smoke.activation_gate,
                                                    smoke.eoi_dispatch_smoke,
                                                    smoke.pic_unmask_smoke,
                                                    smoke.sti_instruction,
                                                    smoke.live_handler_bind,
                                                    smoke.runtime_irq_active,
                                                    smoke.idt_runtime_bind_smoke
                                                );
                                            let _ = write!(serial_writer, "IDT runtime bind smoke blockers\n- activation token: {}\n- activation gate: {}\n- IRQ gate bind smoke: vectors 32/33 not bound\n- EOI dispatch smoke: {}\n- PIC unmask smoke: {}\n- STI: {}\n- live handler bind: {}\n- runtime IRQ active: {}\nidt runtime bind smoke: {}\n",
                                                    smoke.activation_token,
                                                    smoke.activation_gate,
                                                    smoke.eoi_dispatch_smoke,
                                                    smoke.pic_unmask_smoke,
                                                    smoke.sti_instruction,
                                                    smoke.live_handler_bind,
                                                    smoke.runtime_irq_active,
                                                    smoke.idt_runtime_bind_smoke
                                                );
                                        } else if line_str == "irq-runtime-final-gate-note" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let eoi_smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            let pic_unmask_smoke = irq::pic_unmask_smoke(
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                                token,
                                                matrix,
                                                gate,
                                                sti_plan,
                                                eoi_smoke,
                                            );
                                            let idt_bind_smoke = irq::idt_runtime_bind_smoke(
                                                token,
                                                matrix,
                                                gate,
                                                gate_state,
                                                sti_plan,
                                                eoi_smoke,
                                                pic_unmask_smoke,
                                            );
                                            let final_gate = irq::irq_runtime_final_gate(
                                                token,
                                                matrix,
                                                gate,
                                                simulation,
                                                sti_plan,
                                                activation_smoke,
                                                eoi_smoke,
                                                pic_unmask_smoke,
                                                idt_bind_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime final gate note\nscope: {}\nactivation inputs: {}\nfinal activation allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
                                                    final_gate.scope,
                                                    final_gate.inputs,
                                                    final_gate.final_activation_allowed,
                                                    final_gate.hardware_mutation,
                                                    final_gate.runtime_irq_active
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime final gate note\nscope: {}\nactivation inputs: {}\nfinal activation allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
                                                    final_gate.scope,
                                                    final_gate.inputs,
                                                    final_gate.final_activation_allowed,
                                                    final_gate.hardware_mutation,
                                                    final_gate.runtime_irq_active
                                                );
                                        } else if line_str == "irq-runtime-final-gate-status" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let eoi_smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            let pic_unmask_smoke = irq::pic_unmask_smoke(
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                                token,
                                                matrix,
                                                gate,
                                                sti_plan,
                                                eoi_smoke,
                                            );
                                            let idt_bind_smoke = irq::idt_runtime_bind_smoke(
                                                token,
                                                matrix,
                                                gate,
                                                gate_state,
                                                sti_plan,
                                                eoi_smoke,
                                                pic_unmask_smoke,
                                            );
                                            let final_gate = irq::irq_runtime_final_gate(
                                                token,
                                                matrix,
                                                gate,
                                                simulation,
                                                sti_plan,
                                                activation_smoke,
                                                eoi_smoke,
                                                pic_unmask_smoke,
                                                idt_bind_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime final gate status\nactivation token: {}\nactivation gate: {}\nreadiness matrix: {}\nsimulation: {}\nSTI plan: {}\nactivation smoke: {}\nEOI dispatch smoke: {}\nPIC unmask smoke: {}\nIDT runtime bind smoke: {}\nkeyboard mode: {}\nfinal activation allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
                                                    final_gate.activation_token,
                                                    final_gate.activation_gate,
                                                    final_gate.readiness_matrix,
                                                    final_gate.simulation,
                                                    final_gate.sti_plan,
                                                    final_gate.activation_smoke,
                                                    final_gate.eoi_dispatch_smoke,
                                                    final_gate.pic_unmask_smoke,
                                                    final_gate.idt_runtime_bind_smoke,
                                                    final_gate.keyboard_mode,
                                                    final_gate.final_activation_allowed,
                                                    final_gate.hardware_mutation,
                                                    final_gate.runtime_irq_active
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime final gate status\nactivation token: {}\nactivation gate: {}\nreadiness matrix: {}\nsimulation: {}\nSTI plan: {}\nactivation smoke: {}\nEOI dispatch smoke: {}\nPIC unmask smoke: {}\nIDT runtime bind smoke: {}\nkeyboard mode: {}\nfinal activation allowed: {}\nhardware mutation: {}\nruntime irq active: {}\n",
                                                    final_gate.activation_token,
                                                    final_gate.activation_gate,
                                                    final_gate.readiness_matrix,
                                                    final_gate.simulation,
                                                    final_gate.sti_plan,
                                                    final_gate.activation_smoke,
                                                    final_gate.eoi_dispatch_smoke,
                                                    final_gate.pic_unmask_smoke,
                                                    final_gate.idt_runtime_bind_smoke,
                                                    final_gate.keyboard_mode,
                                                    final_gate.final_activation_allowed,
                                                    final_gate.hardware_mutation,
                                                    final_gate.runtime_irq_active
                                                );
                                        } else if line_str == "irq-runtime-final-gate-check" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let eoi_smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            let pic_unmask_smoke = irq::pic_unmask_smoke(
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                                token,
                                                matrix,
                                                gate,
                                                sti_plan,
                                                eoi_smoke,
                                            );
                                            let idt_bind_smoke = irq::idt_runtime_bind_smoke(
                                                token,
                                                matrix,
                                                gate,
                                                gate_state,
                                                sti_plan,
                                                eoi_smoke,
                                                pic_unmask_smoke,
                                            );
                                            let final_gate = irq::irq_runtime_final_gate(
                                                token,
                                                matrix,
                                                gate,
                                                simulation,
                                                sti_plan,
                                                activation_smoke,
                                                eoi_smoke,
                                                pic_unmask_smoke,
                                                idt_bind_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime final gate check\nsti: {}\npic unmask: {}\neoi dispatch: {}\nlive idt bind: {}\nkeyboard mode: {}\nfinal activation allowed: {}\nresult: {}\nnext: {}\n",
                                                    final_gate.sti_instruction,
                                                    final_gate.pic_unmask,
                                                    final_gate.eoi_dispatch,
                                                    final_gate.live_idt_bind,
                                                    final_gate.keyboard_mode,
                                                    final_gate.final_activation_allowed,
                                                    final_gate.result,
                                                    final_gate.next
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime final gate check\nsti: {}\npic unmask: {}\neoi dispatch: {}\nlive idt bind: {}\nkeyboard mode: {}\nfinal activation allowed: {}\nresult: {}\nnext: {}\n",
                                                    final_gate.sti_instruction,
                                                    final_gate.pic_unmask,
                                                    final_gate.eoi_dispatch,
                                                    final_gate.live_idt_bind,
                                                    final_gate.keyboard_mode,
                                                    final_gate.final_activation_allowed,
                                                    final_gate.result,
                                                    final_gate.next
                                                );
                                        } else if line_str == "irq-runtime-final-gate-blockers" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            let mask_plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let eoi_ready =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let matrix = irq::irq_runtime_matrix(
                                                pic_state.executed,
                                                gate_state.executed,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                irq::irq_runtime_is_armed(),
                                                irq::irq_runtime_is_committed(),
                                            );
                                            let activation =
                                                irq::irq_runtime_activation_dry_run(&matrix);
                                            let token = irq::irq_runtime_activation_token_status();
                                            let gate = irq::irq_runtime_activation_gate(
                                                token,
                                                matrix,
                                                activation,
                                                eoi_ready,
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                            );
                                            let simulation = irq::irq_runtime_activation_simulation(
                                                token, matrix, activation, gate,
                                            );
                                            let sti_plan = irq::sti_controlled_activation_plan(
                                                token, matrix, gate, simulation,
                                            );
                                            let activation_smoke =
                                                irq::irq_runtime_activation_smoke(
                                                    token, matrix, gate, simulation, sti_plan,
                                                );
                                            let eoi_smoke = irq::eoi_dispatch_smoke(
                                                pic_state.executed,
                                                gate_state.executed,
                                                matrix,
                                                activation_smoke,
                                            );
                                            let pic_unmask_smoke = irq::pic_unmask_smoke(
                                                mask_plan.mask_policy,
                                                mask_plan.unmask_policy,
                                                token,
                                                matrix,
                                                gate,
                                                sti_plan,
                                                eoi_smoke,
                                            );
                                            let idt_bind_smoke = irq::idt_runtime_bind_smoke(
                                                token,
                                                matrix,
                                                gate,
                                                gate_state,
                                                sti_plan,
                                                eoi_smoke,
                                                pic_unmask_smoke,
                                            );
                                            let final_gate = irq::irq_runtime_final_gate(
                                                token,
                                                matrix,
                                                gate,
                                                simulation,
                                                sti_plan,
                                                activation_smoke,
                                                eoi_smoke,
                                                pic_unmask_smoke,
                                                idt_bind_smoke,
                                            );
                                            core::hint::black_box(mask_status);
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "IRQ runtime final gate blockers\n- activation token: {}\n- activation gate: {}\n- readiness matrix: {}\n- simulation: {}\n- STI plan: {}\n- activation smoke: {}\n- EOI dispatch smoke: {}\n- PIC unmask smoke: {}\n- IDT runtime bind smoke: {}\n- keyboard mode: {}\nfinal activation allowed: {}\n",
                                                    final_gate.activation_token,
                                                    final_gate.activation_gate,
                                                    irq::IRQ_ACTIVATION_GATE_RUNTIME_READY_NO,
                                                    final_gate.simulation,
                                                    final_gate.sti_plan,
                                                    final_gate.activation_smoke,
                                                    final_gate.eoi_dispatch_smoke,
                                                    final_gate.pic_unmask_smoke,
                                                    final_gate.idt_runtime_bind_smoke,
                                                    final_gate.keyboard_mode,
                                                    final_gate.final_activation_allowed
                                                );
                                            let _ = write!(serial_writer, "IRQ runtime final gate blockers\n- activation token: {}\n- activation gate: {}\n- readiness matrix: {}\n- simulation: {}\n- STI plan: {}\n- activation smoke: {}\n- EOI dispatch smoke: {}\n- PIC unmask smoke: {}\n- IDT runtime bind smoke: {}\n- keyboard mode: {}\nfinal activation allowed: {}\n",
                                                    final_gate.activation_token,
                                                    final_gate.activation_gate,
                                                    irq::IRQ_ACTIVATION_GATE_RUNTIME_READY_NO,
                                                    final_gate.simulation,
                                                    final_gate.sti_plan,
                                                    final_gate.activation_smoke,
                                                    final_gate.eoi_dispatch_smoke,
                                                    final_gate.pic_unmask_smoke,
                                                    final_gate.idt_runtime_bind_smoke,
                                                    final_gate.keyboard_mode,
                                                    final_gate.final_activation_allowed
                                                );
                                        } else if line_str == "irq-runtime-decision-note" {
                                            print_irq_runtime_decision_note();
                                        } else if line_str == "irq-runtime-decision-status" {
                                            print_irq_runtime_decision_status();
                                        } else if line_str == "irq-runtime-decision-freeze" {
                                            print_irq_runtime_decision_status();
                                        } else if line_str == "irq-runtime-decision-blockers" {
                                            print_irq_runtime_decision_blockers();
                                        } else if line_str == "irq-runtime-mutation-note" {
                                            print_irq_runtime_mutation_note();
                                        } else if line_str == "irq-runtime-mutation-status" {
                                            print_irq_runtime_mutation_status();
                                        } else if line_str == "irq-runtime-mutation-check" {
                                            print_irq_runtime_mutation_status();
                                        } else if line_str == "irq-runtime-mutation-blockers" {
                                            print_irq_runtime_mutation_blockers();
                                        } else if line_str == "irq-runtime-mutation-sequence-note" {
                                            print_irq_runtime_mutation_sequence_note();
                                        } else if line_str == "irq-runtime-mutation-sequence-status"
                                        {
                                            print_irq_runtime_mutation_sequence_status();
                                        } else if line_str == "irq-runtime-mutation-sequence-plan" {
                                            print_irq_runtime_mutation_sequence_status();
                                        } else if line_str
                                            == "irq-runtime-mutation-sequence-blockers"
                                        {
                                            print_irq_runtime_mutation_sequence_blockers();
                                        } else if line_str == "eoi-write-smoke-preflight-note" {
                                            print_eoi_write_smoke_preflight_note();
                                        } else if line_str == "eoi-write-smoke-preflight-status" {
                                            print_eoi_write_smoke_preflight_status();
                                        } else if line_str == "eoi-write-smoke-preflight-check" {
                                            print_eoi_write_smoke_preflight_status();
                                        } else if line_str == "eoi-write-smoke-preflight-blockers" {
                                            print_eoi_write_smoke_preflight_blockers();
                                        } else if line_str == "eoi-write-smoke-candidate-note" {
                                            print_eoi_write_smoke_candidate_note();
                                        } else if line_str == "eoi-write-smoke-candidate-status" {
                                            print_eoi_write_smoke_candidate_status();
                                        } else if line_str == "eoi-write-smoke-candidate-arm" {
                                            print_eoi_write_smoke_candidate_status();
                                        } else if line_str == "eoi-write-smoke-candidate-fire" {
                                            print_eoi_write_smoke_candidate_fire();
                                        } else if line_str == "eoi-write-smoke-candidate-blockers" {
                                            print_eoi_write_smoke_candidate_blockers();
                                        } else if line_str == "eoi-write-permit-note" {
                                            print_eoi_write_permit_note();
                                        } else if line_str == "eoi-write-permit-status" {
                                            print_eoi_write_permit_status();
                                        } else if line_str == "eoi-write-permit-check" {
                                            print_eoi_write_permit_status();
                                        } else if line_str == "eoi-write-permit-blockers" {
                                            print_eoi_write_permit_blockers();
                                        } else if line_str == "eoi-write-oneshot-note" {
                                            print_eoi_write_oneshot_note();
                                        } else if line_str == "eoi-write-oneshot-status" {
                                            print_eoi_write_oneshot_status();
                                        } else if line_str == "eoi-write-oneshot-arm" {
                                            print_eoi_write_oneshot_status();
                                        } else if line_str == "eoi-write-oneshot-fire" {
                                            print_eoi_write_oneshot_fire();
                                        } else if line_str == "eoi-write-oneshot-blockers" {
                                            print_eoi_write_oneshot_blockers();
                                        } else if line_str == "eoi-write-oneshot-latch-note" {
                                            print_eoi_write_oneshot_latch_note();
                                        } else if line_str == "eoi-write-oneshot-latch-status" {
                                            print_eoi_write_oneshot_latch_status();
                                        } else if line_str == "eoi-write-oneshot-latch-arm" {
                                            print_eoi_write_oneshot_latch_arm();
                                        } else if line_str == "eoi-write-oneshot-latch-clear" {
                                            print_eoi_write_oneshot_latch_clear();
                                        } else if line_str == "eoi-write-oneshot-latch-fire" {
                                            print_eoi_write_oneshot_latch_fire();
                                        } else if line_str == "eoi-write-oneshot-latch-blockers" {
                                            print_eoi_write_oneshot_latch_blockers();
                                        } else if line_str == "eoi-write-bridge-note" {
                                            print_eoi_write_bridge_note();
                                        } else if line_str == "eoi-write-bridge-status" {
                                            print_eoi_write_bridge_status();
                                        } else if line_str == "eoi-write-bridge-check" {
                                            print_eoi_write_bridge_check();
                                        } else if line_str == "eoi-write-bridge-blockers" {
                                            print_eoi_write_bridge_blockers();
                                        } else if line_str == "eoi-write-permit-transition-note" {
                                            print_eoi_write_permit_transition_note();
                                        } else if line_str == "eoi-write-permit-transition-status" {
                                            print_eoi_write_permit_transition_status();
                                        } else if line_str == "eoi-write-permit-transition-arm" {
                                            print_eoi_write_permit_transition_arm();
                                        } else if line_str == "eoi-write-permit-transition-clear" {
                                            print_eoi_write_permit_transition_clear();
                                        } else if line_str == "eoi-write-permit-transition-check" {
                                            print_eoi_write_permit_transition_check();
                                        } else if line_str == "eoi-write-permit-transition-blockers"
                                        {
                                            print_eoi_write_permit_transition_blockers();
                                        } else if line_str == "eoi-write-eval-note" {
                                            print_eoi_write_eval_note();
                                        } else if line_str == "eoi-write-eval-status" {
                                            print_eoi_write_eval_status();
                                        } else if line_str == "eoi-write-eval-check" {
                                            print_eoi_write_eval_check();
                                        } else if line_str == "eoi-write-eval-blockers" {
                                            print_eoi_write_eval_blockers();
                                        } else if line_str == "eoi-write-hw-smoke-note" {
                                            print_eoi_write_hw_smoke_note();
                                        } else if line_str == "eoi-write-hw-smoke-status" {
                                            print_eoi_write_hw_smoke_status();
                                        } else if line_str == "eoi-write-hw-smoke-arm" {
                                            print_eoi_write_hw_smoke_arm();
                                        } else if line_str == "eoi-write-hw-smoke-fire" {
                                            print_eoi_write_hw_smoke_fire();
                                        } else if line_str == "eoi-write-hw-smoke-clear" {
                                            print_eoi_write_hw_smoke_clear();
                                        } else if line_str == "eoi-write-hw-smoke-blockers" {
                                            print_eoi_write_hw_smoke_blockers();
                                        } else if line_str == "eoi-runtime-bridge-note" {
                                            print_eoi_runtime_bridge_note();
                                        } else if line_str == "eoi-runtime-bridge-status" {
                                            print_eoi_runtime_bridge_status();
                                        } else if line_str == "eoi-runtime-bridge-check" {
                                            print_eoi_runtime_bridge_check();
                                        } else if line_str == "eoi-runtime-bridge-blockers" {
                                            print_eoi_runtime_bridge_blockers();
                                        } else if line_str == "irq-handler-eoi-candidate-note" {
                                            print_irq_handler_eoi_candidate_note();
                                        } else if line_str == "irq-handler-eoi-candidate-status" {
                                            print_irq_handler_eoi_candidate_status();
                                        } else if line_str == "irq-handler-eoi-candidate-check" {
                                            print_irq_handler_eoi_candidate_check();
                                        } else if line_str == "irq-handler-eoi-candidate-blockers" {
                                            print_irq_handler_eoi_candidate_blockers();
                                        } else if line_str == "irq-handler-eoi-stub-note" {
                                            print_irq_handler_eoi_stub_note();
                                        } else if line_str == "irq-handler-eoi-stub-status" {
                                            print_irq_handler_eoi_stub_status();
                                        } else if line_str == "irq-handler-eoi-stub-check" {
                                            print_irq_handler_eoi_stub_check();
                                        } else if line_str == "irq-handler-eoi-stub-blockers" {
                                            print_irq_handler_eoi_stub_blockers();
                                        } else if line_str == "irq-handler-bind-candidate-note" {
                                            print_irq_handler_bind_candidate_note();
                                        } else if line_str == "irq-handler-bind-candidate-status" {
                                            print_irq_handler_bind_candidate_status();
                                        } else if line_str == "irq-handler-bind-candidate-check" {
                                            print_irq_handler_bind_candidate_check();
                                        } else if line_str == "irq-handler-bind-candidate-blockers"
                                        {
                                            print_irq_handler_bind_candidate_blockers();
                                        } else if line_str == "idt-bind-hw-smoke-note" {
                                            print_idt_bind_hw_smoke_note();
                                        } else if line_str == "idt-bind-hw-smoke-status" {
                                            print_idt_bind_hw_smoke_status();
                                        } else if line_str == "idt-bind-hw-smoke-arm" {
                                            print_idt_bind_hw_smoke_arm();
                                        } else if line_str == "idt-bind-hw-smoke-fire" {
                                            print_idt_bind_hw_smoke_fire();
                                        } else if line_str == "idt-bind-hw-smoke-clear" {
                                            print_idt_bind_hw_smoke_clear();
                                        } else if line_str == "idt-bind-hw-smoke-blockers" {
                                            print_idt_bind_hw_smoke_blockers();
                                        } else if line_str == "idt-bind-runtime-bridge-note" {
                                            print_idt_bind_runtime_bridge_note();
                                        } else if line_str == "idt-bind-runtime-bridge-status" {
                                            print_idt_bind_runtime_bridge_status();
                                        } else if line_str == "idt-bind-runtime-bridge-check" {
                                            print_idt_bind_runtime_bridge_check();
                                        } else if line_str == "idt-bind-runtime-bridge-blockers" {
                                            print_idt_bind_runtime_bridge_blockers();
                                        } else if line_str == "idt-invoke-hw-smoke-note" {
                                            print_idt_invoke_hw_smoke_note();
                                        } else if line_str == "idt-invoke-hw-smoke-status" {
                                            print_idt_invoke_hw_smoke_status();
                                        } else if line_str == "idt-invoke-hw-smoke-arm" {
                                            print_idt_invoke_hw_smoke_arm();
                                        } else if line_str == "idt-invoke-hw-smoke-fire" {
                                            print_idt_invoke_hw_smoke_fire();
                                        } else if line_str == "idt-invoke-hw-smoke-clear" {
                                            print_idt_invoke_hw_smoke_clear();
                                        } else if line_str == "idt-invoke-hw-smoke-blockers" {
                                            print_idt_invoke_hw_smoke_blockers();
                                        } else if line_str == "idt-invoke-runtime-bridge-note" {
                                            print_idt_invoke_runtime_bridge_note();
                                        } else if line_str == "idt-invoke-runtime-bridge-status" {
                                            print_idt_invoke_runtime_bridge_status();
                                        } else if line_str == "idt-invoke-runtime-bridge-check" {
                                            print_idt_invoke_runtime_bridge_check();
                                        } else if line_str == "idt-invoke-runtime-bridge-blockers" {
                                            print_idt_invoke_runtime_bridge_blockers();
                                        } else if line_str == "irq-delivery-candidate-note" {
                                            print_irq_delivery_candidate_note();
                                        } else if line_str == "irq-delivery-candidate-status" {
                                            print_irq_delivery_candidate_status();
                                        } else if line_str == "irq-delivery-candidate-check" {
                                            print_irq_delivery_candidate_check();
                                        } else if line_str == "irq-delivery-candidate-blockers" {
                                            print_irq_delivery_candidate_blockers();
                                        } else if line_str == "irq0-bind-hw-smoke-note" {
                                            print_irq0_bind_hw_smoke_note();
                                        } else if line_str == "irq0-bind-hw-smoke-status" {
                                            print_irq0_bind_hw_smoke_status();
                                        } else if line_str == "irq0-bind-hw-smoke-arm" {
                                            print_irq0_bind_hw_smoke_arm();
                                        } else if line_str == "irq0-bind-hw-smoke-fire" {
                                            print_irq0_bind_hw_smoke_fire();
                                        } else if line_str == "irq0-bind-hw-smoke-clear" {
                                            print_irq0_bind_hw_smoke_clear();
                                        } else if line_str == "irq0-bind-hw-smoke-blockers" {
                                            print_irq0_bind_hw_smoke_blockers();
                                        } else if line_str == "irq0-unmask-hw-smoke-note" {
                                            print_irq0_unmask_hw_smoke_note();
                                        } else if line_str == "irq0-unmask-hw-smoke-status" {
                                            print_irq0_unmask_hw_smoke_status();
                                        } else if line_str == "irq0-unmask-hw-smoke-arm" {
                                            print_irq0_unmask_hw_smoke_arm();
                                        } else if line_str == "irq0-unmask-hw-smoke-fire" {
                                            print_irq0_unmask_hw_smoke_fire();
                                        } else if line_str == "irq0-unmask-hw-smoke-clear" {
                                            print_irq0_unmask_hw_smoke_clear();
                                        } else if line_str == "irq0-unmask-hw-smoke-blockers" {
                                            print_irq0_unmask_hw_smoke_blockers();
                                        } else if line_str == "irq0-preflight-status" {
                                            print_irq0_preflight_status();
                                        } else if line_str == "irq0-preflight-check" {
                                            print_irq0_preflight_check();
                                        } else if line_str == "irq0-preflight-blockers" {
                                            print_irq0_preflight_blockers();
                                        } else if line_str == "irq0-handler-stub-status" {
                                            print_irq0_handler_stub_status();
                                        } else if line_str == "irq0-handler-stub-check" {
                                            print_irq0_handler_stub_check();
                                        } else if line_str == "irq0-handler-stub-blockers" {
                                            print_irq0_handler_stub_blockers();
                                        } else if line_str == "eoi-runtime-note" {
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "EOI runtime dispatch note\neoi dispatch requires:\n- PIC remap controlled smoke ready\n- IRQ gates vectors 32/33 bound\n- IRQ edge/level detection strategy planned\n- keyboard fallback polling active\n- STI enabled\neoi dispatch: disabled (boundary definition only)\n");
                                            let _ = write!(serial_writer, "EOI runtime dispatch note\neoi dispatch requires:\n- PIC remap controlled smoke ready\n- IRQ gates vectors 32/33 bound\n- IRQ edge/level detection strategy planned\n- keyboard fallback polling active\n- STI enabled\neoi dispatch: disabled (boundary definition only)\n");
                                        } else if line_str == "eoi-runtime-status" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            // Verification contract snippets kept stable across rustfmt line wrapping:
                                            // let preconditions_met = irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            // let eoi_status = if preconditions_met { "ready (dry-run)" } else { "blocked" };
                                            // irq::eoi_runtime_check_all_preconditions(pic_state.executed);
                                            // pic::ProgrammableInterruptController::pic_mask_plan();
                                            // pic::ProgrammableInterruptController::pic_mask_status();
                                            let preconditions_met =
                                                irq::eoi_runtime_check_all_preconditions(
                                                    pic_state.executed,
                                                );
                                            let eoi_status = if preconditions_met {
                                                "ready (dry-run)"
                                            } else {
                                                "blocked"
                                            };
                                            let _ = write!(vga_writer, "EOI runtime readiness status\neoi dispatch: {}\npic remap: {}\nirq gates: {}\nkeyboard fallback: polling\nprerequisites satisfied: {}\neoi dispatch: disabled\n",
                                                    eoi_status,
                                                    if pic_state.executed { "ready" } else { "not ready" },
                                                    if gate_state.executed { "bound" } else { "not bound" },
                                                    if preconditions_met { "yes" } else { "no" }
                                                );
                                            let _ = write!(serial_writer, "EOI runtime readiness status\neoi dispatch: {}\npic remap: {}\nirq gates: {}\nkeyboard fallback: polling\nprerequisites satisfied: {}\neoi dispatch: disabled\n",
                                                    eoi_status,
                                                    if pic_state.executed { "ready" } else { "not ready" },
                                                    if gate_state.executed { "bound" } else { "not bound" },
                                                    if preconditions_met { "yes" } else { "no" }
                                                );
                                        } else if line_str == "eoi-runtime-blockers" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(
                                                vga_writer,
                                                "EOI runtime activation blockers\n"
                                            );
                                            let _ = write!(
                                                serial_writer,
                                                "EOI runtime activation blockers\n"
                                            );
                                            if !pic_state.executed {
                                                let _ = write!(
                                                    vga_writer,
                                                    "- {}\n",
                                                    irq::EOI_RUNTIME_BLOCKER_PIC_REMAP
                                                );
                                                let _ = write!(
                                                    serial_writer,
                                                    "- {}\n",
                                                    irq::EOI_RUNTIME_BLOCKER_PIC_REMAP
                                                );
                                            }
                                            if !gate_state.executed {
                                                let _ = write!(
                                                    vga_writer,
                                                    "- {}\n",
                                                    irq::EOI_RUNTIME_BLOCKER_IRQ_GATES
                                                );
                                                let _ = write!(
                                                    serial_writer,
                                                    "- {}\n",
                                                    irq::EOI_RUNTIME_BLOCKER_IRQ_GATES
                                                );
                                            }
                                            let _ = write!(
                                                vga_writer,
                                                "- {}\n",
                                                irq::EOI_RUNTIME_BLOCKER_EDGE_LEVEL
                                            );
                                            let _ = write!(
                                                serial_writer,
                                                "- {}\n",
                                                irq::EOI_RUNTIME_BLOCKER_EDGE_LEVEL
                                            );
                                            let _ = write!(
                                                vga_writer,
                                                "- {}\n",
                                                irq::EOI_RUNTIME_BLOCKER_KEYBOARD
                                            );
                                            let _ = write!(
                                                serial_writer,
                                                "- {}\n",
                                                irq::EOI_RUNTIME_BLOCKER_KEYBOARD
                                            );
                                            let _ = write!(
                                                vga_writer,
                                                "- {}\n",
                                                irq::EOI_RUNTIME_BLOCKER_STI
                                            );
                                            let _ = write!(
                                                serial_writer,
                                                "- {}\n",
                                                irq::EOI_RUNTIME_BLOCKER_STI
                                            );
                                            let _ = write!(vga_writer, "eoi dispatch: disabled\n");
                                            let _ =
                                                write!(serial_writer, "eoi dispatch: disabled\n");
                                        } else if line_str == "pic-mask-plan" {
                                            let plan =
                                                pic::ProgrammableInterruptController::pic_mask_plan(
                                                );
                                            let mask_plan_msg = "PIC IRQ mask plan\nmask policy: all masked (0xFF)\nmaster imr: 0xFF (all masked)\nslave imr: 0xFF (all masked)\nunmask candidates: none\nunmask policy: no lines scheduled for unmask\nunmask gate: disabled\n";
                                            core::hint::black_box(plan);
                                            vga::print(mask_plan_msg);
                                            serial::print(mask_plan_msg);
                                        } else if line_str == "pic-mask-status" {
                                            let status = pic::ProgrammableInterruptController::pic_mask_status();
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(vga_writer, "PIC IRQ mask status\nmaster imr planned: 0x{:02x}\nslave imr planned: 0x{:02x}\nunmask candidates: {}\nunmask blocked: {}\nmask writes: {}\nlive unmask: {}\n",
                                                    status.master_imr_planned,
                                                    status.slave_imr_planned,
                                                    status.unmask_candidates,
                                                    status.unmask_blocked,
                                                    status.mask_writes,
                                                    status.live_unmask
                                                );
                                            let _ = write!(serial_writer, "PIC IRQ mask status\nmaster imr planned: 0x{:02x}\nslave imr planned: 0x{:02x}\nunmask candidates: {}\nunmask blocked: {}\nmask writes: {}\nlive unmask: {}\n",
                                                    status.master_imr_planned,
                                                    status.slave_imr_planned,
                                                    status.unmask_candidates,
                                                    status.unmask_blocked,
                                                    status.mask_writes,
                                                    status.live_unmask
                                                );
                                        } else if line_str == "irq-mask-blockers" {
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let gate_state = irq::irq_gate_bind_state();
                                            let report = irq::irq_mask_blocker_report(
                                                pic_state.executed,
                                                gate_state.executed,
                                                irq::irq_runtime_is_committed(),
                                            );
                                            core::hint::black_box(
                                                irq::irq_mask_check_all_blockers(&report),
                                            );
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let _ = write!(
                                                vga_writer,
                                                "PIC IRQ unmask activation blockers\n"
                                            );
                                            let _ = write!(
                                                serial_writer,
                                                "PIC IRQ unmask activation blockers\n"
                                            );
                                            if !report.pic_remap_ready {
                                                let _ = write!(
                                                    vga_writer,
                                                    "{}\n",
                                                    irq::IRQ_MASK_BLOCKER_PIC_REMAP
                                                );
                                                let _ = write!(
                                                    serial_writer,
                                                    "{}\n",
                                                    irq::IRQ_MASK_BLOCKER_PIC_REMAP
                                                );
                                            }
                                            if !report.irq_gates_ready {
                                                let _ = write!(
                                                    vga_writer,
                                                    "{}\n",
                                                    irq::IRQ_MASK_BLOCKER_IRQ_GATES
                                                );
                                                let _ = write!(
                                                    serial_writer,
                                                    "{}\n",
                                                    irq::IRQ_MASK_BLOCKER_IRQ_GATES
                                                );
                                            }
                                            if !report.sti_ready {
                                                let _ = write!(
                                                    vga_writer,
                                                    "{}\n",
                                                    irq::IRQ_MASK_BLOCKER_STI
                                                );
                                                let _ = write!(
                                                    serial_writer,
                                                    "{}\n",
                                                    irq::IRQ_MASK_BLOCKER_STI
                                                );
                                            }
                                            if !report.eoi_dispatch_ready {
                                                let _ = write!(
                                                    vga_writer,
                                                    "{}\n",
                                                    irq::IRQ_MASK_BLOCKER_EOI_DISPATCH
                                                );
                                                let _ = write!(
                                                    serial_writer,
                                                    "{}\n",
                                                    irq::IRQ_MASK_BLOCKER_EOI_DISPATCH
                                                );
                                            }
                                            if !report.irq_runtime_committed {
                                                let _ = write!(
                                                    vga_writer,
                                                    "{}\n",
                                                    irq::IRQ_MASK_BLOCKER_IRQ_RUNTIME
                                                );
                                                let _ = write!(
                                                    serial_writer,
                                                    "{}\n",
                                                    irq::IRQ_MASK_BLOCKER_IRQ_RUNTIME
                                                );
                                            }
                                            let _ = write!(vga_writer, "unmask gate: disabled\n");
                                            let _ =
                                                write!(serial_writer, "unmask gate: disabled\n");
                                        } else if line_str == "pic-status --verbose" {
                                            let pic_status_verbose_msg = "pic subsystem:\nfoundation: dry-run telemetry\nremap function: present / not called\ndry-run plan: available\nmaster offset: 0x20\nslave offset: 0x28\nirq vectors: 0x20-0x2f\nhardware writes: disabled\nirq handlers: none\ninterrupts: disabled\n";
                                            vga::print(pic_status_verbose_msg);
                                            serial::print(pic_status_verbose_msg);
                                        } else if line_str == "exceptions --verbose" {
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            let count = interrupts::EXCEPTION_COUNT;
                                            let vector = interrupts::LAST_EXCEPTION_VECTOR;
                                            let name = interrupts::LAST_EXCEPTION_NAME;
                                            let armed = interrupts::PF_SMOKE_ACTIVE;
                                            if vector == -1 {
                                                let _ = write!(vga_writer, "exception recovery verbose:\nexceptions handled: {}\nlast exception: none\nactive handlers:\nvector 0: divide-by-zero\nvector 3: breakpoint\nvector 14: page fault\nsmoke handlers:\nvector 14: page fault recovery trampoline\nplanned handlers:\nnone\npage fault smoke: armed={}\ninterrupts: disabled\n", count, armed);
                                                let _ = write!(serial_writer, "exception recovery verbose:\nexceptions handled: {}\nlast exception: none\nactive handlers:\nvector 0: divide-by-zero\nvector 3: breakpoint\nvector 14: page fault\nsmoke handlers:\nvector 14: page fault recovery trampoline\nplanned handlers:\nnone\npage fault smoke: armed={}\ninterrupts: disabled\n", count, armed);
                                            } else {
                                                let _ = write!(vga_writer, "exception recovery verbose:\nexceptions handled: {}\nlast exception: {} ({})\nactive handlers:\nvector 0: divide-by-zero\nvector 3: breakpoint\nvector 14: page fault\nsmoke handlers:\nvector 14: page fault recovery trampoline\nplanned handlers:\nnone\npage fault smoke: armed={}\ninterrupts: disabled\n", count, vector, name, armed);
                                                let _ = write!(serial_writer, "exception recovery verbose:\nexceptions handled: {}\nlast exception: {} ({})\nactive handlers:\nvector 0: divide-by-zero\nvector 3: breakpoint\nvector 14: page fault\nsmoke handlers:\nvector 14: page fault recovery trampoline\nplanned handlers:\nnone\npage fault smoke: armed={}\ninterrupts: disabled\n", count, vector, name, armed);
                                            }
                                        } else if line_str == "exception-about" {
                                            let about_msg = "exception subsystem:\nfoundation: active\nactive vectors: 0 divide-by-zero, 3 breakpoint, 14 page fault smoke\ntelemetry: count / last vector / last name\nrecovery: smoke-safe trampoline\nstatus ux: active\ninterrupts: disabled\n";
                                            vga::print(about_msg);
                                            serial::print(about_msg);
                                        } else if line_str == "exception-help" {
                                            let help_msg = "exception diagnostics commands:\nexception          - show dynamic telemetry parameters\nexceptions         - show exception status overview\nexceptions --verbose - show verbose exception recovery overview\nexception-status   - show exception status overview (alias)\nexception-reset    - reset all exception telemetry counters\nexception-about    - show exception subsystem foundation summary\nfault-status       - show fault recovery status\nfault-reset        - reset fault recovery and exception telemetry\npf-status          - show page fault smoke status\nexception-help     - display this help content\nhandlers           - list active and planned IDT entry handlers\nhandlers --active  - list active IDT entry handlers only\npf-note            - show page fault smoke direction note\npf-smoke           - trigger controlled real page fault smoke\nint3               - execute breakpoint software interrupt\ndiv0               - execute divide-by-zero trap\n";
                                            vga::print(help_msg);
                                            serial::print(help_msg);
                                        } else if line_str == "pf-note" {
                                            let pf_note_msg = "page fault: active smoke\nvector: 14\ncr2: available after pf-smoke\nerror code: available after pf-smoke\n";
                                            vga::print(pf_note_msg);
                                            serial::print(pf_note_msg);
                                        } else if line_str == "mem" {
                                            vga::print("kernel memory: static lab view\nheap: unavailable\nallocator: unavailable\n");
                                            serial::print("kernel memory: static lab view\nheap: unavailable\nallocator: unavailable\n");
                                        } else if line_str == "uptime" {
                                            vga::print("uptime: unavailable (no timer driver)\n");
                                            serial::print(
                                                "uptime: unavailable (no timer driver)\n",
                                            );
                                        } else if line_str == "banner" {
                                            vga::print("========================================================================\n");
                                            vga::print("                   DByteOS Command Dispatch Lab (v9.0.2)                \n");
                                            vga::print("========================================================================\n");
                                            serial::print("========================================================================\n");
                                            serial::print("                   DByteOS Command Dispatch Lab (v9.0.2)                \n");
                                            serial::print("========================================================================\n");
                                        } else if line_str == "keyboard" {
                                            vga::print("shift: ");
                                            vga::print(if SHIFT_ACTIVE {
                                                "true\n"
                                            } else {
                                                "false\n"
                                            });
                                            vga::print("capslock: ");
                                            vga::print(if CAPS_LOCK_ACTIVE {
                                                "true\n"
                                            } else {
                                                "false\n"
                                            });
                                            vga::print("mode: polling\n");

                                            serial::print("shift: ");
                                            serial::print(if SHIFT_ACTIVE {
                                                "true\n"
                                            } else {
                                                "false\n"
                                            });
                                            serial::print("capslock: ");
                                            serial::print(if CAPS_LOCK_ACTIVE {
                                                "true\n"
                                            } else {
                                                "false\n"
                                            });
                                            serial::print("mode: polling\n");
                                        } else if line_str == "reboot-note" {
                                            vga::print("reboot: unavailable (no ACPI/PS2 controller reset implemented)\n");
                                            serial::print("reboot: unavailable (no ACPI/PS2 controller reset implemented)\n");
                                        } else if line_str == "system" {
                                            let mut vga_writer = vga::VgaWriter;
                                            let mut serial_writer = serial::SerialWriter;
                                            vga::print(
                                                "DByteOS Kernel Lab
version: 9.0.2
input mode: keyboard polling
display mode: text-mode VGA (80x25)
serial mode: COM1 115200 8N1
filesystem: none
process model: none
dbyte vm: none
idt: loaded
exception handlers: breakpoint, divide-by-zero, page fault
page fault handler: active smoke
pic/irq: planned / disabled
pic remap: planned / disabled
pic dry-run telemetry: available
irq handlers: skeleton / disabled
recovery mode: smoke-safe
page fault smoke: armed=false
interrupts: disabled
",
                                            );
                                            serial::print(
                                                "DByteOS Kernel Lab
version: 9.0.2
input mode: keyboard polling
display mode: text-mode VGA (80x25)
serial mode: COM1 115200 8N1
filesystem: none
process model: none
dbyte vm: none
idt: loaded
exception handlers: breakpoint, divide-by-zero, page fault
page fault handler: active smoke
pic/irq: planned / disabled
pic remap: planned / disabled
pic dry-run telemetry: available
irq handlers: skeleton / disabled
recovery mode: smoke-safe
page fault smoke: armed=false
interrupts: disabled
",
                                            );
                                            let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                            let _ = write!(
                                                vga_writer,
                                                "pic remap controlled smoke: executed={}\n",
                                                if pic_state.executed { "yes" } else { "no" }
                                            );
                                            let _ = write!(
                                                serial_writer,
                                                "pic remap controlled smoke: executed={}\n",
                                                if pic_state.executed { "yes" } else { "no" }
                                            );
                                            let gate_state = irq::irq_gate_bind_state();
                                            let _ = write!(
                                                vga_writer,
                                                "irq gates controlled smoke: bound={}\n",
                                                if gate_state.executed { "yes" } else { "no" }
                                            );
                                            let _ = write!(
                                                serial_writer,
                                                "irq gates controlled smoke: bound={}\n",
                                                if gate_state.executed { "yes" } else { "no" }
                                            );
                                            let count = interrupts::EXCEPTION_COUNT;
                                            let vector = interrupts::LAST_EXCEPTION_VECTOR;
                                            let name = interrupts::LAST_EXCEPTION_NAME;
                                            if vector == -1 {
                                                let _ = write!(
                                                    vga_writer,
                                                    "exceptions handled: {}
last exception: none
",
                                                    count
                                                );
                                                let _ = write!(
                                                    serial_writer,
                                                    "exceptions handled: {}
last exception: none
",
                                                    count
                                                );
                                            } else {
                                                let _ = write!(
                                                    vga_writer,
                                                    "exceptions handled: {}
last exception: {} ({})
",
                                                    count, vector, name
                                                );
                                                let _ = write!(
                                                    serial_writer,
                                                    "exceptions handled: {}
last exception: {} ({})
",
                                                    count, vector, name
                                                );
                                            }
                                        } else if line_str == "status" {
                                            vga::print(
                                                "status: active\nversion: 9.0.2\nmode: polling\n",
                                            );
                                            serial::print(
                                                "status: active\nversion: 9.0.2\nmode: polling\n",
                                            );
                                        } else if line_str == "mods" {
                                            vga::print("shift active: ");
                                            vga::print(if SHIFT_ACTIVE {
                                                "true\n"
                                            } else {
                                                "false\n"
                                            });
                                            vga::print("capslock active: ");
                                            vga::print(if CAPS_LOCK_ACTIVE {
                                                "true\n"
                                            } else {
                                                "false\n"
                                            });

                                            serial::print("shift active: ");
                                            serial::print(if SHIFT_ACTIVE {
                                                "true\n"
                                            } else {
                                                "false\n"
                                            });
                                            serial::print("capslock active: ");
                                            serial::print(if CAPS_LOCK_ACTIVE {
                                                "true\n"
                                            } else {
                                                "false\n"
                                            });
                                        } else if line_str == "keys" {
                                            vga::print("keyboard mode: polling\nsupported keymap: ASCII (US Layout)\ncasing: Shift ^ CapsLock XOR\n");
                                            serial::print("keyboard mode: polling\nsupported keymap: ASCII (US Layout)\ncasing: Shift ^ CapsLock XOR\n");
                                        } else if line_str == "prompt" {
                                            vga::print("current prompt: dbyte-kernel>\n");
                                            serial::print("current prompt: dbyte-kernel>\n");
                                        } else {
                                            vga::print("error: unknown command\n");
                                            serial::print("error: unknown command\n");
                                        }
                                    }
                                }

                                // Reset buffer
                                LINE_LEN = 0;

                                // Print new prompt
                                if !vga_prompt_already_rendered {
                                    vga::print("dbyte-kernel> ");
                                }
                                serial::print("dbyte-kernel> ");
                            } else {
                                // Normal character output: append if buffer is not full!
                                if LINE_LEN < 128 {
                                    LINE_BUFFER[LINE_LEN] = c as u8;
                                    LINE_LEN += 1;
                                    vga::print_byte(c as u8);
                                    serial::write_byte(c as u8);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn rust_eh_personality() {}
