# HOPPYSOS DByte migration seam

Status: post-v13.1 development contract. The sealed `v13.1.0` commit and tag
remain immutable. HOPPYSOS may advance independently in its non-Git workspace;
this proof never rewrites that live tree. Verification is bound instead to a
hash-pinned frozen x64 checkpoint authority and a verifier-owned scratch copy.

## Architecture

```text
HOPPYSOS BIOS boot/stage2
        -> fixed load address 0x00010000
        -> metadata-free ELF64 machine boundary (transitional)
        -> DByte ELF64 objects
        -> DByte internal linker and checked flat writer
        -> deterministic kernel.bin (maximum 252 sectors)
```

DBTASM is a textual frontend over the compiler's existing bounded x86_64
encoder. The compiler and assembler share instruction encoding, ELF64 object
writing, relocation validation, diagnostics, and the internal linker. There
is no second encoder, external linker, arbitrary section extraction, or
`incbin` relocation workaround.

Frozen authority is declared by
`examples/hoppysos_flat_seam/frozen_authority_v1.json`. It binds the checkpoint
`pre-irq-dispatch-20260913-030442/kernel.asm` (`E03FFADF...F778`), boot/stage2,
and all 22 transitive kernel includes. That set must reproduce the original
120,592-byte canonical kernel SHA-256 `D175C493...CFB`; the live top-level x64
kernel is observed for drift but is never a proof input.

## Proven migration stages

`scripts/verify_hoppysos_flat_seam.ps1` builds every proof input twice and
boots verifier-owned QEMU instances at 32 and 64 MiB.

1. A metadata-free NASM `ET_REL` boundary, a DBTASM boundary, and DByte code
   link into a bounded flat payload. This proves cross-object calls and the
   requested fixed image base.
2. The complete frozen HOPPYSOS kernel is converted to `ET_REL` by one reviewed
   generated-source change (`ORG` to ELF section/entry declarations). DByte's
   linked flat result must be byte-identical to the canonical NASM flat kernel.
3. A second generated source removes the complete ASM PMM implementation and
   its two owned counters. The unchanged HOPPYSOS call sites resolve to DByte
   implementations of initialization, allocation, release, self-test, guard
   self-test, and counter storage. Linking without the DByte PMM object is a
   required negative failure.
4. The live O1 image additionally calls a DByte-owned bounded cooperative task
   runtime. The scheduler is initialized during boot and then called repeatedly
   from the real HOPPYSOS event-resume loop. `DBYTE_SCHED_LIVE` is emitted only
   after at least three live scheduler calls. O0 and O1 task objects remain
   deterministic; O1 is used in the live image because the frozen stage2 payload
   ceiling is 252 sectors.
5. The IRQ0/PIT counter is DByte-owned on the live interrupt path. The generated
   proof source removes the ASM `irq0_count` storage and replaces the IRQ0 body
   with a minimal machine-boundary wrapper that preserves all DBYTE64 caller-saved
   registers, calls `dbyte_irq0_tick64`, acknowledges the PIC, restores context,
   and returns with `iretq`. Boot waits on `dbyte_irq0_count64` and emits
   `DBYTE_IRQ0_LIVE` only after three hardware timer interrupts.
6. A new x86-64 privilege foundation is generated from the frozen x64 source;
   legacy i386 Ring3/syscall code is reference-only and is never promoted to
   current evidence. The proof installs a 64-bit GDT/TSS while preserving the
   existing `CODE64_SEL=0x18`, loads TSS selector `0x30`, exposes only the
   2-4 MiB leaf mapping to CPL3, copies a tiny stub to `0x00200000`, enters CPL3
   with `iretq`, invokes DPL3 vector `0x80`, records exactly one syscall through
   DByte-owned state, resumes the saved Ring0 continuation, then removes the
   temporary user mapping. The live O1 image remains below the 252-sector ceiling.

The full boot must reach `HOPPYSOS64:PMM_OK`, `PAGE_ALLOC_OK`,
`PMM_GUARD_OK`, `DBYTE_TASK_OK`, `DBYTE_IRQ0_LIVE`, `DBYTE_TSS_OK`,
`DBYTE_RING3_SYSCALL_OK`, `DBYTE_RING0_RETURN_OK`, `DBYTE_SCHED_LIVE`, and
`READY`, with no unexpected fault, triple fault, or reset. The DBYRT64
successor proof deliberately executes exactly one CPL3 `#UD`, `#GP`, and
non-present `#PF`; each is contained through HostServicesV2, restores kernel
CR3/stack, and reclaims eight private frames before the next case. Both
missing-PMM and missing-task-runtime links must fail without publishing an
output. Each required debug marker must appear exactly once, the three expected
fault trace records must match vector/error/address exactly, and the trace must
contain exactly the two normal startup resets. The v7 receipt also fails closed if HEAD/tag identity,
protected dirty files, the frozen manifest/bundle, or any hash-bound authority
source moves during a run.

## Remaining boundary

HOPPYSOS boot and most kernel machine code still originate in NASM. The proof
now establishes a bounded single-process x86-64 lifecycle: compact user ELF
loading, a private CR3, owner-qualified file access, user-pointer validation,
fault containment, and exact frame reclaim. It does not claim fork/exec, a
general syscall ABI, multi-process scheduling, or preemption. A no-NASM
production build still requires consumer-driven DBTASM instruction/directive
coverage. Physical storage writes remain disarmed; the migration verifier
creates only disposable QEMU images below its ignored evidence root.
