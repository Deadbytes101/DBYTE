# DByteOS Kernel Lab Guide (v7.0.1)

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

