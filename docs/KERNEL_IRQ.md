# DByteOS Kernel IRQ/PIC Safety Notes (v8.14.0)

DByteOS Kernel Lab `v8.14.0` is an IRQ Runtime Activation Preconditions 2 release. It consolidates PIC remap state, IRQ gate bind state, EOI strategy state, keyboard fallback state, and pf-smoke state into unified preflight, status, and blockers commands. The `pic-remap-arm` command must still run before `pic-remap-smoke`; only that explicit command path may write the PIC ICW sequence and mask all IRQ lines afterward. The `irq-gate-arm` / `irq-gate-bind-smoke` path may install IDT vectors `32` and `33` only after explicit arming, with smoke stubs that return through `iretd`. Runtime IRQ readiness remains blocked. No boot path installs gates, no EOI is actively dispatched, `sti` remains disabled, PIC IRQ lines remain masked, and keyboard input remains polling-only through PS/2 ports `0x64` and `0x60`.

This milestone still implements an EOI strategy foundation on top of the IRQ handler skeleton while keeping the IRQ gate plan and disabled bind path dormant and adding a preflight status surface. It adds no new runtime IRQ behavior, no active IDT bind path, and no dry-bind readiness path.

## PIC Remap Plan

The 8259A PIC pair routes hardware interrupt requests into CPU interrupt vectors. The planned remap moves IRQs away from CPU exception vectors and into `0x20-0x2f`.

| Controller | IRQ Lines  | Ports                        | Planned Vector Offset |
| ---------- | ---------- | ---------------------------- | --------------------- |
| Master PIC | IRQ0-IRQ7  | `0x20` command / `0x21` data | `0x20`                |
| Slave PIC  | IRQ8-IRQ15 | `0xA0` command / `0xA1` data | `0x28`                |

PIC remap dry-run telemetry remains available, and `v8.14.0` adds a separate controlled IDT gate bind smoke path for IRQ0/IRQ1. Initialization Command Words are dispatched only after `pic-remap-arm` followed by `pic-remap-smoke`; no boot path remaps the PIC, no EOI is sent, and no `sti` runs. IRQ gates 32/33 are installed only by `irq-gate-arm` followed by `irq-gate-bind-smoke`.

## Remap Controlled Smoke Foundation

PIC Remap State Telemetry remains available through state/history/preflight commands while IRQ gate binding controlled smoke is tested separately.

- `remap_plan()` returns the planned remap offsets, IRQ vector range, and disabled mask state.
- `remap_disabled()` documents the ICW1-ICW4 sequence and returns the plan without touching hardware.
- `remap_disabled()` returns the documentation-only plan through `remap_plan()`.
- `irq_map_plan()` returns the documentation-only IRQ0-IRQ15 vector map for dry-run telemetry.
- `pic_remap_smoke_arm()` arms the one-shot smoke path.
- `pic_remap_controlled_smoke()` writes the ICW sequence only when armed, then masks all PIC IRQ lines and clears the arm flag.
- `pic_remap_smoke_status()` reports arm/executed state without touching hardware.
- `pic_remap_state()`, `pic_remap_history()`, and `pic_remap_preflight()` report controlled smoke telemetry without touching hardware.
- The remap smoke function is not called from boot, IDT setup, IRQ setup, or keyboard input paths.
- IRQ vectors `0x20-0x2f` are planned only.

## IRQ Handler Skeleton Foundation

- `kernel-lab/src/irq.rs` compiles documentation-only IRQ0 timer and IRQ1 keyboard skeletons.
- `IRQ0_VECTOR = 32` and `IRQ1_VECTOR = 33` define the future remapped vectors.
- `IrqHandlerSkeleton`, `irq0_timer_skeleton()`, `irq1_keyboard_skeleton()`, and `irq_handler_skeletons()` describe the planned handlers without binding them.
- `IrqGatePlan`, `irq0_timer_gate_plan()`, `irq1_keyboard_gate_plan()`, and `irq_gate_plan()` describe the dormant gate binding plan without touching IDT, PIC, EOI, or interrupt state.
- `IrqGateBindDisabledStep`, `IrqGateBindDisabledStatus`, and `bind_irq_gates_disabled()` describe the disabled bind path without accepting an IDT reference, mutating IDT entries, remapping PIC, dispatching EOI, or enabling interrupts.
- `IrqRuntimeReadiness`, `IrqRuntimeRisk`, `IrqRuntimePreflight`, and their helpers describe readiness, risk, and preflight telemetry without accepting IDT/PIC references or changing runtime state.
- The skeletons are not called from boot, shell commands, IDT setup, PIC setup, or keyboard input paths.
- IRQ0/IRQ1 smoke assembly wrappers exist only as dormant IDT targets for the controlled bind smoke path. They return with `iretd`, perform no EOI, perform no port I/O, and are not hardware-triggered because `sti` remains disabled and PIC IRQ lines remain masked.

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
- **Gate Status**: Both gates remain unbound at boot. `idt::IDT.entries[32].set_handler` and `idt::IDT.entries[33].set_handler` exist only inside the armed `irq-gate-bind-smoke` command path.
- **Command Surface**: `irq-gate-plan` reads the compiled helper plan and prints the dormant route for IRQ0/IRQ1. It does not run during boot and does not bind either vector.
- **Disabled Bind Path**: `bind_irq_gates_disabled()` is a telemetry helper for the future IRQ0/IRQ1 gate bind sequence. It is read only by `irq-bind-note` and `irq-bind-status`, never during boot, and never installs IDT entries.
- **Controlled Bind Smoke**: `irq-gate-arm`, `irq-gate-bind-smoke`, and `irq-gate-bind-status` expose a one-shot IDT bind smoke for vectors `32/33`. Binding remains dormant because PIC IRQ lines stay masked, EOI dispatch is disabled, `sti` is disabled, and keyboard input remains polling-only.
- **Bind State Telemetry**: `irq-gate-state`, `irq-gate-history`, and `irq-gate-preflight` report controlled bind telemetry without touching hardware. The `system` command syncs `irq gates controlled smoke: bound=yes|no`.
- **Readiness Gate**: `irq-readiness`, `irq-risk`, and `irq-preflight` read compiled helper telemetry only. They report that runtime IRQ remains blocked even though PIC remap controlled smoke and gate bind controlled smoke exist, because EOI dispatch, hardware IRQ unmasking, and `sti` remain unavailable.

## v8.14.0 IRQ Gate Bind State Telemetry & Static Guards

This release adds read-only IRQ gate bind state/history/preflight telemetry and dynamic `handlers` / `system` sync without enabling runtime IRQ behavior.
Verification guards enforce that `IRQ0_VECTOR` stays `32`, `IRQ1_VECTOR` stays
`33`, `irq-handlers` output remains exact, disabled bind and readiness command output remains exact, handlers/system documentation stays
in sync, IDT vectors `32` and `33` are not bound at boot and are bound only inside the armed `irq-gate-bind-smoke` command path, `asm!("sti")` is absent, PIC
remap smoke is command-path only, `kernel-lab/src/pic.rs` is the only source allowed to write PIC ports,
keyboard input remains polling-only, and `pf-smoke` mechanics remain unchanged.
The `irq-gate-plan` command is guarded as the only runtime command-path read of
`irq::irq_gate_plan()`; boot remains free of IRQ gate helper calls.
The `bind_irq_gates_disabled()` helper is guarded as command-path telemetry only;
boot remains free of disabled bind helper calls. The `IrqGatePlan` and disabled
bind status field shapes, vector constants, and exact printed telemetry contracts
are pinned by verification so future IRQ work cannot silently turn the plan into
active IDT, PIC, or EOI behavior.
The readiness helpers are guarded as command-path telemetry only; boot remains
free of readiness/preflight helper calls and `ready for runtime irq` remains `no`.
The PIC remap telemetry helpers are guarded as command-path/system telemetry only;
boot remains free of state/history/preflight helper activation.
The IRQ gate bind smoke helpers are guarded as command-path telemetry only; boot
remains free of vector `32/33` binding, PIC unmasking, EOI dispatch, and STI.
The IRQ gate bind state/history/preflight helpers are guarded as command-path/system
telemetry only; boot remains free of state/history/preflight helper activation.

## IRQ Glossary

- **ICW1 (`0x11`)**: planned initialization command.
- **ICW2 (`0x20` / `0x28`)**: planned master/slave remap offsets.
- **ICW3 (`0x04` / `0x02`)**: planned master/slave cascade wiring.
- **ICW4 (`0x01`)**: planned 8086 mode.
- **IRQ0 timer**: skeleton planned PIT timer interrupt; bind smoke stub is dormant in `v8.14.0`.
- **IRQ1 keyboard**: skeleton planned PS/2 keyboard interrupt; bind smoke stub is dormant in `v8.14.0`.
- **IRQ vectors 32-47**: planned remapped CPU vector range for IRQ0-IRQ15.
- **EOI**: End Of Interrupt command planned for future PIC acknowledgements.
- **STI**: Set Interrupt Flag instruction; not used in `v8.14.0`.

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

```txt
IRQ bind note:
IRQ0 timer gate: disabled bind path only
IRQ1 keyboard gate: disabled bind path only
IDT entries: planned / not installed
PIC remap: disabled
EOI dispatch: disabled
interrupts: disabled
```

```txt
IRQ bind status:
helper: bind_irq_gates_disabled
boot call: no
IDT vector 32: unbound
IDT vector 33: unbound
active IRQ0 handler: none
active IRQ1 handler: none
keyboard input: polling-only
```

```txt
IRQ gate bind smoke armed
mode: controlled bind smoke
next: irq-gate-bind-smoke
interrupts: disabled
pic irq mask: masked
eoi dispatch: disabled
```

```txt
IRQ gate bind controlled smoke
guard: not armed
result: blocked
next: irq-gate-arm
```

```txt
IRQ gate bind controlled smoke
guard: armed
IDT vector 32: bound to IRQ0 timer smoke stub
IDT vector 33: bound to IRQ1 keyboard smoke stub
pic irq mask: masked
sti: disabled
eoi dispatch: disabled
keyboard input: polling-only
result: bound / dormant
```

```txt
IRQ gate bind smoke status
armed: no
executed: no
IDT vector 32: unbound
IDT vector 33: unbound
active IRQ0 handler: smoke stub / dormant
active IRQ1 handler: smoke stub / dormant
pic irq mask: masked
sti: disabled
eoi dispatch: disabled
keyboard input: polling-only
```

```txt
IRQ gate bind smoke status
armed: no
executed: yes
IDT vector 32: bound
IDT vector 33: bound
active IRQ0 handler: smoke stub / dormant
active IRQ1 handler: smoke stub / dormant
pic irq mask: masked
sti: disabled
eoi dispatch: disabled
keyboard input: polling-only
```

```txt
IRQ gate bind state
armed: no
executed: no
IDT vector 32: unbound
IDT vector 33: unbound
active IRQ0 handler: smoke stub / dormant
active IRQ1 handler: smoke stub / dormant
bind expected: yes
bind applied: no
irq runtime: disabled
pic irq mask: masked
sti: disabled
eoi dispatch: disabled
keyboard input: polling-only
```

```txt
IRQ gate bind state
armed: no
executed: yes
IDT vector 32: bound
IDT vector 33: bound
active IRQ0 handler: smoke stub / dormant
active IRQ1 handler: smoke stub / dormant
bind expected: yes
bind applied: yes
irq runtime: disabled
pic irq mask: masked
sti: disabled
eoi dispatch: disabled
keyboard input: polling-only
```

```txt
IRQ gate bind history
arm command: available
smoke command: available
last smoke executed: no
idt binds: controlled command path only
boot bind: no
```

```txt
IRQ gate bind history
arm command: available
smoke command: available
last smoke executed: yes
idt binds: controlled command path only
boot bind: no
```

```txt
IRQ gate bind preflight
guard: command armed required
bind path: ready
IDT vector 32: unbound
IDT vector 33: unbound
pic irq mask: masked
sti: disabled
eoi dispatch: disabled
keyboard input: polling-only
result: telemetry only
```

```txt
irq gates controlled smoke: bound=no
```

```txt
irq gates controlled smoke: bound=yes
```

```txt
PIC remap smoke armed
mode: controlled smoke
next: pic-remap-smoke
interrupts: disabled
irq gates: unbound
```

```txt
PIC remap controlled smoke
guard: armed
icw sequence: written
master offset: 0x20
slave offset: 0x28
mask after remap: 0xff
sti: disabled
irq gates: unbound
eoi dispatch: disabled
result: remapped / masked
```

```txt
PIC remap controlled smoke
guard: not armed
result: blocked
next: pic-remap-arm
```

```txt
PIC remap smoke status
armed: no
executed: no
master offset: 0x20
slave offset: 0x28
mask after remap: 0xff
sti: disabled
irq gates: unbound
eoi dispatch: disabled
```

```txt
PIC remap smoke status
armed: no
executed: yes
master offset: 0x20
slave offset: 0x28
mask after remap: 0xff
sti: disabled
irq gates: unbound
eoi dispatch: disabled
```

```txt
PIC remap state
armed: no
executed: no
master offset: 0x20
slave offset: 0x28
icw sequence expected: yes
icw sequence applied: no
mask after remap: 0xff
irq runtime: disabled
```

```txt
PIC remap history
arm command: available
smoke command: available
last smoke executed: no
icw writes: controlled command path only
boot remap: no
```

```txt
PIC remap preflight
guard: command armed required
icw sequence: ready
master offset: 0x20
slave offset: 0x28
mask after remap: 0xff
sti: disabled
irq gates: unbound
eoi dispatch: disabled
result: telemetry only
```

```txt
pic remap controlled smoke: executed=no
```

```txt
IRQ runtime readiness
idt exceptions: ok
irq gate plan: ok
eoi strategy: ok
pic remap: controlled smoke only
sti: disabled
keyboard fallback: polling
ready for runtime irq: no
```

```txt
IRQ runtime risk
runtime irq: blocked
reason: IRQ0/IRQ1 gates are not bound
required before enable: IDT gate bind, PIC remap, EOI dispatch, handler stubs
sti allowed: no
```

```txt
IRQ runtime preflight
IDT exceptions 0/3/14: pass
IRQ vectors 32/33: unbound
bind path: disabled
EOI dispatch: disabled
PIC remap: controlled smoke only
keyboard fallback: polling
pf-smoke: unchanged
result: blocked
```

## Safety Boundaries

- No `asm!("sti")`.
- No boot-time PIC remap call or unarmed ICW dispatch.
- PIC hardware writes are limited to the armed `pic-remap-smoke` command path in `kernel-lab/src/pic.rs`.
- No boot-time IRQ IDT bindings beyond existing exception vectors `0`, `3`, and `14`.
- IDT vectors `32/33` may be bound only by the armed `irq-gate-bind-smoke` command path.
- No IRQ1 keyboard hardware-active handler.
- No IRQ0 PIT hardware-active handler.
- No boot-time call to `bind_irq_gates_disabled()`.
- No boot-time call to runtime readiness helpers.
- No boot-time call to PIC remap state telemetry helpers.
- No EOI dispatch.
- No keyboard polling path rewrite.
- No change to `pf-smoke` mechanics and no `asm!("int 14")`.
