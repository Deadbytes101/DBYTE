# DByte Native Kernel Runtime

`v11.4.0` adds the first dynamic kernel-memory runtime implemented entirely in
ordinary DByte modules. It uses the existing systems language, paging
intrinsics, ELF32 object format, and in-process linker without adding syntax,
compiler-owned allocation, textual assembly, or a host runtime shim.

## Executed path

```text
Multiboot memory map
  -> reserved and allocation-ownership bitmaps
  -> releasable 4 KiB physical frames
  -> page-aligned arena page table in linked BSS
  -> fixed 0xC1000000..0xC1400000 supervisor arena
  -> transactional map and unmap with invlpg
  -> page-backed address-ordered first-fit heap
  -> split, exact reuse, adjacent coalescing, and tail decommit
  -> DByte memset, memcpy, and overlap-safe memmove
```

The frame allocator keeps reservation state separate from live ownership.
Only an aligned frame returned by `allocate` can be released; invalid,
reserved, unallocated, and repeated releases do not change either bitmap. A
successful release rewinds first fit so reuse is deterministic.

The heap accepts power-of-two alignment from 1 through 4096. A zero-size
allocation succeeds without mutation and returns null; `free(null)` is also a
successful no-op. Each block has boundary sizes, requested size, payload
offset, address-ordered free links, an encoded payload back-reference, state,
magic, and checksum. Arithmetic is checked before growth. Invalid frees leave
metadata unchanged, while checksum failure poisons the heap and makes later
operations fail closed with a stable corruption status.

Growth allocates, maps, and zeroes whole pages. A failed operation rolls back
only the pages and frames acquired by that operation. Coalesced free space at
the high end decommits full trailing pages, releases their physical frames,
and invalidates their TLB entries while retaining at least one metadata page.
Interior free pages remain mapped and are available for immediate reuse.

## Runtime acceptance

The verifier double-builds all objects, ELF, and BIN, then boots the exact
DByte-linked ELF with both 32 MiB and 64 MiB of QEMU RAM. Both runs must emit
these exact CRLF serial bytes:

```text
DBYTEOS NATIVE RUNTIME
VIRTUAL ARENA ONLINE
KERNEL HEAP ONLINE
ALLOC 001 OK
ALLOC 002 OK
FREE REUSE OK
ALIGNMENT OK
MEMORY OPS OK
RUNTIME CHECK OK
```

Port `0xE9` carries separate hexadecimal evidence for arena bounds,
allocation and reuse addresses, 4096-byte alignment, committed and high-water
pages, physical frame history, map/unmap/invlpg counts, coalescing, final error
state, CR0, and CR3. The proof exercises invalid alignment, size overflow,
outside/interior/double free, and mapping collision without poisoning valid
metadata. Any unexpected page fault, triple fault, dynamic dependency,
external generator, or orphan verifier QEMU process fails the gate.

Run:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_runtime_core.ps1
```

Ignored evidence is written under `tmp/native_v11.4.0/`.

## Deliberate boundary

This foundation provides one bounded kernel allocator. It does not add libc,
a compiler allocator, scheduler, threads, processes, userspace, filesystem,
disk, USB, networking, audio, GUI, SMP, sparse interior decommit, or physical
memory above the existing non-PAE 32-bit contract.
