# X86_64 Linux Runtime (v12.2)

Status: normative release contract for `v12.2.0`.

`x86_64-dbyte-linux` produces a static ELF64 executable with an internal
`_start`. Entry supports `main()` and the checked three-argument
`main(argc, argv, envp)` form. The return status reaches the Linux `exit`
system call without libc or a dynamic loader.

Versioned runtime capabilities cover `exit`, `read`, `write`, `mmap`, `munmap`,
`openat`, `close`, and `clock_gettime`. Raw syscall emission is restricted to
the canonical runtime capability surface. Ordinary source uses checked runtime
wrappers and cannot request arbitrary syscall numbers.

`SystemAllocator` uses `mmap`/`munmap`. Resize allocates replacement storage,
copies the bounded common prefix, and unmaps the old allocation only after the
replacement succeeds. Failure preserves the original allocation. The runtime
does not import libc, compiler-rt, or host allocation machinery.

`extern "C"` uses SysV AMD64. C fixture objects may be produced by the pinned
Clang command but are consumed by DByte's ELF64 parser, archive extractor, and
linker. The final image remains static and DByte-linked.
