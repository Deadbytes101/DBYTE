# DByte Native Kernel Block Device Foundation

`v11.8.0` adds a standalone DByteOS primary-ATA proof without changing the
language, context ABI, or native compiler surface. Existing DByte intrinsics emit
the 16-bit `in ax, dx` and `out dx, ax` instructions used for PIO data transfer.

The proof performs a bounded ATA IDENTIFY probe, requires LBA28 support, and
accepts one outstanding sector request. Task-context setup validates the LBA and
an aligned 512-byte buffer, programs one READ or WRITE command, and blocks on the
existing scheduler semaphore. Vector `0x2E` reads device status, transfers exactly
256 words for READ, completes WRITE status, posts once, and sends slave-PIC EOI
before the master cascade EOI.

The verifier creates fresh 8 MiB raw images under `tmp/native_v11.8.0/` for both
32 and 64 MiB QEMU boots. LBA8 contains the fixed block-test seed and LBA9 is a
scratch sector. It checks exact guest read data, guest write/readback, host-side
post-run bytes, real IRQ14 traces, PIC masks, task blocking, deterministic objects,
and the absence of runtime dependencies or CPU faults. These images are disposable
evidence and are never release inputs.

Run the proof with:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_block_device_core.ps1
```

AHCI, DMA, LBA48, ATAPI, request queues, partitions, caches, and filesystems are
outside this milestone.
