#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;

mod vga;
mod serial;
mod mem;
mod idt;
mod pic;
mod interrupts;

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
    vga::print("                   DByteOS Command Dispatch Lab (v7.0.0)                \n");
    vga::print("========================================================================\n\n");
    vga::print("[OK] Bootstrap entry point successfully resolved.\n");
    vga::print("[OK] Text-mode VGA framebuffer driver loaded.\n");

    unsafe {
        serial::init();
    }
    vga::print("[OK] Freestanding COM1 serial port driver loaded.\n\n");

    vga::print("Status: Keyboard Listener Active (polling mode)\n");
    vga::print("Press keys inside the QEMU graphical display window.\n\n");

    // Print to serial console for QEMU Boot Smoke automated detection
    serial::print("DByteOS Kernel Lab\n");
    serial::print("version: 7.0.0\n");
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
                                            vga::print("commands: help about version clear echo mem uptime banner keyboard reboot-note system cls status mods keys prompt\n");
                                            serial::print("commands: help about version clear echo mem uptime banner keyboard reboot-note system cls status mods keys prompt\n");
                                        } else if line_str == "about" {
                                            vga::print("DByteOS Kernel Lab\n");
                                            serial::print("DByteOS Kernel Lab\n");
                                        } else if line_str == "version" {
                                            vga::print("DByteOS Kernel Lab 7.0.0\n");
                                            serial::print("DByteOS Kernel Lab 7.0.0\n");
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
                                        } else if line_str == "mem" {
                                            vga::print("kernel memory: static lab view\nheap: unavailable\nallocator: unavailable\n");
                                            serial::print("kernel memory: static lab view\nheap: unavailable\nallocator: unavailable\n");
                                        } else if line_str == "uptime" {
                                            vga::print("uptime: unavailable (no timer driver)\n");
                                            serial::print("uptime: unavailable (no timer driver)\n");
                                        } else if line_str == "banner" {
                                            vga::print("========================================================================\n");
                                            vga::print("                   DByteOS Command Dispatch Lab (v7.0.0)                \n");
                                            vga::print("========================================================================\n");
                                            serial::print("========================================================================\n");
                                            serial::print("                   DByteOS Command Dispatch Lab (v7.0.0)                \n");
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
                                            vga::print("DByteOS Kernel Lab\nversion: 7.0.0\ninput mode: keyboard polling\ndisplay mode: text-mode VGA (80x25)\nserial mode: COM1 115200 8N1\nfilesystem: none\nprocess model: none\ndbyte vm: none\ninterrupts: planned / disabled\n");
                                            serial::print("DByteOS Kernel Lab\nversion: 7.0.0\ninput mode: keyboard polling\ndisplay mode: text-mode VGA (80x25)\nserial mode: COM1 115200 8N1\nfilesystem: none\nprocess model: none\ndbyte vm: none\ninterrupts: planned / disabled\n");
                                        } else if line_str == "status" {
                                            vga::print("status: active\nversion: 7.0.0\nmode: polling\n");
                                            serial::print("status: active\nversion: 7.0.0\nmode: polling\n");
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
