# DBYTE64 ABI (v12.2)

Status: normative release contract for `v12.2.0`.

DBYTE64 scalar argument slots are `RAX`, `RDX`, `RCX`, `R8`, `R9`, and `R10`.
Hidden arguments are ordered closure environment, aggregate-result pointer,
then source parameters. Aggregate values are caller-created aligned stack
copies and do not consume scalar slots. Overflow scalar arguments occupy
eight-byte stack slots in right-to-left order.

The caller aligns `RSP` to 16 bytes immediately before `call`; DBYTE64 has no
red zone. `RBX`, `RBP`, and `R12` through `R15` are callee-saved. Scalar,
pointer, `i64`, and `u64` results use `RAX`. Aggregate results use caller-owned
storage passed as the hidden sret argument, and the callee returns that pointer
in `RAX`.

A closure value is `{code, environment}`. Captures occupy the generated
environment in source order; a capturing indirect call supplies the hidden
environment slot. A noncapturing function value is a checked code address.

`CallConvention::C` remains target-neutral in HIR. Linux maps it to SysV AMD64;
Windows maps it to Win64. C aggregate interoperability remains pointer-only
through compatible `@repr(C)` structures. Varargs, aggregate-by-value C calls,
float/SIMD classes, and unsupported large returns are rejected.

I686 `i64`/`u64` layout is eight bytes with four-byte alignment. Values are a
little-endian low/high pair, return in `EDX:EAX`, and use deterministic
compiler-owned helpers where a single i686 instruction cannot express the
operation. No compiler-rt or host runtime is linked.

## Current additive amendment (v13.1)

The rejection of C floating-point classes above is the frozen v12.2 boundary,
not the current toolchain boundary. Since v13.1, explicit `f32` and `f64`
arguments and results use the target C ABI directly: SysV64 uses its XMM
classification and Win64 uses positional `XMM0` through `XMM3`; scalar float
results use `XMM0`. The canonical HIR retains `CallConvention::C`, and target
legalization selects the ABI. No integer-register substitution, host runtime,
LLVM, or backend fallback is permitted. Varargs and aggregate-by-value C calls
remain unsupported.
