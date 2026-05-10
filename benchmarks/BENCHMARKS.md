# DByte Benchmarks

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
