#![no_std]
#![no_main]
#![allow(static_mut_refs)]

use core::arch::global_asm;
use core::panic::PanicInfo;

mod vga;
mod serial;
mod mem;
mod idt;
mod pic;
mod irq;
mod interrupts;
mod page_fault;

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
        .skip 16384            /* 16 KiB stack */
    stack_top:
    "#
);

static mut SHIFT_ACTIVE: bool = false;
static mut CAPS_LOCK_ACTIVE: bool = false;

static mut LINE_BUFFER: [u8; 128] = [0; 128];
static mut LINE_LEN: usize = 0;

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

#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    vga::clear_screen();
    vga::print("========================================================================\n");
    vga::print("                   DByteOS Command Dispatch Lab (v8.12.0)                \n");
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
    serial::print("version: 8.12.0\n");
    serial::print("status: booted\n");
    serial::print("target: i686 multiboot\n\n");

    serial::print("DByteOS Keyboard Lab\n");
    serial::print("status: listening\n");

    // Print initial prompt
    vga::print("dbyte-kernel> ");
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
                    let _ = write!(writer, "[MODIFIER] Shift: {}, CapsLock: {}\n", shift_val, caps_val);
                }

                // Ignore break codes for standard typing (scancode >= 0x80)
                if scancode < 0x80 {
                    // Exclude modifier keys from printing directly as printable key characters
                    if scancode != 0x2A && scancode != 0x36 && scancode != 0x3A {
                        if let Some(c) = scancode_to_ascii(scancode, SHIFT_ACTIVE, CAPS_LOCK_ACTIVE) {
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

                                if LINE_LEN > 0 {
                                    // Convert and process submitted line
                                    if let Ok(line_str) = core::str::from_utf8(&LINE_BUFFER[..LINE_LEN]) {
                                        if line_str == "help" {
                                            vga::print("commands: help about version clear echo mem uptime banner keyboard reboot-note system cls status mods keys prompt int3 div0 exception exception-reset handlers handlers --active exception-status exceptions exceptions --verbose exception-help exception-about fault-status fault-reset pf-note pf-status pf-smoke irq-note irq-status irq-handlers eoi-note eoi-status irq-gates irq-gate-status irq-gate-plan irq-gate-arm irq-gate-bind-smoke irq-gate-bind-status irq-bind-note irq-bind-status irq-readiness irq-risk irq-preflight pic-note pic-status pic-plan pic-remap-arm pic-remap-smoke pic-remap-status pic-remap-state pic-remap-history pic-remap-preflight irq-map pic-status --verbose\n");
                                            serial::print("commands: help about version clear echo mem uptime banner keyboard reboot-note system cls status mods keys prompt int3 div0 exception exception-reset handlers handlers --active exception-status exceptions exceptions --verbose exception-help exception-about fault-status fault-reset pf-note pf-status pf-smoke irq-note irq-status irq-handlers eoi-note eoi-status irq-gates irq-gate-status irq-gate-plan irq-gate-arm irq-gate-bind-smoke irq-gate-bind-status irq-bind-note irq-bind-status irq-readiness irq-risk irq-preflight pic-note pic-status pic-plan pic-remap-arm pic-remap-smoke pic-remap-status pic-remap-state pic-remap-history pic-remap-preflight irq-map pic-status --verbose\n");
                                        } else if line_str == "about" {
                                            vga::print("DByteOS Kernel Lab\n");
                                            serial::print("DByteOS Kernel Lab\n");
                                        } else if line_str == "version" {
                                            vga::print("DByteOS Kernel Lab 8.12.0\n");
                                            serial::print("DByteOS Kernel Lab 8.12.0\n");
                                        } else if line_str == "clear" || line_str == "cls" {
                                            vga::clear_screen();
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
                                              interrupts::PF_SMOKE_RECOVERY_EIP = interrupts::pf_smoke_recovery_asm as *const () as u32;
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
                                              serial::print("exception telemetry: reset successfully\n");
                                          } else if line_str == "fault-reset" {
                                              interrupts::EXCEPTION_COUNT = 0;
                                              interrupts::LAST_EXCEPTION_VECTOR = -1;
                                              interrupts::LAST_EXCEPTION_NAME = "none";
                                              interrupts::PF_SMOKE_ACTIVE = false;
                                              interrupts::PF_SMOKE_RECOVERY_EIP = 0;
                                              vga::print("fault recovery: reset successfully\n");
                                              serial::print("fault recovery: reset successfully\n");
                                          } else if line_str == "handlers" {
                                              let handlers_msg = "active handlers:\nvector 0: divide-by-zero\nvector 3: breakpoint\nvector 14: page fault\nplanned handlers:\nnone\nirq handlers:\nskeleton planned: irq0 timer, irq1 keyboard\nactive: none\n";
                                              vga::print(handlers_msg);
                                              serial::print(handlers_msg);
                                          } else if line_str == "handlers --active" {
                                              let active_handlers_msg = "active handlers:\nvector 0: divide-by-zero\nvector 3: breakpoint\nvector 14: page fault\n";
                                              vga::print(active_handlers_msg);
                                              serial::print(active_handlers_msg);
                                          } else if line_str == "exception-status" || line_str == "exceptions" {
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
                                                  idt::IDT.entries[32].set_handler(interrupts::irq0_timer_gate_smoke_asm as *const ());
                                                  idt::IDT.entries[33].set_handler(interrupts::irq1_keyboard_gate_smoke_asm as *const ());
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
                                            serial::print("uptime: unavailable (no timer driver)\n");
                                        } else if line_str == "banner" {
                                            vga::print("========================================================================\n");
                                            vga::print("                   DByteOS Command Dispatch Lab (v8.12.0)                \n");
                                            vga::print("========================================================================\n");
                                            serial::print("========================================================================\n");
                                            serial::print("                   DByteOS Command Dispatch Lab (v8.12.0)                \n");
                                            serial::print("========================================================================\n");
                                        } else if line_str == "keyboard" {
                                            vga::print("shift: ");
                                            vga::print(if SHIFT_ACTIVE { "true\n" } else { "false\n" });
                                            vga::print("capslock: ");
                                            vga::print(if CAPS_LOCK_ACTIVE { "true\n" } else { "false\n" });
                                            vga::print("mode: polling\n");

                                            serial::print("shift: ");
                                            serial::print(if SHIFT_ACTIVE { "true\n" } else { "false\n" });
                                            serial::print("capslock: ");
                                            serial::print(if CAPS_LOCK_ACTIVE { "true\n" } else { "false\n" });
                                            serial::print("mode: polling\n");
                                        } else if line_str == "reboot-note" {
                                            vga::print("reboot: unavailable (no ACPI/PS2 controller reset implemented)\n");
                                            serial::print("reboot: unavailable (no ACPI/PS2 controller reset implemented)\n");
                                        } else if line_str == "system" {
                                             let mut vga_writer = vga::VgaWriter;
                                             let mut serial_writer = serial::SerialWriter;
                                             vga::print("DByteOS Kernel Lab
version: 8.12.0
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
");
                                             serial::print("DByteOS Kernel Lab
version: 8.12.0
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
");
                                             let pic_state = pic::ProgrammableInterruptController::pic_remap_state();
                                             let _ = write!(vga_writer, "pic remap controlled smoke: executed={}\n",
                                                 if pic_state.executed { "yes" } else { "no" }
                                             );
                                             let _ = write!(serial_writer, "pic remap controlled smoke: executed={}\n",
                                                 if pic_state.executed { "yes" } else { "no" }
                                             );
                                             let count = interrupts::EXCEPTION_COUNT;
                                             let vector = interrupts::LAST_EXCEPTION_VECTOR;
                                             let name = interrupts::LAST_EXCEPTION_NAME;
                                             if vector == -1 {
                                                 let _ = write!(vga_writer, "exceptions handled: {}
last exception: none
", count);
                                                 let _ = write!(serial_writer, "exceptions handled: {}
last exception: none
", count);
                                             } else {
                                                 let _ = write!(vga_writer, "exceptions handled: {}
last exception: {} ({})
", count, vector, name);
                                                 let _ = write!(serial_writer, "exceptions handled: {}
last exception: {} ({})
", count, vector, name);
                                             }
                                         } else if line_str == "status" {
                                            vga::print("status: active\nversion: 8.12.0\nmode: polling\n");
                                            serial::print("status: active\nversion: 8.12.0\nmode: polling\n");
                                        } else if line_str == "mods" {
                                            vga::print("shift active: ");
                                            vga::print(if SHIFT_ACTIVE { "true\n" } else { "false\n" });
                                            vga::print("capslock active: ");
                                            vga::print(if CAPS_LOCK_ACTIVE { "true\n" } else { "false\n" });

                                            serial::print("shift active: ");
                                            serial::print(if SHIFT_ACTIVE { "true\n" } else { "false\n" });
                                            serial::print("capslock active: ");
                                            serial::print(if CAPS_LOCK_ACTIVE { "true\n" } else { "false\n" });
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
                                vga::print("dbyte-kernel> ");
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
