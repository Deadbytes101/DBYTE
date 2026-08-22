# DByte Native Kernel Blocking

`v11.6.0` extends the single-core CPL0 scheduler with real sleeping and blocking.
Normal tasks cross a kernel-internal `int 0x81` boundary so the CPU and existing
interrupt stub create the same canonical `PUSHAD` plus EIP/CS/EFLAGS resume image
used by IRQ0. No copied register context, runtime shim, or textual assembly exists.

`task_sleep_ticks` accepts intervals below half the `u32` tick space. IRQ0 performs
a bounded eight-task scan and uses a signed wrapping delta to wake deadlines safely
across tick rollover. Sleeping and blocked tasks remain mapped but are never selected.

An `Event` owns eight fixed FIFO waiter slots. Signals are non-sticky: an empty
signal succeeds and is forgotten, while a non-empty signal makes exactly the oldest
waiter READY. `event_signal` preserves incoming IF and is safe from normal task or
ordinary maskable hardware IRQ context. It never schedules, acknowledges hardware,
or sends PIC EOI; wait and sleep remain task-context-only operations.

The proof boots at both 32 and 64 MiB. One worker sleeps without spinning, two
waiters prove FIFO order, another runnable task advances while peers are blocked,
and separate probes prove empty task/IRQ pulses are not remembered. Idle reclaims
all exited worker stacks and frames before a probe verifies deterministic reuse.

Run the deterministic objects, relocation/disassembly checks, and QEMU proof:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_blocking_core.ps1
```

Ignored evidence is written under `tmp/native_v11.6.0/`.
