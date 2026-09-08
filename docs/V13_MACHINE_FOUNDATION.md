# V13 Freestanding Machine Foundation

Status: v13.0.0 is sealed as `VIRTUAL_QUALIFIED_SOFTWARE_FINAL` at commit `3b68536dc280dade3afde1620eeffbeb0309ec97` with annotated tag `v13.0.0`. G1-G5 virtual acceptance passed under the release verifiers and bound receipts described in [V13_SOFTWARE_RELEASE.md](V13_SOFTWARE_RELEASE.md). Real BIOSTAR hardware remains PENDING; real storage remains DISARMED; those physical boundaries are explicitly outside the software-final classification.

## Baseline and delivery boundary

The immutable authority is v12.4.0, commit
`a1a5c2458065f6f295d1c3e77df62a34846226e3`, annotated tag object
`28c074777f46bd9e8885f42c49f903ba34e19254`.
The sealed v13.0.0 release extends that frozen authority through a BIOS-first virtual machine foundation. Its release classification is `VIRTUAL_QUALIFIED_SOFTWARE_FINAL`, not physical hardware certification. Exact-commit verification, the annotated tag, deterministic packaging, clean-package smoke, and remote branch/tag verification completed under the release gates. Frozen packages and tags remain immutable; physical storage writes remain separately disarmed. The generation-specific audit and evidence below retain their historical scope and do not substitute for the final release receipts.

## Evidence-grounded capability audit

Evidence below is static source inspection of the frozen release, not a new CPU
execution receipt. Existing verifier names identify reproduction paths, not claims
that those verifiers were rerun for this documentation/tools-only delivery.

| Subsystem | Source / validation path | Classification and promotion work |
| --- | --- | --- |
| Canonical semantics | `crates/dbyte_hir/src/lib.rs`: HirIntrinsic, intrinsic signature/profile checks; `executable.rs`: concrete bodies | Reusable unchanged as authority; each new machine capability must be resolved here before MIR. |
| Native lowering | `crates/dbyte_native/src/cli.rs`: compile_mir_i686_project_objects; `modern_native.rs`: compile_mir_object_source; `native_mir_lower.rs`: lower_program_to_native_mir_with_link_names | Reusable pipeline. Never route new firmware/kernel code through AST or legacy systems checker. |
| Machine representation | `native_mir.rs`: legalize_project, allocate_registers; `native_select_i686.rs` and `native_select_x86_64.rs`: selectors | Reusable representation/ABI engines. Intrinsic existence in HIR or i686 does not imply x86_64 support. |
| Encoders | `x86.rs`, `x86_64.rs` | Reusable bounded instruction emission. Add operand/capacity tests for each required new form. |
| Objects/linking | `elf32.rs`, `elf64.rs`, `coff64.rs`, `pe64.rs`, `linker.rs`, `target.rs` | Reusable format machinery. Windows PE entry/import policy is not an EFI application contract; target.rs currently has no UEFI or x86_64 DByte-user target. |
| x86_64 boot | `multiboot2.rs`: wrap_payload, write_bootstrap, write_transition_data; `verify_modern_native_none.ps1` | Proof-only bootstrap topology; retain as oracle. Generated transition contains MSR/control-register operations, but those bytes do not expose an executable DByte kernel primitive. |
| x86_64 hardware surface | `native_select_x86_64.rs`: select_intrinsic match accepts IoIn8, IoOut8, CpuCli, CpuHlt (plus hosted OS capabilities); rejects interrupt/task addresses | Width/ABI adaptation required. Explicit volatile intrinsic selection, descriptor operations and full interrupt entry/resume remain gaps. MIR memory flags alone do not establish usable MMIO ordering. |
| i686 interrupts | `native_select_i686.rs`: select_interrupt_stub/select_intrinsic; `examples/native_user_process_core/idt.dby`, `gdt.dby` (Tss32 and cpu_ltr) | Reuse interrupt ownership and failure rules. PUSHAD/IRETD, descriptor widths and TSS32 must be replaced by a documented 64-bit save image/TSS64/IRETQ path. |
| Frames and paging | `examples/native_user_process_core/frames.dby`, `paging.dby`, `boot.dby` | Reuse reservation/ownership/rollback algorithms after removing 32-bit range assumptions. Four-level translation, firmware reservations, cache attributes and DMA restrictions are missing. |
| Heap and runtime | `runtime/v12.1.0`, `runtime/v12.2.0`, `runtime/v12.3.0`; proof heap modules | Reuse allocator/Drop/collections contracts; use explicit kernel provider. OS-hosted SystemAllocator is not a freestanding provider. |
| Scheduler/blocking | `examples/native_user_process_core/scheduler.dby`, semaphore/event modules in historical proofs | Reuse bounded queue, wake, EOI and deferred reap rules. Resume image, stack slots and checksum/address fields require 64-bit review. |
| Input | `keyboard.dby`, `byte_ring.dby` and IRQ-I/O verifier | Ring/semaphore behavior reusable. i8042/D2 injection is a proof transport, not evidence of a real keyboard path. |
| Storage | `ata.dby`; `verify_native_block_device_core.ps1` | ATA primary-master PIO is topology-specific. Reuse request ownership/completion/error contracts; select one discovered real controller later. |
| DBFS1 | dbfs/layout/bitmap/inode/directory modules, filesystem_runtime, file_handles, block_cache in `examples/native_user_process_core` | Reuse on-disk encoding and validated ownership rules. Replace direct ATA dependency with a narrow sector service, add durability and VFS dispatch. Do not reinterpret old disk bytes. |
| Ring3 | `processes.dby`; `verify_native_user_process_core.ps1` | Reuse FD isolation, worker completion, copy validation and fault containment. ELF32 loader, CR3, TSS and saved context are not x86_64-ready. |
| Discovery | Search of tracked native sources/docs for UEFI, ACPI/MADT/MCFG, PCI enumeration | No production discovery chain identified. Add bounded firmware/table readers before driver initialization. A negative search is not hardware evidence. |

## Hardcoded assumptions to retire or explicitly retain

| Location | Current assumption | Required boundary |
| --- | --- | --- |
| multiboot2.rs / ELF64_MULTIBOOT2.md | Wrapper 1 MiB, structures below 2 MiB, payload 2 MiB; identity map 1 GiB using 2 MiB pages | G1 keeps this BIOS wrapper but validates the Multiboot2 handoff and reserves the reviewed image range; broader paging and any future UEFI path are later boundaries. |
| boot.dby | Historical Multiboot1 magic 0x2badb002; map length <=64 KiB; 32-bit map arithmetic | G1 does not reuse this parser: `native_v13_machine/multiboot2.dby` validates Multiboot2 tag sizes/extents and normalizes checked u64 regions through BootInfo. |
| frames.dby | Two 128 KiB bitmaps describe 4 GiB of 4 KiB frames | Firmware-derived capacity and explicit metadata storage ownership. |
| paging.dby | Kernel/page structures below 4 MiB; 32-bit masks; heap C1000000-C1200000; stacks C1200000-C1400000; scratch C1400000 | New documented canonical x86_64 VA layout and four-level tables. |
| scheduler/processes | MAX_TASKS=8, MAX_PROCESSES=4; four-page stack slots; user top C0000000 | Keep bounded counts initially; redesign addresses and resume layout, not resource policy. |
| serial.dby | COM1 0x3f8 | Optional discovered/configured diagnostic device; framebuffer receipt when unavailable. |
| pic/pit | PIC ports 20/21/A0/A1, vectors 20/28; PIT divisor 11932; masks F8/BF | MADT/APIC/IOAPIC routing and measured timer source. No assumption of legacy IRQ topology. |
| keyboard.dby | ports 60/64; D2 injection; 65,536 polling attempts and 64-byte drain | Hardware presence/transport evidence; retain bounded waits, use elapsed-time deadline once timer exists. |
| ata.dby | 1f0-1f7/3f6 primary master; LBA28; 512-byte sectors; 1,000,000 polls | Real PCI controller, DMA and sector-size capability checks; reject unsupported media geometry. |
| DBFS1 | base LBA64, 64 inodes x64 bytes, eight direct blocks, 64 entries per directory, file <=4096 bytes | Explicit first-filesystem limitations; reserve a dedicated volume. Do not format arbitrary whole disks. |
| proof verifiers | QEMU IDE/raw 8 MiB disk, RAM matrices, exact IRQ/tick counts, debugcon E9, bounded boot timeout | Keep virtual oracle receipts separate from real timing/controller/disk evidence. |
| user process | compact two-segment ELF32, fixed user VAs, 19 FD slots, software INT80 | New checked ELF64/user ABI; preserve stable logical errors and ownership where applicable. |

## Dependency-ordered generations

**G0:** audit, collectors, fixtures, and hardware selection criteria.

**G1 (current terminal PASS):** BIOS-first Multiboot2 x86_64 kernel path with checked two-argument entry ABI, versioned BootInfo, bounded memory-map normalization, ACPI RSDP/RSDT/MADT/FADT validation, APIC/IOAPIC/timer discovery, read-only PCI mechanism #1 enumeration, COM1/VGA legacy diagnostics, deterministic ELF/ISO packaging, and permanent halt after `G1 COMPLETE`. No PCI config writes beyond selector writes to `0xCF8`; `0xCFC` is read-only. No storage writes. UEFI is deferred outside the G1 boundary.

**G2:** CPUID and feature checks, MSRs as used by APIC/CPU setup, descriptor/control
register operations, width-correct volatile MMIO, necessary I/O widths and explicit
compiler/device ordering. Define TSS64/IST and complete GPR/IRETQ save image before
scheduling. Build firmware-derived physical ownership, four-level paging, reserved
regions, permissions and guards; test >4 GiB physical addresses and DMA constraints.
Add primitives only with concrete consumers and O0/O1 side-effect tests. Volatile is
not a cache-type declaration or a device memory barrier.

**G3:** validate RSDP/XSDT/MADT/FADT/MCFG checksums, lengths and referenced physical
ranges; enumerate PCI bridges/functions from discovered topology. Read-only discovery
does not size BARs by writing them or enable bus mastering. Bring up BSP/APIC/IOAPIC,
interrupt overrides and a calibrated timer. Unsupported firmware requiring AML or
unimplemented routing must fail explicitly; no guessed interrupt routing. No SMP.

**G4:** after hardware selection, implement one storage family and the observed
input path. Reuse completion semaphore/request contracts with bounded error/reset
handling. Add sector service -> DBFS1 -> minimal VFS and durability/flush proof.
If USB is necessary for selected input, make that an explicit dependency before
interactive acceptance. Firmware keyboard input cannot satisfy post-handoff input.

**G5:** x86_64 DByte user ELF, private address spaces, worker syscall boundary,
owner-qualified FDs, validated copyin/copyout and deferred fault/exit reclaim.
Run an interactive shell loading multiple programs from disk; prove persistence
after reboot and survival after hostile userspace faults. GUI/network are deferred.

Each generation has source tests, O0/O1 behavior/ABI checks, deterministic double
build, disassembly, bounded QEMU execution and negative fault/error tests. Malformed ACPI/Multiboot2/ELF, overlap/overflow, invalid resources, allocation rollback and interrupt frame tests become mandatory as their implementations arrive. UEFI-specific negative tests remain deferred until a UEFI path exists.

## Hardware acceptance and write authorization

The collectors and receipt format are documented in [MACHINE_INVENTORY.md](MACHINE_INVENTORY.md).
No remote-access contract exists for the physical candidate. G1 is explicitly BIOS-first: the certified artifact is the Multiboot2 ISO from the terminal verifier. Real BIOSTAR promotion requires booting that exact hash and returning serial/fault evidence. UEFI capability may be inventoried later, but it is not a G1 promotion requirement and does not replace the BIOS evidence path.

Record PRIMARY_REAL_HW_TARGET, BOOT_PATH, STORAGE_DRIVER, INPUT_DRIVER and
DEDICATED_TEST_DISK only after reviewing inventory. Unknown topology stays UNSELECTED.
No inference from motherboard name or CPU age is accepted as device discovery.

Before any real write gate, require explicit user confirmation of a disposable
disk's model, serial, current device path, capacity and absence of valuable data.
Separate boot-medium provisioning from filesystem test-disk writes. Inventory is
not authorization to initialize either medium. Revalidate identity, controller and
system/boot associations immediately before a run and inside the guest before write
submission. Unknown, changed, OS/system or ambiguous disks mean **NO WRITE**. Host
device paths alone are not stable identity; RAID/virtual backing ambiguity blocks.
Test setup must isolate non-test disks from experimental driver/DMA access.

Real receipts bind machine fingerprint, firmware/configuration evidence, artifact
hashes, disk identities, boot/input/output capture, discovery, memory/interrupts,
storage flush, reboot/readback, userspace execution and fault containment. Fingerprint
matching alone proves none of these. QEMU has its own receipt, not a hardware waiver.

Current status: G1-G5 virtual acceptance = PASS; v13.0.0 FINAL = SEALED at commit `3b68536dc280dade3afde1620eeffbeb0309ec97`; annotated tag `v13.0.0` peels to that exact commit; canonical Windows package SHA-256 is `40468ef823543b23638d633fbfc5e7e82b6da23ce4659238a3c427f2916337d9`; final receipt SHA-256 is `bf177552f4c75e70d6c7dd38b7de6a48f5520d71b3fed6f43aadb08556f5412f`. REAL BIOSTAR = PENDING; REAL STORAGE = DISARMED; FULL OS = PENDING. The earlier certified G1 candidate remains bound to evidence root `tmp/v13_g1/generation1-91cfde928e954d228cc03d24527472ba`, ELF SHA-256 `263081D0B9B5B9DB209BE2474C5CFBD4FA320787D075876BD39C8768A221BCA4`, and ISO SHA-256 `B1E62CAA6C90A3B6175AF403FF9A4B5668A556557A928F723A5DD3EFE68A4D02` (11,427,840 bytes). Historical-byte gates run from immutable exports; that G1 evidence remains historical and does not replace the final v13 receipts.

Normative external references for later implementation: [UEFI Boot Services](https://uefi.org/specs/UEFI/2.10_A/07_Services_Boot_Services.html),
[UEFI x64 image/entry conventions](https://uefi.org/specs/UEFI/2.10/02_Overview.html),
[ACPI table model](https://uefi.org/specs/ACPI/6.6/05_ACPI_Software_Programming_Model.html).
