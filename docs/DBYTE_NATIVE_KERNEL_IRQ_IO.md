# DByte Native Kernel IRQ I/O

`v11.7.0` extends the single-core CPL0 scheduler with a real interrupt-driven
raw keyboard path. It retains the canonical `PUSHAD` plus EIP/CS/EFLAGS resume
image and adds `SEMAPHORE_WAIT` as request kind 3 on the existing `int 0x81`
blocking boundary. No second context ABI or textual assembly is introduced.

The counting `Semaphore` has a bounded eight-task FIFO waiter queue and a
checked maximum count. A wait consumes an existing token atomically or blocks
the current task. A post wakes exactly the oldest waiter, otherwise stores one
token; posting at the maximum fails without mutation. Posting preserves the
incoming interrupt flag and never schedules, acknowledges hardware, or sends EOI.

The keyboard driver owns a static 64-byte FIFO ring and semaphore. It configures
the i8042 with bounded status polling, installs a DPL0 interrupt gate at vector
`0x21`, and unmasks only PIC IRQ0 and IRQ1. The IRQ1 handler reads one raw byte
from port `0x60`, pushes it without overwrite, posts the semaphore, and sends
exactly one master PIC EOI. It performs no allocation, mapping, logging, blocking,
or scheduling.

The acceptance proof injects `1E 30 2E 20` through i8042 command `0xD2` after
the reader is blocked. QEMU must deliver four real IRQ1 interrupts, the reader
must resume with the exact FIFO bytes, and another runnable task must advance
while input is unavailable. Separate probes cover sticky tokens, FIFO waiter
order, maximum-count rejection, byte 65 drop, complete reclamation, and exact
slot/frame reuse.

Run deterministic object/link checks and both 32/64 MiB QEMU boots with:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_irq_io_core.ps1
```

Ignored ELF, BIN, disassembly, hashes, serial, debugcon, and QEMU trace evidence
is written under `tmp/native_v11.7.0/`.
