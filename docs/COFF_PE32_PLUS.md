# AMD64 COFF and PE32+ (v12.2)

Status: normative release contract for `v12.2.0`.

DByte writes and parses AMD64 COFF objects and creates PE32+ console images
with its internal linker. Section ordering, timestamps, image layout, symbol
resolution, imports, and IAT construction are deterministic. Malformed ranges,
relocation overflow, duplicate or missing symbols, incompatible machine type,
and mixed archive members fail transactionally.

The executable import boundary is exactly `KERNEL32.dll`. The versioned
allowlist is `ExitProcess`, `GetStdHandle`, `ReadFile`, `WriteFile`,
`CloseHandle`, `VirtualAlloc`, `VirtualFree`, and
`QueryPerformanceCounter`. No CRT, Rust runtime, external linker, arbitrary
DLL escape hatch, or unstable NT syscall number is part of the target.

Deterministic `ar` archives may contain canonical ELF32, ELF64, or AMD64 COFF
members, but never mixed formats or targets. Extraction is symbol-driven and
reaches a fixed point before the final link.
