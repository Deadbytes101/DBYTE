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
