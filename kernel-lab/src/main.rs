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
    vga::print("                   DByteOS Command Dispatch Lab (v8.0.0)                \n");
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
    serial::print("version: 8.0.0\n");
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
                                            vga::print("commands: help about version clear echo mem uptime banner keyboard reboot-note system cls status mods keys prompt int3 div0 exception exception-reset handlers handlers --active exception-status exceptions exceptions --verbose exception-help exception-about fault-status fault-reset pf-note pf-status pf-smoke\n");
                                            serial::print("commands: help about version clear echo mem uptime banner keyboard reboot-note system cls status mods keys prompt int3 div0 exception exception-reset handlers handlers --active exception-status exceptions exceptions --verbose exception-help exception-about fault-status fault-reset pf-note pf-status pf-smoke\n");
                                        } else if line_str == "about" {
                                            vga::print("DByteOS Kernel Lab\n");
                                            serial::print("DByteOS Kernel Lab\n");
                                        } else if line_str == "version" {
                                            vga::print("DByteOS Kernel Lab 8.0.0\n");
                                            serial::print("DByteOS Kernel Lab 8.0.0\n");
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
                                              let handlers_msg = "active handlers:\nvector 0: divide-by-zero\nvector 3: breakpoint\nvector 14: page fault\nplanned handlers:\nnone\n";
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
                                            vga::print("                   DByteOS Command Dispatch Lab (v8.0.0)                \n");
                                            vga::print("========================================================================\n");
                                            serial::print("========================================================================\n");
                                            serial::print("                   DByteOS Command Dispatch Lab (v8.0.0)                \n");
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
version: 8.0.0
input mode: keyboard polling
display mode: text-mode VGA (80x25)
serial mode: COM1 115200 8N1
filesystem: none
process model: none
dbyte vm: none
idt: loaded
exception handlers: breakpoint, divide-by-zero, page fault
page fault handler: active smoke
recovery mode: smoke-safe
page fault smoke: armed=false
interrupts: disabled
");
                                             serial::print("DByteOS Kernel Lab
version: 8.0.0
input mode: keyboard polling
display mode: text-mode VGA (80x25)
serial mode: COM1 115200 8N1
filesystem: none
process model: none
dbyte vm: none
idt: loaded
exception handlers: breakpoint, divide-by-zero, page fault
page fault handler: active smoke
recovery mode: smoke-safe
page fault smoke: armed=false
interrupts: disabled
");
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
                                            vga::print("status: active\nversion: 8.0.0\nmode: polling\n");
                                            serial::print("status: active\nversion: 8.0.0\nmode: polling\n");
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
