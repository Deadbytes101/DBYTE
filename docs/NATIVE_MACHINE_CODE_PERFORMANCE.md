# DByte Native Machine Code Performance Contract

Status: normative contract for v12.4.0

This document defines the v12.4 native backend architecture and its
measurement authority for the release.

## Semantic boundary

Concrete HIR is the only semantic input. Native code generation consumes
resolved types, layouts, symbols, calls, ownership actions, cleanup edges, and
intrinsics without resolving source names or repeating type, borrow, generic,
Drop, or profile decisions.

The production pipeline is:

```text
Concrete HIR
-> Native MIR
-> target legalization
-> O0 or O1 optimization
-> deterministic linear-scan allocation
-> instruction selection and peephole
-> bounded DByte encoder
-> DByte object and executable writers
```

Native MIR consists of deterministic basic blocks, target-sized virtual
registers, explicit stack objects, explicit memory effects, resolved calls and
intrinsics, relocations, and source spans. Scalar locals live in virtual
registers. Only aggregates and locals whose address is observably required may
receive stack objects before register allocation.

Legacy direct-HIR DBYTE64 encoding and i686 evaluation-stack lowering are
differential-test oracles only once the v12.4 production routes are complete.
There is no AST, systems-semantic, or source-name fallback.

## Optimization levels

`--opt-level 0` and `--opt-level 1` select Native MIR O0 and O1 for native
builds. O1 is the default. Integrated object and executable builds use one
level throughout compilation; standalone linking does not optimize.

O0 retains the same MIR and instruction selector as O1 but disables optimizing
passes and uses deterministic conservative spilling. O1 applies, in fixed
order, CFG cleanup, constant and copy propagation, dead pure temporary
elimination, bounded local CSE, alias-safe memory forwarding, address and
immediate folding, compare-branch fusion, constant branch simplification,
fault-preserving arithmetic strength reduction, and post-selection peepholes.

Neither level may alter fault or error timing, volatile operations, bounds and
borrow checks, cleanup and Drop order, allocator events, calls, or observable
memory effects. In particular, division by zero and signed minimum divided by
minus one retain their architectural fault behavior.

## Register allocation and frames

Allocation uses deterministic live intervals and explicit call-clobber sets.
Values live across calls cannot occupy caller-clobbered registers. Fixed ABI
operands for calls, shifts, and division are legalized before selection. Spill
slots are aligned and may be reused only when their intervals do not overlap.

DBYTE64 reserves RSP, RBP, and R11. DBYTE32 reserves ESP and EBP. Callee-saved
registers are saved only when assigned. Frames contain only addressable locals,
aggregates, spills, and the maximum outgoing-call area; Win64 additionally
retains its required 32-byte shadow space. DBYTE32, DBYTE64, i686 cdecl,
SysV64, and Win64 external contracts do not change in v12.4.

## Inspection

`--dump-native-mir` prints deterministic per-function blocks, virtual
registers, stack objects, typed operations, terminators, and source spans.
Machine inspection reports MIR and selected instruction counts, text and frame
bytes, virtual and assigned registers, spills, reloads, loads, stores, calls,
and branches. Dumps are observational and cannot change emitted bytes.

## Performance authority

Linux x86_64 is the primary wall-time authority. Exact v12.3.0 tag exports form
the baseline in two independent roots. Candidate O0 and O1 artifacts are built
independently and verified before execution. Linux measurements run with the
manifested single-logical-CPU affinity inherited by the measured children to
bound scheduler migration noise. Windows measurements create each child
suspended at HIGH process priority, apply the manifested CPU affinity, and only
then resume execution; failure to establish any launch control is a hard
measurement error. Compilation and startup are reported separately.

Each measured workload uses three discarded warmups and nine samples. The
manifested workload multiplier executes the same artifact eight times per
sample and records the summed duration; it is identical for baseline and
candidate and is never selected by workload name. After warmup, both platform
runners use the same manifested untimed settling interval before sampling so
host inspection and scheduler transients do not dilute the execution signal.
The receipt records both controls. Median, minimum, maximum, and MAD are
recorded. Relative MAD must not exceed five percent after at most three
complete attempts. Linux O1 must achieve at least a 1.25 geometric-mean
speedup over v12.3 and no workload may fall below 0.90. Windows enforces the
0.90 regression floor. i686 and x86_64-none enforce ABI, correctness, and
code-quality deltas.

The corpus covers integer and branch loops, direct and indirect calls,
recursion, aggregates, arrays, Buffer and String traversal, HashMap and Deque,
allocator churn, 64-bit arithmetic, and the tracked 151-opcode NMOS 6502
workload. O0 and O1 must agree on output, exit or error class, final memory,
Drop order, allocator events, and ownership state.

Hardware counters are optional. When unavailable, receipts record the failed
capability probe; wall-time, object, relocation, ABI, and disassembly gates
remain mandatory. Reference C, Rust, and Zig builds are gap telemetry only and
are never semantic authorities or release thresholds.

## Release boundary

The v12.4 release is blocked until every native target uses Native MIR in
production, fresh performance and 6502 receipts pass, all native artifacts
double-build identically, historical verifiers pass from immutable exports,
and the repository is clean. LLVM, Cranelift, external compiler backends, JIT,
PGO, LTO, SIMD, new language semantics, website changes, and DByteOS work are
outside this milestone.
