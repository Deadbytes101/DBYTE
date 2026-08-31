# DByte VM Performance and Collections Contract

Status: normative contract for v12.3.0

This document defines the v12.3 canonical VM representation, measurement
authority, and collection runtime contracts.

## Canonical VM authority

Production VM execution has one semantic input: `ConcreteHirProject`. The VM
compiler lowers each concrete function to a `CanonicalBytecodeBody` containing
a flat instruction vector and deterministic frame layout. Instructions retain
source spans but do not contain executable expressions, statements, or nested
blocks.

Control flow is represented by finalized instruction indices. Direct calls use
`FunctionInstanceId`; closures use `ClosureId` plus their concrete capture
environment. Locals and temporaries use dense frame slots. Only address-taken,
borrowed, or captured locals require stable cells.

The executor uses one explicit instruction pointer and a reusable frame stack.
It does not recurse through bytecode dispatch, rediscover names or types, or
invoke the legacy AST compiler. The maximum of 16 active DByte calls is checked
at the call boundary and reports `maximum call depth exceeded`.

O0 and O1 use the same flat representation and dispatcher. O1 may perform only
side-effect-safe constant and branch folding, direct-call binding, dead pure
temporary elimination, redundant copy or move removal, alias-safe load/store
cleanup, and conservative local fusion. It must preserve error timing, bounds,
borrows, allocation events, cleanup order, and Drop behavior.

## Performance authority

Benchmarks compare the exact `v12.2.1^{}` baseline with the candidate after
analysis and bytecode lowering. Compilation and startup are reported separately
from execution. Each full receipt uses three discarded warmups and nine measured
runs, with median as the decision statistic. Relative MAD must be at most five
percent after at most three complete attempts.

The core geometric-mean speedup must be at least 1.25x and no established case
may fall below 0.90x without explicit review. Profiled execution collects opcode,
call, allocation, bytecode-size, frame, stack, and live-allocation telemetry in
a separate executor specialization; normal production dispatch contains no
counter path.

The tracked NMOS 6502 corpus owns its ROMs, opcode matrix, traces, and expected
state. It covers all 151 documented legal opcodes, including decimal arithmetic,
stack/control behavior, address wrapping, signed branches, and the indirect JMP
page-wrap quirk. The million-instruction historical workload must preserve exact
state and achieve at least 1.20x speedup.

## Collection modules

Collections are versioned DByte runtime modules. They use explicit
`AllocatorRef`, borrow, move, and Drop contracts; the compiler has no collection
intrinsics and supplies no implicit Hash, Eq, Clone, or allocator behavior.

### HashMap and HashSet

`HashMap[K,V]` accepts non-capturing typed hash and equality function pointers.
It uses cached `u64` hashes, power-of-two capacity, linear probing, a maximum
75-percent load, and wrap-safe backward-shift deletion without tombstones.
Growth allocates and populates a replacement table before commit, so allocation
failure leaves the original map unchanged. Duplicate insertion retains the
stored key, consumes the incoming key, and returns the old value.

`HashSet[T]` is the same table core with `unit` values. Integer helpers use the
canonical SplitMix64 finalizer; byte-string helpers use deterministic FNV-1a
64-bit. There is no hidden seed, pointer hashing, or public iteration contract.

### Deque and Queue

`Deque[T]` is an allocator-backed circular buffer. Capacity grows from zero to
eight and then doubles transactionally. Front/back push and pop, indexed borrow,
reserve, clear, length, and empty queries preserve logical order across wrap and
growth. Clear and Drop destroy elements from logical back to front exactly once.

`Queue[T]` is a strict-FIFO wrapper over the Deque core and does not own a second
storage engine.

### Buffer

`Buffer[T]` owns fixed-length contiguous storage. Its ABI fields are data,
length, allocator context, and allocator vtable: 16 bytes on i686 and 32 bytes on
x86_64. Its length and backing address do not change during its lifetime.

Safe creation is through `buffer.from_fn` or, for canonically Copy elements,
`buffer.from_slice`. Zero length uses the canonical non-null dangling pointer and
does not call the provider. There is no public uninitialized, zeroed, resize,
reserve, push, or pop operation. Access uses checked `get`/`get_mut` or borrowed
slices. Drop destroys elements in reverse index order and frees the backing
allocation once.

## Target contract

The same collection proof must execute through hosted Concrete HIR, flat VM O0
and O1, i686, x86_64-none, x86_64-linux, and x86_64-windows. Native outputs must
come from DByte objects and internal linkers, retain the required callback and
allocator relocations before link, and have no unexpected dynamic or external
runtime dependency.
