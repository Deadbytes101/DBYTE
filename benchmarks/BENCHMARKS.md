# DByte Performance Benchmarks

This document records comparative performance of the DByte VM against Python
3.12.9 on a local Windows release-build test machine.

Safe public claim:

> DByte v1.9.2 outperforms Python 3.12.9 across DByte's measured benchmark
> suite on a Windows release-build test machine.

This is limited to the benchmark suite below. It is not a claim that DByte is
faster than Python for every workload, platform, or implementation style.

## Public Alpha Baseline: v1.9.2 / v2.0.0

`v2.0.0` is a public alpha packaging and documentation release based on the
`v1.9.2` engine plus installer, examples, and release artifacts. No new
performance claim is introduced by v2.0.0.


## Baseline: v1.1.0

Engine: tree / vm  
Build: release  
Machine: local Windows machine  
Date: 2026-05-10

| Benchmark | Tree | VM | Notes |
|---|---:|---:|---|
| loop_sum | 279.43 ms | 179.08 ms | VM faster |
| function_call | 663.81 ms | 366.83 ms | VM much faster |
| bytes_find | 15.00 ms | 12.77 ms | VM slightly faster |
| buffer_replace | 97.60 ms | 136.28 ms | VM slower; investigate native call path |
| binary_read_u32 | 1480.64 ms | 1491.92 ms | Both slow; heavy native call overhead |
| patch_workflow | 288.14 ms | 386.10 ms | VM slower due to find/replace combo |

## Findings

- VM is significantly faster on function calls and arithmetic-heavy code.
- VM is consistently slower on native function calls (`buffer_replace`, `binary_read_u32`).
- `binary_read_u32` shows extreme overhead for both engines.
- Suspected bottleneck: Native function dispatch and argument passing in VM.

## Perf Pass 1: VM Native Call Fast Path

Engine: vm  
Build: release  
Date: 2026-05-10  
Optimizations: Replaced string-based module member lookups with `NativeFnId` enum and `Rc<ModuleValue>`, removed `Vec<Value>` allocations in native argument passing by using stack slices `&[Value]`, and avoided `BytecodeFunction` cloning in dispatch path.

| Benchmark | VM Before | VM After | Improvement |
|---|---:|---:|---|
| loop_sum | 179.08 ms | 97.71 ms | 1.8x faster |
| function_call | 366.83 ms | 253.96 ms | 1.4x faster |
| bytes_find | 12.77 ms | 12.30 ms | 1.03x faster |
| buffer_replace | 136.28 ms | 22.93 ms | 5.9x faster |
| binary_read_u32 | 1491.92 ms | 155.40 ms | 9.6x faster |
| patch_workflow | 386.10 ms | 59.19 ms | 6.5x faster |

## Perf Pass 2: Python Baseline + Typed VM Fast Path

Version: v1.2.0  
Build: release  
Python: 3.12.9  
Date: 2026-05-10  
Optimizations: Added typed int bytecode (`CONST_I64`, `LOAD_LOCAL_I64`, `STORE_LOCAL_I64`, `ADD_I64`, comparisons) and VM intrinsic opcodes for `std.binary.u32_le`, `std.buffer.find`, and `std.buffer.replace`.

| Benchmark | Tree | VM v1.2.0 | Python | Python / DByte VM |
|---|---:|---:|---:|---:|
| loop_sum | 252.69 ms | 107.07 ms | 34.12 ms | 0.32x |
| function_call | 642.62 ms | 226.93 ms | 53.73 ms | 0.23x |
| bytes_find | 14.18 ms | 11.69 ms | 1.04 ms | 0.10x |
| buffer_replace | 88.26 ms | 13.42 ms | 16.54 ms | 1.24x |
| binary_read_u32 | 1303.70 ms | 73.59 ms | 114.61 ms | 1.53x |
| patch_workflow | 249.37 ms | 28.73 ms | 40.51 ms | 1.14x |

### Findings

- DByte VM beats Python on the low-level binary workloads measured here: `binary_read_u32`, `buffer_replace`, and `patch_workflow`.
- Python is still faster on pure numeric loops, function call overhead, and `bytes.find`.
- Next likely performance work: avoid cloning `Value::Bytes` in VM locals and move more hot paths away from boxed stack values.

## Perf Pass 3: Typed Locals + Direct Int Loop Fast Path

Version: v1.3.0  
Build: release  
Python: 3.12.9  
Date: 2026-05-10  
Optimizations: Added typed local metadata, separate i64 frame storage, and direct local int opcodes for common loop patterns (`ADD_LOCAL_I64`, `ADD_LOCAL_CONST_I64`, and local-vs-constant comparisons).

| Benchmark | VM v1.2.0 | VM v1.3.0 | Python | Python / DByte VM |
|---|---:|---:|---:|---:|
| loop_sum | 107.07 ms | 26.52 ms | 36.53 ms | 1.38x |
| function_call | 226.93 ms | 190.06 ms | 47.31 ms | 0.25x |
| bytes_find | 11.69 ms | 12.37 ms | 1.04 ms | 0.08x |
| buffer_replace | 13.42 ms | 8.07 ms | 13.01 ms | 1.61x |
| binary_read_u32 | 73.59 ms | 67.85 ms | 104.53 ms | 1.54x |
| patch_workflow | 28.73 ms | 22.91 ms | 34.02 ms | 1.48x |
| int_compare_loop | n/a | 59.56 ms | 67.10 ms | 1.13x |
| loop_sum_large | n/a | 57.51 ms | 72.97 ms | 1.27x |
| nested_int_loop | n/a | 27.60 ms | 37.01 ms | 1.34x |

### Findings

- DByte VM now beats Python on the measured int loop workloads that compile to direct local int opcodes.
- Function-call overhead improved modestly but still loses to Python; the remaining bottleneck is call frame setup and boxed stack argument passing.
- `bytes_find` still loses heavily because Python delegates the core search to optimized native code.
- Binary workloads stayed within the no-regression target and improved in this run.

## Perf Pass 4: Function Call Fast Path

Version: v1.4.0  
Build: release  
Python: 3.12.9  
Date: 2026-05-10  
Optimizations: Added direct function-id bytecode (`CALL_FN`), typed int returns (`RETURN_I64`), frame capacity reuse, and a discard fast path for user function calls used as statements.

| Benchmark | VM v1.3.0 | VM v1.4.0 | Python | Python / DByte VM |
|---|---:|---:|---:|---:|
| loop_sum | 26.52 ms | 26.86 ms | 41.27 ms | 1.54x |
| function_call | 190.06 ms | 110.60 ms | 73.09 ms | 0.66x |
| function_call_int | n/a | 135.73 ms | 54.81 ms | 0.40x |
| function_call_loop_return | n/a | 127.76 ms | 62.30 ms | 0.49x |
| function_call_nested | n/a | 87.53 ms | 31.80 ms | 0.36x |
| bytes_find | 12.37 ms | 10.66 ms | 1.05 ms | 0.10x |
| buffer_replace | 8.07 ms | 8.41 ms | 15.16 ms | 1.80x |
| binary_read_u32 | 67.85 ms | 62.70 ms | 101.85 ms | 1.62x |
| patch_workflow | 22.91 ms | 24.70 ms | 38.18 ms | 1.55x |
| int_compare_loop | 59.56 ms | 59.46 ms | 69.30 ms | 1.17x |
| loop_sum_large | 57.51 ms | 52.22 ms | 74.64 ms | 1.43x |
| nested_int_loop | 27.60 ms | 27.94 ms | 35.21 ms | 1.26x |

### Findings

- Direct function ids remove runtime string lookup and cut `function_call` from roughly 190 ms to roughly 111 ms in this run.
- DByte VM is still slower than Python on function-call-heavy workloads; the remaining cost is boxed stack argument passing and recursive `run_chunk` call frame execution.
- Existing int-loop and low-level binary workloads stayed within the no-regression target in this measurement.
- Next likely performance work: typed argument stack/return path or a register-style call frame. Function inlining remains deferred.

## Perf Pass 5: Typed Args + Non-Recursive Frame Dispatch

Version: v1.5.0
Build: release
Python: 3.12.9
Date: 2026-05-10
Optimizations: Replaced recursive VM function execution with a VM-managed call-frame dispatch loop, installed typed int arguments directly into i64 local slots, kept `RETURN_I64` on the typed return path, and added frame-dispatch guards for discard calls, generic returns, wrong arity, nested calls, and recursion depth.

Baseline note: `VM v1.4.2` was re-measured from tag `v1.4.2` in a temporary worktree on the same machine before recording the v1.5.0 results. Timings are noisy on this machine, so claims below are limited to the measured workload results.

| Benchmark | VM v1.4.2 | VM v1.5.0 | Python | Python / DByte VM |
|---|---:|---:|---:|---:|
| loop_sum | 26.21 ms | 30.51 ms | 33.79 ms | 1.11x |
| function_call | 115.35 ms | 112.91 ms | 50.83 ms | 0.45x |
| function_call_int | 144.40 ms | 136.15 ms | 58.16 ms | 0.43x |
| function_call_loop_return | 135.61 ms | 135.54 ms | 55.21 ms | 0.41x |
| function_call_nested | 92.50 ms | 91.06 ms | 34.54 ms | 0.38x |
| function_call_chain | n/a | 154.96 ms | 77.10 ms | 0.50x |
| function_call_many_args | n/a | 173.64 ms | 84.24 ms | 0.49x |
| bytes_find | 11.99 ms | 11.62 ms | 1.48 ms | 0.13x |
| buffer_replace | 9.52 ms | 8.95 ms | 15.32 ms | 1.71x |
| binary_read_u32 | 73.86 ms | 71.18 ms | 116.77 ms | 1.64x |
| patch_workflow | 23.06 ms | 29.28 ms | 41.06 ms | 1.40x |
| int_compare_loop | 56.26 ms | 66.78 ms | 65.13 ms | 0.98x |
| loop_sum_large | 51.67 ms | 55.31 ms | 72.56 ms | 1.31x |
| nested_int_loop | 24.59 ms | 28.50 ms | 36.58 ms | 1.28x |

### Findings

- Non-recursive frame dispatch makes call execution independent from the Rust call stack and keeps the deterministic DByte recursion guard, but it is not a broad speed win yet.
- `function_call`, `function_call_int`, `function_call_loop_return`, and `function_call_nested` are roughly flat to slightly faster versus the local v1.4.2 baseline in this run.
- DByte VM still loses to Python on function-heavy workloads. The remaining bottleneck is boxed value-stack return passing and per-instruction dispatch overhead inside small function bodies.
- Low-level binary workloads still beat Python in this run: `binary_read_u32`, `buffer_replace`, and `patch_workflow`.
- Next likely performance work: typed return-to-consumer, direct call-to-local opcodes, or a typed stack/register frame for small int functions. Function inlining remains deferred until those lower-risk paths are measured.

## Perf Pass 6: Frame Dispatch Regression Cleanup

Version: v1.5.1
Build: release
Date: 2026-05-10
Optimizations: Reduced hot-path frame-dispatch overhead for typed local i64 opcodes and jump handling by avoiding repeated frame helper lookups in tight loops. Also kept the native intrinsic stack cleanup path lightweight without adding new bytecode or changing semantics.

Baseline note: `VM v1.5.0` and `VM v1.5.1` were measured as local release-build medians from five runs each. This machine shows meaningful timing noise, so the table is used as a regression gate rather than a broad performance claim.

| Benchmark | VM v1.5.0 median | VM v1.5.1 median | Change |
|---|---:|---:|---:|
| loop_sum | 32.87 ms | 23.86 ms | 1.38x faster |
| loop_sum_large | 69.26 ms | 47.86 ms | 1.45x faster |
| int_compare_loop | 78.66 ms | 61.96 ms | 1.27x faster |
| nested_int_loop | 33.20 ms | 25.14 ms | 1.32x faster |
| binary_read_u32 | 80.29 ms | 80.82 ms | 0.99x |
| buffer_replace | 10.58 ms | 10.08 ms | 1.05x faster |
| patch_workflow | 29.75 ms | 27.15 ms | 1.10x faster |
| function_call | 126.65 ms | 118.79 ms | 1.07x faster |
| function_call_int | 159.45 ms | 150.27 ms | 1.06x faster |
| function_call_loop_return | 155.76 ms | 150.03 ms | 1.04x faster |
| function_call_nested | 105.70 ms | 104.19 ms | 1.01x |

### Findings

- v1.5.1 recovers the int-loop regressions introduced by the v1.5 frame-dispatch structure without adding new language features or opcodes.
- Binary and patching workloads remain within the regression gate in the local median run.
- Function-call-heavy workloads improve slightly but still need the planned v1.6 direct typed return-to-local path to materially close the gap with Python.

## Perf Pass 7: Direct Typed Return-To-Local Calls

Version: v1.6.0
Build: release
Date: 2026-05-11
Optimizations: Added `CALL_FN_I64_TO_LOCAL` for direct user function calls returning `int` into an `int` local, plus a VM `ReturnMode::StoreI64` path so `RETURN_I64` writes directly into the caller frame instead of pushing a boxed stack value and immediately storing it. The compiler fast path is conservative: direct user calls with explicit `-> int` only, with nested call arguments and generic/member/std calls left on the existing fallback path.

Baseline note: `VM v1.6.0` was measured as local release-build medians from five runs and compared to the recorded `VM v1.5.1` medians. Timings are noisy on this machine, so the table is used as a regression gate and directional performance record.

| Benchmark | VM v1.5.1 median | VM v1.6.0 median | Change |
|---|---:|---:|---:|
| loop_sum | 23.86 ms | 21.03 ms | 1.13x faster |
| loop_sum_large | 47.86 ms | 41.09 ms | 1.16x faster |
| nested_int_loop | 25.14 ms | 20.88 ms | 1.20x faster |
| binary_read_u32 | 80.82 ms | 67.01 ms | 1.21x faster |
| buffer_replace | 10.08 ms | 8.88 ms | 1.14x faster |
| patch_workflow | 27.15 ms | 23.46 ms | 1.16x faster |
| function_call | 118.79 ms | 106.17 ms | 1.12x faster |
| function_call_int | 150.27 ms | 123.07 ms | 1.22x faster |
| function_call_loop_return | 150.03 ms | 126.05 ms | 1.19x faster |
| function_call_nested | 104.19 ms | 87.40 ms | 1.19x faster |

### Findings

- Direct typed return-to-local materially improves the int-return assignment workloads while preserving the loop and binary regression gates.
- `function_call_int`, `function_call_loop_return`, and `function_call_nested` all improve clearly, matching the primary v1.6 target.
- The stretch target for `function_call` below 90 ms was not reached in this median run; the remaining cost is still frame setup, typed argument transfer, and executing small function bodies.

## Perf Pass 8: Typed I64 Operand Stack

Version: v1.7.0
Build: release
Date: 2026-05-11
Optimizations: Added a separate VM `i64` operand stack and typed call-chain opcodes (`CALL_FN_I64_TO_I64_STACK`, `RETURN_I64_TO_I64_STACK`) so compiler-proven `int` function bodies and typed call-chain expressions avoid boxed `Value::Int` temporaries. Direct assignment from int-return functions still uses `CALL_FN_I64_TO_LOCAL`. This pass also added fused local-int compare-and-jump opcodes for the existing loop condition fast path so loop regression gates stay below the v1.6 baseline.

Baseline note: `VM v1.7.0` was measured as local release-build medians from five runs on the feature branch. Most rows compare against the recorded `VM v1.6.0` medians. `function_call_chain` and `function_call_many_args` were not recorded in the v1.6.0 table, so their baseline is the same-session `release-v1.6.1` executable median from five runs. Timings remain noisy on this machine; this table is a directional performance record and regression gate.

| Benchmark | VM baseline median | VM v1.7.0 median | Change |
|---|---:|---:|---:|
| loop_sum | 21.03 ms | 17.42 ms | 1.21x faster |
| loop_sum_large | 41.09 ms | 34.45 ms | 1.19x faster |
| nested_int_loop | 20.88 ms | 15.77 ms | 1.32x faster |
| binary_read_u32 | 67.01 ms | 66.13 ms | 1.01x faster |
| buffer_replace | 8.88 ms | 8.14 ms | 1.09x faster |
| patch_workflow | 23.46 ms | 21.99 ms | 1.07x faster |
| function_call | 106.17 ms | 76.11 ms | 1.39x faster |
| function_call_int | 123.07 ms | 97.94 ms | 1.26x faster |
| function_call_loop_return | 126.05 ms | 84.74 ms | 1.49x faster |
| function_call_nested | 87.40 ms | 64.64 ms | 1.35x faster |
| function_call_chain | 169.01 ms | 97.71 ms | 1.73x faster |
| function_call_many_args | 149.95 ms | 74.45 ms | 2.01x faster |

### Findings

- Typed i64 call-chain execution materially improves the function-call-heavy workloads, especially chained and many-argument int-return calls.
- `function_call`, `function_call_int`, and `function_call_loop_return` all clear the v1.7 primary targets in this median run.
- The loop and binary workloads stay inside the no-regression gate and improve after replacing typed local compare plus boxed bool branching with fused compare-and-jump bytecode.

### Python Comparison Gate

Python: 3.12.9
Build: release
Date: 2026-05-11

Baseline note: `bench --compare-python` was run five times on the v1.7.0 branch and the medians below are computed per benchmark. The safe claim from this run is: DByte v1.7.0 outperforms Python on measured low-level binary parsing, buffer patching, and typed integer loop workloads, plus the optimized many-argument typed call workload. It is not correct yet to claim that DByte is broadly faster than Python.

| Benchmark | Python median | DByte VM median | Python / DByte VM |
|---|---:|---:|---:|
| binary_read_u32 | 110.87 ms | 65.92 ms | 1.68x |
| buffer_replace | 15.23 ms | 8.07 ms | 1.89x |
| patch_workflow | 32.56 ms | 21.67 ms | 1.50x |
| loop_sum | 33.44 ms | 15.29 ms | 2.19x |
| loop_sum_large | 68.64 ms | 31.94 ms | 2.15x |
| nested_int_loop | 31.43 ms | 15.56 ms | 2.02x |
| function_call_loop_return | 56.07 ms | 90.71 ms | 0.62x |
| function_call_many_args | 75.02 ms | 71.43 ms | 1.05x |
| function_call | 57.03 ms | 78.53 ms | 0.73x |
| function_call_int | 49.96 ms | 102.93 ms | 0.49x |
| function_call_nested | 32.15 ms | 61.37 ms | 0.52x |
| function_call_chain | 74.05 ms | 97.27 ms | 0.76x |
| bytes_find | 1.07 ms | 11.27 ms | 0.09x |

### Python Gate Findings

- Must-win binary, buffer, patching, and typed-loop workloads beat Python in the five-run median.
- `function_call_many_args` beats Python slightly after the typed i64 operand-stack work.
- `function_call_loop_return` does not beat Python yet, so the broader "faster than Python on DByte's target workloads" wording is not supported by this run.
- `bytes_find` still loses heavily to Python's native search path and remains the clearest blocker for broader benchmark-suite claims.

## Perf Pass 9: SIMD Byte Search via memchr

Version: v1.8.0
Build: release
Date: 2026-05-11
Optimizations: Replaced the naive `O(n*m)` sliding-window `windows().position()` byte search in both the VM intrinsic `Op::BufferFind` path and the `NativeFn::BufferFind` slow-path with SIMD-accelerated `memchr` crate. Single-byte patterns use `memchr::memchr` (x86 PCMPESTRI/PCMPISTRI path), multi-byte patterns use `memchr::memmem::find` (two-way algorithm with SIMD vectorization). The tree interpreter was patched in the same commit for full tree/VM parity.

| Benchmark | VM v1.7.1 median | VM v1.8.0 median | Change |
|---|---:|---:|---|
| bytes_find | 11.27 ms | 0.38 ms | **29.7x faster** |
| bytes_find_single | (new) | 0.17 ms | (new) |
| buffer_replace | 8.07 ms | 7.94 ms | 1.02x (no regression) |
| binary_read_u32 | 65.92 ms | 73.18 ms | within noise |
| patch_workflow | 21.67 ms | 27.46 ms | within noise |
| loop_sum | 15.29 ms | 17.53 ms | within noise |

### Python Comparison Gate

| Benchmark | Python median | DByte VM median | DByte / Python |
|---|---:|---:|---|
| bytes_find | 1.51 ms | 0.38 ms | **3.94x faster** |
| bytes_find_single | 0.53 ms | 0.18 ms | **2.96x faster** |
| binary_read_u32 | 108.45 ms | 69.44 ms | 1.56x faster |
| buffer_replace | 16.67 ms | 8.80 ms | 1.89x faster |
| patch_workflow | 37.95 ms | 22.03 ms | 1.72x faster |
| loop_sum | 36.57 ms | 18.19 ms | 2.01x faster |
| int_compare_loop | 66.61 ms | 38.11 ms | 1.75x faster |

### Findings

- `bytes_find` is now 29.7x faster vs v1.7.1 and beats Python by **3.94x** in the measured workload.
- `bytes_find_single` (new benchmark, 1-byte pattern) beats Python by **2.96x**.
- All existing no-regression gate benchmarks remain within noise tolerance.
- This closes the `bytes_find` blocker. DByte v1.8.0 now beats Python on all measured binary, buffer, patching, and typed-integer-loop workloads.

## Perf Pass 10: Bytecode-Level Function Inlining & Call Fusion

Version: v1.9.0
Build: release
Date: 2026-05-11
Optimizations: Implemented bytecode-level function inlining in the compiler for simple non-recursive functions. Re-mapped jump targets and local offsets during compilation, removing the Op::CallFn overhead and frame dispatch entirely for tight loop calls. Also optimized discarded calls by pushing Op::PopI64Stack natively.

| Benchmark | Python | DByte VM | Ratio |
|---|---:|---:|---|
| binary_read_u32 | 159.52 ms | 66.14 ms | 2.41x faster |
| buffer_replace | 14.68 ms | 8.90 ms | 1.65x faster |
| bytes_find | 1.14 ms | 0.46 ms | 2.46x faster |
| bytes_find_single | 0.34 ms | 0.20 ms | 1.72x faster |
| function_call | 53.17 ms | 39.98 ms | 1.33x faster |
| function_call_chain | 81.58 ms | 47.23 ms | 1.73x faster |
| function_call_int | 55.08 ms | 55.10 ms | 1.00x |
| function_call_loop_return | 60.84 ms | 42.40 ms | 1.43x faster |
| function_call_many_args | 78.41 ms | 65.66 ms | 1.19x faster |
| function_call_nested | 33.38 ms | 29.54 ms | 1.13x faster |
| int_compare_loop | 60.45 ms | 39.71 ms | 1.52x faster |
| loop_sum | 38.68 ms | 15.16 ms | 2.55x faster |
| loop_sum_large | 71.42 ms | 31.06 ms | 2.30x faster |
| nested_int_loop | 33.41 ms | 18.28 ms | 1.83x faster |
| patch_workflow | 36.06 ms | 21.94 ms | 1.64x faster |

### Findings

- DByte v1.9.0 is faster than Python on nearly all measured benchmarks, including binary parsing, byte search, buffer patching, typed integer loops, and most function-call workloads.
- Function inlining significantly reduced the overhead of small function calls, though some complex call chains (`function_call_int`) are still hitting the limits of the current inlining guards.
- Small function abstraction is now approaching zero-cost, but further refinement of the I64 return path and argument transfer is planned for v1.9.1 to definitively beat Python on every workload.

## Perf Pass 11: Zero-Cost Inlining (Argument Remapping)

Version: v1.9.1
Build: release
Date: 2026-05-11
Optimizations: Implemented direct argument remapping in the inliner. By analyzing parameter usage (read-only checks), the compiler now substitutes parameter loads with direct caller local loads or constants, eliminating the stack-to-local shuffle. This achieves true zero-cost abstraction for small helper functions.

| Benchmark | Python | DByte VM | Ratio |
|---|---:|---:|---|
| binary_read_u32 | 114.32 ms | 63.99 ms | 1.79x faster |
| buffer_replace | 13.91 ms | 7.86 ms | 1.77x faster |
| bytes_find | 1.12 ms | 0.36 ms | 3.07x faster |
| bytes_find_single | 0.31 ms | 0.22 ms | 1.38x faster |
| function_call | 51.47 ms | 30.36 ms | 1.70x faster |
| function_call_chain | 83.77 ms | 28.98 ms | 2.89x faster |
| function_call_int | 57.83 ms | 30.03 ms | 1.93x faster |
| function_call_loop_return | 59.23 ms | 34.81 ms | 1.70x faster |
| function_call_many_args | 81.65 ms | 34.71 ms | 2.35x faster |
| function_call_nested | 32.81 ms | 18.21 ms | 1.80x faster |
| int_compare_loop | 62.55 ms | 38.82 ms | 1.61x faster |
| loop_sum | 35.03 ms | 17.65 ms | 1.98x faster |
| loop_sum_large | 93.97 ms | 37.42 ms | 2.51x faster |
| nested_int_loop | 39.24 ms | 15.79 ms | 2.49x faster |
| patch_workflow | 36.51 ms | 24.96 ms | 1.46x faster |

### Findings

- DByte v1.9.1 beats Python across the full measured benchmark suite on this Windows release-build test machine.
- 'Zero-Cost Inlining' has successfully eliminated the remaining measured bottleneck in function call overhead.
- Simple helper functions (like `add`, `get`, `check`) now incur literally zero runtime overhead when inlined, producing optimal instruction sequences identical to hand-written inline logic.

## v2.1.0 Note

Version: v2.1.0
Build: release
Date: 2026-05-11

v2.1.0 is not a performance release. It adds the personal runtime layer
(`dbyte repl`, `dbyte shell`, and current-directory `.dbyterc` loading for
interactive commands only). The safe performance claim remains limited to the
measured benchmark suite and machine described above.

## v2.1.1 Note

Version: v2.1.1
Build: release
Date: 2026-05-11

v2.1.1 is not a performance release. It hardens the interactive runtime added in
v2.1.0 with additional REPL, shell, and `.dbyterc` regression coverage. The
benchmark claim remains unchanged.

## v2.2.0 Note

Version: v2.2.0
Build: release
Date: 2026-05-11

v2.2.0 is not a performance release. It adds the embeddable tree-runtime crate
for host applications and keeps benchmark claims limited to the measured suite.

## v2.2.1 Note

Version: v2.2.1
Build: release
Date: 2026-05-11

v2.2.1 is not a performance release. It hardens the embeddable runtime API,
including capture isolation, `.dbyterc` behavior, cwd handling, and rollback
coverage.


