# DByteOS Kernel PIC/IRQ Direction Foundation (v8.1.0)

DByteOS Kernel Lab `v8.1.0` documents the PIC/IRQ direction foundation. This is a direction-only release: PIC/IRQ remains planned / disabled, maskable interrupts remain disabled, and keyboard input remains polling-only through PS/2 ports `0x64` and `0x60`.

## PIC Remap Plan

The 8259A PIC pair routes hardware interrupt requests into CPU interrupt vectors.

| Controller | IRQ Lines | Ports | Planned Vector Offset |
| --- | --- | --- | --- |
| Master PIC | IRQ0-IRQ7 | `0x20` command / `0x21` data | `0x20` |
| Slave PIC | IRQ8-IRQ15 | `0xA0` command / `0xA1` data | `0x28` |

PIC remap is documented only. No Initialization Command Words are dispatched, no EOI is sent, and no IRQ gate is installed in the IDT.

## IRQ Glossary

- **IRQ0 timer**: planned PIT timer interrupt; disabled in `v8.1.0`.
- **IRQ1 keyboard**: planned PS/2 keyboard interrupt; disabled in `v8.1.0`.
- **IRQ vectors 32-47**: planned remapped CPU vector range for IRQ0-IRQ15.
- **EOI**: End Of Interrupt command planned for future PIC acknowledgements.
- **STI**: Set Interrupt Flag instruction; not used in `v8.1.0`.

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

## Safety Boundaries

- No `asm!("sti")`.
- No PIC remap call or ICW dispatch.
- No active IRQ IDT bindings beyond existing exception vectors `0`, `3`, and `14`.
- No IRQ1 keyboard handler.
- No IRQ0 PIT handler.
- No keyboard polling path rewrite.
- No change to `pf-smoke` mechanics and no `asm!("int 14")`.
