# DByteOS Kernel IRQ/PIC Safety Notes (v10.18.0)

DByteOS Kernel Lab `v10.18.0` is a Controlled IRQ Handler EOI Path Candidate Foundation release. It adds a read-only, unreachable candidate layer for a future handler-side EOI path while keeping handler-triggered EOI denied, runtime IRQ inactive, `sti` disabled, PIC lines masked, live IRQ handlers unbound, keyboard polling unchanged, and the single manual PIC_EOI callsite unchanged. `v10.17.2` is a Controlled PIC_EOI Runtime Bridge Readiness Hardening release. It adds no commands and preserves `v10.17.1` sticky proof behavior while tightening verifier guards around bridge output, proof source, read-only isolation, and the single manual PIC_EOI callsite. `v10.17.1` is a Controlled PIC_EOI Runtime Bridge Session Proof Repair release. It splits sticky boot-session proof from transient manual hw-smoke performed telemetry while keeping handler-triggered EOI denied, runtime IRQ inactive, `sti` disabled, PIC lines masked, live IRQ handlers unbound, keyboard polling unchanged, and the single manual PIC_EOI callsite unchanged. `v10.17.0` is a Controlled PIC_EOI Runtime Bridge Readiness Foundation release. It adds a read-only bridge above the manual PIC_EOI smoke proof while keeping handler-triggered EOI denied, runtime IRQ inactive, `sti` disabled, PIC lines masked, live IRQ handlers unbound, and keyboard polling unchanged. `v10.16.1` is a First Controlled PIC_EOI Write Smoke Hardening release. It preserves the `v10.16.0` manual one-shot hardware smoke outputs while tightening verifier guards around the exact state sequence, single allowlisted callsite, forbidden runtime activation paths, and unchanged keyboard polling. `v10.16.0` is a First Controlled PIC_EOI Write Smoke Foundation release. It adds a manual one-shot shell command path that may perform exactly one `write_pic_port(PIC_MASTER_COMMAND, PIC_EOI)` after explicit arming while keeping slave EOI writes, IRQ runtime activation, `sti`, PIC unmask, live IDT binding, and keyboard IRQ mode disabled. `v10.15.1` is a Controlled EOI Write Permit Evaluation Hardening release. It adds no commands and preserves the `v10.15.0` read-only evaluator output while tightening verifier guards around exact output, reader ordering, evaluator isolation, stale metadata, and forbidden hardware paths. `v10.15.0` is a Controlled EOI Write Permit Evaluation Foundation release. It adds a read-only evaluator above the permit transition model while keeping evaluation readiness denied, the real permit denied, bridge readiness denied, and all hardware mutation blocked. `v10.14.1` is a Controlled EOI Write Permit Transition Model Hardening release. It adds no commands and preserves the `v10.14.0` software-only transition behavior while tightening verifier guards around exact output, state sequence, transition isolation, stale metadata, and forbidden hardware paths. `v10.14.0` is a Controlled EOI Write Permit Transition Model Foundation release. It adds a software-only transition state above the denied permit model while keeping the real permit denied, bridge readiness denied, and all hardware mutation blocked. `v10.13.1` is a Controlled EOI Write One-Shot Permit Bridge Hardening release. It adds no commands and preserves the `v10.13.0` read-only bridge behavior while tightening verifier guards around exact output, read ordering, bridge isolation, stale metadata, and forbidden hardware paths. `v10.13.0` is a Controlled EOI Write One-Shot Permit Bridge Foundation release. It adds a read-only bridge between the denied permit model and the software-only one-shot latch while keeping bridge readiness denied and performing no PIC EOI hardware write. `v10.12.1` is a Controlled EOI Write One-Shot Latch Hardening release. It adds no commands and preserves the `v10.12.0` software-only latch behavior while tightening verifier guards around latch state transitions, exact output, stale metadata, and forbidden hardware paths. `v10.12.0` is a Controlled EOI Write One-Shot Latch Foundation release. It adds a software-only one-shot latch layer while keeping fire blocked by the permit model and performing no PIC EOI hardware write. `v10.11.1` is a Controlled EOI Write One-Shot Command Path Hardening release. It hardens the existing read-only one-shot command path without setting a latch, enabling fire, or touching hardware. `v10.11.0` is a Controlled EOI Write One-Shot Command Path Foundation release. It adds a read-only one-shot command path after the hardened permit model without setting a latch, enabling fire, or touching hardware. `v10.10.1` is a Controlled EOI Write Permit Model Hardening release. It hardens the existing `v10.10.0` read-only permit model without changing permit output or touching hardware. `v10.10.0` is a Controlled EOI Write Permit Model Foundation release. It adds a read-only permit model before any first real PIC EOI write while keeping permit denied. The checklist, decision, sequencer, preflight, candidate, permit, one-shot, latch-fire, bridge, transition, and evaluation outputs remain blocked. The `pic-remap-arm` command must still run before `pic-remap-smoke`; only that explicit command path may write the PIC ICW sequence and mask all IRQ lines afterward. The `irq-gate-arm` / `irq-gate-bind-smoke` path may install IDT vectors `32` and `33` only after explicit arming, with smoke stubs that return through `iretd`. Runtime IRQ readiness remains blocked. No boot path installs gates, no IRQ-driven EOI is dispatched, `sti` remains disabled, PIC IRQ lines remain masked, live IDT runtime binding remains disabled, and keyboard input remains polling-only through PS/2 ports `0x64` and `0x60`.

`v10.15.0` evaluates the software EOI write chain without changing it. The evaluator reads the permit model, one-shot latch, bridge, transition state, final gate, mutation checklist, preflight, and candidate telemetry, then reports `evaluation ready: no` with the permit, bridge, first-write, hardware, and runtime fields still denied.

`v10.15.1` is hardening-only. The evaluator command outputs remain unchanged from `v10.15.0`; verifier guards now lock exact output, reader ordering, helper and dispatcher isolation, no latch/transition/permit/bridge mutation, no positive evaluator state, no hardware write path, no live IRQ path, and unchanged keyboard polling.

`v10.16.0` allows the first real PIC EOI hardware smoke, but only through `eoi-write-hw-smoke-fire` after `eoi-write-hw-smoke-arm`. A successful fire writes exactly one `PIC_EOI` to `PIC_MASTER_COMMAND`, consumes the one-shot latch, and leaves IRQ runtime inactive; a second fire without re-arm is blocked.

`v10.16.1` is hardening-only. The hw-smoke command outputs remain unchanged from `v10.16.0`; verifier guards now lock the exact manual sequence, the single manual fire callsite, no slave EOI write, no handler-triggered EOI, no looped write, no `sti`, no PIC unmask, no live IRQ0/IRQ1, no live IDT bind, no keyboard IRQ switch, and `runtime irq active: no`.

`v10.17.0` adds a read-only runtime bridge readiness layer. It reads session-local manual PIC_EOI smoke proof and existing software readiness state, then reports `runtime bridge ready: no`, `handler-triggered EOI allowed: no`, `runtime irq active: no`, `sti: disabled`, `pic unmask: disabled`, `live irq handlers: no`, and `keyboard mode: polling`.

`v10.17.1` repairs the bridge proof source. The bridge reads sticky boot-session proof set only after a successful manual hw-smoke fire, while transient `first PIC_EOI write performed` can still reset to `no` on clear.

`v10.17.2` is hardening-only. The runtime bridge command outputs remain unchanged from `v10.17.1`; verifier guards now lock sticky proof source isolation, read-only bridge surfaces, no handler-triggered EOI, and the single manual PIC_EOI write boundary.

`v10.18.0` adds a read-only IRQ handler EOI path candidate. It reads runtime bridge readiness and reports `handler EOI candidate ready: no`, `handler-triggered EOI allowed: no`, `live handler bind: no`, `PIC_EOI callsites: 1 manual-only`, `runtime irq active: no`, `sti: disabled`, `pic unmask: disabled`, and `keyboard mode: polling`.

`v10.14.1` is hardening-only. The transition command outputs remain unchanged from `v10.14.0`; verifier guards now lock the denied/unarmed sequence, single true/false store paths, read-only status/check/blockers surfaces, no latch mutation, no permit mutation, no positive permit state, and no hardware write path.

`v10.14.0` permits software transition telemetry only. `eoi-write-permit-transition-arm` sets `permit transition armed: yes`, `eoi-write-permit-transition-clear` returns it to `no`, and `eoi-write-permit-transition-check` remains denied without granting a permit. `permit granted: no`, `bridge ready: no`, `first PIC_EOI write allowed: no`, `hardware mutation: no`, and `runtime irq active: no` remain mandatory.

`v10.13.1` is hardening-only. The bridge still reads permit telemetry, reads latch telemetry, derives readiness as denied, and reports blockers without setting or clearing the latch. `bridge ready: no`, `first PIC_EOI write allowed: no`, `hardware mutation: no`, and `runtime irq active: no` remain mandatory.

`v10.13.0` bridges latch and permit telemetry only. `eoi-write-bridge-status` and `eoi-write-bridge-check` report `bridge ready: no`, `permit granted: no`, `first PIC_EOI write allowed: no`, `hardware mutation: no`, and `runtime irq active: no`. The bridge never sets or clears the latch.

`v10.12.1` is hardening-only. The allowed mutation remains limited to `EOI_WRITE_ONESHOT_LATCH_ARMED: AtomicBool`; arm is the only path that stores `true`, clear is the only path that stores `false`, and fire only reads the latch. The hardened sequence is: initial unarmed, unarmed fire blocked before any hardware write, arm to armed, armed fire blocked by the permit model, status remains armed, clear to unarmed, status remains unarmed.

`v10.12.0` permits software latch telemetry only. `eoi-write-oneshot-latch-arm` sets `one-shot armed: yes`, `eoi-write-oneshot-latch-clear` returns it to `no`, and blocked `eoi-write-oneshot-latch-fire` does not clear the latch. `first PIC_EOI write performed: no`, `hardware mutation: no`, and `runtime irq active: no` remain mandatory.

`v10.11.1` is not a latch or EOI write release. It hardens the one-shot command path only: `one-shot armed: no`, `fire allowed: no`, `first PIC_EOI write performed: no`, and no `PIC_EOI` write.

`v10.11.0` is not an EOI write release. It defines the one-shot command path only: `one-shot armed: no`, `fire allowed: no`, `first PIC_EOI write performed: no`, and no `PIC_EOI` write.

`v10.10.1` is not an EOI write release. It hardens permit telemetry only: `permit granted: no`, `first PIC_EOI write allowed: no`, and no `PIC_EOI` write.

`v10.9.1` is not an EOI write release. It hardens the existing candidate contract; `eoi-write-smoke-candidate-fire` is still dry-run blocked and does not write `PIC_EOI`.

`v10.9.0` is not an EOI write activation release. It adds candidate commands for the first-write decision point, but `eoi-write-smoke-candidate-fire` is still dry-run blocked and does not write `PIC_EOI`.

`v10.8.1` is not a PIC EOI write release. It hardens the existing `v10.8.0` first-write preflight contract without enabling EOI writes, PIC unmasking, STI, live IRQ0/IRQ1 binding, or keyboard IRQ mode.

`v10.8.0` is not a PIC EOI write release. It adds verification and command preflight around the first-write decision point without enabling EOI writes, PIC unmasking, STI, live IRQ0/IRQ1 binding, or keyboard IRQ mode.

`v10.7.1` is not a mutation release. It adds verification guards around the sequencer surface, exact command output, read-only helper/dispatcher isolation, and stale `v10.7.0` metadata without enabling EOI writes, PIC unmasking, STI, live IRQ0/IRQ1 binding, or keyboard IRQ mode.

This carries forward the IRQ Runtime Activation Preconditions 2 release contract as a stricter final gate.

This milestone still implements an EOI strategy foundation on top of the IRQ handler skeleton while keeping the IRQ gate plan and disabled bind path dormant and adding a preflight status surface. It adds no new runtime IRQ behavior, no active IDT bind path, and no dry-bind readiness path.

## PIC Remap Plan

The 8259A PIC pair routes hardware interrupt requests into CPU interrupt vectors. The planned remap moves IRQs away from CPU exception vectors and into `0x20-0x2f`.

| Controller | IRQ Lines  | Ports                        | Planned Vector Offset |
| ---------- | ---------- | ---------------------------- | --------------------- |
| Master PIC | IRQ0-IRQ7  | `0x20` command / `0x21` data | `0x20`                |
| Slave PIC  | IRQ8-IRQ15 | `0xA0` command / `0xA1` data | `0x28`                |

PIC remap dry-run telemetry remains available, and `v9.0.2` adds a separate controlled IDT gate bind smoke path for IRQ0/IRQ1. Initialization Command Words are dispatched only after `pic-remap-arm` followed by `pic-remap-smoke`; no boot path remaps the PIC, no EOI is sent, and no `sti` runs. IRQ gates 32/33 are installed only by `irq-gate-arm` followed by `irq-gate-bind-smoke`.

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

## Controlled Runtime EOI Dispatch Smoke Foundation

`v10.1.0` adds read-only EOI dispatch smoke commands that describe how runtime EOI acknowledgement would be wired after the existing activation smoke layer. This is controlled dry-run telemetry only: no `PIC_EOI` command is written to PIC ports, `sti` remains disabled, PIC IRQ lines remain masked, live IRQ0/IRQ1 handlers remain disabled, and keyboard input remains polling-only.

Commands:

```text
eoi-dispatch-smoke-note
eoi-dispatch-smoke-status
eoi-dispatch-smoke-plan
eoi-dispatch-smoke-blockers
```

Expected baseline output:

```text
EOI dispatch smoke status
eoi dispatch smoke: blocked
dispatch mode: dry-run
pic remap smoke: not ready
irq gates: not bound
pic eoi writes: disabled
sti instruction: disabled
pic unmask: disabled
keyboard mode: polling
runtime irq active: no
```

The smoke plan models IRQ0 and IRQ1 as master-PIC EOI routes only. Slave-PIC cascade routing remains documented for future IRQs and is not dispatched by this milestone.

## Controlled Runtime EOI Dispatch Smoke Hardening

`v10.1.1` hardens the controlled EOI dispatch smoke surface without adding runtime behavior. No output wording changes from v10.1.0 are introduced.

Verification now pins the four `eoi-dispatch-smoke-*` command templates, the rendered QEMU snapshots, the helper and command blocks as read-only surfaces, and the absence of actual master/slave command-port `PIC_EOI` writes. The existing runtime invariants remain locked: no `sti`, no PIC IRQ unmask, no live IRQ0/IRQ1 handlers, no keyboard IRQ path, no runtime IRQ active state, and keyboard input remains polling-only.

## Controlled PIC Mask Unmask Smoke Foundation

`v10.2.0` adds read-only PIC unmask smoke commands that describe how a future IRQ line unmask decision would be staged after PIC mask planning, readiness matrix, activation token/gate, EOI boundary, STI plan, and EOI dispatch smoke telemetry exist. This is controlled dry-run telemetry only: no PIC data-port unmask writes are emitted, target IRQ lines remain `none`, live unmask remains `no`, `sti` remains disabled, live IRQ0/IRQ1 handlers remain disabled, runtime EOI dispatch remains disabled, and runtime IRQ remains inactive.

Commands:

```text
pic-unmask-smoke-note
pic-unmask-smoke-status
pic-unmask-smoke-plan
pic-unmask-smoke-blockers
```

Expected baseline output:

```text
PIC unmask smoke status
pic unmask smoke: blocked
dispatch mode: dry-run
target IRQ lines: none
pic mask policy: all masked (0xFF)
activation token: absent
activation gate: activation blocked
EOI boundary: disabled
STI plan: blocked
EOI dispatch smoke: blocked
live unmask: no
hardware mutation: no
runtime irq active: no
```

## Controlled PIC Mask Unmask Smoke Hardening

`v10.2.1` hardens the controlled PIC unmask smoke surface without adding runtime behavior or changing output wording from v10.2.0.

Verification now pins the four `pic-unmask-smoke-*` command templates, rendered QEMU snapshots, helper and command blocks as read-only surfaces, and the absence of PIC data-port unmask writes through `write_pic_port(PIC_MASTER_DATA, ...)` or `write_pic_port(PIC_SLAVE_DATA, ...)`. The existing runtime invariants remain locked: no `sti`, no runtime EOI dispatch, no live IRQ0/IRQ1 handlers, no keyboard IRQ path, no runtime IRQ active state, and keyboard input remains polling-only.

## Controlled IDT Runtime Bind Smoke Foundation

`v10.3.0` adds read-only IDT runtime bind smoke commands that describe how future runtime vector/handler binding would be staged after activation token/gate, readiness matrix, controlled IRQ gate bind smoke state, EOI dispatch smoke, PIC unmask smoke, and STI plan telemetry exist. This is controlled dry-run telemetry only: no IDT handlers are bound, no `set_handler(` call is made by this surface, no `sti` runs, PIC IRQ lines remain masked, runtime EOI dispatch remains disabled, live IRQ0/IRQ1 handlers remain disabled, and runtime IRQ remains inactive.

Commands:

```text
idt-runtime-bind-smoke-note
idt-runtime-bind-smoke-status
idt-runtime-bind-smoke-plan
idt-runtime-bind-smoke-blockers
```

Expected baseline output:

```text
IDT runtime bind smoke status
idt runtime bind smoke: blocked
dispatch mode: dry-run
target vectors: 32/33 planned
irq gate bind smoke: not bound
EOI dispatch smoke: blocked
PIC unmask smoke: blocked
STI plan: blocked
live handler bind: no
hardware mutation: no
runtime irq active: no
```

## Controlled IDT Runtime Bind Smoke Hardening

`v10.3.1` hardens the controlled IDT runtime bind smoke surface without adding commands, changing output wording, or changing runtime behavior. The `idt-runtime-bind-smoke-*` commands remain a smoke plan and contract surface only; they do not bind runtime handlers, do not call `set_handler(`, do not enable `sti`, do not unmask PIC IRQ lines, do not dispatch runtime EOI, and do not activate live IRQ0/IRQ1 paths.

Verification now pins the four command templates, rendered QEMU snapshots, helper and command blocks as read-only surfaces, and the rule that `idt::IDT.entries[32].set_handler` / `idt::IDT.entries[33].set_handler` remain allowed only inside the older armed `irq-gate-bind-smoke` controlled smoke path. Keyboard input remains polling-only through PS/2 status/scancode reads.

## Controlled IRQ Runtime Readiness Final Gate

`v10.4.0` adds final gate release-proof commands that aggregate the existing read-only runtime readiness stack. `v10.4.1` hardens that surface without changing the rendered command output or runtime state. This is a foundation gate only: final activation remains disallowed, hardware mutation remains `no`, runtime IRQ remains inactive, `sti` remains disabled, PIC unmask remains disabled, EOI dispatch remains disabled, live IDT runtime binding remains `no`, and keyboard input remains polling-only.

Commands:

```text
irq-runtime-final-gate-note
irq-runtime-final-gate-status
irq-runtime-final-gate-check
irq-runtime-final-gate-blockers
```

Expected baseline output:

```text
IRQ runtime final gate status
activation token: absent
activation gate: activation blocked
readiness matrix: blocked
simulation: simulation blocked
STI plan: blocked
activation smoke: blocked
EOI dispatch smoke: blocked
PIC unmask smoke: blocked
IDT runtime bind smoke: blocked
keyboard mode: polling
final activation allowed: no
hardware mutation: no
runtime irq active: no
```

## Controlled Activation Decision Freeze

`v10.5.0` is a Controlled Activation Decision Freeze release. It adds a decision freeze layer above the final gate. `v10.5.1` hardens that surface without changing the rendered command output or runtime state. The decision is a read-only contract surface only: activation remains `frozen blocked`, final activation remains disallowed, hardware mutation remains `no`, runtime IRQ remains inactive, `sti` remains disabled, PIC unmask remains disabled, EOI dispatch remains disabled, live IDT runtime binding remains `no`, and keyboard input remains polling-only.

Commands:

```text
irq-runtime-decision-note
irq-runtime-decision-status
irq-runtime-decision-freeze
irq-runtime-decision-blockers
```

Expected baseline output:

```text
IRQ runtime activation decision
activation decision: frozen blocked
final activation allowed: no
runtime irq active: no
hardware mutation: no
sti: disabled
pic unmask: disabled
eoi dispatch: disabled
live idt bind: no
keyboard mode: polling
```

Expected blockers:

```text
IRQ runtime activation decision blockers
- STI instruction disabled
- PIC unmask disabled
- EOI dispatch disabled
- live IDT bind disabled
- keyboard IRQ path disabled
- runtime IRQ active state disabled
activation decision: frozen blocked
```

## Controlled Hardware Mutation Readiness Checklist

`v10.6.0` adds a read-only checklist above the frozen activation decision. It does not add live mutation smoke and does not change the decision output. Hardware mutation remains not ready, the activation decision remains `frozen blocked`, runtime IRQ remains inactive, and every mutation category remains disabled.

`v10.6.1` hardens the checklist without adding commands, changing output wording, or changing runtime behavior. Verification now pins the exact `irq-runtime-mutation-*` command templates, the helper-only dispatcher blocks, the read-only helper/snapshot/print surfaces, the stale `10.6.0` metadata guard, the forbidden positive mutation states, and the `256 KiB` bootstrap stack stability proof.

Commands:

```text
irq-runtime-mutation-note
irq-runtime-mutation-status
irq-runtime-mutation-check
irq-runtime-mutation-blockers
```

Expected baseline output:

```text
IRQ runtime hardware mutation readiness
hardware mutation ready: no
activation decision: frozen blocked
final activation allowed: no
runtime irq active: no
sti mutation: disabled
pic unmask mutation: disabled
eoi dispatch mutation: disabled
idt live bind mutation: disabled
keyboard irq mutation: disabled
```

Expected blockers:

```text
IRQ runtime hardware mutation blockers
- activation decision frozen blocked
- final activation disallowed
- runtime IRQ active state disabled
- STI mutation disabled
- PIC unmask mutation disabled
- EOI dispatch mutation disabled
- IDT live bind mutation disabled
- keyboard IRQ mutation disabled
hardware mutation ready: no
```

## Controlled Mutation Smoke Sequencer Foundation

`v10.7.0` adds a read-only mutation smoke sequencer above the hardware mutation checklist. It does not add live mutation smoke and does not change the checklist output. The sequence remains not ready, no next mutation step is selected, no mutation steps are allowed, hardware mutation remains `no`, runtime IRQ remains inactive, and keyboard input remains polling-only.

Commands:

```text
irq-runtime-mutation-sequence-note
irq-runtime-mutation-sequence-status
irq-runtime-mutation-sequence-plan
irq-runtime-mutation-sequence-blockers
```

Expected baseline output:

```text
IRQ runtime mutation smoke sequence
mutation sequence ready: no
hardware mutation: no
runtime irq active: no
next mutation step: none
allowed mutation steps: none
sti: disabled
pic unmask: disabled
eoi dispatch: disabled
live idt bind: no
keyboard mode: polling
```

Expected blockers:

```text
IRQ runtime mutation smoke sequence blockers
- activation decision frozen blocked
- final activation disallowed
- hardware mutation checklist not ready
- runtime IRQ active state disabled
- STI disabled
- PIC unmask disabled
- EOI dispatch disabled
- live IDT bind disabled
- keyboard mode polling
mutation sequence ready: no
```

## Controlled EOI Write Smoke Preflight

`v10.8.0` adds a read-only preflight before any first PIC EOI write candidate. It reads the mutation sequencer, mutation readiness checklist, decision freeze, final gate, EOI dispatch smoke boundary, PIC unmask smoke boundary, IDT runtime bind smoke boundary, STI plan, and keyboard fallback. It does not write PIC command ports and does not select a target IRQ line.

Commands:

```text
eoi-write-smoke-preflight-note
eoi-write-smoke-preflight-status
eoi-write-smoke-preflight-check
eoi-write-smoke-preflight-blockers
```

Expected baseline output:

```text
EOI write smoke preflight
eoi write smoke preflight: blocked
first PIC_EOI write allowed: no
hardware mutation: no
runtime irq active: no
target command port: none
target irq line: none
eoi dispatch: disabled
sti: disabled
pic unmask: disabled
live idt bind: no
keyboard mode: polling
```

Expected blockers:

```text
EOI write smoke preflight blockers
- mutation sequence ready: no
- hardware mutation checklist ready: no
- activation decision frozen blocked
- final activation disallowed
- EOI dispatch disabled
- PIC unmask disabled
- IDT live bind disabled
- STI disabled
- keyboard mode polling
first PIC_EOI write allowed: no
```

## First Controlled EOI Write Smoke Candidate

`v10.9.0` adds a candidate surface for the first controlled PIC EOI write. The surface is still read-only: `arm` only reports blocked candidate status, and `fire` only reports dry-run blocked. It does not write PIC command ports, select a target IRQ line, unmask PIC lines, enable `sti`, bind live IDT handlers, or switch keyboard input away from polling.

Commands:

```text
eoi-write-smoke-candidate-note
eoi-write-smoke-candidate-status
eoi-write-smoke-candidate-arm
eoi-write-smoke-candidate-fire
eoi-write-smoke-candidate-blockers
```

Expected baseline output:

```text
EOI write smoke candidate
eoi write smoke candidate: blocked
candidate armed: no
first PIC_EOI write performed: no
hardware mutation: no
runtime irq active: no
target command port: none
target irq line: none
eoi dispatch: disabled
sti: disabled
pic unmask: disabled
live idt bind: no
keyboard mode: polling
```

Expected fire output:

```text
EOI write smoke candidate fire
fire result: dry-run blocked
first PIC_EOI write performed: no
target command port: none
target irq line: none
hardware mutation: no
runtime irq active: no
```

Expected blockers:

```text
EOI write smoke candidate blockers
- eoi write preflight blocked
- first PIC_EOI write allowed: no
- mutation sequence ready: no
- hardware mutation checklist ready: no
- activation decision frozen blocked
- final activation disallowed
- EOI dispatch disabled
- PIC unmask disabled
- IDT live bind disabled
- STI disabled
- keyboard mode polling
first PIC_EOI write performed: no
```

## Controlled EOI Write Permit Model Foundation

`v10.10.0` adds a read-only permit model before any first controlled PIC EOI write. The permit remains denied and does not arm, fire, or write a PIC command port.

## Controlled EOI Write Permit Model Hardening

`v10.10.1` hardens the `v10.10.0` permit model contract without changing command output or enabling any hardware path. The permit helper and command dispatchers remain read-only: no `PIC_EOI` command is written, `sti` remains disabled, PIC IRQ lines remain masked, live IRQ runtime remains disabled, and keyboard input remains polling-only.

Commands:

```text
eoi-write-permit-note
eoi-write-permit-status
eoi-write-permit-check
eoi-write-permit-blockers
```

Expected status/check baseline:

```text
EOI write permit model
permit granted: no
first PIC_EOI write allowed: no
target command port: none
target value: none
target irq line: none
hardware mutation: no
runtime irq active: no
fire command: dry-run blocked
```

Expected blockers:

```text
EOI write permit blockers
- activation decision frozen blocked
- final gate denied
- mutation checklist denied
- mutation sequencer denied
- EOI write candidate fire blocked
- STI disabled
- PIC unmask disabled
- live IRQ runtime disabled
permit granted: no
```

## Controlled EOI Write One-Shot Command Path Foundation

`v10.11.0` adds the future one-shot command path for a first controlled PIC EOI write. The path is read-only in this release: `eoi-write-oneshot-arm` does not set a persistent latch, `eoi-write-oneshot-fire` is blocked by the permit model, and no PIC command port is written.

Commands:

```text
eoi-write-oneshot-note
eoi-write-oneshot-status
eoi-write-oneshot-arm
eoi-write-oneshot-fire
eoi-write-oneshot-blockers
```

Expected status/arm baseline:

```text
EOI write one-shot command path
one-shot armed: no
fire allowed: no
first PIC_EOI write performed: no
target command port: none
target value: none
hardware mutation: no
runtime irq active: no
```

Expected fire:

```text
EOI write one-shot fire
error: EOI one-shot fire blocked by permit model
first PIC_EOI write performed: no
hardware mutation: no
```

Expected blockers:

```text
EOI write one-shot blockers
- permit granted: no
- first PIC_EOI write allowed: no
- hardware mutation: no
- runtime irq active: no
- STI disabled
- PIC unmask disabled
- live IRQ runtime disabled
first PIC_EOI write performed: no
```

## Controlled EOI Write One-Shot Command Path Hardening

`v10.11.1` hardens the one-shot command path from `v10.11.0` without adding latch state or enabling fire. `eoi-write-oneshot-arm` remains read-only and reports the same denied status snapshot. `eoi-write-oneshot-fire` remains blocked by the permit model and performs no PIC command port write.

## Controlled EOI Write One-Shot Latch Foundation

`v10.12.0` adds a software-only latch layer for the controlled first EOI write path. The latch is telemetry state only: it may be armed or cleared from the shell, but it never grants permit, never dispatches `PIC_EOI`, never unmasks the PIC, never enables `sti`, never binds live IRQ0/IRQ1 handlers, and never changes keyboard IRQ mode.

Commands:

```txt
eoi-write-oneshot-latch-note
eoi-write-oneshot-latch-status
eoi-write-oneshot-latch-arm
eoi-write-oneshot-latch-clear
eoi-write-oneshot-latch-fire
eoi-write-oneshot-latch-blockers
```

Latch semantics:

- `eoi-write-oneshot-latch-arm` sets `one-shot armed: yes`.
- `eoi-write-oneshot-latch-clear` sets `one-shot armed: no`.
- `eoi-write-oneshot-latch-fire` reads the latch and remains blocked by the permit model.
- Blocked fire does not clear the latch.
- Clear is the only command that returns the latch to `no`.
- `first PIC_EOI write performed: no`, `hardware mutation: no`, and `runtime irq active: no` remain invariant.

## Controlled EOI Write One-Shot Latch Hardening

`v10.12.1` is a hardening-only release for the `v10.12.0` latch namespace. It adds no commands, preserves existing latch output, and extends guards so the software latch cannot be confused with hardware readiness.

Hardened latch state sequence:

```txt
initial one-shot armed: no
unarmed fire: blocked by latch state before hardware write
arm: one-shot armed: yes
armed fire: blocked by permit model
status after blocked fire: one-shot armed: yes
clear: one-shot armed: no
status after clear: one-shot armed: no
```

Hardening invariants:

```txt
AtomicBool owner: EOI_WRITE_ONESHOT_LATCH_ARMED
arm store path: only eoi_write_oneshot_latch_arm stores true
clear store path: only eoi_write_oneshot_latch_clear stores false
fire store path: none
first PIC_EOI write performed: no
hardware mutation: no
runtime irq active: no
keyboard mode: polling
```

## Controlled EOI Write One-Shot Permit Bridge Foundation

`v10.13.0` adds a read-only bridge between the `v10.10.0` permit model and the `v10.12.0` software latch. It derives readiness from those two telemetry surfaces only; it does not arm, clear, fire, write a PIC command port, unmask PIC lines, enable `sti`, bind live IDT handlers, or switch keyboard input away from polling.

Commands:

```txt
eoi-write-bridge-note
eoi-write-bridge-status
eoi-write-bridge-check
eoi-write-bridge-blockers
```

Bridge baseline:

```txt
bridge: read-only telemetry bridge
permit granted: no
one-shot armed: yes/no
bridge ready: no
first PIC_EOI write allowed: no
target command port: none
target value: none
hardware mutation: no
runtime irq active: no
```

Bridge blockers:

```txt
- latch not armed
- permit denied
- first PIC_EOI write allowed: no
- hardware mutation: no
- runtime irq active: no
- STI disabled
- PIC unmask disabled
- live IRQ runtime disabled
bridge ready: no
```

## Controlled EOI Write One-Shot Permit Bridge Hardening

`v10.13.1` preserves the `v10.13.0` command output and bridge behavior. The hardening layer verifies that the bridge reads the permit model before the latch status, calls the bridge derivation helper afterward, and never calls latch arm, latch clear, latch store, PIC EOI write, `sti`, PIC unmask, live IDT bind, or keyboard IRQ mode paths.

## Controlled EOI Write Permit Transition Model Foundation

`v10.14.0` adds a software-only transition state above the denied permit model. The transition can be armed and cleared in software, but it never turns `permit granted` to `yes` and never makes the bridge ready.

Commands:

```txt
eoi-write-permit-transition-note
eoi-write-permit-transition-status
eoi-write-permit-transition-arm
eoi-write-permit-transition-clear
eoi-write-permit-transition-check
eoi-write-permit-transition-blockers
```

Transition sequence:

```txt
initial: permit transition armed: no
arm: permit transition armed: yes
check: permit granted: no
check: bridge ready: no
check: first PIC_EOI write allowed: no
clear: permit transition armed: no
```

Hardening invariants:

```txt
transition state: software-only permit transition
permit granted: no
bridge ready: no
first PIC_EOI write allowed: no
hardware mutation: no
runtime irq active: no
keyboard mode: polling
```

## Controlled EOI Write Permit Transition Model Hardening

`v10.14.1` preserves the `v10.14.0` command output and transition behavior. The hardening layer verifies the transition state sequence, keeps `arm` as the only true store path, keeps `clear` as the only false store path, and proves status/check/blockers remain read-only.

Additional guards ensure transition code cannot mutate the one-shot latch, cannot mutate the underlying permit model, cannot produce positive permit or bridge readiness states, and cannot reach a PIC EOI hardware write path.

Hardened sequence:

```txt
initial: permit transition armed: no
initial: permit granted: no
check: transition check remains denied
arm: permit transition armed: yes
check: permit granted: no
check: bridge ready: no
status: permit transition armed: yes
clear: permit transition armed: no
status: permit transition armed: no
```

## Controlled EOI Write Permit Evaluation Hardening

`v10.15.1` adds no commands and keeps the evaluator read-only. It hardens the existing evaluator contract by verifying exact rendered output, read ordering, helper isolation, dispatcher isolation, denied readiness fields, and the absence of latch, transition, permit, bridge, PIC, STI, unmask, IRQ, IDT, or keyboard-mode mutation.

## Controlled PIC_EOI Runtime Bridge Readiness Foundation

`v10.17.1` keeps the bridge read-only and repairs its proof input. The bridge reports whether the current boot session has proven the manual smoke, then keeps handler-triggered EOI and runtime IRQ activation denied.

Commands:

```txt
eoi-runtime-bridge-note
eoi-runtime-bridge-status
eoi-runtime-bridge-check
eoi-runtime-bridge-blockers
```

Runtime bridge invariants:

```txt
manual PIC_EOI smoke proven: session-local yes/no
proof source: sticky boot-session flag
transient performed telemetry: may reset on clear
runtime bridge ready: no
handler-triggered EOI allowed: no
runtime irq active: no
sti: disabled
pic unmask: disabled
live irq handlers: no
keyboard mode: polling
PIC_EOI write callsites: exactly 1 manual-only
```

## Controlled IRQ Handler EOI Path Candidate Foundation

`v10.18.0` adds an unreachable candidate layer for a future handler-side EOI path. The candidate reads runtime bridge readiness but is not called from interrupt handlers, boot, live IDT bind, timer IRQ, or keyboard IRQ paths.

Commands:

```txt
irq-handler-eoi-candidate-note
irq-handler-eoi-candidate-status
irq-handler-eoi-candidate-check
irq-handler-eoi-candidate-blockers
```

Candidate invariants:

```txt
runtime bridge ready: no
handler EOI candidate ready: no
handler-triggered EOI allowed: no
live handler bind: no
PIC_EOI callsites: 1 manual-only
runtime irq active: no
sti: disabled
pic unmask: disabled
keyboard mode: polling
handler invocation: unreachable
```

## First Controlled PIC_EOI Write Smoke Foundation

`v10.16.0` adds a manual one-shot hardware smoke path for the first real PIC EOI write. It is not IRQ runtime activation and must never run from a handler, boot path, loop, timer IRQ, keyboard IRQ, or live IDT bind.

Commands:

```txt
eoi-write-hw-smoke-note
eoi-write-hw-smoke-status
eoi-write-hw-smoke-arm
eoi-write-hw-smoke-fire
eoi-write-hw-smoke-clear
eoi-write-hw-smoke-blockers
```

Hardware smoke invariants:

```txt
manual shell command path only
target command port: PIC_MASTER_COMMAND
target value: PIC_EOI
slave PIC command write: forbidden
PIC_EOI write callsites: exactly 1
successful fire consumes latch: yes
repeated fire without re-arm: blocked
sti: disabled
PIC unmask: disabled
runtime irq active: no
keyboard mode: polling
```

## Controlled EOI Write Permit Evaluation Foundation

`v10.15.0` adds a read-only evaluator above the transition model. The evaluator reads existing software telemetry and reports why the first PIC EOI write remains denied; it does not store latch state, store transition state, grant a permit, change bridge readiness, or touch hardware.

Commands:

```txt
eoi-write-eval-note
eoi-write-eval-status
eoi-write-eval-check
eoi-write-eval-blockers
```

Evaluation invariants:

```txt
evaluation ready: no
permit granted: no
bridge ready: no
first PIC_EOI write allowed: no
hardware mutation: no
runtime irq active: no
keyboard mode: polling
```

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

## v9.0.2 IRQ Gate Bind State Telemetry & Static Guards

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

## IRQ Runtime Activation Foundation

The 9.0.2 milestone introduces the safety latch foundation for runtime IRQ activation.
- irq-runtime-arm: Arms the activation sequence.
- irq-runtime-commit: Commits the sequence (currently a dry-run).
- irq-runtime-status: Reports the runtime irq activation telemetry (\rmed / standby\, \committed (dry-run)\, or \locked\).

## IRQ Glossary

- **ICW1 (`0x11`)**: planned initialization command.
- **ICW2 (`0x20` / `0x28`)**: planned master/slave remap offsets.
- **ICW3 (`0x04` / `0x02`)**: planned master/slave cascade wiring.
- **ICW4 (`0x01`)**: planned 8086 mode.
- **IRQ0 timer**: skeleton planned PIT timer interrupt; bind smoke stub is dormant in `v9.0.2`.
- **IRQ1 keyboard**: skeleton planned PS/2 keyboard interrupt; bind smoke stub is dormant in `v9.0.2`.
- **IRQ vectors 32-47**: planned remapped CPU vector range for IRQ0-IRQ15.
- **EOI**: End Of Interrupt command planned for future PIC acknowledgements.
- **STI**: Set Interrupt Flag instruction; not used in `v9.0.2`.

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
