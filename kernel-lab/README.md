# DByteOS Kernel Lab (v6.3.1)

Welcome to the **DByteOS Kernel Lab**! This sandbox is a bare-metal experimental laboratory separated entirely from the stable `dbyteos` userland computing environment.

## Lab Directory Layout
- `i686-unknown-none.json`: Custom target definition for freestanding 32-bit x86.
- `boot/linker.ld`: Linker script locating the Multiboot section at `1MB` physical memory.
- `src/main.rs`: Freestanding bootloader entry point utilizing inline LLVM assembly.
- `src/vga.rs`: Framebuffer writing driver mapped directly to `0xB8000`.
- `scripts/`: Powershell pipelines for easy bootstrapping, building, and running.

## Running the Kernel Lab

1. **Bootstrap Target:**
   ```powershell
   powershell .\scripts\bootstrap.ps1
   ```

2. **Compile to Multiboot ELF:**
   ```powershell
   powershell .\scripts\build.ps1
   ```

3. **Virtualized Boot (QEMU):**
   ```powershell
   powershell .\scripts\run.ps1
   ```

This uses direct kernel booting `-kernel target/i686-unknown-none/debug/dbyte_kernel` under QEMU to instantly run your freestanding Rust ELF kernel prototype!
