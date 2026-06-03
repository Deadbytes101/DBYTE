#![allow(dead_code)]

//! IRQ handler skeleton foundation.
//!
//! This module documents the first hardware IRQ handler shapes without making
//! them hardware-active. It contains no PIC EOI writes and no port I/O. The
//! symbols are compiled so verification can guard the intended IRQ0/IRQ1
//! contract before runtime IRQ activation exists.

use core::sync::atomic::{AtomicBool, Ordering};

/// Planned CPU vector for IRQ0 after the future PIC remap.
pub const IRQ0_VECTOR: u8 = 32;

/// Planned CPU vector for IRQ1 after the future PIC remap.
pub const IRQ1_VECTOR: u8 = 33;

/// Planned display name for IRQ0.
pub const IRQ0_NAME: &str = "timer";

/// Planned display name for IRQ1.
pub const IRQ1_NAME: &str = "keyboard";

/// Dormant gate state shared by planned IRQ gate descriptors.
pub const IRQ_GATE_STATE_DORMANT: &str = "dormant / disabled";

/// Disabled IDT binding state for planned IRQ gates.
pub const IRQ_IDT_BINDING_DISABLED: &str = "disabled";

/// Disabled PIC remap state for planned IRQ gates.
pub const IRQ_PIC_REMAP_DISABLED: &str = "disabled";

/// Controlled smoke-only PIC remap state for readiness telemetry.
pub const IRQ_PIC_REMAP_CONTROLLED_SMOKE_ONLY: &str = "controlled smoke only";

/// Disabled EOI dispatch state for planned IRQ gates.
pub const IRQ_EOI_DISPATCH_DISABLED: &str = "disabled";

/// Disabled maskable interrupt state for planned IRQ gates.
pub const IRQ_INTERRUPTS_DISABLED: &str = "disabled";

/// Disabled IDT install state for the future IRQ gate bind path.
pub const IRQ_IDT_INSTALL_DISABLED: &str = "planned / not installed";

/// Disabled bind helper symbol name exposed for telemetry.
pub const IRQ_BIND_DISABLED_HELPER: &str = "bind_irq_gates_disabled";

/// Boot call state for the disabled bind helper.
pub const IRQ_BIND_BOOT_CALL_DISABLED: &str = "no";

/// Runtime IDT vector state for planned IRQ gates.
pub const IRQ_VECTOR_UNBOUND: &str = "unbound";

/// Runtime handler state for planned IRQ gates.
pub const IRQ_ACTIVE_HANDLER_NONE: &str = "none";

/// Keyboard input mode while IRQ1 remains disabled.
pub const IRQ_KEYBOARD_INPUT_POLLING_ONLY: &str = "polling-only";

/// Disabled bind path note for each planned IRQ gate.
pub const IRQ_BIND_PATH_DISABLED_ONLY: &str = "disabled bind path only";

/// Positive readiness state for pre-runtime checks.
pub const IRQ_READINESS_OK: &str = "ok";

/// Runtime IRQ readiness result while IRQ activation remains blocked.
pub const IRQ_RUNTIME_READY_NO: &str = "no";

/// Runtime IRQ blocker state.
pub const IRQ_RUNTIME_BLOCKED: &str = "blocked";

/// Runtime IRQ risk reason while IRQ gates remain unbound.
pub const IRQ_RUNTIME_RISK_REASON: &str = "IRQ0/IRQ1 gates are not bound";

/// Runtime IRQ prerequisites before interrupts may be enabled.
pub const IRQ_RUNTIME_REQUIRED_BEFORE_ENABLE: &str =
    "IDT gate bind, PIC remap, EOI dispatch, handler stubs";

/// STI policy for the current readiness milestone.
pub const IRQ_STI_ALLOWED_NO: &str = "no";

/// Stable preflight pass state.
pub const IRQ_PREFLIGHT_PASS: &str = "pass";

/// Page Fault smoke readiness state.
pub const IRQ_PF_SMOKE_UNCHANGED: &str = "unchanged";

/// One-shot guard state string for armed IRQ gate bind smoke.
pub const IRQ_GATE_BIND_SMOKE_GUARD_ARMED: &str = "armed";

/// One-shot guard state string for blocked IRQ gate bind smoke.
pub const IRQ_GATE_BIND_SMOKE_GUARD_NOT_ARMED: &str = "not armed";

/// Controlled bind smoke mode string.
pub const IRQ_GATE_BIND_SMOKE_MODE_CONTROLLED: &str = "controlled bind smoke";

/// Next command after arming the controlled gate bind smoke.
pub const IRQ_GATE_BIND_SMOKE_NEXT: &str = "irq-gate-bind-smoke";

/// Command to arm the controlled gate bind smoke.
pub const IRQ_GATE_BIND_ARM_NEXT: &str = "irq-gate-arm";

/// PIC mask state while IRQ gates are bound only for smoke.
pub const IRQ_PIC_IRQ_MASK_MASKED: &str = "masked";

/// Bound vector state for controlled IRQ gate smoke.
pub const IRQ_VECTOR_BOUND: &str = "bound";

/// Dormant smoke-stub handler state for controlled IRQ gate smoke.
pub const IRQ_SMOKE_STUB_DORMANT: &str = "smoke stub / dormant";

/// Rendered IRQ0 smoke-stub binding target.
pub const IRQ0_SMOKE_STUB_BINDING: &str = "bound to IRQ0 timer smoke stub";

/// Rendered IRQ1 smoke-stub binding target.
pub const IRQ1_SMOKE_STUB_BINDING: &str = "bound to IRQ1 keyboard smoke stub";

/// Blocked result for unarmed IRQ gate bind smoke.
pub const IRQ_GATE_BIND_RESULT_BLOCKED: &str = "blocked";

/// Dormant result for a successful controlled IRQ gate bind smoke.
pub const IRQ_GATE_BIND_RESULT_BOUND_DORMANT: &str = "bound / dormant";

/// Yes/no telemetry strings for IRQ gate bind state.
pub const IRQ_GATE_BIND_YES: &str = "yes";
pub const IRQ_GATE_BIND_NO: &str = "no";

/// Command availability strings for IRQ gate bind history.
pub const IRQ_GATE_BIND_ARM_COMMAND_AVAILABLE: &str = "available";
pub const IRQ_GATE_BIND_SMOKE_COMMAND_AVAILABLE: &str = "available";

/// Controlled IDT bind path note for history telemetry.
pub const IRQ_GATE_BIND_IDT_BINDS_CONTROLLED_ONLY: &str = "controlled command path only";

/// Boot-time IRQ gate bind state for history telemetry.
pub const IRQ_GATE_BIND_BOOT_BIND_NO: &str = "no";

/// Preflight guard string for IRQ gate bind telemetry.
pub const IRQ_GATE_BIND_GUARD_COMMAND_ARMED_REQUIRED: &str = "command armed required";

/// Preflight bind path readiness string.
pub const IRQ_GATE_BIND_BIND_PATH_READY: &str = "ready";

/// IRQ runtime remains disabled after controlled gate bind smoke.
pub const IRQ_GATE_BIND_IRQ_RUNTIME_DISABLED: &str = "disabled";

/// Preflight result for read-only IRQ gate bind telemetry.
pub const IRQ_GATE_BIND_RESULT_TELEMETRY_ONLY: &str = "telemetry only";

/// Bind expected/applied telemetry strings.
pub const IRQ_GATE_BIND_EXPECTED: &str = "yes";

/// Precondition satisfied state string.
pub const IRQ_RUNTIME_PRECONDITION_SATISFIED: &str = "satisfied";

/// Precondition unsatisfied state string.
pub const IRQ_RUNTIME_PRECONDITION_UNSATISFIED: &str = "unsatisfied";

/// PIC remap precondition blocker message.
pub const IRQ_RUNTIME_BLOCKER_PIC_REMAP: &str =
    "PIC remap: not ready for controlled smoke (run: pic-remap-arm, pic-remap-smoke)";

/// IRQ gates precondition blocker message.
pub const IRQ_RUNTIME_BLOCKER_IRQ_GATES: &str =
    "IRQ gates: vectors 32/33 not bound (run: irq-gate-arm, irq-gate-bind-smoke)";

/// EOI dispatch precondition blocker message.
pub const IRQ_RUNTIME_BLOCKER_EOI_DISPATCH: &str = "EOI dispatch: not enabled";

/// STI precondition blocker message.
pub const IRQ_RUNTIME_BLOCKER_STI: &str = "STI: disabled";

/// Keyboard fallback precondition blocker message (but always satisfied in v9.1.0).
pub const IRQ_RUNTIME_PRECONDITION_KEYBOARD_FALLBACK: &str = "keyboard fallback: polling-only (ok)";

/// Page fault smoke precondition blocker message (but always satisfied in v9.1.0).
pub const IRQ_RUNTIME_PRECONDITION_PF_SMOKE: &str = "pf-smoke: stable (ok)";

/// EOI precondition blocker messages (v9.2.0).
pub const EOI_RUNTIME_BLOCKER_PIC_REMAP: &str =
    "PIC remap: not ready for EOI dispatch (run: pic-remap-arm, pic-remap-smoke)";
pub const EOI_RUNTIME_BLOCKER_IRQ_GATES: &str =
    "IRQ gates: vectors 32/33 not bound for EOI (run: irq-gate-arm, irq-gate-bind-smoke)";
pub const EOI_RUNTIME_BLOCKER_EDGE_LEVEL: &str = "IRQ edge/level: detection strategy not planned";
pub const EOI_RUNTIME_BLOCKER_KEYBOARD: &str = "Keyboard fallback: state unknown";
pub const EOI_RUNTIME_BLOCKER_STI: &str = "STI: not enabled for EOI dispatch";

/// IRQ mask blocker messages (v9.3.0).
pub const IRQ_MASK_BLOCKER_PIC_REMAP: &str = "[BLOCKER] pic remap not executed";
pub const IRQ_MASK_BLOCKER_IRQ_GATES: &str =
    "[BLOCKER] irq gates not bound (vectors 32/33 unbound)";
pub const IRQ_MASK_BLOCKER_STI: &str = "[BLOCKER] sti not enabled";
pub const IRQ_MASK_BLOCKER_EOI_DISPATCH: &str = "[BLOCKER] eoi dispatch not active";
pub const IRQ_MASK_BLOCKER_IRQ_RUNTIME: &str = "[BLOCKER] irq runtime not committed";

/// Runtime readiness matrix constants (v9.4.0).
pub const IRQ_MATRIX_YES: &str = "yes";
pub const IRQ_MATRIX_NO: &str = "no";
pub const IRQ_MATRIX_EOI_READY_DRY_RUN: &str = "ready (dry-run)";
pub const IRQ_MATRIX_EOI_DISABLED: &str = "disabled";
pub const IRQ_MATRIX_UNMASK_POLICY_NO_UNMASK: &str = "no unmask";
pub const IRQ_MATRIX_RUNTIME_LATCH_BLOCKED: &str = "blocked";
pub const IRQ_MATRIX_RUNTIME_LATCH_ARMED: &str = "armed";
pub const IRQ_MATRIX_RUNTIME_LATCH_COMMITTED_DRY_RUN: &str = "committed dry-run";
pub const IRQ_MATRIX_KEYBOARD_MODE_POLLING: &str = "polling";
pub const IRQ_MATRIX_STI_DISABLED: &str = "disabled";
pub const IRQ_MATRIX_RUNTIME_IRQ_ACTIVE_NO: &str = "no";
pub const IRQ_ACTIVATION_DRY_RUN_ALLOWED_NO: &str = "no";
pub const IRQ_ACTIVATION_COMMIT_RESULT_BLOCKED: &str = "blocked by readiness matrix";
pub const IRQ_ACTIVATION_PLAN_NEXT: &str = "execute irq-runtime-activation-plan";
pub const IRQ_ACTIVATION_TOKEN_ABSENT: &str = "absent";
pub const IRQ_ACTIVATION_TOKEN_PRESENT: &str = "present";
pub const IRQ_ACTIVATION_TOKEN_SCOPE: &str = "activation telemetry only";
pub const IRQ_ACTIVATION_TOKEN_HARDWARE_MUTATION_NO: &str = "no";
pub const IRQ_ACTIVATION_TOKEN_PIC_UNMASK_NO: &str = "no";
pub const IRQ_ACTIVATION_TOKEN_LIVE_IRQ_NO: &str = "no";
pub const IRQ_ACTIVATION_GATE_PURPOSE: &str = "controlled activation preconditions";
pub const IRQ_ACTIVATION_GATE_REQUIRED_YES: &str = "yes";
pub const IRQ_ACTIVATION_GATE_MATRIX_REQUIRED_READY: &str = "ready";
pub const IRQ_ACTIVATION_GATE_READINESS_BLOCKED: &str = "blocked";
pub const IRQ_ACTIVATION_GATE_ALLOWED_NO: &str = "no";
pub const IRQ_ACTIVATION_GATE_RESULT_BLOCKED: &str = "activation blocked";
pub const IRQ_ACTIVATION_GATE_NEXT_BLOCKERS: &str = "execute irq-runtime-gate-blockers";
pub const IRQ_ACTIVATION_GATE_DRY_RUN_NOT_ALLOWED: &str = "not allowed";
pub const IRQ_ACTIVATION_GATE_RUNTIME_READY_NO: &str = "runtime irq ready no";
pub const IRQ_ACTIVATION_SIM_PURPOSE: &str = "controlled activation rehearsal";
pub const IRQ_ACTIVATION_SIM_ALLOWED_NO: &str = "no";
pub const IRQ_ACTIVATION_SIM_STI_WOULD_ENABLE_NO: &str = "no";
pub const IRQ_ACTIVATION_SIM_PIC_UNMASK_WOULD_APPLY_NO: &str = "no";
pub const IRQ_ACTIVATION_SIM_EOI_DISPATCH_WOULD_ENABLE_NO: &str = "no";
pub const IRQ_ACTIVATION_SIM_RESULT_BLOCKED: &str = "simulation blocked";
pub const IRQ_ACTIVATION_SIM_NEXT_BLOCKERS: &str = "execute irq-runtime-sim-blockers";
pub const STI_PLAN_TOKEN_REQUIRED: &str = "required";
pub const STI_PLAN_RUNTIME_GATE_NOT_ALLOWED: &str = "not allowed";
pub const STI_PLAN_PIC_UNMASK_DISABLED: &str = "disabled";
pub const STI_PLAN_EOI_DISPATCH_DISABLED: &str = "disabled";
pub const STI_PLAN_ALLOWED_NO: &str = "no";
pub const STI_PLAN_RESULT_BLOCKED: &str = "blocked";
pub const STI_PLAN_NEXT_BLOCKERS: &str = "execute sti-blockers";
pub const IRQ_ACTIVATION_SMOKE_BLOCKED: &str = "blocked";
pub const IRQ_ACTIVATION_SMOKE_RESULT_BLOCKED: &str = "smoke blocked";
pub const IRQ_ACTIVATION_SMOKE_NEXT_BLOCKERS: &str =
    "execute irq-runtime-activation-smoke-blockers";
pub const EOI_DISPATCH_SMOKE_BLOCKED: &str = "blocked";
pub const EOI_DISPATCH_SMOKE_MODE_DRY_RUN: &str = "dry-run";
pub const EOI_DISPATCH_SMOKE_ACK_WRITES_DISABLED: &str = "disabled";
pub const EOI_DISPATCH_SMOKE_RESULT_DRY_RUN_ONLY: &str = "dry-run only";
pub const EOI_DISPATCH_SMOKE_MASTER_ROUTE: &str = "command 0x20 -> port 0x20 (planned)";
pub const EOI_DISPATCH_SMOKE_SLAVE_ROUTE: &str = "command 0x20 -> port 0xA0 then 0x20 (planned)";
pub const EOI_DISPATCH_SMOKE_BLOCKER_PIC_REMAP: &str =
    "PIC remap smoke: not ready for controlled smoke";
pub const EOI_DISPATCH_SMOKE_BLOCKER_IRQ_GATES: &str = "IRQ gates: vectors 32/33 not bound";
pub const EOI_DISPATCH_SMOKE_BLOCKER_ACK_WRITES: &str = "PIC EOI writes: disabled by guard";
pub const EOI_DISPATCH_SMOKE_BLOCKER_STI: &str = "STI: disabled";
pub const EOI_DISPATCH_SMOKE_BLOCKER_PIC_UNMASK: &str = "PIC unmask: disabled";
pub const EOI_DISPATCH_SMOKE_BLOCKER_LIVE_IRQ: &str = "live IRQ0/IRQ1: disabled";
pub const EOI_DISPATCH_SMOKE_BLOCKER_KEYBOARD_IRQ: &str = "keyboard IRQ path: disabled";
pub const PIC_UNMASK_SMOKE_BLOCKED: &str = "blocked";
pub const PIC_UNMASK_SMOKE_MODE_DRY_RUN: &str = "dry-run";
pub const PIC_UNMASK_SMOKE_TARGET_IRQ_LINES_NONE: &str = "none";
pub const PIC_UNMASK_SMOKE_LIVE_UNMASK_NO: &str = "no";
pub const PIC_UNMASK_SMOKE_DATA_WRITES_DISABLED: &str = "disabled";
pub const PIC_UNMASK_SMOKE_RESULT_DRY_RUN_ONLY: &str = "dry-run only";
pub const PIC_UNMASK_SMOKE_BLOCKER_KEYBOARD_IRQ: &str = "keyboard IRQ path: disabled";
pub const IDT_RUNTIME_BIND_SMOKE_BLOCKED: &str = "blocked";
pub const IDT_RUNTIME_BIND_SMOKE_MODE_DRY_RUN: &str = "dry-run";
pub const IDT_RUNTIME_BIND_SMOKE_TARGET_VECTORS: &str = "32/33 planned";
pub const IDT_RUNTIME_BIND_SMOKE_LIVE_HANDLER_BIND_NO: &str = "no";
pub const IDT_RUNTIME_BIND_SMOKE_RESULT_DRY_RUN_ONLY: &str = "dry-run only";
pub const IRQ_RUNTIME_FINAL_GATE_SCOPE: &str = "controlled read-only release proof";
pub const IRQ_RUNTIME_FINAL_GATE_INPUTS: &str =
    "token/gate/matrix/simulation/sti/activation-smoke/eoi/pic-unmask/idt-bind";
pub const IRQ_RUNTIME_FINAL_GATE_ALLOWED_NO: &str = "no";
pub const IRQ_RUNTIME_FINAL_GATE_LIVE_IDT_BIND_NO: &str = "no";
pub const IRQ_RUNTIME_FINAL_GATE_RESULT_BLOCKED: &str = "release proof blocked";
pub const IRQ_RUNTIME_FINAL_GATE_NEXT_NONE: &str = "none";
pub const IRQ_RUNTIME_DECISION_SCOPE: &str = "controlled activation decision freeze";
pub const IRQ_RUNTIME_DECISION_INPUTS: &str =
    "final-gate/activation-smoke/simulation/sti/eoi/pic-unmask/idt-bind/token/gate/matrix/keyboard";
pub const IRQ_RUNTIME_DECISION_FROZEN_BLOCKED: &str = "frozen blocked";
pub const IRQ_RUNTIME_DECISION_BLOCKER_STI: &str = "STI instruction disabled";
pub const IRQ_RUNTIME_DECISION_BLOCKER_PIC_UNMASK: &str = "PIC unmask disabled";
pub const IRQ_RUNTIME_DECISION_BLOCKER_EOI_DISPATCH: &str = "EOI dispatch disabled";
pub const IRQ_RUNTIME_DECISION_BLOCKER_LIVE_IDT_BIND: &str = "live IDT bind disabled";
pub const IRQ_RUNTIME_DECISION_BLOCKER_KEYBOARD_IRQ: &str = "keyboard IRQ path disabled";
pub const IRQ_RUNTIME_DECISION_BLOCKER_RUNTIME_IRQ_ACTIVE: &str =
    "runtime IRQ active state disabled";
pub const IRQ_RUNTIME_MUTATION_SCOPE: &str = "controlled hardware mutation readiness checklist";
pub const IRQ_RUNTIME_MUTATION_INPUTS: &str =
    "decision/final-gate/activation-smoke/sti/eoi/pic-unmask/idt-bind/token/gate/matrix/keyboard";
pub const IRQ_RUNTIME_MUTATION_READY_NO: &str = "no";
pub const IRQ_RUNTIME_MUTATION_DISABLED: &str = "disabled";
pub const IRQ_RUNTIME_MUTATION_BLOCKER_DECISION: &str = "activation decision frozen blocked";
pub const IRQ_RUNTIME_MUTATION_BLOCKER_FINAL: &str = "final activation disallowed";
pub const IRQ_RUNTIME_MUTATION_BLOCKER_RUNTIME_IRQ: &str = "runtime IRQ active state disabled";
pub const IRQ_RUNTIME_MUTATION_BLOCKER_STI: &str = "STI mutation disabled";
pub const IRQ_RUNTIME_MUTATION_BLOCKER_PIC_UNMASK: &str = "PIC unmask mutation disabled";
pub const IRQ_RUNTIME_MUTATION_BLOCKER_EOI_DISPATCH: &str = "EOI dispatch mutation disabled";
pub const IRQ_RUNTIME_MUTATION_BLOCKER_IDT_LIVE_BIND: &str = "IDT live bind mutation disabled";
pub const IRQ_RUNTIME_MUTATION_BLOCKER_KEYBOARD_IRQ: &str = "keyboard IRQ mutation disabled";
pub const IRQ_RUNTIME_MUTATION_SEQUENCE_SCOPE: &str = "controlled mutation smoke sequencer";
pub const IRQ_RUNTIME_MUTATION_SEQUENCE_INPUTS: &str =
    "mutation-checklist/decision/final-gate/activation-smoke/sti/eoi/pic-unmask/idt-bind/token/gate/matrix/keyboard";
pub const IRQ_RUNTIME_MUTATION_SEQUENCE_READY_NO: &str = "no";
pub const IRQ_RUNTIME_MUTATION_SEQUENCE_NEXT_NONE: &str = "none";
pub const IRQ_RUNTIME_MUTATION_SEQUENCE_ALLOWED_NONE: &str = "none";
pub const IRQ_RUNTIME_MUTATION_SEQUENCE_DISABLED: &str = "disabled";
pub const IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_DECISION: &str =
    "activation decision frozen blocked";
pub const IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_FINAL: &str = "final activation disallowed";
pub const IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_MUTATION: &str =
    "hardware mutation checklist not ready";
pub const IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_RUNTIME_IRQ: &str =
    "runtime IRQ active state disabled";
pub const IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_STI: &str = "STI disabled";
pub const IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_PIC_UNMASK: &str = "PIC unmask disabled";
pub const IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_EOI_DISPATCH: &str = "EOI dispatch disabled";
pub const IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_LIVE_IDT_BIND: &str = "live IDT bind disabled";
pub const IRQ_RUNTIME_MUTATION_SEQUENCE_BLOCKER_KEYBOARD: &str = "keyboard mode polling";
pub const EOI_WRITE_SMOKE_PREFLIGHT_SCOPE: &str = "controlled first PIC_EOI write preflight";
pub const EOI_WRITE_SMOKE_PREFLIGHT_INPUTS: &str =
    "mutation-sequence/mutation-checklist/decision/final-gate/eoi-dispatch/pic-unmask/idt-bind/sti/keyboard";
pub const EOI_WRITE_SMOKE_PREFLIGHT_BLOCKED: &str = "blocked";
pub const EOI_WRITE_SMOKE_PREFLIGHT_FIRST_WRITE_ALLOWED_NO: &str = "no";
pub const EOI_WRITE_SMOKE_PREFLIGHT_TARGET_NONE: &str = "none";
pub const EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_SEQUENCE: &str = "mutation sequence ready: no";
pub const EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_MUTATION: &str =
    "hardware mutation checklist ready: no";
pub const EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_DECISION: &str = "activation decision frozen blocked";
pub const EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_FINAL: &str = "final activation disallowed";
pub const EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_EOI: &str = "EOI dispatch disabled";
pub const EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_PIC_UNMASK: &str = "PIC unmask disabled";
pub const EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_IDT: &str = "IDT live bind disabled";
pub const EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_STI: &str = "STI disabled";
pub const EOI_WRITE_SMOKE_PREFLIGHT_BLOCKER_KEYBOARD: &str = "keyboard mode polling";
pub const EOI_WRITE_SMOKE_CANDIDATE_SCOPE: &str = "first controlled PIC_EOI write smoke candidate";
pub const EOI_WRITE_SMOKE_CANDIDATE_INPUTS: &str =
    "eoi-write-preflight/mutation-sequence/mutation-checklist/decision/final-gate/eoi-dispatch/pic-unmask/idt-bind/sti/keyboard";
pub const EOI_WRITE_SMOKE_CANDIDATE_BLOCKED: &str = "blocked";
pub const EOI_WRITE_SMOKE_CANDIDATE_ARMED_NO: &str = "no";
pub const EOI_WRITE_SMOKE_CANDIDATE_FIRE_DRY_RUN_BLOCKED: &str = "dry-run blocked";
pub const EOI_WRITE_SMOKE_CANDIDATE_WRITE_PERFORMED_NO: &str = "no";
pub const EOI_WRITE_SMOKE_CANDIDATE_TARGET_NONE: &str = "none";
pub const EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_PREFLIGHT: &str = "eoi write preflight blocked";
pub const EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_FIRST_ALLOWED: &str = "first PIC_EOI write allowed: no";
pub const EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_SEQUENCE: &str = "mutation sequence ready: no";
pub const EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_MUTATION: &str =
    "hardware mutation checklist ready: no";
pub const EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_DECISION: &str = "activation decision frozen blocked";
pub const EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_FINAL: &str = "final activation disallowed";
pub const EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_EOI: &str = "EOI dispatch disabled";
pub const EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_PIC_UNMASK: &str = "PIC unmask disabled";
pub const EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_IDT: &str = "IDT live bind disabled";
pub const EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_STI: &str = "STI disabled";
pub const EOI_WRITE_SMOKE_CANDIDATE_BLOCKER_KEYBOARD: &str = "keyboard mode polling";
pub const EOI_WRITE_PERMIT_SCOPE: &str = "controlled first PIC_EOI write permit model";
pub const EOI_WRITE_PERMIT_INPUTS: &str =
    "candidate/preflight/mutation-sequence/mutation-checklist/decision/final-gate/eoi-dispatch/pic-unmask/idt-bind/sti/keyboard";
pub const EOI_WRITE_PERMIT_GRANTED_NO: &str = "no";
pub const EOI_WRITE_PERMIT_FIRST_WRITE_ALLOWED_NO: &str = "no";
pub const EOI_WRITE_PERMIT_TARGET_NONE: &str = "none";
pub const EOI_WRITE_PERMIT_FIRE_DRY_RUN_BLOCKED: &str = "dry-run blocked";
pub const EOI_WRITE_PERMIT_BLOCKER_DECISION: &str = "activation decision frozen blocked";
pub const EOI_WRITE_PERMIT_BLOCKER_FINAL_GATE: &str = "final gate denied";
pub const EOI_WRITE_PERMIT_BLOCKER_MUTATION: &str = "mutation checklist denied";
pub const EOI_WRITE_PERMIT_BLOCKER_SEQUENCE: &str = "mutation sequencer denied";
pub const EOI_WRITE_PERMIT_BLOCKER_CANDIDATE_FIRE: &str = "EOI write candidate fire blocked";
pub const EOI_WRITE_PERMIT_BLOCKER_STI: &str = "STI disabled";
pub const EOI_WRITE_PERMIT_BLOCKER_PIC_UNMASK: &str = "PIC unmask disabled";
pub const EOI_WRITE_PERMIT_BLOCKER_LIVE_IRQ: &str = "live IRQ runtime disabled";
pub const EOI_WRITE_ONESHOT_SCOPE: &str = "controlled first PIC_EOI write one-shot command path";
pub const EOI_WRITE_ONESHOT_INPUTS: &str = "permit-model/candidate/preflight/mutation-sequence/mutation-checklist/decision/final-gate/eoi-dispatch/pic-unmask/idt-bind/sti/keyboard";
pub const EOI_WRITE_ONESHOT_ARMED_NO: &str = "no";
pub const EOI_WRITE_ONESHOT_FIRE_ALLOWED_NO: &str = "no";
pub const EOI_WRITE_ONESHOT_WRITE_PERFORMED_NO: &str = "no";
pub const EOI_WRITE_ONESHOT_TARGET_NONE: &str = "none";
pub const EOI_WRITE_ONESHOT_FIRE_BLOCKED_BY_PERMIT: &str =
    "error: EOI one-shot fire blocked by permit model";
pub const EOI_WRITE_ONESHOT_BLOCKER_PERMIT: &str = "permit granted: no";
pub const EOI_WRITE_ONESHOT_BLOCKER_FIRST_ALLOWED: &str = "first PIC_EOI write allowed: no";
pub const EOI_WRITE_ONESHOT_BLOCKER_HARDWARE: &str = "hardware mutation: no";
pub const EOI_WRITE_ONESHOT_BLOCKER_RUNTIME: &str = "runtime irq active: no";
pub const EOI_WRITE_ONESHOT_BLOCKER_STI: &str = "STI disabled";
pub const EOI_WRITE_ONESHOT_BLOCKER_PIC_UNMASK: &str = "PIC unmask disabled";
pub const EOI_WRITE_ONESHOT_BLOCKER_LIVE_IRQ: &str = "live IRQ runtime disabled";
pub const EOI_WRITE_ONESHOT_LATCH_SCOPE: &str =
    "controlled first PIC_EOI write one-shot software latch";
pub const EOI_WRITE_ONESHOT_LATCH_INPUTS: &str =
    "software-latch/permit-model/candidate/preflight/mutation-sequence/mutation-checklist/decision/final-gate";
pub const EOI_WRITE_ONESHOT_LATCH_TELEMETRY_ONLY: &str = "software telemetry only";
pub const EOI_WRITE_ONESHOT_LATCH_ARMED_YES: &str = "yes";
pub const EOI_WRITE_ONESHOT_LATCH_ARMED_NO: &str = "no";
pub const EOI_WRITE_ONESHOT_LATCH_FIRE_ALLOWED_NO: &str = "no";
pub const EOI_WRITE_ONESHOT_LATCH_WRITE_PERFORMED_NO: &str = "no";
pub const EOI_WRITE_ONESHOT_LATCH_TARGET_NONE: &str = "none";
pub const EOI_WRITE_ONESHOT_LATCH_FIRE_BLOCKED_BY_PERMIT: &str =
    "error: EOI one-shot latch fire blocked by permit model";
pub const EOI_WRITE_ONESHOT_LATCH_FIRE_CLEARED_NO: &str = "no";
pub const EOI_WRITE_ONESHOT_LATCH_CLEAR_RESULT: &str = "software latch cleared";
pub const EOI_WRITE_ONESHOT_LATCH_ARM_RESULT: &str = "software latch armed";
pub const EOI_WRITE_ONESHOT_LATCH_BLOCKER_SOFTWARE_ONLY: &str =
    "latch scope: software telemetry only";
pub const EOI_WRITE_ONESHOT_LATCH_BLOCKER_PERMIT: &str = "permit granted: no";
pub const EOI_WRITE_ONESHOT_LATCH_BLOCKER_FIRST_ALLOWED: &str =
    "first PIC_EOI write allowed: no";
pub const EOI_WRITE_ONESHOT_LATCH_BLOCKER_HARDWARE: &str = "hardware mutation: no";
pub const EOI_WRITE_ONESHOT_LATCH_BLOCKER_RUNTIME: &str = "runtime irq active: no";
pub const EOI_WRITE_ONESHOT_LATCH_BLOCKER_STI: &str = "STI disabled";
pub const EOI_WRITE_ONESHOT_LATCH_BLOCKER_PIC_UNMASK: &str = "PIC unmask disabled";
pub const EOI_WRITE_ONESHOT_LATCH_BLOCKER_LIVE_IRQ: &str = "live IRQ runtime disabled";
pub const EOI_WRITE_BRIDGE_SCOPE: &str =
    "controlled first PIC_EOI write one-shot permit bridge";
pub const EOI_WRITE_BRIDGE_INPUTS: &str =
    "software-latch/permit-model/candidate/preflight/mutation-sequence/mutation-checklist/decision/final-gate";
pub const EOI_WRITE_BRIDGE_READ_ONLY: &str = "read-only telemetry bridge";
pub const EOI_WRITE_BRIDGE_READY_NO: &str = "no";
pub const EOI_WRITE_BRIDGE_TARGET_NONE: &str = "none";
pub const EOI_WRITE_BRIDGE_BLOCKER_LATCH_NOT_ARMED: &str = "latch not armed";
pub const EOI_WRITE_BRIDGE_BLOCKER_LATCH_GATED: &str = "latch armed but write still gated";
pub const EOI_WRITE_BRIDGE_BLOCKER_PERMIT_DENIED: &str = "permit denied";
pub const EOI_WRITE_PERMIT_TRANSITION_SCOPE: &str =
    "controlled first PIC_EOI write permit transition model";
pub const EOI_WRITE_PERMIT_TRANSITION_INPUTS: &str =
    "transition-state/permit-model/software-latch/bridge/candidate/preflight/mutation-sequence/mutation-checklist/decision/final-gate";
pub const EOI_WRITE_PERMIT_TRANSITION_SOFTWARE_ONLY: &str =
    "software-only permit transition";
pub const EOI_WRITE_PERMIT_TRANSITION_ARMED_YES: &str = "yes";
pub const EOI_WRITE_PERMIT_TRANSITION_ARMED_NO: &str = "no";
pub const EOI_WRITE_PERMIT_TRANSITION_ARM_RESULT: &str = "software transition armed";
pub const EOI_WRITE_PERMIT_TRANSITION_CLEAR_RESULT: &str =
    "software transition cleared";
pub const EOI_WRITE_PERMIT_TRANSITION_CHECK_RESULT: &str =
    "transition check remains denied";
pub const EOI_WRITE_PERMIT_TRANSITION_BLOCKER_TRANSITION: &str =
    "transition state is software-only";
pub const EOI_WRITE_PERMIT_TRANSITION_BLOCKER_PERMIT: &str = "permit granted: no";
pub const EOI_WRITE_PERMIT_TRANSITION_BLOCKER_BRIDGE: &str = "bridge ready: no";
pub const EOI_WRITE_PERMIT_TRANSITION_BLOCKER_FIRST_ALLOWED: &str =
    "first PIC_EOI write allowed: no";
pub const EOI_WRITE_PERMIT_TRANSITION_BLOCKER_HARDWARE: &str = "hardware mutation: no";
pub const EOI_WRITE_PERMIT_TRANSITION_BLOCKER_RUNTIME: &str = "runtime irq active: no";
pub const EOI_WRITE_PERMIT_TRANSITION_BLOCKER_STI: &str = "STI disabled";
pub const EOI_WRITE_PERMIT_TRANSITION_BLOCKER_PIC_UNMASK: &str = "PIC unmask disabled";
pub const EOI_WRITE_PERMIT_TRANSITION_BLOCKER_LIVE_IRQ: &str =
    "live IRQ runtime disabled";
pub const EOI_WRITE_EVAL_SCOPE: &str =
    "controlled first PIC_EOI write permit evaluation";
pub const EOI_WRITE_EVAL_INPUTS: &str =
    "permit-model/software-latch/bridge/transition/final-gate/mutation-checklist/preflight/candidate";
pub const EOI_WRITE_EVAL_READ_ONLY: &str = "read-only permit evaluation";
pub const EOI_WRITE_EVAL_READY_NO: &str = "no";
pub const EOI_WRITE_EVAL_BLOCKER_PERMIT: &str = "permit model remains denied";
pub const EOI_WRITE_EVAL_BLOCKER_BRIDGE: &str = "bridge remains denied";
pub const EOI_WRITE_EVAL_BLOCKER_TRANSITION: &str =
    "transition state is telemetry-only";
pub const EOI_WRITE_EVAL_BLOCKER_FIRST_WRITE: &str =
    "first PIC_EOI write path is not enabled";
pub const EOI_WRITE_EVAL_BLOCKER_HARDWARE: &str = "hardware mutation remains disabled";
pub const EOI_WRITE_EVAL_BLOCKER_RUNTIME: &str = "runtime IRQ remains inactive";
pub const EOI_RUNTIME_BRIDGE_SCOPE: &str =
    "controlled PIC_EOI runtime bridge readiness";
pub const EOI_RUNTIME_BRIDGE_INPUTS: &str =
    "manual-hw-smoke/permit-evaluator/runtime-gate/keyboard";
pub const EOI_RUNTIME_BRIDGE_READY_NO: &str = "no";
pub const EOI_RUNTIME_BRIDGE_HANDLER_ALLOWED_NO: &str = "no";
pub const EOI_RUNTIME_BRIDGE_RUNTIME_ACTIVE_NO: &str = "no";
pub const EOI_RUNTIME_BRIDGE_STI_DISABLED: &str = "disabled";
pub const EOI_RUNTIME_BRIDGE_PIC_UNMASK_DISABLED: &str = "disabled";
pub const EOI_RUNTIME_BRIDGE_LIVE_IRQ_HANDLERS_NO: &str = "no";
pub const EOI_RUNTIME_BRIDGE_KEYBOARD_POLLING: &str = "polling";
pub const EOI_RUNTIME_BRIDGE_BLOCKER_DISPATCH: &str =
    "runtime IRQ dispatch remains disabled";
pub const EOI_RUNTIME_BRIDGE_BLOCKER_STI: &str = "STI remains disabled";
pub const EOI_RUNTIME_BRIDGE_BLOCKER_PIC_LINES: &str = "PIC lines remain masked";
pub const EOI_RUNTIME_BRIDGE_BLOCKER_LIVE_HANDLERS: &str =
    "live IRQ0/IRQ1 handlers remain unbound";
pub const EOI_RUNTIME_BRIDGE_BLOCKER_HANDLER_EOI: &str =
    "handler-triggered EOI path is not enabled";
pub const IRQ_HANDLER_EOI_CANDIDATE_SCOPE: &str =
    "controlled IRQ handler EOI path candidate";
pub const IRQ_HANDLER_EOI_CANDIDATE_INPUTS: &str = "runtime-bridge-readiness";
pub const IRQ_HANDLER_EOI_CANDIDATE_READY_NO: &str = "no";
pub const IRQ_HANDLER_EOI_CANDIDATE_HANDLER_ALLOWED_NO: &str = "no";
pub const IRQ_HANDLER_EOI_CANDIDATE_LIVE_BIND_NO: &str = "no";
pub const IRQ_HANDLER_EOI_CANDIDATE_PIC_EOI_CALLSITES: &str = "1 manual-only";
pub const IRQ_HANDLER_EOI_CANDIDATE_RUNTIME_ACTIVE_NO: &str = "no";
pub const IRQ_HANDLER_EOI_CANDIDATE_STI_DISABLED: &str = "disabled";
pub const IRQ_HANDLER_EOI_CANDIDATE_PIC_UNMASK_DISABLED: &str = "disabled";
pub const IRQ_HANDLER_EOI_CANDIDATE_KEYBOARD_POLLING: &str = "polling";
pub const IRQ_HANDLER_EOI_CANDIDATE_BLOCKER_BRIDGE: &str =
    "runtime bridge readiness remains denied";
pub const IRQ_HANDLER_EOI_CANDIDATE_BLOCKER_HANDLER_EOI: &str =
    "handler-triggered EOI remains disabled";
pub const IRQ_HANDLER_EOI_CANDIDATE_BLOCKER_LIVE_HANDLERS: &str =
    "live IRQ0/IRQ1 handlers remain unbound";
pub const IRQ_HANDLER_EOI_CANDIDATE_BLOCKER_MANUAL_ONLY: &str =
    "PIC_EOI write remains manual-only";
pub const IRQ_HANDLER_EOI_CANDIDATE_BLOCKER_RUNTIME: &str =
    "runtime IRQ dispatch remains disabled";
pub const IRQ_HANDLER_EOI_STUB_SCOPE: &str = "controlled IRQ handler EOI stub";
pub const IRQ_HANDLER_EOI_STUB_INPUTS: &str = "handler-eoi-candidate";
pub const IRQ_HANDLER_EOI_STUB_EXISTS_YES: &str = "yes";
pub const IRQ_HANDLER_EOI_STUB_LIVE_BIND_NO: &str = "no";
pub const IRQ_HANDLER_EOI_STUB_INVOCATION_ALLOWED_NO: &str = "no";
pub const IRQ_HANDLER_EOI_STUB_PERFORMS_WRITE_NO: &str = "no";
pub const IRQ_HANDLER_EOI_STUB_HANDLER_ALLOWED_NO: &str = "no";
pub const IRQ_HANDLER_EOI_STUB_PIC_EOI_CALLSITES: &str = "1 manual-only";
pub const IRQ_HANDLER_EOI_STUB_RUNTIME_ACTIVE_NO: &str = "no";
pub const IRQ_HANDLER_EOI_STUB_STI_DISABLED: &str = "disabled";
pub const IRQ_HANDLER_EOI_STUB_PIC_UNMASK_DISABLED: &str = "disabled";
pub const IRQ_HANDLER_EOI_STUB_KEYBOARD_POLLING: &str = "polling";
pub const IRQ_HANDLER_EOI_STUB_BLOCKER_UNBOUND: &str =
    "stub remains unbound from live IRQ path";
pub const IRQ_HANDLER_EOI_STUB_BLOCKER_INVOCATION: &str =
    "stub invocation remains disabled";
pub const IRQ_HANDLER_EOI_STUB_BLOCKER_HANDLER_EOI: &str =
    "handler-triggered EOI remains disabled";
pub const IRQ_HANDLER_EOI_STUB_BLOCKER_MANUAL_ONLY: &str =
    "PIC_EOI write remains manual-only";
pub const IRQ_HANDLER_EOI_STUB_BLOCKER_RUNTIME: &str =
    "runtime IRQ dispatch remains disabled";
pub const IRQ_HANDLER_BIND_CANDIDATE_SCOPE: &str =
    "controlled IRQ handler bind candidate";
pub const IRQ_HANDLER_BIND_CANDIDATE_INPUTS: &str = "handler-eoi-stub";
pub const IRQ_HANDLER_BIND_CANDIDATE_EXISTS_YES: &str = "yes";
pub const IRQ_HANDLER_BIND_CANDIDATE_READY_NO: &str = "no";
pub const IRQ_HANDLER_BIND_CANDIDATE_LIVE_IDT_BIND_NO: &str = "no";
pub const IRQ_HANDLER_BIND_CANDIDATE_IRQ_REACHABLE_NO: &str = "no";
pub const IRQ_HANDLER_BIND_CANDIDATE_HANDLER_ALLOWED_NO: &str = "no";
pub const IRQ_HANDLER_BIND_CANDIDATE_RUNTIME_ACTIVE_NO: &str = "no";
pub const IRQ_HANDLER_BIND_CANDIDATE_STI_DISABLED: &str = "disabled";
pub const IRQ_HANDLER_BIND_CANDIDATE_PIC_UNMASK_DISABLED: &str = "disabled";
pub const IRQ_HANDLER_BIND_CANDIDATE_KEYBOARD_POLLING: &str = "polling";
pub const IRQ_HANDLER_BIND_CANDIDATE_BLOCKER_IDT_BIND: &str =
    "live IDT bind remains disabled";
pub const IRQ_HANDLER_BIND_CANDIDATE_BLOCKER_IRQ_REGISTRATION: &str =
    "IRQ0/IRQ1 handler registration remains disabled";
pub const IRQ_HANDLER_BIND_CANDIDATE_BLOCKER_STUB_INVOCATION: &str =
    "stub invocation remains disabled";
pub const IRQ_HANDLER_BIND_CANDIDATE_BLOCKER_HANDLER_EOI: &str =
    "handler-triggered EOI remains disabled";
pub const IRQ_HANDLER_BIND_CANDIDATE_BLOCKER_RUNTIME: &str =
    "runtime IRQ dispatch remains disabled";
pub const IDT_BIND_RUNTIME_BRIDGE_SCOPE: &str =
    "controlled IDT bind runtime bridge readiness";
pub const IDT_BIND_RUNTIME_BRIDGE_INPUTS: &str = "idt-bind-hw-smoke/handler-bind-candidate";
pub const IDT_BIND_RUNTIME_BRIDGE_READY_NO: &str = "no";
pub const IDT_BIND_RUNTIME_BRIDGE_LIVE_BIND_ALLOWED_NO: &str = "no";
pub const IDT_BIND_RUNTIME_BRIDGE_IRQ_REACHABLE_NO: &str = "no";
pub const IDT_BIND_RUNTIME_BRIDGE_INTERRUPT_ALLOWED_NO: &str = "no";
pub const IDT_BIND_RUNTIME_BRIDGE_RUNTIME_ACTIVE_NO: &str = "no";
pub const IDT_BIND_RUNTIME_BRIDGE_STI_DISABLED: &str = "disabled";
pub const IDT_BIND_RUNTIME_BRIDGE_PIC_UNMASK_DISABLED: &str = "disabled";
pub const IDT_BIND_RUNTIME_BRIDGE_KEYBOARD_POLLING: &str = "polling";
pub const IDT_BIND_RUNTIME_BRIDGE_BLOCKER_PROOF: &str =
    "manual IDT bind proof is required before runtime bridge consideration";
pub const IDT_BIND_RUNTIME_BRIDGE_BLOCKER_LIVE_BIND: &str =
    "live IRQ bind remains disabled";
pub const IDT_BIND_RUNTIME_BRIDGE_BLOCKER_IRQ_REACHABLE: &str =
    "IRQ handler reachability remains disabled";
pub const IDT_BIND_RUNTIME_BRIDGE_BLOCKER_INTERRUPT: &str =
    "interrupt invocation remains disabled";
pub const IDT_BIND_RUNTIME_BRIDGE_BLOCKER_RUNTIME: &str =
    "runtime IRQ dispatch remains disabled";
pub const IDT_INVOKE_RUNTIME_BRIDGE_SCOPE: &str =
    "controlled IDT invocation runtime bridge readiness";
pub const IDT_INVOKE_RUNTIME_BRIDGE_INPUTS: &str =
    "idt-bind-hw-smoke/idt-invoke-hw-smoke/bind-runtime-bridge/handler-bind-candidate/stub";
pub const IDT_INVOKE_RUNTIME_BRIDGE_READY_NO: &str = "no";
pub const IDT_INVOKE_RUNTIME_BRIDGE_LIVE_DELIVERY_ALLOWED_NO: &str = "no";
pub const IDT_INVOKE_RUNTIME_BRIDGE_HARDWARE_REACHABLE_NO: &str = "no";
pub const IDT_INVOKE_RUNTIME_BRIDGE_HANDLER_EOI_ALLOWED_NO: &str = "no";
pub const IDT_INVOKE_RUNTIME_BRIDGE_RUNTIME_ACTIVE_NO: &str = "no";
pub const IDT_INVOKE_RUNTIME_BRIDGE_STI_DISABLED: &str = "disabled";
pub const IDT_INVOKE_RUNTIME_BRIDGE_PIC_UNMASK_DISABLED: &str = "disabled";
pub const IDT_INVOKE_RUNTIME_BRIDGE_KEYBOARD_POLLING: &str = "polling";
pub const IDT_INVOKE_RUNTIME_BRIDGE_BLOCKER_BIND_PROOF: &str =
    "manual IDT bind proof remains required";
pub const IDT_INVOKE_RUNTIME_BRIDGE_BLOCKER_INVOKE_PROOF: &str =
    "manual IDT invocation proof remains required";
pub const IDT_INVOKE_RUNTIME_BRIDGE_BLOCKER_DELIVERY: &str =
    "live IRQ delivery remains disabled";
pub const IDT_INVOKE_RUNTIME_BRIDGE_BLOCKER_HARDWARE_REACHABLE: &str =
    "IRQ handler hardware reachability remains disabled";
pub const IDT_INVOKE_RUNTIME_BRIDGE_BLOCKER_RUNTIME: &str =
    "runtime IRQ dispatch remains disabled";

static mut IRQ_GATE_BIND_SMOKE_ARMED: bool = false;
static mut IRQ_GATE_BIND_SMOKE_EXECUTED: bool = false;

static mut IRQ_RUNTIME_ARMED: bool = false;
static mut IRQ_RUNTIME_COMMITTED: bool = false;
static mut IRQ_RUNTIME_ACTIVATION_TOKEN_PRESENT: bool = false;

static EOI_WRITE_ONESHOT_LATCH_ARMED: AtomicBool = AtomicBool::new(false);
static EOI_WRITE_PERMIT_TRANSITION_ARMED: AtomicBool = AtomicBool::new(false);

/// Documentation-only representation of a future IRQ handler.
pub struct IrqHandlerSkeleton {
    pub irq: u8,
    pub vector: u8,
    pub name: &'static str,
    pub state: &'static str,
}

/// Documentation-only representation of a future IRQ IDT gate plan.
#[derive(Copy, Clone)]
pub struct IrqGatePlan {
    pub irq: u8,
    pub vector: u8,
    pub name: &'static str,
    pub gate_state: &'static str,
    pub idt_binding: &'static str,
    pub pic_remap: &'static str,
    pub eoi_dispatch: &'static str,
    pub interrupts: &'static str,
}

/// Documentation-only step in the future IRQ gate binding path.
#[derive(Copy, Clone)]
pub struct IrqGateBindDisabledStep {
    pub irq: u8,
    pub vector: u8,
    pub name: &'static str,
    pub bind_path: &'static str,
    pub idt_install: &'static str,
    pub pic_remap: &'static str,
    pub eoi_dispatch: &'static str,
    pub interrupts: &'static str,
}

/// Documentation-only status for the disabled IRQ gate binding helper.
#[derive(Copy, Clone)]
pub struct IrqGateBindDisabledStatus {
    pub helper: &'static str,
    pub boot_call: &'static str,
    pub irq0_vector: u8,
    pub irq0_state: &'static str,
    pub irq1_vector: u8,
    pub irq1_state: &'static str,
    pub irq0_active_handler: &'static str,
    pub irq1_active_handler: &'static str,
    pub keyboard_input: &'static str,
    pub steps: [IrqGateBindDisabledStep; 2],
}

/// Documentation-only readiness status for future runtime IRQ activation.
#[derive(Copy, Clone)]
pub struct IrqRuntimeReadiness {
    pub idt_exceptions: &'static str,
    pub irq_gate_plan: &'static str,
    pub eoi_strategy: &'static str,
    pub pic_remap: &'static str,
    pub sti: &'static str,
    pub keyboard_fallback: &'static str,
    pub ready_for_runtime_irq: &'static str,
}

/// Documentation-only risk summary for blocked IRQ runtime activation.
#[derive(Copy, Clone)]
pub struct IrqRuntimeRisk {
    pub runtime_irq: &'static str,
    pub reason: &'static str,
    pub required_before_enable: &'static str,
    pub sti_allowed: &'static str,
}

/// Aggregated read-only readiness matrix for future IRQ runtime activation.
#[derive(Copy, Clone, Debug)]
pub struct IrqRuntimeMatrix {
    pub pic_remap_smoke: &'static str,
    pub irq_gate_bind_smoke: &'static str,
    pub eoi_runtime_boundary: &'static str,
    pub pic_mask_policy: &'static str,
    pub unmask_policy: &'static str,
    pub runtime_latch: &'static str,
    pub keyboard_mode: &'static str,
    pub sti: &'static str,
    pub runtime_irq_active: &'static str,
    pub smoke_prerequisites: &'static str,
}

/// Read-only activation dry-run decision derived from the readiness matrix.
#[derive(Copy, Clone)]
pub struct IrqRuntimeActivationDryRun {
    pub allowed: bool,
    pub allowed_text: &'static str,
    pub result: &'static str,
    pub next: &'static str,
}

/// Telemetry-only activation token state for future IRQ runtime activation.
#[derive(Copy, Clone)]
pub struct IrqRuntimeActivationTokenTelemetry {
    pub token_state: &'static str,
    pub token_scope: &'static str,
    pub hardware_mutation: &'static str,
    pub sti: &'static str,
    pub pic_unmask: &'static str,
    pub live_irq0_irq1: &'static str,
    pub runtime_eoi_dispatch: &'static str,
    pub keyboard_mode: &'static str,
}

/// Read-only controlled activation gate telemetry for future IRQ runtime activation.
#[derive(Copy, Clone)]
pub struct IrqRuntimeActivationGate {
    pub token_gate: &'static str,
    pub readiness_matrix: &'static str,
    pub dry_run_commit_allowed: &'static str,
    pub eoi_runtime_boundary: &'static str,
    pub pic_mask_policy: &'static str,
    pub unmask_policy: &'static str,
    pub hardware_mutation: &'static str,
    pub activation_allowed: &'static str,
    pub runtime_irq_active: &'static str,
    pub result: &'static str,
    pub next: &'static str,
}

/// Read-only simulation result for rehearsing future IRQ runtime activation.
#[derive(Copy, Clone)]
pub struct IrqRuntimeActivationSimulation {
    pub token_gate: &'static str,
    pub readiness_matrix: &'static str,
    pub gate_decision: &'static str,
    pub dry_run_commit_allowed: &'static str,
    pub eoi_runtime_boundary: &'static str,
    pub hardware_mutation: &'static str,
    pub simulated_activation_allowed: &'static str,
    pub sti_would_enable: &'static str,
    pub pic_unmask_would_apply: &'static str,
    pub eoi_dispatch_would_enable: &'static str,
    pub keyboard_mode: &'static str,
    pub runtime_irq_active: &'static str,
    pub result: &'static str,
    pub next: &'static str,
}

/// Read-only STI activation plan derived from the controlled activation telemetry stack.
#[derive(Copy, Clone)]
pub struct StiControlledActivationPlan {
    pub activation_token: &'static str,
    pub token_gate: &'static str,
    pub runtime_gate: &'static str,
    pub readiness_matrix: &'static str,
    pub simulation: &'static str,
    pub eoi_runtime_boundary: &'static str,
    pub pic_unmask_policy: &'static str,
    pub pic_unmask: &'static str,
    pub eoi_dispatch: &'static str,
    pub hardware_mutation: &'static str,
    pub keyboard_mode: &'static str,
    pub sti_instruction: &'static str,
    pub sti_allowed: &'static str,
    pub runtime_irq_active: &'static str,
    pub result: &'static str,
    pub next: &'static str,
}

/// Read-only activation smoke foundation derived from the pre-activation telemetry stack.
#[derive(Copy, Clone)]
pub struct IrqRuntimeActivationSmoke {
    pub activation_token: &'static str,
    pub runtime_gate: &'static str,
    pub readiness_matrix: &'static str,
    pub simulation: &'static str,
    pub sti_plan: &'static str,
    pub eoi_runtime_boundary: &'static str,
    pub pic_unmask: &'static str,
    pub eoi_dispatch: &'static str,
    pub hardware_mutation: &'static str,
    pub keyboard_mode: &'static str,
    pub sti_instruction: &'static str,
    pub runtime_irq_active: &'static str,
    pub activation_smoke: &'static str,
    pub result: &'static str,
    pub next: &'static str,
}

#[derive(Copy, Clone)]
pub struct EoiDispatchSmoke {
    pub eoi_dispatch_smoke: &'static str,
    pub dispatch_mode: &'static str,
    pub pic_remap_smoke: &'static str,
    pub irq_gates: &'static str,
    pub pic_eoi_writes: &'static str,
    pub sti_instruction: &'static str,
    pub pic_unmask: &'static str,
    pub keyboard_mode: &'static str,
    pub runtime_irq_active: &'static str,
    pub hardware_mutation: &'static str,
    pub master_eoi_route: &'static str,
    pub slave_eoi_route: &'static str,
    pub result: &'static str,
}

#[derive(Copy, Clone)]
pub struct PicUnmaskSmoke {
    pub pic_unmask_smoke: &'static str,
    pub dispatch_mode: &'static str,
    pub target_irq_lines: &'static str,
    pub pic_mask_policy: &'static str,
    pub unmask_policy: &'static str,
    pub activation_token: &'static str,
    pub activation_gate: &'static str,
    pub eoi_runtime_boundary: &'static str,
    pub sti_plan: &'static str,
    pub sti_instruction: &'static str,
    pub eoi_dispatch_smoke: &'static str,
    pub live_unmask: &'static str,
    pub pic_data_writes: &'static str,
    pub hardware_mutation: &'static str,
    pub runtime_irq_active: &'static str,
    pub keyboard_mode: &'static str,
    pub result: &'static str,
}

#[derive(Copy, Clone)]
pub struct IdtRuntimeBindSmoke {
    pub idt_runtime_bind_smoke: &'static str,
    pub dispatch_mode: &'static str,
    pub target_vectors: &'static str,
    pub activation_token: &'static str,
    pub activation_gate: &'static str,
    pub irq_gate_bind_smoke: &'static str,
    pub eoi_dispatch_smoke: &'static str,
    pub pic_unmask_smoke: &'static str,
    pub sti_plan: &'static str,
    pub sti_instruction: &'static str,
    pub live_handler_bind: &'static str,
    pub hardware_mutation: &'static str,
    pub runtime_irq_active: &'static str,
    pub keyboard_mode: &'static str,
    pub result: &'static str,
}

#[derive(Copy, Clone)]
pub struct IrqRuntimeFinalGate {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub activation_token: &'static str,
    pub activation_gate: &'static str,
    pub readiness_matrix: &'static str,
    pub simulation: &'static str,
    pub sti_plan: &'static str,
    pub activation_smoke: &'static str,
    pub eoi_dispatch_smoke: &'static str,
    pub pic_unmask_smoke: &'static str,
    pub idt_runtime_bind_smoke: &'static str,
    pub keyboard_mode: &'static str,
    pub final_activation_allowed: &'static str,
    pub hardware_mutation: &'static str,
    pub runtime_irq_active: &'static str,
    pub sti_instruction: &'static str,
    pub pic_unmask: &'static str,
    pub eoi_dispatch: &'static str,
    pub live_idt_bind: &'static str,
    pub result: &'static str,
    pub next: &'static str,
}

#[derive(Copy, Clone)]
pub struct IrqRuntimeActivationDecision {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub activation_decision: &'static str,
    pub final_activation_allowed: &'static str,
    pub runtime_irq_active: &'static str,
    pub hardware_mutation: &'static str,
    pub sti_instruction: &'static str,
    pub pic_unmask: &'static str,
    pub eoi_dispatch: &'static str,
    pub live_idt_bind: &'static str,
    pub keyboard_mode: &'static str,
    pub activation_smoke: &'static str,
    pub simulation: &'static str,
    pub sti_plan: &'static str,
    pub eoi_dispatch_smoke: &'static str,
    pub pic_unmask_smoke: &'static str,
    pub idt_runtime_bind_smoke: &'static str,
    pub activation_token: &'static str,
    pub activation_gate: &'static str,
    pub readiness_matrix: &'static str,
}

#[derive(Copy, Clone)]
pub struct IrqRuntimeHardwareMutationChecklist {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub hardware_mutation_ready: &'static str,
    pub activation_decision: &'static str,
    pub final_activation_allowed: &'static str,
    pub runtime_irq_active: &'static str,
    pub sti_mutation: &'static str,
    pub pic_unmask_mutation: &'static str,
    pub eoi_dispatch_mutation: &'static str,
    pub idt_live_bind_mutation: &'static str,
    pub keyboard_input_mutation: &'static str,
    pub activation_smoke: &'static str,
    pub sti_plan: &'static str,
    pub eoi_dispatch_smoke: &'static str,
    pub pic_unmask_smoke: &'static str,
    pub idt_runtime_bind_smoke: &'static str,
    pub activation_token: &'static str,
    pub activation_gate: &'static str,
    pub readiness_matrix: &'static str,
    pub keyboard_mode: &'static str,
}

#[derive(Copy, Clone)]
pub struct IrqRuntimeMutationSmokeSequence {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub mutation_sequence_ready: &'static str,
    pub hardware_mutation: &'static str,
    pub runtime_irq_active: &'static str,
    pub next_mutation_step: &'static str,
    pub allowed_mutation_steps: &'static str,
    pub sti_instruction: &'static str,
    pub pic_unmask: &'static str,
    pub eoi_dispatch: &'static str,
    pub live_idt_bind: &'static str,
    pub keyboard_mode: &'static str,
    pub hardware_mutation_ready: &'static str,
    pub activation_decision: &'static str,
    pub final_activation_allowed: &'static str,
    pub activation_smoke: &'static str,
    pub sti_plan: &'static str,
    pub eoi_dispatch_smoke: &'static str,
    pub pic_unmask_smoke: &'static str,
    pub idt_runtime_bind_smoke: &'static str,
    pub activation_token: &'static str,
    pub activation_gate: &'static str,
    pub readiness_matrix: &'static str,
}

#[derive(Copy, Clone)]
pub struct EoiWriteSmokePreflight {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub eoi_write_smoke_preflight: &'static str,
    pub first_pic_eoi_write_allowed: &'static str,
    pub hardware_mutation: &'static str,
    pub runtime_irq_active: &'static str,
    pub target_command_port: &'static str,
    pub target_irq_line: &'static str,
    pub eoi_dispatch: &'static str,
    pub sti_instruction: &'static str,
    pub pic_unmask: &'static str,
    pub live_idt_bind: &'static str,
    pub keyboard_mode: &'static str,
    pub mutation_sequence_ready: &'static str,
    pub hardware_mutation_ready: &'static str,
    pub activation_decision: &'static str,
    pub final_activation_allowed: &'static str,
    pub eoi_dispatch_smoke: &'static str,
    pub pic_unmask_smoke: &'static str,
    pub idt_runtime_bind_smoke: &'static str,
    pub sti_plan: &'static str,
}

#[derive(Copy, Clone)]
pub struct EoiWriteSmokeCandidate {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub eoi_write_smoke_candidate: &'static str,
    pub candidate_armed: &'static str,
    pub fire_result: &'static str,
    pub first_pic_eoi_write_performed: &'static str,
    pub hardware_mutation: &'static str,
    pub runtime_irq_active: &'static str,
    pub target_command_port: &'static str,
    pub target_irq_line: &'static str,
    pub eoi_dispatch: &'static str,
    pub sti_instruction: &'static str,
    pub pic_unmask: &'static str,
    pub live_idt_bind: &'static str,
    pub keyboard_mode: &'static str,
    pub eoi_write_preflight: &'static str,
    pub first_pic_eoi_write_allowed: &'static str,
    pub mutation_sequence_ready: &'static str,
    pub hardware_mutation_ready: &'static str,
    pub activation_decision: &'static str,
    pub final_activation_allowed: &'static str,
}

#[derive(Copy, Clone)]
pub struct EoiWritePermitModel {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub permit_granted: &'static str,
    pub first_pic_eoi_write_allowed: &'static str,
    pub target_command_port: &'static str,
    pub target_value: &'static str,
    pub target_irq_line: &'static str,
    pub hardware_mutation: &'static str,
    pub runtime_irq_active: &'static str,
    pub fire_command: &'static str,
    pub activation_decision: &'static str,
    pub final_activation_allowed: &'static str,
    pub hardware_mutation_ready: &'static str,
    pub mutation_sequence_ready: &'static str,
    pub candidate_fire_result: &'static str,
    pub sti_instruction: &'static str,
    pub pic_unmask: &'static str,
    pub live_idt_bind: &'static str,
    pub keyboard_mode: &'static str,
}

#[derive(Copy, Clone)]
pub struct EoiWriteOneShotCommandPath {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub one_shot_armed: &'static str,
    pub fire_allowed: &'static str,
    pub first_pic_eoi_write_performed: &'static str,
    pub target_command_port: &'static str,
    pub target_value: &'static str,
    pub hardware_mutation: &'static str,
    pub runtime_irq_active: &'static str,
    pub fire_result: &'static str,
    pub permit_granted: &'static str,
    pub first_pic_eoi_write_allowed: &'static str,
    pub sti_instruction: &'static str,
    pub pic_unmask: &'static str,
    pub live_idt_bind: &'static str,
    pub keyboard_mode: &'static str,
}

#[derive(Copy, Clone)]
pub struct EoiWriteOneShotLatch {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub latch: &'static str,
    pub one_shot_armed: &'static str,
    pub fire_allowed: &'static str,
    pub first_pic_eoi_write_performed: &'static str,
    pub target_command_port: &'static str,
    pub target_value: &'static str,
    pub hardware_mutation: &'static str,
    pub runtime_irq_active: &'static str,
    pub fire_result: &'static str,
    pub fire_cleared_latch: &'static str,
    pub permit_granted: &'static str,
    pub first_pic_eoi_write_allowed: &'static str,
    pub sti_instruction: &'static str,
    pub pic_unmask: &'static str,
    pub live_idt_bind: &'static str,
    pub keyboard_mode: &'static str,
}

#[derive(Copy, Clone)]
pub struct EoiWriteBridge {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub bridge: &'static str,
    pub latch: &'static str,
    pub one_shot_armed: &'static str,
    pub permit_granted: &'static str,
    pub bridge_ready: &'static str,
    pub first_pic_eoi_write_allowed: &'static str,
    pub target_command_port: &'static str,
    pub target_value: &'static str,
    pub hardware_mutation: &'static str,
    pub runtime_irq_active: &'static str,
    pub blocker_latch: &'static str,
    pub blocker_permit: &'static str,
    pub sti_instruction: &'static str,
    pub pic_unmask: &'static str,
    pub live_idt_bind: &'static str,
    pub keyboard_mode: &'static str,
}

#[derive(Copy, Clone)]
pub struct EoiWritePermitTransition {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub transition: &'static str,
    pub permit_transition_armed: &'static str,
    pub permit_granted: &'static str,
    pub bridge_ready: &'static str,
    pub first_pic_eoi_write_allowed: &'static str,
    pub target_command_port: &'static str,
    pub target_value: &'static str,
    pub hardware_mutation: &'static str,
    pub runtime_irq_active: &'static str,
    pub blocker_transition: &'static str,
    pub blocker_permit: &'static str,
    pub blocker_bridge: &'static str,
    pub blocker_first_allowed: &'static str,
    pub blocker_hardware: &'static str,
    pub blocker_runtime: &'static str,
    pub blocker_sti: &'static str,
    pub blocker_pic_unmask: &'static str,
    pub blocker_live_irq: &'static str,
    pub sti_instruction: &'static str,
    pub pic_unmask: &'static str,
    pub live_idt_bind: &'static str,
    pub keyboard_mode: &'static str,
}

#[derive(Copy, Clone)]
pub struct EoiWritePermitEvaluation {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub evaluation: &'static str,
    pub evaluation_ready: &'static str,
    pub one_shot_armed: &'static str,
    pub permit_transition_armed: &'static str,
    pub permit_granted: &'static str,
    pub bridge_ready: &'static str,
    pub first_pic_eoi_write_allowed: &'static str,
    pub hardware_mutation: &'static str,
    pub runtime_irq_active: &'static str,
    pub blocker_permit: &'static str,
    pub blocker_bridge: &'static str,
    pub blocker_transition: &'static str,
    pub blocker_first_write: &'static str,
    pub blocker_hardware: &'static str,
    pub blocker_runtime: &'static str,
    pub sti_instruction: &'static str,
    pub pic_unmask: &'static str,
    pub live_idt_bind: &'static str,
    pub keyboard_mode: &'static str,
}

#[derive(Copy, Clone)]
pub struct EoiRuntimeBridgeReadiness {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub manual_pic_eoi_smoke_proven: &'static str,
    pub runtime_bridge_ready: &'static str,
    pub handler_triggered_eoi_allowed: &'static str,
    pub runtime_irq_active: &'static str,
    pub sti: &'static str,
    pub pic_unmask: &'static str,
    pub live_irq_handlers: &'static str,
    pub keyboard_mode: &'static str,
    pub blocker_dispatch: &'static str,
    pub blocker_sti: &'static str,
    pub blocker_pic_lines: &'static str,
    pub blocker_live_handlers: &'static str,
    pub blocker_handler_eoi: &'static str,
}

#[derive(Copy, Clone)]
pub struct IrqHandlerEoiCandidate {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub runtime_bridge_ready: &'static str,
    pub handler_eoi_candidate_ready: &'static str,
    pub handler_triggered_eoi_allowed: &'static str,
    pub live_handler_bind: &'static str,
    pub pic_eoi_callsites: &'static str,
    pub runtime_irq_active: &'static str,
    pub sti: &'static str,
    pub pic_unmask: &'static str,
    pub keyboard_mode: &'static str,
    pub blocker_bridge: &'static str,
    pub blocker_handler_eoi: &'static str,
    pub blocker_live_handlers: &'static str,
    pub blocker_manual_only: &'static str,
    pub blocker_runtime: &'static str,
}

#[derive(Copy, Clone)]
pub struct IrqHandlerEoiStub {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub stub_exists: &'static str,
    pub stub_bound_to_live_irq_path: &'static str,
    pub stub_invocation_allowed: &'static str,
    pub stub_performs_pic_eoi_write: &'static str,
    pub handler_triggered_eoi_allowed: &'static str,
    pub pic_eoi_callsites: &'static str,
    pub runtime_irq_active: &'static str,
    pub sti: &'static str,
    pub pic_unmask: &'static str,
    pub keyboard_mode: &'static str,
    pub blocker_unbound: &'static str,
    pub blocker_invocation: &'static str,
    pub blocker_handler_eoi: &'static str,
    pub blocker_manual_only: &'static str,
    pub blocker_runtime: &'static str,
}

#[derive(Copy, Clone)]
pub struct IrqHandlerBindCandidate {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub stub_exists: &'static str,
    pub bind_candidate_exists: &'static str,
    pub bind_candidate_ready: &'static str,
    pub live_idt_bind_performed: &'static str,
    pub irq_handler_reachable: &'static str,
    pub handler_triggered_eoi_allowed: &'static str,
    pub runtime_irq_active: &'static str,
    pub sti: &'static str,
    pub pic_unmask: &'static str,
    pub keyboard_mode: &'static str,
    pub blocker_idt_bind: &'static str,
    pub blocker_irq_registration: &'static str,
    pub blocker_stub_invocation: &'static str,
    pub blocker_handler_eoi: &'static str,
    pub blocker_runtime: &'static str,
}

#[derive(Copy, Clone)]
pub struct IdtBindRuntimeBridgeReadiness {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub manual_idt_bind_smoke_proven_this_boot: &'static str,
    pub runtime_idt_bridge_ready: &'static str,
    pub live_irq_bind_allowed: &'static str,
    pub irq_handler_reachable: &'static str,
    pub interrupt_invocation_allowed: &'static str,
    pub runtime_irq_active: &'static str,
    pub sti: &'static str,
    pub pic_unmask: &'static str,
    pub keyboard_mode: &'static str,
    pub blocker_proof: &'static str,
    pub blocker_live_bind: &'static str,
    pub blocker_irq_reachable: &'static str,
    pub blocker_interrupt: &'static str,
    pub blocker_runtime: &'static str,
}

#[derive(Copy, Clone)]
pub struct IdtInvokeRuntimeBridgeReadiness {
    pub scope: &'static str,
    pub inputs: &'static str,
    pub manual_idt_bind_smoke_proven_this_boot: &'static str,
    pub manual_idt_invocation_smoke_proven_this_boot: &'static str,
    pub runtime_invocation_bridge_ready: &'static str,
    pub live_irq_delivery_allowed: &'static str,
    pub irq_handler_reachable_from_hardware: &'static str,
    pub handler_triggered_eoi_allowed: &'static str,
    pub runtime_irq_active: &'static str,
    pub sti: &'static str,
    pub pic_unmask: &'static str,
    pub keyboard_mode: &'static str,
    pub blocker_bind_proof: &'static str,
    pub blocker_invoke_proof: &'static str,
    pub blocker_delivery: &'static str,
    pub blocker_hardware_reachable: &'static str,
    pub blocker_runtime: &'static str,
}

/// Documentation-only preflight result for future IRQ runtime activation.
#[derive(Copy, Clone)]
pub struct IrqRuntimePreflight {
    pub idt_exceptions: &'static str,
    pub irq_vectors: &'static str,
    pub bind_path: &'static str,
    pub eoi_dispatch: &'static str,
    pub pic_remap: &'static str,
    pub keyboard_fallback: &'static str,
    pub pf_smoke: &'static str,
    pub result: &'static str,
}

/// Command-facing arm status for the controlled IRQ gate bind smoke path.
#[derive(Copy, Clone)]
pub struct IrqGateBindSmokeArmStatus {
    pub mode: &'static str,
    pub next: &'static str,
    pub interrupts: &'static str,
    pub pic_irq_mask: &'static str,
    pub eoi_dispatch: &'static str,
}

/// Command-facing result for an attempted controlled IRQ gate bind smoke.
#[derive(Copy, Clone)]
pub struct IrqGateBindSmokeResult {
    pub guard: &'static str,
    pub irq0_vector_state: &'static str,
    pub irq1_vector_state: &'static str,
    pub pic_irq_mask: &'static str,
    pub sti: &'static str,
    pub eoi_dispatch: &'static str,
    pub keyboard_input: &'static str,
    pub result: &'static str,
    pub next: Option<&'static str>,
}

/// Read-only state telemetry for the controlled IRQ gate bind smoke path.
#[derive(Copy, Clone, Debug)]
pub struct IrqGateBindStateTelemetry {
    pub armed: bool,
    pub executed: bool,
    pub irq0_vector: u8,
    pub irq0_vector_state: &'static str,
    pub irq1_vector: u8,
    pub irq1_vector_state: &'static str,
    pub irq0_active_handler: &'static str,
    pub irq1_active_handler: &'static str,
    pub bind_expected: &'static str,
    pub bind_applied: &'static str,
    pub irq_runtime: &'static str,
    pub pic_irq_mask: &'static str,
    pub sti: &'static str,
    pub eoi_dispatch: &'static str,
    pub keyboard_input: &'static str,
}

/// Read-only history telemetry for the controlled IRQ gate bind smoke path.
#[derive(Copy, Clone, Debug)]
pub struct IrqGateBindHistoryTelemetry {
    pub arm_command: &'static str,
    pub smoke_command: &'static str,
    pub last_smoke_executed: &'static str,
    pub idt_binds: &'static str,
    pub boot_bind: &'static str,
}

/// Read-only preflight telemetry for the controlled IRQ gate bind smoke path.
#[derive(Copy, Clone, Debug)]
pub struct IrqGateBindPreflightTelemetry {
    pub guard: &'static str,
    pub bind_path: &'static str,
    pub irq0_vector: u8,
    pub irq0_vector_state: &'static str,
    pub irq1_vector: u8,
    pub irq1_vector_state: &'static str,
    pub pic_irq_mask: &'static str,
    pub sti: &'static str,
    pub eoi_dispatch: &'static str,
    pub keyboard_input: &'static str,
    pub result: &'static str,
}

/// Command-facing status for the controlled IRQ gate bind smoke path.
#[derive(Copy, Clone)]
pub struct IrqGateBindSmokeStatus {
    pub armed: bool,
    pub executed: bool,
    pub irq0_vector: u8,
    pub irq0_vector_state: &'static str,
    pub irq1_vector: u8,
    pub irq1_vector_state: &'static str,
    pub irq0_active_handler: &'static str,
    pub irq1_active_handler: &'static str,
    pub pic_irq_mask: &'static str,
    pub sti: &'static str,
    pub eoi_dispatch: &'static str,
    pub keyboard_input: &'static str,
}

/// Documentation-only timer IRQ skeleton.
pub fn irq0_timer_skeleton() -> IrqHandlerSkeleton {
    IrqHandlerSkeleton {
        irq: 0,
        vector: IRQ0_VECTOR,
        name: IRQ0_NAME,
        state: "skeleton / disabled",
    }
}

/// Documentation-only keyboard IRQ skeleton.
pub fn irq1_keyboard_skeleton() -> IrqHandlerSkeleton {
    IrqHandlerSkeleton {
        irq: 1,
        vector: IRQ1_VECTOR,
        name: IRQ1_NAME,
        state: "skeleton / disabled",
    }
}

/// Returns the planned IRQ0/IRQ1 skeleton contract without touching hardware.
pub fn irq_handler_skeletons() -> [IrqHandlerSkeleton; 2] {
    [irq0_timer_skeleton(), irq1_keyboard_skeleton()]
}

/// Documentation-only timer IRQ gate plan.
pub fn irq0_timer_gate_plan() -> IrqGatePlan {
    IrqGatePlan {
        irq: 0,
        vector: IRQ0_VECTOR,
        name: IRQ0_NAME,
        gate_state: IRQ_GATE_STATE_DORMANT,
        idt_binding: IRQ_IDT_BINDING_DISABLED,
        pic_remap: IRQ_PIC_REMAP_DISABLED,
        eoi_dispatch: IRQ_EOI_DISPATCH_DISABLED,
        interrupts: IRQ_INTERRUPTS_DISABLED,
    }
}

/// Documentation-only keyboard IRQ gate plan.
pub fn irq1_keyboard_gate_plan() -> IrqGatePlan {
    IrqGatePlan {
        irq: 1,
        vector: IRQ1_VECTOR,
        name: IRQ1_NAME,
        gate_state: IRQ_GATE_STATE_DORMANT,
        idt_binding: IRQ_IDT_BINDING_DISABLED,
        pic_remap: IRQ_PIC_REMAP_DISABLED,
        eoi_dispatch: IRQ_EOI_DISPATCH_DISABLED,
        interrupts: IRQ_INTERRUPTS_DISABLED,
    }
}

/// Returns the planned IRQ0/IRQ1 gate contract without touching hardware.
pub fn irq_gate_plan() -> [IrqGatePlan; 2] {
    [irq0_timer_gate_plan(), irq1_keyboard_gate_plan()]
}

/// Returns the disabled IRQ0/IRQ1 gate bind path without installing gates.
pub fn bind_irq_gates_disabled() -> IrqGateBindDisabledStatus {
    IrqGateBindDisabledStatus {
        helper: IRQ_BIND_DISABLED_HELPER,
        boot_call: IRQ_BIND_BOOT_CALL_DISABLED,
        irq0_vector: IRQ0_VECTOR,
        irq0_state: IRQ_VECTOR_UNBOUND,
        irq1_vector: IRQ1_VECTOR,
        irq1_state: IRQ_VECTOR_UNBOUND,
        irq0_active_handler: IRQ_ACTIVE_HANDLER_NONE,
        irq1_active_handler: IRQ_ACTIVE_HANDLER_NONE,
        keyboard_input: IRQ_KEYBOARD_INPUT_POLLING_ONLY,
        steps: [
            IrqGateBindDisabledStep {
                irq: 0,
                vector: IRQ0_VECTOR,
                name: IRQ0_NAME,
                bind_path: IRQ_BIND_PATH_DISABLED_ONLY,
                idt_install: IRQ_IDT_INSTALL_DISABLED,
                pic_remap: IRQ_PIC_REMAP_DISABLED,
                eoi_dispatch: IRQ_EOI_DISPATCH_DISABLED,
                interrupts: IRQ_INTERRUPTS_DISABLED,
            },
            IrqGateBindDisabledStep {
                irq: 1,
                vector: IRQ1_VECTOR,
                name: IRQ1_NAME,
                bind_path: IRQ_BIND_PATH_DISABLED_ONLY,
                idt_install: IRQ_IDT_INSTALL_DISABLED,
                pic_remap: IRQ_PIC_REMAP_DISABLED,
                eoi_dispatch: IRQ_EOI_DISPATCH_DISABLED,
                interrupts: IRQ_INTERRUPTS_DISABLED,
            },
        ],
    }
}

/// Returns readiness telemetry for future IRQ runtime activation.
pub fn irq_runtime_readiness() -> IrqRuntimeReadiness {
    let plan = irq_gate_plan();
    let disabled_bind = bind_irq_gates_disabled();

    IrqRuntimeReadiness {
        idt_exceptions: IRQ_READINESS_OK,
        irq_gate_plan: if plan[0].vector == IRQ0_VECTOR && plan[1].vector == IRQ1_VECTOR {
            IRQ_READINESS_OK
        } else {
            IRQ_RUNTIME_BLOCKED
        },
        eoi_strategy: IRQ_READINESS_OK,
        pic_remap: IRQ_PIC_REMAP_CONTROLLED_SMOKE_ONLY,
        sti: plan[0].interrupts,
        keyboard_fallback: disabled_bind.keyboard_input,
        ready_for_runtime_irq: IRQ_RUNTIME_READY_NO,
    }
}

/// Returns risk telemetry for why runtime IRQ activation remains blocked.
pub fn irq_runtime_risk() -> IrqRuntimeRisk {
    IrqRuntimeRisk {
        runtime_irq: IRQ_RUNTIME_BLOCKED,
        reason: IRQ_RUNTIME_RISK_REASON,
        required_before_enable: IRQ_RUNTIME_REQUIRED_BEFORE_ENABLE,
        sti_allowed: IRQ_STI_ALLOWED_NO,
    }
}

/// Returns preflight telemetry without installing IRQ gates or touching hardware.
pub fn irq_runtime_preflight() -> IrqRuntimePreflight {
    let disabled_bind = bind_irq_gates_disabled();

    IrqRuntimePreflight {
        idt_exceptions: IRQ_PREFLIGHT_PASS,
        irq_vectors: disabled_bind.irq0_state,
        bind_path: IRQ_IDT_BINDING_DISABLED,
        eoi_dispatch: IRQ_EOI_DISPATCH_DISABLED,
        pic_remap: IRQ_PIC_REMAP_CONTROLLED_SMOKE_ONLY,
        keyboard_fallback: disabled_bind.keyboard_input,
        pf_smoke: IRQ_PF_SMOKE_UNCHANGED,
        result: IRQ_RUNTIME_BLOCKED,
    }
}

/// Arms the explicit command-only IRQ gate bind smoke path.
pub fn irq_gate_bind_smoke_arm() -> IrqGateBindSmokeArmStatus {
    unsafe {
        IRQ_GATE_BIND_SMOKE_ARMED = true;
    }

    IrqGateBindSmokeArmStatus {
        mode: IRQ_GATE_BIND_SMOKE_MODE_CONTROLLED,
        next: IRQ_GATE_BIND_SMOKE_NEXT,
        interrupts: IRQ_INTERRUPTS_DISABLED,
        pic_irq_mask: IRQ_PIC_IRQ_MASK_MASKED,
        eoi_dispatch: IRQ_EOI_DISPATCH_DISABLED,
    }
}

/// Returns whether the controlled IRQ gate bind smoke path is armed.
pub fn irq_gate_bind_smoke_is_armed() -> bool {
    unsafe { IRQ_GATE_BIND_SMOKE_ARMED }
}

/// Arms the runtime IRQ activation safety latch.
pub fn irq_runtime_arm() {
    unsafe {
        IRQ_RUNTIME_ARMED = true;
    }
}

/// Returns whether the runtime IRQ activation safety latch is armed.
pub fn irq_runtime_is_armed() -> bool {
    unsafe { IRQ_RUNTIME_ARMED }
}

/// Commits the runtime IRQ activation if armed.
pub fn irq_runtime_commit() {
    unsafe {
        if IRQ_RUNTIME_ARMED {
            IRQ_RUNTIME_COMMITTED = true;
            IRQ_RUNTIME_ARMED = false;
        }
    }
}

/// Checks if PIC remap precondition is satisfied.
pub fn irq_runtime_check_pic_remap_precondition() -> bool {
    // Need to use pic module, but we can't import it here due to circular deps
    // Instead, we'll mark this via a getter function from main that passes state
    false // Will be properly checked in main
}

/// Checks if IRQ gate bind precondition is satisfied.
pub fn irq_runtime_check_irq_gate_bind_precondition() -> bool {
    let status = irq_gate_bind_smoke_status();
    status.executed
}

/// Checks if all critical preconditions are met for runtime commitment.
/// This function must be called by the dispatcher with external state.
pub fn irq_runtime_check_all_preconditions(pic_remap_executed: bool) -> bool {
    let gate_bind_ok = irq_runtime_check_irq_gate_bind_precondition();
    let pic_remap_ok = pic_remap_executed;
    // EOI dispatch and STI are always "disabled" in v9.1.0, so they block but are documented
    // Keyboard and pf-smoke are always "ok" in v9.1.0
    gate_bind_ok && pic_remap_ok
}

/// Checks if all critical preconditions are met for EOI dispatch (v9.2.0).
/// This function must be called by the dispatcher with external state.
pub fn eoi_runtime_check_all_preconditions(pic_remap_executed: bool) -> bool {
    let gate_bind_ok = irq_runtime_check_irq_gate_bind_precondition();
    let pic_remap_ok = pic_remap_executed;
    // In v9.2.0: EOI boundary definition, not activation
    // All preconditions must be satisfied for EOI to be "ready" (but still disabled)
    gate_bind_ok && pic_remap_ok
}

/// Returns whether the runtime IRQ activation has been committed.
pub fn irq_runtime_is_committed() -> bool {
    unsafe { IRQ_RUNTIME_COMMITTED }
}

/// Records a successful command-path IRQ gate smoke bind after IDT entries are installed.
pub fn irq_gate_bind_smoke_mark_bound() -> IrqGateBindSmokeResult {
    unsafe {
        IRQ_GATE_BIND_SMOKE_ARMED = false;
        IRQ_GATE_BIND_SMOKE_EXECUTED = true;
    }

    IrqGateBindSmokeResult {
        guard: IRQ_GATE_BIND_SMOKE_GUARD_ARMED,
        irq0_vector_state: IRQ0_SMOKE_STUB_BINDING,
        irq1_vector_state: IRQ1_SMOKE_STUB_BINDING,
        pic_irq_mask: IRQ_PIC_IRQ_MASK_MASKED,
        sti: IRQ_INTERRUPTS_DISABLED,
        eoi_dispatch: IRQ_EOI_DISPATCH_DISABLED,
        keyboard_input: IRQ_KEYBOARD_INPUT_POLLING_ONLY,
        result: IRQ_GATE_BIND_RESULT_BOUND_DORMANT,
        next: None,
    }
}

/// Returns a blocked IRQ gate bind smoke result without touching IDT entries.
pub fn irq_gate_bind_smoke_blocked() -> IrqGateBindSmokeResult {
    IrqGateBindSmokeResult {
        guard: IRQ_GATE_BIND_SMOKE_GUARD_NOT_ARMED,
        irq0_vector_state: IRQ_VECTOR_UNBOUND,
        irq1_vector_state: IRQ_VECTOR_UNBOUND,
        pic_irq_mask: IRQ_PIC_IRQ_MASK_MASKED,
        sti: IRQ_INTERRUPTS_DISABLED,
        eoi_dispatch: IRQ_EOI_DISPATCH_DISABLED,
        keyboard_input: IRQ_KEYBOARD_INPUT_POLLING_ONLY,
        result: IRQ_GATE_BIND_RESULT_BLOCKED,
        next: Some(IRQ_GATE_BIND_ARM_NEXT),
    }
}

/// Returns current controlled IRQ gate bind smoke status without touching hardware.
pub fn irq_gate_bind_smoke_status() -> IrqGateBindSmokeStatus {
    let executed = unsafe { IRQ_GATE_BIND_SMOKE_EXECUTED };

    IrqGateBindSmokeStatus {
        armed: unsafe { IRQ_GATE_BIND_SMOKE_ARMED },
        executed,
        irq0_vector: IRQ0_VECTOR,
        irq0_vector_state: if executed {
            IRQ_VECTOR_BOUND
        } else {
            IRQ_VECTOR_UNBOUND
        },
        irq1_vector: IRQ1_VECTOR,
        irq1_vector_state: if executed {
            IRQ_VECTOR_BOUND
        } else {
            IRQ_VECTOR_UNBOUND
        },
        irq0_active_handler: IRQ_SMOKE_STUB_DORMANT,
        irq1_active_handler: IRQ_SMOKE_STUB_DORMANT,
        pic_irq_mask: IRQ_PIC_IRQ_MASK_MASKED,
        sti: IRQ_INTERRUPTS_DISABLED,
        eoi_dispatch: IRQ_EOI_DISPATCH_DISABLED,
        keyboard_input: IRQ_KEYBOARD_INPUT_POLLING_ONLY,
    }
}

/// Returns read-only IRQ gate bind state telemetry without touching hardware.
pub fn irq_gate_bind_state() -> IrqGateBindStateTelemetry {
    let status = irq_gate_bind_smoke_status();

    IrqGateBindStateTelemetry {
        armed: status.armed,
        executed: status.executed,
        irq0_vector: status.irq0_vector,
        irq0_vector_state: status.irq0_vector_state,
        irq1_vector: status.irq1_vector,
        irq1_vector_state: status.irq1_vector_state,
        irq0_active_handler: status.irq0_active_handler,
        irq1_active_handler: status.irq1_active_handler,
        bind_expected: IRQ_GATE_BIND_EXPECTED,
        bind_applied: if status.executed {
            IRQ_GATE_BIND_YES
        } else {
            IRQ_GATE_BIND_NO
        },
        irq_runtime: IRQ_GATE_BIND_IRQ_RUNTIME_DISABLED,
        pic_irq_mask: status.pic_irq_mask,
        sti: status.sti,
        eoi_dispatch: status.eoi_dispatch,
        keyboard_input: status.keyboard_input,
    }
}

/// Returns read-only IRQ gate bind command history telemetry without touching hardware.
pub fn irq_gate_bind_history() -> IrqGateBindHistoryTelemetry {
    let status = irq_gate_bind_smoke_status();

    IrqGateBindHistoryTelemetry {
        arm_command: IRQ_GATE_BIND_ARM_COMMAND_AVAILABLE,
        smoke_command: IRQ_GATE_BIND_SMOKE_COMMAND_AVAILABLE,
        last_smoke_executed: if status.executed {
            IRQ_GATE_BIND_YES
        } else {
            IRQ_GATE_BIND_NO
        },
        idt_binds: IRQ_GATE_BIND_IDT_BINDS_CONTROLLED_ONLY,
        boot_bind: IRQ_GATE_BIND_BOOT_BIND_NO,
    }
}

/// Returns read-only IRQ gate bind preflight telemetry without touching hardware.
pub fn irq_gate_bind_preflight() -> IrqGateBindPreflightTelemetry {
    let status = irq_gate_bind_smoke_status();

    IrqGateBindPreflightTelemetry {
        guard: IRQ_GATE_BIND_GUARD_COMMAND_ARMED_REQUIRED,
        bind_path: IRQ_GATE_BIND_BIND_PATH_READY,
        irq0_vector: status.irq0_vector,
        irq0_vector_state: if status.executed {
            IRQ_VECTOR_BOUND
        } else {
            IRQ_VECTOR_UNBOUND
        },
        irq1_vector: status.irq1_vector,
        irq1_vector_state: if status.executed {
            IRQ_VECTOR_BOUND
        } else {
            IRQ_VECTOR_UNBOUND
        },
        pic_irq_mask: status.pic_irq_mask,
        sti: status.sti,
        eoi_dispatch: status.eoi_dispatch,
        keyboard_input: status.keyboard_input,
        result: IRQ_GATE_BIND_RESULT_TELEMETRY_ONLY,
    }
}

/// Structured report of all IRQ unmask activation blockers (v9.3.0).
///
/// Each field reflects whether the corresponding subsystem precondition
/// is satisfied. In v9.3.0 all blockers are active (all_clear = false).
#[derive(Copy, Clone, Debug)]
pub struct IrqMaskBlockerReport {
    /// PIC remap controlled smoke has been executed.
    pub pic_remap_ready: bool,
    /// IRQ gate bind smoke has been executed (vectors 32/33 bound).
    pub irq_gates_ready: bool,
    /// STI has been enabled. Hardcoded false in v9.3.0.
    pub sti_ready: bool,
    /// EOI dispatch is active. Hardcoded false in v9.3.0.
    pub eoi_dispatch_ready: bool,
    /// IRQ runtime has been committed.
    pub irq_runtime_committed: bool,
    /// True only when every field above is true. Always false in v9.3.0.
    pub all_clear: bool,
}

/// Builds an `IrqMaskBlockerReport` from external state passed by the dispatcher.
///
/// `sti_ready` and `eoi_dispatch_ready` are hardcoded `false` in v9.3.0
/// because neither STI nor EOI dispatch is enabled at this milestone.
pub fn irq_mask_blocker_report(
    pic_remap_executed: bool,
    irq_gates_bound: bool,
    irq_runtime_committed: bool,
) -> IrqMaskBlockerReport {
    // v9.3.0: STI and EOI dispatch remain disabled by invariant.
    let sti_ready = false;
    let eoi_dispatch_ready = false;
    let all_clear = pic_remap_executed
        && irq_gates_bound
        && sti_ready
        && eoi_dispatch_ready
        && irq_runtime_committed;
    IrqMaskBlockerReport {
        pic_remap_ready: pic_remap_executed,
        irq_gates_ready: irq_gates_bound,
        sti_ready,
        eoi_dispatch_ready,
        irq_runtime_committed,
        all_clear,
    }
}

/// Returns `true` only when every blocker in `report` is cleared.
///
/// Convenience helper so callers can pass the struct around and query
/// the aggregate result without re-reading individual fields.
pub fn irq_mask_check_all_blockers(report: &IrqMaskBlockerReport) -> bool {
    report.all_clear
}

/// Builds the v9.4.0 aggregate runtime readiness matrix from dispatcher state.
pub fn irq_runtime_matrix(
    pic_remap_executed: bool,
    irq_gates_bound: bool,
    eoi_runtime_ready: bool,
    pic_mask_policy: &'static str,
    runtime_armed: bool,
    runtime_committed: bool,
) -> IrqRuntimeMatrix {
    let runtime_latch = if runtime_committed {
        IRQ_MATRIX_RUNTIME_LATCH_COMMITTED_DRY_RUN
    } else if runtime_armed {
        IRQ_MATRIX_RUNTIME_LATCH_ARMED
    } else {
        IRQ_MATRIX_RUNTIME_LATCH_BLOCKED
    };
    IrqRuntimeMatrix {
        pic_remap_smoke: if pic_remap_executed {
            IRQ_MATRIX_YES
        } else {
            IRQ_MATRIX_NO
        },
        irq_gate_bind_smoke: if irq_gates_bound {
            IRQ_MATRIX_YES
        } else {
            IRQ_MATRIX_NO
        },
        eoi_runtime_boundary: if eoi_runtime_ready {
            IRQ_MATRIX_EOI_READY_DRY_RUN
        } else {
            IRQ_MATRIX_EOI_DISABLED
        },
        pic_mask_policy,
        unmask_policy: IRQ_MATRIX_UNMASK_POLICY_NO_UNMASK,
        runtime_latch,
        keyboard_mode: IRQ_MATRIX_KEYBOARD_MODE_POLLING,
        sti: IRQ_MATRIX_STI_DISABLED,
        runtime_irq_active: IRQ_MATRIX_RUNTIME_IRQ_ACTIVE_NO,
        smoke_prerequisites: if pic_remap_executed && irq_gates_bound {
            IRQ_MATRIX_YES
        } else {
            IRQ_MATRIX_NO
        },
    }
}

/// Derives the v9.5.0 activation dry-run decision from the matrix only.
pub fn irq_runtime_activation_dry_run(matrix: &IrqRuntimeMatrix) -> IrqRuntimeActivationDryRun {
    core::hint::black_box(matrix);
    IrqRuntimeActivationDryRun {
        allowed: false,
        allowed_text: IRQ_ACTIVATION_DRY_RUN_ALLOWED_NO,
        result: IRQ_ACTIVATION_COMMIT_RESULT_BLOCKED,
        next: IRQ_ACTIVATION_PLAN_NEXT,
    }
}

/// Returns telemetry-only activation token state.
pub fn irq_runtime_activation_token_status() -> IrqRuntimeActivationTokenTelemetry {
    let token_present = unsafe { IRQ_RUNTIME_ACTIVATION_TOKEN_PRESENT };
    IrqRuntimeActivationTokenTelemetry {
        token_state: if token_present {
            IRQ_ACTIVATION_TOKEN_PRESENT
        } else {
            IRQ_ACTIVATION_TOKEN_ABSENT
        },
        token_scope: IRQ_ACTIVATION_TOKEN_SCOPE,
        hardware_mutation: IRQ_ACTIVATION_TOKEN_HARDWARE_MUTATION_NO,
        sti: IRQ_MATRIX_STI_DISABLED,
        pic_unmask: IRQ_ACTIVATION_TOKEN_PIC_UNMASK_NO,
        live_irq0_irq1: IRQ_ACTIVATION_TOKEN_LIVE_IRQ_NO,
        runtime_eoi_dispatch: IRQ_MATRIX_EOI_DISABLED,
        keyboard_mode: IRQ_MATRIX_KEYBOARD_MODE_POLLING,
    }
}

/// Arms only the activation token telemetry flag.
pub fn irq_runtime_activation_token_arm() -> IrqRuntimeActivationTokenTelemetry {
    unsafe {
        IRQ_RUNTIME_ACTIVATION_TOKEN_PRESENT = true;
    }
    irq_runtime_activation_token_status()
}

/// Clears only the activation token telemetry flag.
pub fn irq_runtime_activation_token_clear() -> IrqRuntimeActivationTokenTelemetry {
    unsafe {
        IRQ_RUNTIME_ACTIVATION_TOKEN_PRESENT = false;
    }
    irq_runtime_activation_token_status()
}

/// Derives the v9.7.0 controlled activation gate without mutating runtime state.
pub fn irq_runtime_activation_gate(
    token: IrqRuntimeActivationTokenTelemetry,
    matrix: IrqRuntimeMatrix,
    activation: IrqRuntimeActivationDryRun,
    eoi_runtime_ready: bool,
    pic_mask_policy: &'static str,
    unmask_policy: &'static str,
) -> IrqRuntimeActivationGate {
    IrqRuntimeActivationGate {
        token_gate: token.token_state,
        readiness_matrix: IRQ_ACTIVATION_GATE_READINESS_BLOCKED,
        dry_run_commit_allowed: activation.allowed_text,
        eoi_runtime_boundary: if eoi_runtime_ready {
            IRQ_MATRIX_EOI_READY_DRY_RUN
        } else {
            IRQ_MATRIX_EOI_DISABLED
        },
        pic_mask_policy,
        unmask_policy,
        hardware_mutation: IRQ_ACTIVATION_TOKEN_HARDWARE_MUTATION_NO,
        activation_allowed: IRQ_ACTIVATION_GATE_ALLOWED_NO,
        runtime_irq_active: matrix.runtime_irq_active,
        result: IRQ_ACTIVATION_GATE_RESULT_BLOCKED,
        next: IRQ_ACTIVATION_GATE_NEXT_BLOCKERS,
    }
}

/// Derives the v9.8.0 controlled activation simulation without mutating runtime state.
pub fn irq_runtime_activation_simulation(
    token: IrqRuntimeActivationTokenTelemetry,
    matrix: IrqRuntimeMatrix,
    activation: IrqRuntimeActivationDryRun,
    gate: IrqRuntimeActivationGate,
) -> IrqRuntimeActivationSimulation {
    IrqRuntimeActivationSimulation {
        token_gate: token.token_state,
        readiness_matrix: gate.readiness_matrix,
        gate_decision: gate.result,
        dry_run_commit_allowed: activation.allowed_text,
        eoi_runtime_boundary: gate.eoi_runtime_boundary,
        hardware_mutation: IRQ_ACTIVATION_TOKEN_HARDWARE_MUTATION_NO,
        simulated_activation_allowed: IRQ_ACTIVATION_SIM_ALLOWED_NO,
        sti_would_enable: IRQ_ACTIVATION_SIM_STI_WOULD_ENABLE_NO,
        pic_unmask_would_apply: IRQ_ACTIVATION_SIM_PIC_UNMASK_WOULD_APPLY_NO,
        eoi_dispatch_would_enable: IRQ_ACTIVATION_SIM_EOI_DISPATCH_WOULD_ENABLE_NO,
        keyboard_mode: matrix.keyboard_mode,
        runtime_irq_active: matrix.runtime_irq_active,
        result: IRQ_ACTIVATION_SIM_RESULT_BLOCKED,
        next: IRQ_ACTIVATION_SIM_NEXT_BLOCKERS,
    }
}

/// Derives the v9.9.0 controlled STI plan without enabling CPU interrupts.
pub fn sti_controlled_activation_plan(
    token: IrqRuntimeActivationTokenTelemetry,
    matrix: IrqRuntimeMatrix,
    gate: IrqRuntimeActivationGate,
    simulation: IrqRuntimeActivationSimulation,
) -> StiControlledActivationPlan {
    StiControlledActivationPlan {
        activation_token: token.token_state,
        token_gate: simulation.token_gate,
        runtime_gate: gate.result,
        readiness_matrix: gate.readiness_matrix,
        simulation: simulation.result,
        eoi_runtime_boundary: gate.eoi_runtime_boundary,
        pic_unmask_policy: gate.unmask_policy,
        pic_unmask: STI_PLAN_PIC_UNMASK_DISABLED,
        eoi_dispatch: STI_PLAN_EOI_DISPATCH_DISABLED,
        hardware_mutation: simulation.hardware_mutation,
        keyboard_mode: matrix.keyboard_mode,
        sti_instruction: matrix.sti,
        sti_allowed: STI_PLAN_ALLOWED_NO,
        runtime_irq_active: matrix.runtime_irq_active,
        result: STI_PLAN_RESULT_BLOCKED,
        next: STI_PLAN_NEXT_BLOCKERS,
    }
}

/// Derives the v10.0.0 activation smoke foundation without mutating runtime state.
pub fn irq_runtime_activation_smoke(
    token: IrqRuntimeActivationTokenTelemetry,
    matrix: IrqRuntimeMatrix,
    gate: IrqRuntimeActivationGate,
    simulation: IrqRuntimeActivationSimulation,
    sti_plan: StiControlledActivationPlan,
) -> IrqRuntimeActivationSmoke {
    IrqRuntimeActivationSmoke {
        activation_token: token.token_state,
        runtime_gate: gate.result,
        readiness_matrix: gate.readiness_matrix,
        simulation: simulation.result,
        sti_plan: sti_plan.result,
        eoi_runtime_boundary: sti_plan.eoi_runtime_boundary,
        pic_unmask: sti_plan.pic_unmask,
        eoi_dispatch: sti_plan.eoi_dispatch,
        hardware_mutation: IRQ_ACTIVATION_TOKEN_HARDWARE_MUTATION_NO,
        keyboard_mode: matrix.keyboard_mode,
        sti_instruction: matrix.sti,
        runtime_irq_active: matrix.runtime_irq_active,
        activation_smoke: IRQ_ACTIVATION_SMOKE_BLOCKED,
        result: IRQ_ACTIVATION_SMOKE_RESULT_BLOCKED,
        next: IRQ_ACTIVATION_SMOKE_NEXT_BLOCKERS,
    }
}

/// Derives the v10.1.0 controlled EOI dispatch smoke without dispatching EOI.
pub fn eoi_dispatch_smoke(
    pic_remap_executed: bool,
    irq_gates_bound: bool,
    matrix: IrqRuntimeMatrix,
    smoke: IrqRuntimeActivationSmoke,
) -> EoiDispatchSmoke {
    EoiDispatchSmoke {
        eoi_dispatch_smoke: EOI_DISPATCH_SMOKE_BLOCKED,
        dispatch_mode: EOI_DISPATCH_SMOKE_MODE_DRY_RUN,
        pic_remap_smoke: if pic_remap_executed {
            "controlled smoke available"
        } else {
            "not ready"
        },
        irq_gates: if irq_gates_bound {
            "bound"
        } else {
            "not bound"
        },
        pic_eoi_writes: EOI_DISPATCH_SMOKE_ACK_WRITES_DISABLED,
        sti_instruction: matrix.sti,
        pic_unmask: smoke.pic_unmask,
        keyboard_mode: matrix.keyboard_mode,
        runtime_irq_active: matrix.runtime_irq_active,
        hardware_mutation: smoke.hardware_mutation,
        master_eoi_route: EOI_DISPATCH_SMOKE_MASTER_ROUTE,
        slave_eoi_route: EOI_DISPATCH_SMOKE_SLAVE_ROUTE,
        result: EOI_DISPATCH_SMOKE_RESULT_DRY_RUN_ONLY,
    }
}

/// Derives the v10.2.0 controlled PIC unmask smoke without writing PIC masks.
pub fn pic_unmask_smoke(
    pic_mask_policy: &'static str,
    unmask_policy: &'static str,
    token: IrqRuntimeActivationTokenTelemetry,
    matrix: IrqRuntimeMatrix,
    gate: IrqRuntimeActivationGate,
    sti_plan: StiControlledActivationPlan,
    eoi_smoke: EoiDispatchSmoke,
) -> PicUnmaskSmoke {
    PicUnmaskSmoke {
        pic_unmask_smoke: PIC_UNMASK_SMOKE_BLOCKED,
        dispatch_mode: PIC_UNMASK_SMOKE_MODE_DRY_RUN,
        target_irq_lines: PIC_UNMASK_SMOKE_TARGET_IRQ_LINES_NONE,
        pic_mask_policy,
        unmask_policy,
        activation_token: token.token_state,
        activation_gate: gate.result,
        eoi_runtime_boundary: gate.eoi_runtime_boundary,
        sti_plan: sti_plan.result,
        sti_instruction: matrix.sti,
        eoi_dispatch_smoke: eoi_smoke.eoi_dispatch_smoke,
        live_unmask: PIC_UNMASK_SMOKE_LIVE_UNMASK_NO,
        pic_data_writes: PIC_UNMASK_SMOKE_DATA_WRITES_DISABLED,
        hardware_mutation: IRQ_ACTIVATION_TOKEN_HARDWARE_MUTATION_NO,
        runtime_irq_active: matrix.runtime_irq_active,
        keyboard_mode: matrix.keyboard_mode,
        result: PIC_UNMASK_SMOKE_RESULT_DRY_RUN_ONLY,
    }
}

/// Derives the v10.3.0 controlled IDT runtime bind smoke without binding handlers.
pub fn idt_runtime_bind_smoke(
    token: IrqRuntimeActivationTokenTelemetry,
    matrix: IrqRuntimeMatrix,
    gate: IrqRuntimeActivationGate,
    gate_state: IrqGateBindStateTelemetry,
    sti_plan: StiControlledActivationPlan,
    eoi_smoke: EoiDispatchSmoke,
    pic_unmask_smoke: PicUnmaskSmoke,
) -> IdtRuntimeBindSmoke {
    IdtRuntimeBindSmoke {
        idt_runtime_bind_smoke: IDT_RUNTIME_BIND_SMOKE_BLOCKED,
        dispatch_mode: IDT_RUNTIME_BIND_SMOKE_MODE_DRY_RUN,
        target_vectors: IDT_RUNTIME_BIND_SMOKE_TARGET_VECTORS,
        activation_token: token.token_state,
        activation_gate: gate.result,
        irq_gate_bind_smoke: if gate_state.executed {
            "bound"
        } else {
            "not bound"
        },
        eoi_dispatch_smoke: eoi_smoke.eoi_dispatch_smoke,
        pic_unmask_smoke: pic_unmask_smoke.pic_unmask_smoke,
        sti_plan: sti_plan.result,
        sti_instruction: matrix.sti,
        live_handler_bind: IDT_RUNTIME_BIND_SMOKE_LIVE_HANDLER_BIND_NO,
        hardware_mutation: IRQ_ACTIVATION_TOKEN_HARDWARE_MUTATION_NO,
        runtime_irq_active: matrix.runtime_irq_active,
        keyboard_mode: matrix.keyboard_mode,
        result: IDT_RUNTIME_BIND_SMOKE_RESULT_DRY_RUN_ONLY,
    }
}

/// Derives the v10.4.0 final IRQ runtime gate without mutating runtime state.
pub fn irq_runtime_final_gate(
    token: IrqRuntimeActivationTokenTelemetry,
    matrix: IrqRuntimeMatrix,
    gate: IrqRuntimeActivationGate,
    simulation: IrqRuntimeActivationSimulation,
    sti_plan: StiControlledActivationPlan,
    activation_smoke: IrqRuntimeActivationSmoke,
    eoi_smoke: EoiDispatchSmoke,
    pic_unmask_smoke: PicUnmaskSmoke,
    idt_bind_smoke: IdtRuntimeBindSmoke,
) -> IrqRuntimeFinalGate {
    IrqRuntimeFinalGate {
        scope: IRQ_RUNTIME_FINAL_GATE_SCOPE,
        inputs: IRQ_RUNTIME_FINAL_GATE_INPUTS,
        activation_token: token.token_state,
        activation_gate: gate.result,
        readiness_matrix: gate.readiness_matrix,
        simulation: simulation.result,
        sti_plan: sti_plan.result,
        activation_smoke: activation_smoke.activation_smoke,
        eoi_dispatch_smoke: eoi_smoke.eoi_dispatch_smoke,
        pic_unmask_smoke: pic_unmask_smoke.pic_unmask_smoke,
        idt_runtime_bind_smoke: idt_bind_smoke.idt_runtime_bind_smoke,
        keyboard_mode: matrix.keyboard_mode,
        final_activation_allowed: IRQ_RUNTIME_FINAL_GATE_ALLOWED_NO,
        hardware_mutation: IRQ_ACTIVATION_TOKEN_HARDWARE_MUTATION_NO,
        runtime_irq_active: matrix.runtime_irq_active,
        sti_instruction: matrix.sti,
        pic_unmask: sti_plan.pic_unmask,
        eoi_dispatch: sti_plan.eoi_dispatch,
        live_idt_bind: IRQ_RUNTIME_FINAL_GATE_LIVE_IDT_BIND_NO,
        result: IRQ_RUNTIME_FINAL_GATE_RESULT_BLOCKED,
        next: IRQ_RUNTIME_FINAL_GATE_NEXT_NONE,
    }
}

/// Freezes the controlled activation decision without mutating runtime state.
pub fn irq_runtime_decision_freeze(
    final_gate: IrqRuntimeFinalGate,
    activation_smoke: IrqRuntimeActivationSmoke,
    simulation: IrqRuntimeActivationSimulation,
    sti_plan: StiControlledActivationPlan,
    eoi_smoke: EoiDispatchSmoke,
    pic_unmask_smoke: PicUnmaskSmoke,
    idt_bind_smoke: IdtRuntimeBindSmoke,
) -> IrqRuntimeActivationDecision {
    IrqRuntimeActivationDecision {
        scope: IRQ_RUNTIME_DECISION_SCOPE,
        inputs: IRQ_RUNTIME_DECISION_INPUTS,
        activation_decision: IRQ_RUNTIME_DECISION_FROZEN_BLOCKED,
        final_activation_allowed: final_gate.final_activation_allowed,
        runtime_irq_active: final_gate.runtime_irq_active,
        hardware_mutation: final_gate.hardware_mutation,
        sti_instruction: final_gate.sti_instruction,
        pic_unmask: final_gate.pic_unmask,
        eoi_dispatch: final_gate.eoi_dispatch,
        live_idt_bind: final_gate.live_idt_bind,
        keyboard_mode: final_gate.keyboard_mode,
        activation_smoke: activation_smoke.activation_smoke,
        simulation: simulation.result,
        sti_plan: sti_plan.result,
        eoi_dispatch_smoke: eoi_smoke.eoi_dispatch_smoke,
        pic_unmask_smoke: pic_unmask_smoke.pic_unmask_smoke,
        idt_runtime_bind_smoke: idt_bind_smoke.idt_runtime_bind_smoke,
        activation_token: final_gate.activation_token,
        activation_gate: final_gate.activation_gate,
        readiness_matrix: final_gate.readiness_matrix,
    }
}

/// Summarizes future hardware mutation readiness without mutating runtime state.
pub fn irq_runtime_mutation_check(
    decision: IrqRuntimeActivationDecision,
    final_gate: IrqRuntimeFinalGate,
    activation_smoke: IrqRuntimeActivationSmoke,
    sti_plan: StiControlledActivationPlan,
    eoi_smoke: EoiDispatchSmoke,
    pic_unmask_smoke: PicUnmaskSmoke,
    idt_bind_smoke: IdtRuntimeBindSmoke,
) -> IrqRuntimeHardwareMutationChecklist {
    IrqRuntimeHardwareMutationChecklist {
        scope: IRQ_RUNTIME_MUTATION_SCOPE,
        inputs: IRQ_RUNTIME_MUTATION_INPUTS,
        hardware_mutation_ready: IRQ_RUNTIME_MUTATION_READY_NO,
        activation_decision: decision.activation_decision,
        final_activation_allowed: final_gate.final_activation_allowed,
        runtime_irq_active: decision.runtime_irq_active,
        sti_mutation: IRQ_RUNTIME_MUTATION_DISABLED,
        pic_unmask_mutation: IRQ_RUNTIME_MUTATION_DISABLED,
        eoi_dispatch_mutation: IRQ_RUNTIME_MUTATION_DISABLED,
        idt_live_bind_mutation: IRQ_RUNTIME_MUTATION_DISABLED,
        keyboard_input_mutation: IRQ_RUNTIME_MUTATION_DISABLED,
        activation_smoke: activation_smoke.activation_smoke,
        sti_plan: sti_plan.result,
        eoi_dispatch_smoke: eoi_smoke.eoi_dispatch_smoke,
        pic_unmask_smoke: pic_unmask_smoke.pic_unmask_smoke,
        idt_runtime_bind_smoke: idt_bind_smoke.idt_runtime_bind_smoke,
        activation_token: decision.activation_token,
        activation_gate: decision.activation_gate,
        readiness_matrix: decision.readiness_matrix,
        keyboard_mode: decision.keyboard_mode,
    }
}

/// Orders future mutation smoke steps without enabling any mutation path.
pub fn irq_runtime_mutation_sequence(
    mutation: IrqRuntimeHardwareMutationChecklist,
    decision: IrqRuntimeActivationDecision,
    final_gate: IrqRuntimeFinalGate,
    activation_smoke: IrqRuntimeActivationSmoke,
    sti_plan: StiControlledActivationPlan,
    eoi_smoke: EoiDispatchSmoke,
    pic_unmask_smoke: PicUnmaskSmoke,
    idt_bind_smoke: IdtRuntimeBindSmoke,
) -> IrqRuntimeMutationSmokeSequence {
    IrqRuntimeMutationSmokeSequence {
        scope: IRQ_RUNTIME_MUTATION_SEQUENCE_SCOPE,
        inputs: IRQ_RUNTIME_MUTATION_SEQUENCE_INPUTS,
        mutation_sequence_ready: IRQ_RUNTIME_MUTATION_SEQUENCE_READY_NO,
        hardware_mutation: final_gate.hardware_mutation,
        runtime_irq_active: final_gate.runtime_irq_active,
        next_mutation_step: IRQ_RUNTIME_MUTATION_SEQUENCE_NEXT_NONE,
        allowed_mutation_steps: IRQ_RUNTIME_MUTATION_SEQUENCE_ALLOWED_NONE,
        sti_instruction: final_gate.sti_instruction,
        pic_unmask: final_gate.pic_unmask,
        eoi_dispatch: final_gate.eoi_dispatch,
        live_idt_bind: final_gate.live_idt_bind,
        keyboard_mode: final_gate.keyboard_mode,
        hardware_mutation_ready: mutation.hardware_mutation_ready,
        activation_decision: decision.activation_decision,
        final_activation_allowed: final_gate.final_activation_allowed,
        activation_smoke: activation_smoke.activation_smoke,
        sti_plan: sti_plan.result,
        eoi_dispatch_smoke: eoi_smoke.eoi_dispatch_smoke,
        pic_unmask_smoke: pic_unmask_smoke.pic_unmask_smoke,
        idt_runtime_bind_smoke: idt_bind_smoke.idt_runtime_bind_smoke,
        activation_token: mutation.activation_token,
        activation_gate: mutation.activation_gate,
        readiness_matrix: mutation.readiness_matrix,
    }
}

/// Preflights the first future PIC EOI write without touching hardware.
pub fn eoi_write_smoke_preflight(
    sequence: IrqRuntimeMutationSmokeSequence,
    mutation: IrqRuntimeHardwareMutationChecklist,
    decision: IrqRuntimeActivationDecision,
    final_gate: IrqRuntimeFinalGate,
    sti_plan: StiControlledActivationPlan,
    eoi_smoke: EoiDispatchSmoke,
    pic_unmask_smoke: PicUnmaskSmoke,
    idt_bind_smoke: IdtRuntimeBindSmoke,
) -> EoiWriteSmokePreflight {
    EoiWriteSmokePreflight {
        scope: EOI_WRITE_SMOKE_PREFLIGHT_SCOPE,
        inputs: EOI_WRITE_SMOKE_PREFLIGHT_INPUTS,
        eoi_write_smoke_preflight: EOI_WRITE_SMOKE_PREFLIGHT_BLOCKED,
        first_pic_eoi_write_allowed: EOI_WRITE_SMOKE_PREFLIGHT_FIRST_WRITE_ALLOWED_NO,
        hardware_mutation: final_gate.hardware_mutation,
        runtime_irq_active: final_gate.runtime_irq_active,
        target_command_port: EOI_WRITE_SMOKE_PREFLIGHT_TARGET_NONE,
        target_irq_line: EOI_WRITE_SMOKE_PREFLIGHT_TARGET_NONE,
        eoi_dispatch: final_gate.eoi_dispatch,
        sti_instruction: final_gate.sti_instruction,
        pic_unmask: final_gate.pic_unmask,
        live_idt_bind: final_gate.live_idt_bind,
        keyboard_mode: final_gate.keyboard_mode,
        mutation_sequence_ready: sequence.mutation_sequence_ready,
        hardware_mutation_ready: mutation.hardware_mutation_ready,
        activation_decision: decision.activation_decision,
        final_activation_allowed: final_gate.final_activation_allowed,
        eoi_dispatch_smoke: eoi_smoke.eoi_dispatch_smoke,
        pic_unmask_smoke: pic_unmask_smoke.pic_unmask_smoke,
        idt_runtime_bind_smoke: idt_bind_smoke.idt_runtime_bind_smoke,
        sti_plan: sti_plan.result,
    }
}

/// Models the first controlled PIC EOI write candidate without touching hardware.
pub fn eoi_write_smoke_candidate(
    preflight: EoiWriteSmokePreflight,
    sequence: IrqRuntimeMutationSmokeSequence,
    mutation: IrqRuntimeHardwareMutationChecklist,
    decision: IrqRuntimeActivationDecision,
    final_gate: IrqRuntimeFinalGate,
) -> EoiWriteSmokeCandidate {
    EoiWriteSmokeCandidate {
        scope: EOI_WRITE_SMOKE_CANDIDATE_SCOPE,
        inputs: EOI_WRITE_SMOKE_CANDIDATE_INPUTS,
        eoi_write_smoke_candidate: EOI_WRITE_SMOKE_CANDIDATE_BLOCKED,
        candidate_armed: EOI_WRITE_SMOKE_CANDIDATE_ARMED_NO,
        fire_result: EOI_WRITE_SMOKE_CANDIDATE_FIRE_DRY_RUN_BLOCKED,
        first_pic_eoi_write_performed: EOI_WRITE_SMOKE_CANDIDATE_WRITE_PERFORMED_NO,
        hardware_mutation: final_gate.hardware_mutation,
        runtime_irq_active: final_gate.runtime_irq_active,
        target_command_port: EOI_WRITE_SMOKE_CANDIDATE_TARGET_NONE,
        target_irq_line: EOI_WRITE_SMOKE_CANDIDATE_TARGET_NONE,
        eoi_dispatch: final_gate.eoi_dispatch,
        sti_instruction: final_gate.sti_instruction,
        pic_unmask: final_gate.pic_unmask,
        live_idt_bind: final_gate.live_idt_bind,
        keyboard_mode: final_gate.keyboard_mode,
        eoi_write_preflight: preflight.eoi_write_smoke_preflight,
        first_pic_eoi_write_allowed: preflight.first_pic_eoi_write_allowed,
        mutation_sequence_ready: sequence.mutation_sequence_ready,
        hardware_mutation_ready: mutation.hardware_mutation_ready,
        activation_decision: decision.activation_decision,
        final_activation_allowed: final_gate.final_activation_allowed,
    }
}

/// Models the permit gate for a future first PIC EOI write without touching hardware.
pub fn eoi_write_permit_model(
    candidate: EoiWriteSmokeCandidate,
    preflight: EoiWriteSmokePreflight,
    sequence: IrqRuntimeMutationSmokeSequence,
    mutation: IrqRuntimeHardwareMutationChecklist,
    decision: IrqRuntimeActivationDecision,
    final_gate: IrqRuntimeFinalGate,
) -> EoiWritePermitModel {
    EoiWritePermitModel {
        scope: EOI_WRITE_PERMIT_SCOPE,
        inputs: EOI_WRITE_PERMIT_INPUTS,
        permit_granted: EOI_WRITE_PERMIT_GRANTED_NO,
        first_pic_eoi_write_allowed: EOI_WRITE_PERMIT_FIRST_WRITE_ALLOWED_NO,
        target_command_port: EOI_WRITE_PERMIT_TARGET_NONE,
        target_value: EOI_WRITE_PERMIT_TARGET_NONE,
        target_irq_line: EOI_WRITE_PERMIT_TARGET_NONE,
        hardware_mutation: final_gate.hardware_mutation,
        runtime_irq_active: final_gate.runtime_irq_active,
        fire_command: EOI_WRITE_PERMIT_FIRE_DRY_RUN_BLOCKED,
        activation_decision: decision.activation_decision,
        final_activation_allowed: final_gate.final_activation_allowed,
        hardware_mutation_ready: mutation.hardware_mutation_ready,
        mutation_sequence_ready: sequence.mutation_sequence_ready,
        candidate_fire_result: candidate.fire_result,
        sti_instruction: final_gate.sti_instruction,
        pic_unmask: final_gate.pic_unmask,
        live_idt_bind: final_gate.live_idt_bind,
        keyboard_mode: preflight.keyboard_mode,
    }
}

/// Models the future one-shot command path for a first PIC EOI write without firing it.
pub fn eoi_write_oneshot_command_path(permit: EoiWritePermitModel) -> EoiWriteOneShotCommandPath {
    EoiWriteOneShotCommandPath {
        scope: EOI_WRITE_ONESHOT_SCOPE,
        inputs: EOI_WRITE_ONESHOT_INPUTS,
        one_shot_armed: EOI_WRITE_ONESHOT_ARMED_NO,
        fire_allowed: EOI_WRITE_ONESHOT_FIRE_ALLOWED_NO,
        first_pic_eoi_write_performed: EOI_WRITE_ONESHOT_WRITE_PERFORMED_NO,
        target_command_port: EOI_WRITE_ONESHOT_TARGET_NONE,
        target_value: EOI_WRITE_ONESHOT_TARGET_NONE,
        hardware_mutation: permit.hardware_mutation,
        runtime_irq_active: permit.runtime_irq_active,
        fire_result: EOI_WRITE_ONESHOT_FIRE_BLOCKED_BY_PERMIT,
        permit_granted: permit.permit_granted,
        first_pic_eoi_write_allowed: permit.first_pic_eoi_write_allowed,
        sti_instruction: permit.sti_instruction,
        pic_unmask: permit.pic_unmask,
        live_idt_bind: permit.live_idt_bind,
        keyboard_mode: permit.keyboard_mode,
    }
}

fn eoi_write_oneshot_latch_from_state(
    permit: EoiWritePermitModel,
    armed: bool,
) -> EoiWriteOneShotLatch {
    EoiWriteOneShotLatch {
        scope: EOI_WRITE_ONESHOT_LATCH_SCOPE,
        inputs: EOI_WRITE_ONESHOT_LATCH_INPUTS,
        latch: EOI_WRITE_ONESHOT_LATCH_TELEMETRY_ONLY,
        one_shot_armed: if armed {
            EOI_WRITE_ONESHOT_LATCH_ARMED_YES
        } else {
            EOI_WRITE_ONESHOT_LATCH_ARMED_NO
        },
        fire_allowed: EOI_WRITE_ONESHOT_LATCH_FIRE_ALLOWED_NO,
        first_pic_eoi_write_performed: EOI_WRITE_ONESHOT_LATCH_WRITE_PERFORMED_NO,
        target_command_port: EOI_WRITE_ONESHOT_LATCH_TARGET_NONE,
        target_value: EOI_WRITE_ONESHOT_LATCH_TARGET_NONE,
        hardware_mutation: permit.hardware_mutation,
        runtime_irq_active: permit.runtime_irq_active,
        fire_result: EOI_WRITE_ONESHOT_LATCH_FIRE_BLOCKED_BY_PERMIT,
        fire_cleared_latch: EOI_WRITE_ONESHOT_LATCH_FIRE_CLEARED_NO,
        permit_granted: permit.permit_granted,
        first_pic_eoi_write_allowed: permit.first_pic_eoi_write_allowed,
        sti_instruction: permit.sti_instruction,
        pic_unmask: permit.pic_unmask,
        live_idt_bind: permit.live_idt_bind,
        keyboard_mode: permit.keyboard_mode,
    }
}

/// Reads the software-only one-shot latch without touching hardware.
pub fn eoi_write_oneshot_latch_status(permit: EoiWritePermitModel) -> EoiWriteOneShotLatch {
    let armed = EOI_WRITE_ONESHOT_LATCH_ARMED.load(Ordering::SeqCst);
    eoi_write_oneshot_latch_from_state(permit, armed)
}

/// Arms the software-only one-shot latch without enabling EOI writes.
pub fn eoi_write_oneshot_latch_arm(permit: EoiWritePermitModel) -> EoiWriteOneShotLatch {
    EOI_WRITE_ONESHOT_LATCH_ARMED.store(true, Ordering::SeqCst);
    eoi_write_oneshot_latch_from_state(permit, true)
}

/// Clears the software-only one-shot latch without touching hardware.
pub fn eoi_write_oneshot_latch_clear(permit: EoiWritePermitModel) -> EoiWriteOneShotLatch {
    EOI_WRITE_ONESHOT_LATCH_ARMED.store(false, Ordering::SeqCst);
    eoi_write_oneshot_latch_from_state(permit, false)
}

/// Reads the latch and reports permit-blocked fire without clearing the latch.
pub fn eoi_write_oneshot_latch_fire(permit: EoiWritePermitModel) -> EoiWriteOneShotLatch {
    let armed = EOI_WRITE_ONESHOT_LATCH_ARMED.load(Ordering::SeqCst);
    eoi_write_oneshot_latch_from_state(permit, armed)
}

/// Bridges the denied permit model and software latch as read-only telemetry.
pub fn eoi_write_bridge(
    permit: EoiWritePermitModel,
    latch: EoiWriteOneShotLatch,
) -> EoiWriteBridge {
    EoiWriteBridge {
        scope: EOI_WRITE_BRIDGE_SCOPE,
        inputs: EOI_WRITE_BRIDGE_INPUTS,
        bridge: EOI_WRITE_BRIDGE_READ_ONLY,
        latch: latch.latch,
        one_shot_armed: latch.one_shot_armed,
        permit_granted: permit.permit_granted,
        bridge_ready: EOI_WRITE_BRIDGE_READY_NO,
        first_pic_eoi_write_allowed: EOI_WRITE_PERMIT_FIRST_WRITE_ALLOWED_NO,
        target_command_port: EOI_WRITE_BRIDGE_TARGET_NONE,
        target_value: EOI_WRITE_BRIDGE_TARGET_NONE,
        hardware_mutation: permit.hardware_mutation,
        runtime_irq_active: permit.runtime_irq_active,
        blocker_latch: if latch.one_shot_armed == EOI_WRITE_ONESHOT_LATCH_ARMED_YES {
            EOI_WRITE_BRIDGE_BLOCKER_LATCH_GATED
        } else {
            EOI_WRITE_BRIDGE_BLOCKER_LATCH_NOT_ARMED
        },
        blocker_permit: EOI_WRITE_BRIDGE_BLOCKER_PERMIT_DENIED,
        sti_instruction: permit.sti_instruction,
        pic_unmask: permit.pic_unmask,
        live_idt_bind: permit.live_idt_bind,
        keyboard_mode: permit.keyboard_mode,
    }
}

fn eoi_write_permit_transition_from_state(
    bridge: EoiWriteBridge,
    armed: bool,
) -> EoiWritePermitTransition {
    EoiWritePermitTransition {
        scope: EOI_WRITE_PERMIT_TRANSITION_SCOPE,
        inputs: EOI_WRITE_PERMIT_TRANSITION_INPUTS,
        transition: EOI_WRITE_PERMIT_TRANSITION_SOFTWARE_ONLY,
        permit_transition_armed: if armed {
            EOI_WRITE_PERMIT_TRANSITION_ARMED_YES
        } else {
            EOI_WRITE_PERMIT_TRANSITION_ARMED_NO
        },
        permit_granted: EOI_WRITE_PERMIT_GRANTED_NO,
        bridge_ready: EOI_WRITE_BRIDGE_READY_NO,
        first_pic_eoi_write_allowed: EOI_WRITE_PERMIT_FIRST_WRITE_ALLOWED_NO,
        target_command_port: EOI_WRITE_PERMIT_TARGET_NONE,
        target_value: EOI_WRITE_PERMIT_TARGET_NONE,
        hardware_mutation: bridge.hardware_mutation,
        runtime_irq_active: bridge.runtime_irq_active,
        blocker_transition: EOI_WRITE_PERMIT_TRANSITION_BLOCKER_TRANSITION,
        blocker_permit: EOI_WRITE_PERMIT_TRANSITION_BLOCKER_PERMIT,
        blocker_bridge: EOI_WRITE_PERMIT_TRANSITION_BLOCKER_BRIDGE,
        blocker_first_allowed: EOI_WRITE_PERMIT_TRANSITION_BLOCKER_FIRST_ALLOWED,
        blocker_hardware: EOI_WRITE_PERMIT_TRANSITION_BLOCKER_HARDWARE,
        blocker_runtime: EOI_WRITE_PERMIT_TRANSITION_BLOCKER_RUNTIME,
        blocker_sti: EOI_WRITE_PERMIT_TRANSITION_BLOCKER_STI,
        blocker_pic_unmask: EOI_WRITE_PERMIT_TRANSITION_BLOCKER_PIC_UNMASK,
        blocker_live_irq: EOI_WRITE_PERMIT_TRANSITION_BLOCKER_LIVE_IRQ,
        sti_instruction: bridge.sti_instruction,
        pic_unmask: bridge.pic_unmask,
        live_idt_bind: bridge.live_idt_bind,
        keyboard_mode: bridge.keyboard_mode,
    }
}

/// Reads the software-only permit transition state without touching hardware.
pub fn eoi_write_permit_transition_status(
    bridge: EoiWriteBridge,
) -> EoiWritePermitTransition {
    let armed = EOI_WRITE_PERMIT_TRANSITION_ARMED.load(Ordering::SeqCst);
    eoi_write_permit_transition_from_state(bridge, armed)
}

/// Arms the software-only permit transition state without granting a permit.
pub fn eoi_write_permit_transition_arm(bridge: EoiWriteBridge) -> EoiWritePermitTransition {
    EOI_WRITE_PERMIT_TRANSITION_ARMED.store(true, Ordering::SeqCst);
    eoi_write_permit_transition_from_state(bridge, true)
}

/// Clears the software-only permit transition state without touching hardware.
pub fn eoi_write_permit_transition_clear(bridge: EoiWriteBridge) -> EoiWritePermitTransition {
    EOI_WRITE_PERMIT_TRANSITION_ARMED.store(false, Ordering::SeqCst);
    eoi_write_permit_transition_from_state(bridge, false)
}

/// Checks the software transition state while keeping the permit denied.
pub fn eoi_write_permit_transition_check(bridge: EoiWriteBridge) -> EoiWritePermitTransition {
    let armed = EOI_WRITE_PERMIT_TRANSITION_ARMED.load(Ordering::SeqCst);
    eoi_write_permit_transition_from_state(bridge, armed)
}

/// Evaluates the software EOI write state chain without mutating any layer.
pub fn eoi_write_permit_evaluation(
    permit: EoiWritePermitModel,
    latch: EoiWriteOneShotLatch,
    bridge: EoiWriteBridge,
    transition: EoiWritePermitTransition,
) -> EoiWritePermitEvaluation {
    EoiWritePermitEvaluation {
        scope: EOI_WRITE_EVAL_SCOPE,
        inputs: EOI_WRITE_EVAL_INPUTS,
        evaluation: EOI_WRITE_EVAL_READ_ONLY,
        evaluation_ready: EOI_WRITE_EVAL_READY_NO,
        one_shot_armed: latch.one_shot_armed,
        permit_transition_armed: transition.permit_transition_armed,
        permit_granted: EOI_WRITE_PERMIT_GRANTED_NO,
        bridge_ready: EOI_WRITE_BRIDGE_READY_NO,
        first_pic_eoi_write_allowed: EOI_WRITE_PERMIT_FIRST_WRITE_ALLOWED_NO,
        hardware_mutation: permit.hardware_mutation,
        runtime_irq_active: permit.runtime_irq_active,
        blocker_permit: EOI_WRITE_EVAL_BLOCKER_PERMIT,
        blocker_bridge: EOI_WRITE_EVAL_BLOCKER_BRIDGE,
        blocker_transition: EOI_WRITE_EVAL_BLOCKER_TRANSITION,
        blocker_first_write: EOI_WRITE_EVAL_BLOCKER_FIRST_WRITE,
        blocker_hardware: EOI_WRITE_EVAL_BLOCKER_HARDWARE,
        blocker_runtime: EOI_WRITE_EVAL_BLOCKER_RUNTIME,
        sti_instruction: bridge.sti_instruction,
        pic_unmask: bridge.pic_unmask,
        live_idt_bind: bridge.live_idt_bind,
        keyboard_mode: bridge.keyboard_mode,
    }
}

/// Reports whether the manual PIC_EOI smoke can bridge toward runtime handlers.
pub fn eoi_runtime_bridge_readiness(
    manual_pic_eoi_smoke_proven: &'static str,
    _evaluation: EoiWritePermitEvaluation,
    _runtime_dispatch_ready: bool,
    _gate_state: IrqGateBindStateTelemetry,
) -> EoiRuntimeBridgeReadiness {
    EoiRuntimeBridgeReadiness {
        scope: EOI_RUNTIME_BRIDGE_SCOPE,
        inputs: EOI_RUNTIME_BRIDGE_INPUTS,
        manual_pic_eoi_smoke_proven,
        runtime_bridge_ready: EOI_RUNTIME_BRIDGE_READY_NO,
        handler_triggered_eoi_allowed: EOI_RUNTIME_BRIDGE_HANDLER_ALLOWED_NO,
        runtime_irq_active: EOI_RUNTIME_BRIDGE_RUNTIME_ACTIVE_NO,
        sti: EOI_RUNTIME_BRIDGE_STI_DISABLED,
        pic_unmask: EOI_RUNTIME_BRIDGE_PIC_UNMASK_DISABLED,
        live_irq_handlers: EOI_RUNTIME_BRIDGE_LIVE_IRQ_HANDLERS_NO,
        keyboard_mode: EOI_RUNTIME_BRIDGE_KEYBOARD_POLLING,
        blocker_dispatch: EOI_RUNTIME_BRIDGE_BLOCKER_DISPATCH,
        blocker_sti: EOI_RUNTIME_BRIDGE_BLOCKER_STI,
        blocker_pic_lines: EOI_RUNTIME_BRIDGE_BLOCKER_PIC_LINES,
        blocker_live_handlers: EOI_RUNTIME_BRIDGE_BLOCKER_LIVE_HANDLERS,
        blocker_handler_eoi: EOI_RUNTIME_BRIDGE_BLOCKER_HANDLER_EOI,
    }
}

/// Derives a read-only candidate for a future handler-side PIC_EOI path.
pub fn irq_handler_eoi_candidate(
    bridge: EoiRuntimeBridgeReadiness,
) -> IrqHandlerEoiCandidate {
    IrqHandlerEoiCandidate {
        scope: IRQ_HANDLER_EOI_CANDIDATE_SCOPE,
        inputs: IRQ_HANDLER_EOI_CANDIDATE_INPUTS,
        runtime_bridge_ready: bridge.runtime_bridge_ready,
        handler_eoi_candidate_ready: IRQ_HANDLER_EOI_CANDIDATE_READY_NO,
        handler_triggered_eoi_allowed: IRQ_HANDLER_EOI_CANDIDATE_HANDLER_ALLOWED_NO,
        live_handler_bind: IRQ_HANDLER_EOI_CANDIDATE_LIVE_BIND_NO,
        pic_eoi_callsites: IRQ_HANDLER_EOI_CANDIDATE_PIC_EOI_CALLSITES,
        runtime_irq_active: IRQ_HANDLER_EOI_CANDIDATE_RUNTIME_ACTIVE_NO,
        sti: IRQ_HANDLER_EOI_CANDIDATE_STI_DISABLED,
        pic_unmask: IRQ_HANDLER_EOI_CANDIDATE_PIC_UNMASK_DISABLED,
        keyboard_mode: IRQ_HANDLER_EOI_CANDIDATE_KEYBOARD_POLLING,
        blocker_bridge: IRQ_HANDLER_EOI_CANDIDATE_BLOCKER_BRIDGE,
        blocker_handler_eoi: IRQ_HANDLER_EOI_CANDIDATE_BLOCKER_HANDLER_EOI,
        blocker_live_handlers: IRQ_HANDLER_EOI_CANDIDATE_BLOCKER_LIVE_HANDLERS,
        blocker_manual_only: IRQ_HANDLER_EOI_CANDIDATE_BLOCKER_MANUAL_ONLY,
        blocker_runtime: IRQ_HANDLER_EOI_CANDIDATE_BLOCKER_RUNTIME,
    }
}

/// Derives a read-only placeholder for a future handler-side EOI stub.
pub fn irq_handler_eoi_stub(candidate: IrqHandlerEoiCandidate) -> IrqHandlerEoiStub {
    IrqHandlerEoiStub {
        scope: IRQ_HANDLER_EOI_STUB_SCOPE,
        inputs: IRQ_HANDLER_EOI_STUB_INPUTS,
        stub_exists: IRQ_HANDLER_EOI_STUB_EXISTS_YES,
        stub_bound_to_live_irq_path: IRQ_HANDLER_EOI_STUB_LIVE_BIND_NO,
        stub_invocation_allowed: IRQ_HANDLER_EOI_STUB_INVOCATION_ALLOWED_NO,
        stub_performs_pic_eoi_write: IRQ_HANDLER_EOI_STUB_PERFORMS_WRITE_NO,
        handler_triggered_eoi_allowed: candidate.handler_triggered_eoi_allowed,
        pic_eoi_callsites: candidate.pic_eoi_callsites,
        runtime_irq_active: IRQ_HANDLER_EOI_STUB_RUNTIME_ACTIVE_NO,
        sti: IRQ_HANDLER_EOI_STUB_STI_DISABLED,
        pic_unmask: IRQ_HANDLER_EOI_STUB_PIC_UNMASK_DISABLED,
        keyboard_mode: IRQ_HANDLER_EOI_STUB_KEYBOARD_POLLING,
        blocker_unbound: IRQ_HANDLER_EOI_STUB_BLOCKER_UNBOUND,
        blocker_invocation: IRQ_HANDLER_EOI_STUB_BLOCKER_INVOCATION,
        blocker_handler_eoi: IRQ_HANDLER_EOI_STUB_BLOCKER_HANDLER_EOI,
        blocker_manual_only: IRQ_HANDLER_EOI_STUB_BLOCKER_MANUAL_ONLY,
        blocker_runtime: IRQ_HANDLER_EOI_STUB_BLOCKER_RUNTIME,
    }
}

/// Derives a read-only bind candidate above the unbound handler EOI stub.
pub fn irq_handler_bind_candidate(stub: IrqHandlerEoiStub) -> IrqHandlerBindCandidate {
    IrqHandlerBindCandidate {
        scope: IRQ_HANDLER_BIND_CANDIDATE_SCOPE,
        inputs: IRQ_HANDLER_BIND_CANDIDATE_INPUTS,
        stub_exists: stub.stub_exists,
        bind_candidate_exists: IRQ_HANDLER_BIND_CANDIDATE_EXISTS_YES,
        bind_candidate_ready: IRQ_HANDLER_BIND_CANDIDATE_READY_NO,
        live_idt_bind_performed: IRQ_HANDLER_BIND_CANDIDATE_LIVE_IDT_BIND_NO,
        irq_handler_reachable: IRQ_HANDLER_BIND_CANDIDATE_IRQ_REACHABLE_NO,
        handler_triggered_eoi_allowed: stub.handler_triggered_eoi_allowed,
        runtime_irq_active: IRQ_HANDLER_BIND_CANDIDATE_RUNTIME_ACTIVE_NO,
        sti: IRQ_HANDLER_BIND_CANDIDATE_STI_DISABLED,
        pic_unmask: IRQ_HANDLER_BIND_CANDIDATE_PIC_UNMASK_DISABLED,
        keyboard_mode: IRQ_HANDLER_BIND_CANDIDATE_KEYBOARD_POLLING,
        blocker_idt_bind: IRQ_HANDLER_BIND_CANDIDATE_BLOCKER_IDT_BIND,
        blocker_irq_registration: IRQ_HANDLER_BIND_CANDIDATE_BLOCKER_IRQ_REGISTRATION,
        blocker_stub_invocation: IRQ_HANDLER_BIND_CANDIDATE_BLOCKER_STUB_INVOCATION,
        blocker_handler_eoi: IRQ_HANDLER_BIND_CANDIDATE_BLOCKER_HANDLER_EOI,
        blocker_runtime: IRQ_HANDLER_BIND_CANDIDATE_BLOCKER_RUNTIME,
    }
}

/// Reports whether the manual IDT bind smoke can bridge toward live IRQ binding.
pub fn idt_bind_runtime_bridge_readiness(
    manual_idt_bind_smoke_proven_this_boot: &'static str,
    _bind_candidate: IrqHandlerBindCandidate,
) -> IdtBindRuntimeBridgeReadiness {
    IdtBindRuntimeBridgeReadiness {
        scope: IDT_BIND_RUNTIME_BRIDGE_SCOPE,
        inputs: IDT_BIND_RUNTIME_BRIDGE_INPUTS,
        manual_idt_bind_smoke_proven_this_boot,
        runtime_idt_bridge_ready: IDT_BIND_RUNTIME_BRIDGE_READY_NO,
        live_irq_bind_allowed: IDT_BIND_RUNTIME_BRIDGE_LIVE_BIND_ALLOWED_NO,
        irq_handler_reachable: IDT_BIND_RUNTIME_BRIDGE_IRQ_REACHABLE_NO,
        interrupt_invocation_allowed: IDT_BIND_RUNTIME_BRIDGE_INTERRUPT_ALLOWED_NO,
        runtime_irq_active: IDT_BIND_RUNTIME_BRIDGE_RUNTIME_ACTIVE_NO,
        sti: IDT_BIND_RUNTIME_BRIDGE_STI_DISABLED,
        pic_unmask: IDT_BIND_RUNTIME_BRIDGE_PIC_UNMASK_DISABLED,
        keyboard_mode: IDT_BIND_RUNTIME_BRIDGE_KEYBOARD_POLLING,
        blocker_proof: IDT_BIND_RUNTIME_BRIDGE_BLOCKER_PROOF,
        blocker_live_bind: IDT_BIND_RUNTIME_BRIDGE_BLOCKER_LIVE_BIND,
        blocker_irq_reachable: IDT_BIND_RUNTIME_BRIDGE_BLOCKER_IRQ_REACHABLE,
        blocker_interrupt: IDT_BIND_RUNTIME_BRIDGE_BLOCKER_INTERRUPT,
        blocker_runtime: IDT_BIND_RUNTIME_BRIDGE_BLOCKER_RUNTIME,
    }
}

/// Reports whether proven manual IDT invocation can bridge toward IRQ delivery.
pub fn idt_invoke_runtime_bridge_readiness(
    manual_idt_bind_smoke_proven_this_boot: &'static str,
    manual_idt_invocation_smoke_proven_this_boot: &'static str,
    _bind_bridge: IdtBindRuntimeBridgeReadiness,
    _bind_candidate: IrqHandlerBindCandidate,
    _stub: IrqHandlerEoiStub,
) -> IdtInvokeRuntimeBridgeReadiness {
    IdtInvokeRuntimeBridgeReadiness {
        scope: IDT_INVOKE_RUNTIME_BRIDGE_SCOPE,
        inputs: IDT_INVOKE_RUNTIME_BRIDGE_INPUTS,
        manual_idt_bind_smoke_proven_this_boot,
        manual_idt_invocation_smoke_proven_this_boot,
        runtime_invocation_bridge_ready: IDT_INVOKE_RUNTIME_BRIDGE_READY_NO,
        live_irq_delivery_allowed: IDT_INVOKE_RUNTIME_BRIDGE_LIVE_DELIVERY_ALLOWED_NO,
        irq_handler_reachable_from_hardware: IDT_INVOKE_RUNTIME_BRIDGE_HARDWARE_REACHABLE_NO,
        handler_triggered_eoi_allowed: IDT_INVOKE_RUNTIME_BRIDGE_HANDLER_EOI_ALLOWED_NO,
        runtime_irq_active: IDT_INVOKE_RUNTIME_BRIDGE_RUNTIME_ACTIVE_NO,
        sti: IDT_INVOKE_RUNTIME_BRIDGE_STI_DISABLED,
        pic_unmask: IDT_INVOKE_RUNTIME_BRIDGE_PIC_UNMASK_DISABLED,
        keyboard_mode: IDT_INVOKE_RUNTIME_BRIDGE_KEYBOARD_POLLING,
        blocker_bind_proof: IDT_INVOKE_RUNTIME_BRIDGE_BLOCKER_BIND_PROOF,
        blocker_invoke_proof: IDT_INVOKE_RUNTIME_BRIDGE_BLOCKER_INVOKE_PROOF,
        blocker_delivery: IDT_INVOKE_RUNTIME_BRIDGE_BLOCKER_DELIVERY,
        blocker_hardware_reachable: IDT_INVOKE_RUNTIME_BRIDGE_BLOCKER_HARDWARE_REACHABLE,
        blocker_runtime: IDT_INVOKE_RUNTIME_BRIDGE_BLOCKER_RUNTIME,
    }
}
