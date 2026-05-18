# DByteOS Kernel PIC Remap Code Foundation (v8.2.1)

DByteOS Kernel Lab `v8.2.1` hardens the PIC remap code foundation. This is a hardening-only release: PIC/IRQ remains planned / disabled. This release documents and compiles the remap sequence shape, but the remap function is present / not called, no hardware writes are performed, maskable interrupts remain disabled, and keyboard input remains polling-only through PS/2 ports `0x64` and `0x60`.

## PIC Remap Plan

The 8259A PIC pair routes hardware interrupt requests into CPU interrupt vectors. The planned remap moves IRQs away from CPU exception vectors and into `0x20-0x2f`.

| Controller | IRQ Lines | Ports | Planned Vector Offset |
| --- | --- | --- | --- |
| Master PIC | IRQ0-IRQ7 | `0x20` command / `0x21` data | `0x20` |
| Slave PIC | IRQ8-IRQ15 | `0xA0` command / `0xA1` data | `0x28` |

PIC remap code foundation is documented and compiled only. No Initialization Command Words are dispatched, no EOI is sent, no hardware writes are performed, and no IRQ gate is installed in the IDT.

## Remap Code Foundation

- `remap_plan()` returns the planned remap offsets, IRQ vector range, and disabled mask state.
- `remap_disabled()` documents the ICW1-ICW4 sequence and returns the plan without touching hardware.
- `remap_disabled()` returns the documentation-only plan through `remap_plan()`.
- The remap function is present / not called from boot, shell commands, IDT setup, or keyboard input paths.
- IRQ vectors `0x20-0x2f` are planned only.

## IRQ Glossary

- **ICW1 (`0x11`)**: planned initialization command.
- **ICW2 (`0x20` / `0x28`)**: planned master/slave remap offsets.
- **ICW3 (`0x04` / `0x02`)**: planned master/slave cascade wiring.
- **ICW4 (`0x01`)**: planned 8086 mode.
- **IRQ0 timer**: planned PIT timer interrupt; disabled in `v8.2.1`.
- **IRQ1 keyboard**: planned PS/2 keyboard interrupt; disabled in `v8.2.1`.
- **IRQ vectors 32-47**: planned remapped CPU vector range for IRQ0-IRQ15.
- **EOI**: End Of Interrupt command planned for future PIC acknowledgements.
- **STI**: Set Interrupt Flag instruction; not used in `v8.2.1`.

## Status UX

```txt
pic/irq: planned / disabled
pic remap: documented only
irq vectors: 32-47 planned
keyboard irq1: disabled
timer irq0: disabled
interrupts: disabled
```

```txt
irq subsystem:
foundation: planned
pic: not remapped
irq handlers: none
keyboard input: polling-only
timer: unavailable
interrupts: disabled
```

```txt
pic remap: planned / disabled
remap offsets: 0x20 / 0x28
irq vectors: 0x20-0x2f
icw sequence: documented in code
hardware writes: disabled
interrupts: disabled
```

```txt
pic subsystem:
foundation: code planned
remap function: present / not called
master offset: 0x20
slave offset: 0x28
irq handlers: none
interrupts: disabled
```

## Safety Boundaries

- No `asm!("sti")`.
- No PIC remap call or ICW dispatch.
- No hardware writes from `kernel-lab/src/pic.rs`.
- No active IRQ IDT bindings beyond existing exception vectors `0`, `3`, and `14`.
- No IRQ1 keyboard handler.
- No IRQ0 PIT handler.
- No keyboard polling path rewrite.
- No change to `pf-smoke` mechanics and no `asm!("int 14")`.
