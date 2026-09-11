# v13.1.0 portable native float release

Status: v13.1.0 software release boundary. The release identity is valid only when the exact-commit verifier, annotated tag, and deterministic package reproduction all bind to the same commit.

The immutable parent is v13.0.0 at commit `3b68536dc280dade3afde1620eeffbeb0309ec97`. Its annotated tag and historical package are never moved or rewritten.

## Additive scope

- `dbytec check-portable` validates one canonical project across ten supported profile/layout combinations and rejects target-dependent semantics with deterministic diagnostics.
- The portable runtime parity proof executes the same fixed-width source through the hosted tree runtime, canonical VM, Windows x86_64 O0/O1, and Linux x86_64 O0/O1.
- Explicit `f32` and `f64` values retain exact representation bits through canonical HIR, hosted execution, VM bytecode, Native MIR, memory, calls, and returns.
- The owned native backend implements scalar SSE2 arithmetic and comparisons without LLVM, Cranelift, a host runtime, or a backend fallback.
- SysV64 and Win64 C interoperability uses the target floating-point ABI, including XMM argument/return transport and tracked C fixtures.
- Existing integer, pointer, ownership, Drop, allocator, collection, VM, i686, freestanding, Linux, and Windows contracts remain unchanged.

## Hardware boundary

Physical hardware qualification is outside the v13.1.0 software-release gate. Real storage remains `DISARMED`; no physical persistence or full-machine claim is part of this release.

## Release closure

1. Exclude user-owned `examples/smoke.dby` and `examples/.cube-state.bin` from the release commit.
2. Run format, warnings-as-errors Clippy, workspace tests, focused portability/native-float proofs, and the serialized full verifier.
3. Commit only reviewed canonical source, fixtures, documentation, manifests, and verifiers.
4. Rerun release-critical checks against the clean exact commit.
5. Create annotated tag `v13.1.0` only after post-commit gates pass, then verify its peeled commit.
6. Export the tagged commit twice, build deterministic Windows packages independently, require identical ZIP SHA-256, and run clean extraction smoke tests.
7. Seal a receipt binding the commit, tag object, package manifest, package SHA-256, focused/full gates, and the hardware non-goal boundary.
