# X86_64 Windows Runtime (v12.2)

Status: normative release contract for `v12.2.0`.

`x86_64-dbyte-windows` emits a PE32+ console executable. The internal entry
calls `main()` and passes its status to `ExitProcess`. Standard output and
error use `GetStdHandle` and `WriteFile`; input and close use the corresponding
allowlisted Kernel32 operations.

`SystemAllocator` uses `VirtualAlloc` and `VirtualFree`. Resize is
allocate-copy-free and commits only after replacement allocation and bounded
copy succeed. There is no CRT or hidden host allocator.

`extern "C"` follows Win64: `RCX`, `RDX`, `R8`, and `R9` carry scalar slots;
the caller reserves 32 bytes of shadow space and aligns the stack to 16 bytes.
The target preserves all Win64 nonvolatile registers required by the subset.
C aggregates remain pointer-only.

Ordinary DByte code cannot name DLLs or arbitrary imports. All platform calls
are resolved capabilities in canonical HIR and mapped by the Windows target
adapter.
