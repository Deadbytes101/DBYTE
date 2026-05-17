#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;

mod vga;
mod serial;

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

#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    vga::clear_screen();
    vga::print("========================================================================\n");
    vga::print("                      DByteOS Kernel Lab (v6.2.1)                       \n");
    vga::print("========================================================================\n\n");
    vga::print("[OK] Bootstrap entry point successfully resolved.\n");
    vga::print("[OK] Stack pointer esp initialized to 16 KiB.\n");
    vga::print("[OK] Text-mode VGA framebuffer driver loaded.\n");

    unsafe {
        serial::init();
    }
    vga::print("[OK] Freestanding COM1 serial port driver loaded.\n\n");

    vga::print("Status: Boot Successful (beta-userland simulation sandbox)\n\n");
    vga::print("dbyte-kernel-lab> _");

    // Print to serial console for QEMU Boot Smoke automated detection
    serial::print("DByteOS Kernel Lab\n");
    serial::print("version: 6.2.1\n");
    serial::print("status: booted\n");
    serial::print("target: i686 multiboot\n");

    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

