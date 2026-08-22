# DByte Native Kernel Scheduler

`v11.5.0` adds a single-core, CPL0, preemptive round-robin scheduler written in
DByte modules. It consumes the existing interrupt `PUSHAD` plus CPU frame directly;
there is no second register-context structure and no Rust, C, or assembly shim.

The canonical resume image contains EDI, ESI, EBP, the ignored `PUSHAD` ESP slot,
EBX, EDX, ECX, EAX, EIP, CS, and EFLAGS at four-byte offsets 0 through 40.
`native.interrupt_resume` is terminal and emits `mov esp, resume_esp; popad; iretd`.
The linker supplies a three-byte `call eax; ret` first-run trampoline, while
`.dbyte.tasks` carries checked task references only during compile and link.

The scheduler owns `0xC1200000..0xC1400000`. Each of eight deterministic slots is
20 KiB: one unmapped guard page followed by four supervisor-writable stack pages.
TCBs come from the checked kernel heap, while stack mappings and physical frames
are allocated only during spawn and reclaimed only by the idle task. IRQ0 performs
bounded queue mutation, allocates nothing, and sends exactly one master PIC EOI.

Two bounded workers busy-loop without `yield`. IRQ0 preempts both, task A exits
explicitly, task B continues and returns through the linker trampoline, and idle
reclaims both stacks before a probe reuses the lowest slot and released frame.

A separate guard image writes to `0xC1200FFC`. Its error-code #PF handler reads
CR2 once, checks `(error & 7) == 2`, confirms the guard PTE remains absent, and
halts without entering scheduler or EOI paths.

Run the complete deterministic object, disassembly, and 32/64 MiB QEMU proof:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_scheduler_core.ps1
```

Ignored evidence is written under `tmp/native_v11.5.0/{normal,guard}/`.
