# DBYRT64 HOPFS1 Runtime Module Design

Status: approved successor design after `LOCAL FINAL BASELINE — PASS / SEALED`.
The sealed `v13.1.0` commit/tag remain immutable and no public successor release is created by this work.

## Goal

Move growth-heavy DByte userspace/process runtime out of the 252-sector boot kernel into a bounded HOPFS1-hosted `DBYRT64` module, while keeping the current BIOS/stage2 layout, LBA255 journal/scratch contract, and HOPFS1 at LBA256 unchanged.

## Current forcing constraints

- Live successor kernel proof: 126,288 bytes / 247 sectors.
- Stage2 hard ceiling: 252 sectors; only 2,736 bytes / 5 sectors remain.
- HOPFS1 supports files up to 384 sectors.
- Current x64 privilege proof already establishes GDT/TSS, CPL3 entry, DPL3 INT 0x80, DByte syscall state, and return to Ring0.
- Full v13 userspace donor contains private CR3, ELF64 loading, user-copy validation, fault containment, frame reclaim, syscall worker/VFS, and is too large to inline into the remaining kernel budget.

## Chosen architecture

`stage2 -> bounded kernel -> HOPFS1 DBYRT64 module -> USER.ELF -> CPL3`.
The kernel retains machine-boundary ownership, HOPFS1 read access, interrupt entry/exit, and cooperative scheduling hooks. DBYRT64 owns growth-heavy process/userspace logic behind a narrow host ABI.

## Module contract

`DBYRT64` is a deterministic fixed-address executable image produced by DByte tooling, stored as a normal HOPFS1 file, read into verifier-owned RAM, validated before execution, then called through one explicit entry point.

Initial module ABI:

```text
offset  bytes  field
0       8      magic = "DBYRT641"
8       4      abi_version = 1
12      4      header_bytes = 64
16      4      image_bytes = header_bytes + payload_bytes
20      4      entry_offset = header_bytes + linked entry symbol offset
24      4      payload_bytes
28      4      flags = 0
32      4      payload_xor (XOR of little-endian payload dwords)
36      28     reserved = all zero
64      ...    payload, zero-padded to a 16-byte boundary
```

The complete file is limited to 384 HOPFS1 sectors. The payload is linked for
`0x01000040`. The deterministic packer reads the unique `dbyrt64_entry`
function symbol from the DByte-produced ELF64 object and records its checked,
four-byte-aligned payload offset. This remains correct when interrupt stubs
precede the ordinary entry function.

The kernel validates magic/version/bounds/checksum before forming `module_base + entry_offset`. The first host ABI entry is `u64 dbyrt64_entry(usize host_services)`. It validates a fixed-width V2 table, allocates and releases pages through typed DByte function pointers, and returns the fixed success value without retaining the table or allocated pages.

```text
HostServicesV2 (x86_64, 48 bytes)
offset  bytes  field
0       4      abi_version = 2
4       4      table_bytes >= 48
8       8      alloc_page: fn() -> u64
16      8      free_page: fn(u64) -> bool
24      8      read_file: fn(ptr[u8], ptr[u8], usize, ptr[usize]) -> bool
32      8      enter_user: fn(usize, usize, usize, usize) -> i32
40      8      abort_user: fn(u32, usize, u64) -> int
```

The table is kernel-owned and read-only for the duration of the call. Later
capabilities are appended behind a larger `table_bytes`; existing offsets do
not change. A module must validate the minimum byte count before loading any
function address.

The module uses `read_file` to fetch the compact `USER.ELF` into the
reserved module window and applies the donor loader's bounded ELF64 header,
program-header, W^X, alignment, range, and executable-entry checks.

It constructs a private PML4/PDPT/PD plus image and
stack page tables from eight host-owned pages, copies the bounded ELF bytes
into private RX/RW pages, validates the user mappings, and calls the bounded
`enter_user(cr3, entry, stack, owner)` capability. The kernel binds a nonzero
owner identity to the process context, activates that CR3, enters CPL3 through
`iretq`, and services bounded `write/open/read/close/exit` requests through DPL3 INT 0x80,
restores the kernel CR3 and stack, and returns the exit code. The module then
reclaims all eight pages. The 32/64 MiB proof observes the exact user output,
zero exit status, return to Ring0, and an unchanged final PMM ownership count.

The current worker fixture adds one writable data page, so the complete private
space owns eight pages: PML4, PDPT, PD, image PT, stack PT, RX code, RW data,
and stack. Its nine-call proof lifecycle first rejects three invalid user-memory
requests with `EFAULT` (bad write pointer, bad open pointer, and read into the
RX code page), then completes `write -> open -> read -> close -> write -> exit`.
The kernel validates every touched user page through the active private page
tables before dereference, requires writable PTEs for read destinations, and
keeps the exact process owner identity with the open descriptor. Mismatched,
closed, or non-3 descriptors are rejected. The HOPFS1 read remains bounded to
64 bytes, user GPRs are preserved around the kernel filesystem helper, and
every private frame is reclaimed after return to Ring0. The appended
`abort_user(reason, address, error_code)` capability validates a contained
user-fault receipt, restores the kernel CR3 and saved kernel stack, and returns
the fault reason through the suspended `enter_user` call.

The versioned host-services table contains only capabilities required by the
module. No framebuffer, raw ATA write, or unrestricted physical-memory
primitive is exported.

## Loading and memory

The proof module is read from HOPFS1 by the existing read path into a fixed, page-aligned RAM window outside the kernel image and current low-memory scratch structures. The verifier must fail closed on overlap, oversize, malformed header, checksum mismatch, bad entry offset, and missing file.

## Successor slices

1. Completed: HOPFS1 module load, header validation, fixed-address call/return, and kernel `<=252` sectors.
2. Completed for the bounded worker: private CR3, compact user ELF validation/loading, RX/RW/stack mappings, CPL3 entry, Ring0 return, and frame reclaim.
3. Completed: bounded syscall services (`exit`, `write`, `open`, `read`, `close`) boot a real `USER.ELF` and read `DATA.TXT` from HOPFS1.
4. Completed: one cooperative process lifecycle performs create/load/run/exit/reclaim; no preemption, fork, or SMP.
5. Completed: module-owned IDT gates contain one CPL3 `#UD`, `#GP`, and
   non-present `#PF`; each case restores kernel state and reclaims its own
   eight-frame private address space.
6. Remaining: run broad HOPPYSOS and DByte workspace regressions and seal a fresh local successor receipt.

## Non-goals

- No change to `v13.1.0` HEAD/tag.
- No commit, push, tag, publish, or public release without later explicit approval.
- No move of LBA255 or HOPFS1 start at LBA256.
- No physical-storage write authorization; `REAL_STORAGE` remains `DISARMED`.
- No preemptive scheduler, fork/exec, dynamic linker, SMP, or general-purpose POSIX ABI claim.
- No promotion of legacy/i386 evidence into current x86-64 evidence.

## Acceptance

Each slice uses RED -> GREEN proof. Final acceptance requires QEMU 32/64 MiB,
exact-once markers, exactly one expected CPL3 `#UD`, `#GP`, and `#PF` receipt,
no other such fault and no triple fault, deterministic module/image hashes,
kernel <=252 sectors, protected dirty-file hashes unchanged, broad Rust
regression, and a fresh receipt that separates current evidence from
legacy/reference material.
