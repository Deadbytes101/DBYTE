# DByte Native User Process Foundation

Version 11.11.0 adds an additive `i686-dbyte-user` profile. It accepts
`pub fn user_main() -> int`, systems values and pointers, and only the checked
`user.syscall0` through `user.syscall3` boundary. Interrupt declarations,
kernel symbol-address helpers, and every `native.*` operation are rejected.

User programs link to compact ELF32/i386 executables with two `PT_LOAD`
segments: RX text/rodata at `0x00400000` and page-separated RW data/BSS. The
format has no sections, interpreter, dynamic state, relocations, BIN form, or
runtime dependency. `dbytec pack-user` orders named payloads canonically and
stores their SHA-256 digests in a bounded DBUP1 Multiboot bundle.

The native proof validates DBUP1 in the guest, installs its programs through
DBFS1, reloads them through filesystem handles, and constructs one private CR3
per process. User image and stack PTEs carry the User bit; kernel image, heap,
kernel stacks, and copy scratch mappings remain supervisor-only. The first
MiB and the user-stack guard are absent. User copies validate the complete
range before mapping physical pages into fixed supervisor scratch slots.

Ring3 transitions use `CS=0x1b`, `DS/SS=0x23`, `EFLAGS=0x202`, and a packed
104-byte TSS loaded with selector `0x28`. The scheduler retains one context ABI:
PUSHAD followed by EIP, CS, EFLAGS, user ESP, and user SS. A context switch
loads CR3, updates `TSS.esp0`, then resumes through `popad; iretd`.

`INT 0x80` uses EAX for the syscall number/result and EBX/ECX/EDX for three
arguments. Filesystem and serial work executes in a kernel worker after the
caller becomes blocked. Each process has fd0, fd1, fd2, and sixteen isolated
filesystem descriptors. #GP and #PF from CPL3 terminate only the offending
process; the idle reaper releases descriptors, user frames, page tables,
kernel stack pages, and the generation-safe process slot.

Run the complete proof with:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_user_process_core.ps1
```
