#![allow(dead_code)]

//! IRQ handler skeleton foundation.
//!
//! This module documents the first hardware IRQ handler shapes without making
//! them hardware-active. It contains no PIC EOI writes and no port I/O. The
//! symbols are compiled so verification can guard the intended IRQ0/IRQ1
//! contract before runtime IRQ activation exists.

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
pub const IRQ_RUNTIME_BLOCKER_PIC_REMAP: &str = "PIC remap: not ready for controlled smoke (run: pic-remap-arm, pic-remap-smoke)";

/// IRQ gates precondition blocker message.
pub const IRQ_RUNTIME_BLOCKER_IRQ_GATES: &str = "IRQ gates: vectors 32/33 not bound (run: irq-gate-arm, irq-gate-bind-smoke)";

/// EOI dispatch precondition blocker message.
pub const IRQ_RUNTIME_BLOCKER_EOI_DISPATCH: &str = "EOI dispatch: not enabled";

/// STI precondition blocker message.
pub const IRQ_RUNTIME_BLOCKER_STI: &str = "STI: disabled";

/// Keyboard fallback precondition blocker message (but always satisfied in v9.1.0).
pub const IRQ_RUNTIME_PRECONDITION_KEYBOARD_FALLBACK: &str = "keyboard fallback: polling-only (ok)";

/// Page fault smoke precondition blocker message (but always satisfied in v9.1.0).
pub const IRQ_RUNTIME_PRECONDITION_PF_SMOKE: &str = "pf-smoke: stable (ok)";

/// EOI precondition blocker messages (v9.2.0).
pub const EOI_RUNTIME_BLOCKER_PIC_REMAP: &str = "PIC remap: not ready for EOI dispatch (run: pic-remap-arm, pic-remap-smoke)";
pub const EOI_RUNTIME_BLOCKER_IRQ_GATES: &str = "IRQ gates: vectors 32/33 not bound for EOI (run: irq-gate-arm, irq-gate-bind-smoke)";
pub const EOI_RUNTIME_BLOCKER_EDGE_LEVEL: &str = "IRQ edge/level: detection strategy not planned";
pub const EOI_RUNTIME_BLOCKER_KEYBOARD: &str = "Keyboard fallback: state unknown";
pub const EOI_RUNTIME_BLOCKER_STI: &str = "STI: not enabled for EOI dispatch";

/// IRQ mask blocker messages (v9.3.0).
pub const IRQ_MASK_BLOCKER_PIC_REMAP: &str = "[BLOCKER] pic remap not executed";
pub const IRQ_MASK_BLOCKER_IRQ_GATES: &str = "[BLOCKER] irq gates not bound (vectors 32/33 unbound)";
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

static mut IRQ_GATE_BIND_SMOKE_ARMED: bool = false;
static mut IRQ_GATE_BIND_SMOKE_EXECUTED: bool = false;

static mut IRQ_RUNTIME_ARMED: bool = false;
static mut IRQ_RUNTIME_COMMITTED: bool = false;
static mut IRQ_RUNTIME_ACTIVATION_TOKEN_PRESENT: bool = false;

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
    false  // Will be properly checked in main
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
        pic_remap_smoke: if pic_remap_executed { IRQ_MATRIX_YES } else { IRQ_MATRIX_NO },
        irq_gate_bind_smoke: if irq_gates_bound { IRQ_MATRIX_YES } else { IRQ_MATRIX_NO },
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
