# DByteOS Kernel Exception Subsystem Foundation (v8.9.0)

DByteOS Kernel Lab `v8.9.0` preserves the exception subsystem foundation while adding PIC/IRQ direction docs separately in `KERNEL_IRQ.md`. This release does not add new exception vectors, change Page Fault smoke mechanics, enable STI, remap PIC, bind IRQ vectors, or replace keyboard polling.

## Active Vectors

| Vector | Name | State | Trigger |
| --- | --- | --- | --- |
| `0` | divide-by-zero | active controlled trap | `div0` uses controlled `int 0` |
| `3` | breakpoint | active | `int3` |
| `14` | page fault | active smoke | `pf-smoke` controlled real fault |

Planned handlers are currently `none`.

## Foundation Capabilities

- **Telemetry**: count / last vector / last name.
- **Recovery UX**: smoke-safe Page Fault recovery trampoline.
- **Status UX**: `exception-status`, `exceptions`, `exceptions --verbose`, `fault-status`, `pf-status`, and `system`.
- **Handler UX**: `handlers`, `handlers --active`, and `exception-about`.
- **Reset UX**: `exception-reset` and `fault-reset`.
- **Safety Guards**: no `asm!("int 14")`, no `asm!("sti")`, no PIC/IRQ enable/remap, and keyboard input remains polling-based.
- **Page Fault Smoke Mechanics**: `pf-smoke` sets `PF_SMOKE_ACTIVE`, sets `PF_SMOKE_RECOVERY_EIP`, calls `pf_smoke_probe_asm`, and keeps vector 14 bound to `page_fault_handler_asm`.

## Full Exception Journey Smoke

Manual QEMU interaction should follow this foundation path:

```txt
int3 -> exception-status
div0 -> exception-status
pf-smoke -> fault-status
exception-about
```

Expected foundation summary:

```txt
exception subsystem:
foundation: active
active vectors: 0 divide-by-zero, 3 breakpoint, 14 page fault smoke
telemetry: count / last vector / last name
recovery: smoke-safe trampoline
status ux: active
interrupts: disabled
```

`pf-smoke` remains the only controlled real Page Fault trigger. It reads `CR2`, reports the CPU-pushed error code, rewrites the saved EIP to the recovery trampoline, and returns to the shell.
