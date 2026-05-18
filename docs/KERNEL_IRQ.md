# DByteOS Kernel PIC Remap Dry-Run Telemetry (v8.3.0)

DByteOS Kernel Lab `v8.3.0` adds PIC remap dry-run telemetry. PIC/IRQ remains planned / disabled: the remap function is present / not called, dry-run commands expose the planned ICW sequence and IRQ map, no hardware writes are performed, maskable interrupts remain disabled, and keyboard input remains polling-only through PS/2 ports `0x64` and `0x60`.

## PIC Remap Plan

The 8259A PIC pair routes hardware interrupt requests into CPU interrupt vectors. The planned remap moves IRQs away from CPU exception vectors and into `0x20-0x2f`.

| Controller | IRQ Lines | Ports | Planned Vector Offset |
| --- | --- | --- | --- |
| Master PIC | IRQ0-IRQ7 | `0x20` command / `0x21` data | `0x20` |
| Slave PIC | IRQ8-IRQ15 | `0xA0` command / `0xA1` data | `0x28` |

PIC remap dry-run telemetry is documented and compiled only. No Initialization Command Words are dispatched, no EOI is sent, no hardware writes are performed, and no IRQ gate is installed in the IDT.

## Remap Dry-Run Foundation

- `remap_plan()` returns the planned remap offsets, IRQ vector range, and disabled mask state.
- `remap_disabled()` documents the ICW1-ICW4 sequence and returns the plan without touching hardware.
- `remap_disabled()` returns the documentation-only plan through `remap_plan()`.
- `irq_map_plan()` returns the documentation-only IRQ0-IRQ15 vector map for dry-run telemetry.
- The remap function is present / not called from boot, shell commands, IDT setup, or keyboard input paths.
- IRQ vectors `0x20-0x2f` are planned only.

## IRQ Glossary

- **ICW1 (`0x11`)**: planned initialization command.
- **ICW2 (`0x20` / `0x28`)**: planned master/slave remap offsets.
- **ICW3 (`0x04` / `0x02`)**: planned master/slave cascade wiring.
- **ICW4 (`0x01`)**: planned 8086 mode.
- **IRQ0 timer**: planned PIT timer interrupt; disabled in `v8.3.0`.
- **IRQ1 keyboard**: planned PS/2 keyboard interrupt; disabled in `v8.3.0`.
- **IRQ vectors 32-47**: planned remapped CPU vector range for IRQ0-IRQ15.
- **EOI**: End Of Interrupt command planned for future PIC acknowledgements.
- **STI**: Set Interrupt Flag instruction; not used in `v8.3.0`.

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

```txt
pic remap dry-run:
master offset: 0x20
slave offset: 0x28
irq vector range: 0x20-0x2f
icw1: 0x11
icw2 master: 0x20
icw2 slave: 0x28
icw3 master: 0x04
icw3 slave: 0x02
icw4: 0x01
mask after remap: 0xff
hardware writes: disabled
```

```txt
irq map:
irq0 timer -> vector 32 (0x20)
irq1 keyboard -> vector 33 (0x21)
irq2 cascade -> vector 34 (0x22)
irq3 serial2 -> vector 35 (0x23)
irq4 serial1 -> vector 36 (0x24)
irq5 parallel2 -> vector 37 (0x25)
irq6 floppy -> vector 38 (0x26)
irq7 parallel1 -> vector 39 (0x27)
irq8 rtc -> vector 40 (0x28)
irq9 acpi -> vector 41 (0x29)
irq10 reserved -> vector 42 (0x2a)
irq11 reserved -> vector 43 (0x2b)
irq12 mouse -> vector 44 (0x2c)
irq13 fpu -> vector 45 (0x2d)
irq14 primary-ata -> vector 46 (0x2e)
irq15 secondary-ata -> vector 47 (0x2f)
active irq handlers: none
```

```txt
pic subsystem:
foundation: dry-run telemetry
remap function: present / not called
dry-run plan: available
master offset: 0x20
slave offset: 0x28
irq vectors: 0x20-0x2f
hardware writes: disabled
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
