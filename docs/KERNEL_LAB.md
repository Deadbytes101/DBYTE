# DByteOS Kernel Lab Guide (v8.13.1)

> [!WARNING]
> **DByteOS Kernel Lab is a Bare-Metal Experiment.**
> It is not a bootable full OS nor a real production kernel. It is a freestanding sandbox prototype containing no memory allocator, process scheduler, interrupt controllers, or standard driver sets.

Welcome to the **DByteOS Kernel Lab**! This laboratory allows you to compile and run a minimal freestanding x86 kernel from a clean Windows development host with zero external assembler or compiler toolchain dependencies.

## Project Structure
The laboratory is completely isolated inside the `kernel-lab/` directory:
- `kernel-lab/.cargo/config.toml`: Configures the standard `i686-unknown-linux-gnu` target.
- `kernel-lab/boot/linker.ld`: Linker script locating the Multiboot header at `1MB`.
- `kernel-lab/src/main.rs`: Kernel entry point using Rust `global_asm!`.
- `kernel-lab/src/vga.rs`: Simple frame buffer output driver mapped to `0xB8000`.
- `kernel-lab/scripts/`: PowerShell runners for compiling and launching under QEMU.
- `docs/KERNEL_EXCEPTIONS.md`: Kernel Exception Subsystem Foundation overview for active vectors `0 / 3 / 14`, telemetry, recovery UX, and status UX.
- `docs/KERNEL_IRQ.md`: IRQ Handler Skeleton Foundation overview for planned remap offsets, disabled remap function, IRQ glossary, dry-run IRQ map, IRQ0/IRQ1 skeleton status, disabled IRQ status, and polling-only keyboard boundaries.

## Exception Subsystem Foundation

Version `8.13.1` preserves the Exception Subsystem Foundation. The active exception surface is vector `0` divide-by-zero, vector `3` breakpoint, and vector `14` page fault smoke. Status and recovery are exposed through `exception-status`, `exceptions --verbose`, `fault-status`, `pf-status`, `handlers --active`, and `exception-about`.

Version `8.13.1` adds IRQ gate bind state telemetry (`irq-gate-state`, `irq-gate-history`, `irq-gate-preflight`) on top of the `v8.12.1` controlled bind smoke line. The `system` command reports `irq gates controlled smoke: bound=yes|no`, and `handlers` reflects smoke-bound vectors `32/33` after bind smoke. EOI target paths remain compiled but not dispatched; dry-run commands expose status only; keyboard input stays polling-only through PS/2 ports `0x64` / `0x60`. PIC hardware writes remain limited to `pic-remap-arm` / `pic-remap-smoke`. IDT vectors `32` and `33` bind only inside `irq-gate-arm` / `irq-gate-bind-smoke`.

This milestone does not add a new exception vector, does not change `pf-smoke`, does not enable STI, does not remap the PIC or bind IRQ gates at boot, does not unmask PIC IRQ lines, does not dispatch EOI, and keeps keyboard input polling-based.

## Prerequisites
To boot the prototype, you need:
1. **Rustup**: The standard Rust toolchain manager.
2. **QEMU (optional)**: For local bare-metal virtualization.

## Compilation & Run Pipeline

### 1. Bootstrap Target
Run the bootstrap script inside the `kernel-lab` directory to install the `rust-src` component needed for compiling freestanding core crates:
```powershell
cd kernel-lab
powershell .\scripts\bootstrap.ps1
```

### 2. Build the Kernel
Compile the freestanding Multiboot ELF binary:
```powershell
powershell .\scripts\build.ps1
```
The output ELF binary is generated at:
`kernel-lab/target/i686-unknown-linux-gnu/debug/dbyte_kernel`

### 3. Run in QEMU
Launch the built kernel inside the QEMU emulator:
```powershell
powershell .\scripts\run.ps1
```
This executes `qemu-system-i386` with direct kernel loading (`-kernel`), which boots the freestanding ELF file instantly without an external ISO builder!

