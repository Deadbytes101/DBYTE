# DByte Native Memory Core

`v11.3.0` adds a bounded memory-management vertical slice to the native DByteOS
kernel. The image is still compiled only from multi-module `.dby` sources into
DByte ELF32 objects and linked by the in-process DByte linker. Rust
`kernel-lab` remains an independent oracle and is not a runtime dependency.

## Executed path

```text
Multiboot EAX/EBX
  -> typed two-argument DByte entry
  -> validated bounded memory map
  -> reserved-wins 4 KiB frame bitmap
  -> two distinct physical frame allocations
  -> page-aligned directory and tables
  -> CR3 load and CR0.PG enable
  -> supervisor write to unmapped 0x00400000
  -> CPU vector 14 error-code entry stub
  -> DByte page-fault body reads CR2 and error code 2
  -> PTE install and invlpg
  -> iretd retries the original write
```

The relocatable root object carries canonical non-loadable `.dbyte.entry`
metadata. Legacy objects without the section retain the zero-argument entry
contract. The linker owns the image and bootstrap-stack boundary symbols; the
language exposes only the four checked typed intrinsics required by the frame
allocator.

The allocator starts with every 4 KiB frame unavailable, clears type-1 ranges,
then marks all non-usable ranges again so reserved entries win independently of
map order. It also reserves memory below 1 MiB, the linked image, bootstrap
stack, Multiboot information and map, and the page-structure storage embedded
in the linked BSS. This milestone allocates monotonically and has no free API.

## Runtime acceptance

The verifier boots the same deterministic ELF with 32 MiB and 64 MiB RAM. Both
runs must emit these exact CRLF serial bytes:

```text
DBYTEOS NATIVE MEMORY
MEMORY MAP ONLINE
FRAME ALLOCATOR ONLINE
PAGING ONLINE
PAGE FAULT ABI ONLINE
PAGE ALLOC 001
PAGE ALLOC 002
MEMORY CHECK OK
```

Port `0xE9` carries separate hexadecimal telemetry for map count, image and
stack ranges, Multiboot addresses, allocated frames, aligned page structures,
CR0, CR2, CR3, and the page-fault error code. QEMU's interrupt trace must show
`v=0e e=0002` with `CR2=00400000`; a fabricated `int 14`, triple fault, dynamic
runtime dependency, or external generator/linker fails the proof.

Run:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_memory_core.ps1
```

Ignored evidence is written under `tmp/native_v11.3.0/`.

## Deliberate boundary

The memory core uses 32-bit non-PAE 4 KiB pages and identity maps the first
4 MiB. It does not add a heap, freeing, scheduler, threads, filesystem,
userspace, PAE, 64-bit physical arithmetic, Multiboot modules, ACPI, SMP, USB,
networking, audio, or GUI support.
