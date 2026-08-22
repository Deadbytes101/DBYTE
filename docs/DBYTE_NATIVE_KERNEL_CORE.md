# DByte Native Kernel Core

`v11.2.0` is the first DByteOS core whose interrupt architecture is compiled
from multi-module DByte source and executed directly by an i686 CPU. Rust
`kernel-lab` remains a known-good hardware oracle; it is not linked into this
image and provides no runtime service.

## Executed path

```text
Multiboot _start
  -> DByte kernel_main
  -> COM1 serial
  -> DByte GDT and segment reload
  -> DByte IDT
  -> controlled INT3 and iretd
  -> 8259 PIC remap to 0x20/0x28
  -> PIT channel 0 mode 2 at about 100 Hz
  -> hardware IRQ0 DByte entry stub
  -> DByte tick body and master EOI
  -> bounded sti/hlt/cli loop
```

The proof lives under `examples/native_kernel_core/`. Its eight objects are
`arch_x86`, `serial`, `gdt`, `exceptions`, `pic`, `pit`, `idt`, and `kernel`.
The IDT contains 256 packed eight-byte entries; CPU exception vectors 0 through
31 receive class-correct handlers and vector 32 receives the IRQ0 entry.

The GDT contains null, flat ring-zero code, and flat ring-zero data descriptors.
After `lgdt`, generated machine code reloads `DS`, `ES`, `FS`, `GS`, and `SS`
with selector `0x10`, then performs a far jump through code selector `0x08`.

## Runtime acceptance

The verifier requires exact CRLF serial bytes:

```text
DBYTEOS NATIVE CORE
GDT ONLINE
IDT ONLINE
EXCEPTIONS ONLINE
PIC ONLINE
IRQ0 TIMER ONLINE
TICKS 1
TICKS 8
```

The first tick and every subsequent tick execute one bounded `sti; hlt; cli`
cycle. Reaching tick 8 proves repeated hardware delivery and handler-triggered
EOI; the kernel then masks both PICs and returns to the linker's permanent halt
loop with interrupts disabled.

Run the independent proof with:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_kernel_core.ps1
```

The script double-builds every ET_REL, linked ELF, and BIN; compares SHA-256;
checks symbols, relocations, `.dbyte.interrupts`, and final disassembly; and
runs QEMU with `-d int,cpu_reset,guest_errors`. Evidence is ignored under
`tmp/native_v11.2.0/`.

## Deliberate boundary

This milestone remains CPL0-to-CPL0, 32-bit protected mode only. It does not add
userspace transitions, task gates, VM86, paging recovery, scheduler, threads,
heap, filesystem, drivers beyond serial/PIC/PIT, ACPI, SMP, networking, audio,
or GUI facilities. There is no textual assembly, NASM, C, Rust runtime shim, or
external code generator/linker in the native kernel path.
