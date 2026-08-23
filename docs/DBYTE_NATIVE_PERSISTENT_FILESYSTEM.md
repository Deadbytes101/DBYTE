# DByte Native Persistent Filesystem

`v11.9.0` adds DBFS1, a deliberately bounded first-party filesystem implemented
as DByte modules. It uses the existing IRQ14-driven ATA sector API; filesystem
code never accesses ATA ports or performs device-completion polling directly.

DBFS1 uses 512-byte sectors, begins at LBA 64, and reserves LBA 0 through 63.
An 8 MiB proof disk has this deterministic layout:

| Region | LBA range | Contents |
| --- | --- | --- |
| Reserved | 0–63 | Must remain zero |
| Superblock | 64 | Magic, version, geometry, and FNV-1a checksum |
| Bitmap | 65–68 | One bit per data block |
| Inodes | 69–76 | 64 fixed 64-byte inode records |
| Data | 77–16383 | 16,307 file/directory blocks |

Inode 0 is invalid and inode 1 is the root directory. The root owns eight
ordinary data blocks containing 64 fixed directory entries. Regular files have
eight direct blocks and a maximum size of 4096 bytes. Names are 1–31 ASCII bytes
and paths, nested directories, links, permissions, timestamps, and sparse files
are outside this format version.

The public task-context surface provides format, mount, create, write, read,
stat, and delete operations addressed by root filename. A binary semaphore
serializes complete operations across blocking disk I/O. There is no block
cache, journal, crash-recovery claim, or filesystem work in IRQ context.

The verifier boots one generated ELF twice against the same disposable raw
disk for both 32 and 64 MiB RAM. Boot 1 formats DBFS1, writes `PERSIST.TXT`, and
proves exact freed-block reuse. Boot 2 mounts without formatting and reads the
same bytes back. A separate host-side parser then checks the superblock,
checksum, inode and directory geometry, exact bitmap ownership, reserved LBAs,
and persistent SHA-256 without writing or repairing the image.

Run the complete proof with:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_filesystem_core.ps1
```
