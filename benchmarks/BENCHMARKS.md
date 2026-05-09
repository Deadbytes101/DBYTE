# DByte Benchmarks

## Baseline: v1.1.0-dev

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
