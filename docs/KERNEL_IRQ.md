# DByteOS Kernel IRQ Handler Skeleton Foundation (v8.7.0)

DByteOS Kernel Lab `v8.7.0` implements the IRQ Gate Binding Code Foundation on top of the IRQ handler skeleton and EOI strategy foundation. This is a planning and code-foundation-only release: IRQ gate plan structs/helpers are compiled and exposed through `irq-gate-plan`, but no IRQ gate is installed, no EOI is actively dispatched, no hardware writes are performed, PIC/IRQ remains planned / disabled, the remap function is present / not called, dry-run commands expose the planned ICW sequence and IRQ map, maskable interrupts remain disabled, and keyboard input remains polling-only through PS/2 ports `0x64` and `0x60`.

This milestone still implements an EOI strategy foundation on top of the IRQ handler skeleton; v8.7.0 adds only dormant IRQ gate plan helpers and command telemetry.

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

## IRQ Handler Skeleton Foundation

- `kernel-lab/src/irq.rs` compiles documentation-only IRQ0 timer and IRQ1 keyboard skeletons.
- `IRQ0_VECTOR = 32` and `IRQ1_VECTOR = 33` define the future remapped vectors.
- `IrqHandlerSkeleton`, `irq0_timer_skeleton()`, `irq1_keyboard_skeleton()`, and `irq_handler_skeletons()` describe the planned handlers without binding them.
- `IrqGatePlan`, `irq0_timer_gate_plan()`, `irq1_keyboard_gate_plan()`, and `irq_gate_plan()` describe the dormant gate binding plan without touching IDT, PIC, EOI, or interrupt state.
- The skeletons are not called from boot, shell commands, IDT setup, PIC setup, or keyboard input paths.
- No assembly wrapper, active `extern "C"` entrypoint, EOI write, PIC remap call, or port write exists for IRQ0/IRQ1 in `v8.7.0`.

## EOI Strategy Foundation

End Of Interrupt (EOI) processing is a hardware acknowledgment protocol required to clear the In-Service Register (ISR) of the 8259A PIC, allowing subsequent hardware interrupts of equal or lower priority to trigger.

- **PIC_EOI (`0x20`)**: End of Interrupt command value.
- **EoiTarget**: Enumeration representing routing rules:
  - `MasterOnly`: Send EOI command `0x20` to the Master PIC command port (`0x20`).
  - `MasterAndSlave`: Send EOI command `0x20` to both the Master PIC command port (`0x20`) and the Slave PIC command port (`0xA0`).
  - `None`: No EOI is required.
- **EoiPlan**: Struct describing an EOI path, specifying the target and ports.
- **Dry-run Configurations**:
  - `master_eoi_plan()`: returns dry-run master EOI targets.
  - `slave_eoi_plan()`: returns dry-run slave EOI targets.
  - `irq0_timer_eoi_plan()`: returns the planned timer (IRQ0) EOI path.
  - `irq1_keyboard_eoi_plan()`: returns the planned keyboard (IRQ1) EOI path.
  - `eoi_strategy_status()`: returns combined EOI strategy metrics for CLI command dispatch.

No EOI command functions are called in this release; they are compiled solely for verification and system preparation.

## IRQ Gate Binding Plan

To support external hardware interrupts safely, the kernel maps Master and Slave PIC IRQ lines to CPU vectors 32 through 47. The gate binding plan outlines the future installation of these gates in the Interrupt Descriptor Table (IDT).

- **Vector 32 (IRQ0 Timer)**: Mapped to the Programmable Interval Timer (PIT). The IDT gate remains planned, registered as a null/disabled handler, and dormant.
- **Vector 33 (IRQ1 Keyboard)**: Mapped to the PS/2 keyboard controller. The IDT gate remains planned, registered as a null/disabled handler, and dormant.
- **Gate Status**: Both gates remain strictly unbound at runtime. No `idt::IDT.entries[32].set_handler` or `idt::IDT.entries[33].set_handler` calls exist.
- **Command Surface**: `irq-gate-plan` reads the compiled helper plan and prints the dormant route for IRQ0/IRQ1. It does not run during boot and does not bind either vector.

## v8.7.0 Hardening & Static Guards

This release locks the IRQ handler skeleton and gate binding plan as compile-time structure only.
Verification guards enforce that `IRQ0_VECTOR` stays `32`, `IRQ1_VECTOR` stays
`33`, `irq-handlers` output remains exact, handlers/system documentation stays
in sync, IDT vectors `32` and `33` are not bound, `asm!("sti")` is absent, PIC
remap hooks are not called, `kernel-lab/src/pic.rs` performs no `outb` writes,
keyboard input remains polling-only, and `pf-smoke` mechanics remain unchanged.
The `irq-gate-plan` command is guarded as the only runtime command-path read of
`irq::irq_gate_plan()`; boot remains free of IRQ gate helper calls.

## IRQ Glossary

- **ICW1 (`0x11`)**: planned initialization command.
- **ICW2 (`0x20` / `0x28`)**: planned master/slave remap offsets.
- **ICW3 (`0x04` / `0x02`)**: planned master/slave cascade wiring.
- **ICW4 (`0x01`)**: planned 8086 mode.
- **IRQ0 timer**: skeleton planned PIT timer interrupt; disabled in `v8.7.0`.
- **IRQ1 keyboard**: skeleton planned PS/2 keyboard interrupt; disabled in `v8.7.0`.
- **IRQ vectors 32-47**: planned remapped CPU vector range for IRQ0-IRQ15.
- **EOI**: End Of Interrupt command planned for future PIC acknowledgements.
- **STI**: Set Interrupt Flag instruction; not used in `v8.7.0`.

## Status UX

```txt
pic/irq: planned / disabled
pic remap: documented only
irq vectors: 32-47 planned
irq handler skeletons: irq0 timer, irq1 keyboard
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

```txt
irq handlers:
foundation: skeleton / disabled
irq0 timer: skeleton / disabled
irq1 keyboard: skeleton / disabled
vectors: 32 / 33
idt binding: disabled
pic remap: disabled
interrupts: disabled
```

```txt
irq handlers:
skeleton planned: irq0 timer, irq1 keyboard
active: none
```

```txt
EOI strategy: planned / disabled
PIC command: 0x20
master PIC: planned
slave PIC: planned
dispatch: disabled
```

```txt
EOI strategy note:
- EOI means End Of Interrupt.
- Master PIC EOI targets command port 0x20 in the future.
- Slave IRQs require slave EOI plus master cascade acknowledgement in the future.
- IRQ0 timer and IRQ1 keyboard EOI paths are planned only.
- No EOI is dispatched in this milestone.
```

```txt
IRQ Interrupt Gates:
- Vector 32 (0x20): IRQ0 Timer (planned)
- Vector 33 (0x21): IRQ1 Keyboard (planned)
- Handler setup: planned
- Status: dormant / disabled
```

```txt
IDT vector 32 (IRQ0 Timer): disabled / null handler
IDT vector 33 (IRQ1 Keyboard): disabled / null handler
gate binding dispatch: dormant
```

```txt
IRQ Gate Binding Plan:
IRQ0 timer -> vector 32 (0x20)
IRQ1 keyboard -> vector 33 (0x21)
IDT binding: disabled
PIC remap: disabled
EOI dispatch: disabled
interrupts: disabled
state: dormant / disabled
```

## Safety Boundaries

- No `asm!("sti")`.
- No PIC remap call or ICW dispatch.
- No hardware writes from `kernel-lab/src/pic.rs`.
- No active IRQ IDT bindings beyond existing exception vectors `0`, `3`, and `14`.
- No IRQ1 keyboard active handler or IDT binding.
- No IRQ0 PIT active handler or IDT binding.
- No EOI dispatch.
- No keyboard polling path rewrite.
- No change to `pf-smoke` mechanics and no `asm!("int 14")`.
