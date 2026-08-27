# DByte Allocators (v12.1)

Status: normative release contract for `v12.1.0`.

`AllocatorRef` is a Copy, non-owning capability containing a nullable context
pointer and a non-null static `AllocatorVTable`. The vtable exposes DByte-call
convention functions for allocate, transactional resize, and free.

Requests validate power-of-two alignment in `1..4096`. A successful nonzero
allocation is non-null and aligned. Resize failure preserves the old
allocation. Errors are `OutOfMemory`, `UnsupportedAlignment`, `SizeOverflow`,
or `ContractViolation`.

Zero bytes use a canonical internal non-null aligned dangling pointer and do
not call a provider. `0 -> N` is allocate, `N -> 0` frees then returns dangling,
and freeing `(dangling, 0)` is a no-op. The dangling address is not public ABI
and cannot be dereferenced.

`ArenaAllocator` is caller-backed and bump-only; individual free calls are
telemetry-visible no-ops. `FixedAllocator` is a bounded address-ordered
first-fit provider. It splits extents, coalesces adjacent releases, rejects
invalid or double releases fail-closed, and deterministically reuses the lowest
suitable extent. Provider alignment is computed from the absolute backing
address, not merely its relative offset.

Hosted and canonical VM execution share a deterministic byte-addressable
SystemAllocator adapter with identical allocation event streams, transactional
resize, zero-size behavior, and configurable OOM budgets. `KernelAllocator` is
an explicit vtable adapter exercised by the native proof. Freestanding
construction never selects a hidden fallback allocator.
