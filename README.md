<p align="center">
<img src="assets/logo/dbyte-logo.png" width="180" alt="DBYTEOS Logo" />
</p>

<h1 align="center">DBYTE</h1>

<p align="center">
<b>A byte-level programming language, personal userland, and bare-metal kernel laboratory.</b>
</p>

<p align="center">
<b>[ <a href="https://dbytelang.site">Official Site</a> ]</b>
<b>[ <a href="https://dbytelang.site/docs/">Docs</a> ]</b>
<b>[ <a href="benchmarks/BENCHMARKS.md#perf-pass-11-zero-cost-inlining-argument-remapping">Benchmarks</a> ]</b>
<b>[ <a href="docs/DBYTEOS_KERNEL.md">Kernel Notes</a> ]</b>
<b>[ <a href="https://discord.gg/hWuwUbrujb">Discord</a> ]</b>
</p>

<p align="center">
<a href="https://github.com/Deadbytes101/DBYTE/releases/ISO">
<img src="https://img.shields.io/badge/DBYTE-TRY%20ISO%20NOW-178da5?style=flat" alt="Try the DByte ISO" />
</a>
<img src="https://img.shields.io/badge/language-DBYTE-111111?style=flat" alt="Language DBYTE" />
<a href="https://github.com/Deadbytes101/DBYTE/blob/main/LICENSE">
<img src="https://img.shields.io/github/license/Deadbytes101/DBYTE?color=green" alt="GPL-2.0-only License" />
</a>
<a href="CONTRIBUTING.md">
<img src="https://img.shields.io/badge/contributing-open-178da5?style=flat" alt="Contributing" />
</a>
<a href="SECURITY.md">
<img src="https://img.shields.io/badge/security-policy-111111?style=flat" alt="Security Policy" />
</a>
</p>

> [!IMPORTANT]
> This repository is a public working snapshot. DBYTE is still moving fast, and the public source surface is not the final stable release surface.

> [!CAUTION]
> DByteOS and the Kernel Lab are experimental. Run VM and kernel builds in an isolated environment.

## What DBYTE Is

DBYTE is a compact systems-oriented project built around a custom language named **DByte**. The language is designed for binary work, low-level automation, typed integer processing, byte search, buffer patching, and tooling that should stay direct and predictable.

The repo currently contains four connected layers:

- **DByte language toolchain**: lexer, parser, type checker, tree interpreter, bytecode compiler, bytecode VM, project loader, test runner, shell, REPL, and embedding API.
- **DByte native kernel runtime**: a freestanding `i686-dbyte-none` path that owns physical frames, maps a fixed kernel arena, runs a page-backed heap, schedules preemptive kernel tasks, and blocks or wakes them without a host runtime.
- **DByteOS userland**: host-runnable `.dby` programs under `examples/dbyteos/` that model a personal operating environment, shell commands, config, diagnostics, security checks, workspace flows, and system tools.
- **DByteOS Kernel Lab**: a separate freestanding x86 Rust kernel sandbox under `kernel-lab/` for QEMU boot experiments, VGA output, serial logs, IDT and IRQ research, exception surfaces, PIC planning, and controlled runtime probes.

DBYTE is not trying to be a web framework or a general productivity language. It is built for inspecting bytes, moving data, testing low-level ideas, and turning rough system experiments into executable tools.

## Current Snapshot

- Snapshot version: **v12.2.0**
- Default branch: **main**
- Repository visibility: **public**
- Language identity: **DBYTE**.
- GitHub Languages policy: implementation and benchmark languages are excluded from the sidebar; `.dby` files request the `DBYTE` label through `.gitattributes`, and the native Languages panel stays hidden until GitHub Linguist supports DBYTE as an official language.

Tracked line counts are measured from `git ls-files`, so ignored build output, release zips, bundles, VM logs, and `target/` directories are excluded:

- Kernel Rust source: **16,448 lines** across `kernel-lab/src/*.rs`.
- Full Kernel Lab tracked files: **16,584 lines** across kernel sources, linker script, scripts, manifest, and lab README.
- Main tracked source and docs set: **105,892 lines** across **635 files** matching `*.rs`, `*.dby`, `*.toml`, `*.md`, `*.ps1`, `*.ld`, and `*.yml`.
- Total tracked files in the repository: **956**.

## Language Stack

| Layer | Path | Purpose |
| --- | --- | --- |
| AST | `crates/dbyte_ast` | Shared syntax nodes |
| Canonical HIR | `crates/dbyte_hir` | Project resolution, canonical types and IDs, ownership, monomorphization, profile validation, executable HIR |
| Lexer and parser | `crates/dbyte_lexer`, `crates/dbyte_parser` | Source tokenization and grammar parsing |
| Type checker | `crates/dbyte_typeck` | Static checks for DByte programs |
| Tree runtime | `crates/dbyte_interp` | Direct interpreter runtime |
| Bytecode path | `crates/dbyte_bytecode`, `crates/dbyte_compiler`, `crates/dbyte_vm` | Compilation and VM execution |
| Native path | `crates/dbyte_native` | Systems validation, modules, typed IR, i686 encoding, ET_REL objects, static linking, and Multiboot ELF output |
| Modules and projects | `crates/dbyte_module`, `crates/dbyte_project` | Imports, packages, `Dbyte.toml` workflows |
| CLI | `crates/dbyte_cli` | `dbyte run`, `dbyte test`, REPL, shell, tools |
| Embedding | `crates/dbyte_embed` | Rust host integration |
| Kernel bridge | `crates/dbyte_kernel_vm` | Shared kernel VM probe support |

## What It Can Do

- Parse and run DByte scripts with Python-like block syntax.
- Work with typed integers, bytes, buffers, and binary-oriented standard modules.
- Patch buffers, search byte sequences, inspect binary files, and save modified outputs.
- Run DByte projects with `Dbyte.toml`.
- Execute tests with `dbyte test`.
- Launch a DByte REPL or shell.
- Embed DByte in Rust applications.
- Compile generic functions and named generic structs, payload enums and exhaustive matches, `Result` propagation, borrowed slices and `StringView`, explicit-capture closures, lexical `defer`, function pointers, and the supported i686 C ABI through canonical typed HIR.
- Emit deterministic ELF32 objects, static Unix archives, and linked Multiboot or compact user executables with DByte's in-process toolchain.
- Compile and boot a multi-module DByteOS core that releases physical frames, maps and unmaps kernel pages, grows and decommits a checked heap, and executes memory operations in DByte machine code.
- Boot a preemptive ring-0 DByte scheduler whose non-yielding tasks switch from real IRQ0 resume images and whose page-backed stacks have unmapped guards.
- Put DByte tasks to sleep without spinning and wake fixed FIFO Event waiters from task or hardware IRQ context while preserving interrupt state.
- Block a DByte keyboard reader on a counting semaphore, receive raw bytes from real emulated i8042 IRQ1 delivery, and resume through the existing scheduler context image.
- Identify a QEMU ATA disk, transfer complete LBA28 sectors with 16-bit PIO, and wake blocked DByte tasks from real slave-PIC IRQ14 completion.
- Format, mount, mutate, reboot, and independently verify a deterministic DBFS1 root filesystem through the IRQ-driven ATA path.
- Boot the Kernel Lab in QEMU for controlled freestanding kernel experiments.

## Quick Commands

```powershell
cargo check
cargo run -p dbyte_cli -- --version
cargo run -p dbyte_cli -- run examples/hello.dby
cargo run -p dbyte_cli -- test
```

Build the multi-module DByte native kernel runtime without an external code generator, assembler, runtime shim, or linker:

```powershell
cargo run -p dbyte_cli --bin dbytec -- build examples/native_runtime_core/kernel.dby `
    --target i686-dbyte-none --emit elf -o tmp/dbyte_native_runtime_v11.4.0.elf
powershell -ExecutionPolicy Bypass -File scripts/verify_native.ps1
powershell -ExecutionPolicy Bypass -File scripts/verify_native_kernel_core.ps1
powershell -ExecutionPolicy Bypass -File scripts/verify_native_memory_core.ps1
powershell -ExecutionPolicy Bypass -File scripts/verify_native_runtime_core.ps1
```

The `v11.5.0` native scheduler proof builds separate normal and fatal guard images
from DByte objects. The normal image preempts two non-yielding ring-0 tasks from
real PIT IRQ0 delivery, reclaims their page-backed stacks, and proves deterministic
slot/frame reuse. The guard image raises one real supervisor-write page fault on
the unmapped stack guard at `0xC1200FFC`.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_scheduler_core.ps1
```

The `v11.6.0` native blocking proof reuses the scheduler's sole canonical resume
image through a CPL0 `int 0x81` blocking boundary. It proves wrap-safe sleeping,
non-sticky wake-one FIFO Events, task- and IRQ-context signaling, IF preservation,
continued runnable-task progress, and complete stack/frame reclamation.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_blocking_core.ps1
```

The `v11.7.0` native IRQ I/O proof adds a bounded counting semaphore and a
64-byte FIFO ring without changing the context ABI. A DByte proof task injects
four raw bytes through the emulated i8042 output-buffer command; real IRQ1
delivery reads port `0x60`, posts the semaphore, and wakes the blocked reader.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_irq_io_core.ps1
```

The `v11.8.0` native block-device proof identifies a disposable primary ATA
disk, executes bounded 16-bit LBA28 PIO transfers, completes through real IRQ14,
and verifies deterministic sector read/write data without a filesystem or host runtime.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_block_device_core.ps1
```

The `v11.9.0` native persistent-filesystem proof formats DBFS1 on a disposable
raw disk, writes and reuses real data blocks, then reboots the same ELF against
the same disk and reads `PERSIST.TXT` back without formatting or host repair.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_filesystem_core.ps1
```

The `v11.10.0` native filesystem-runtime proof keeps DBFS1 byte-compatible and
adds hierarchical paths, generation-safe task-owned handles, a write-through
sector cache, native integer division/remainder, typed pointer-field access,
and persistent kernel configuration/log state across three boots:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_filesystem_runtime.ps1
```

The `v11.11.0` native user-process proof compiles compact DByte Ring3 ELF32
programs, transports them in a checked DBUP1 Multiboot bundle, installs them
into DBFS1, and exercises private CR3 address spaces, TSS stack transitions,
worker-backed file syscalls, per-process descriptors, and contained #GP/#PF exits:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_user_process_core.ps1
```

The `v12.0.0` language proof freezes DByteOS and exercises the general-purpose
language foundation through canonical typed HIR. Its multi-module QEMU image
executes generics, payload enums and `match`, `Result` propagation, borrowed
views, closures, lexical cleanup, DByte function pointers, and bidirectional
i686 C calls. The same verifier builds a supported v12 user program and runs it
in the frozen Ring3 process core while checking deterministic archives and
non-loadable public-generic metadata.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_v12_hir.ps1
```

The `v12.1.0` runtime foundation adds deterministic affine destruction,
explicit allocator capabilities, owned `Array[T]` and UTF-8 `String` values.
Hosted execution, the bytecode VM, and i686 native lowering consume the same
Concrete HIR cleanup and ownership decisions.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native_runtime_foundation.ps1
```

See [Runtime Foundation](docs/RUNTIME.md), [Allocators](docs/ALLOCATORS.md),
[Array](docs/ARRAY.md), and [String](docs/STRING.md) for the normative v12.1
contracts.

The `v12.2.0` modern native foundation adds one target-driven native seam for
`x86_64-dbyte-none`, `x86_64-dbyte-linux`, and `x86_64-dbyte-windows`. DByte's
internal toolchain emits ELF64, deterministic archives, AMD64 COFF, static
Linux executables, Multiboot2 long-mode images, and PE32+ console executables
without an external production linker. The same Concrete HIR executes the
DBYTE64 ABI, `i64`/`u64`, runtime ownership, allocators, Array, String, and the
target-selected SysV64 or Win64 C ABI.

See [Modern Native Targets](docs/MODERN_NATIVE_TARGETS.md),
[DBYTE64 ABI](docs/DBYTE64_ABI.md), [ELF64 and Multiboot2](docs/ELF64_MULTIBOOT2.md),
[Linux Runtime](docs/LINUX_RUNTIME.md), [COFF and PE32+](docs/COFF_PE32_PLUS.md),
and [Windows Runtime](docs/WINDOWS_RUNTIME.md) for the normative v12.2 contracts.

See [DByte Native Compiler](docs/DBYTE_NATIVE.md) for the freestanding profile,
DBYTE32 ABI, systems types, objects, linker, intrinsic boundary, and binary contract.
See [DByte Native Memory Core](docs/DBYTE_NATIVE_MEMORY_CORE.md) for the
Multiboot entry metadata, physical ownership, paging, and page-fault proof.
See [DByte Native Kernel Runtime](docs/DBYTE_NATIVE_KERNEL_RUNTIME.md) for
releasable frames, arena paging, the checked heap, and memory-operation proof.
See [DByte Native Kernel Scheduler](docs/DBYTE_NATIVE_KERNEL_SCHEDULER.md) for
the resume-image ABI, IRQ0 scheduler, task lifecycle, stack reclamation, and guard proof.
See [DByte Native Kernel Blocking](docs/DBYTE_NATIVE_KERNEL_BLOCKING.md) for
sleep deadlines, the CPL0 blocking trap, FIFO Event semantics, and IRQ-safe wakeups.
See [DByte Native Kernel IRQ I/O](docs/DBYTE_NATIVE_KERNEL_IRQ_IO.md) for
counting semaphore semantics, the byte ring, i8042 setup, IRQ1 ownership, and raw-input proof.
See [DByte Native Kernel Block Device](docs/DBYTE_NATIVE_KERNEL_BLOCK_DEVICE.md) for
ATA IDENTIFY, LBA28 PIO, slave-PIC IRQ14 ownership, and disposable-disk verification.
See [DByte Native Persistent Filesystem](docs/DBYTE_NATIVE_PERSISTENT_FILESYSTEM.md) for
the DBFS1 disk layout, bounded root API, two-boot persistence, and host inspection contract.
See [DByte Native User Process](docs/DBYTE_NATIVE_USER_PROCESS.md) for the user ELF profile,
DBUP1 transport, Ring3/TSS ABI, private CR3 mappings, syscall worker, and fault containment.
See [DByte 12 Language Contract](docs/DBYTE_V12_LANGUAGE_CONTRACT.md) for the
normative canonical HIR, type/layout, ownership, generic, enum, closure, defer,
C ABI, grammar, compatibility, and target-matrix contracts.

Launch the DByte shell with the DByteOS userland profile:

```powershell
cargo run -p dbyte_cli -- shell --rc examples/dbyteos/.dbyterc
```

Build the Kernel Lab:

```powershell
cd kernel-lab
powershell -ExecutionPolicy Bypass -File .\scripts\build.ps1
```

Run the Kernel Lab in QEMU when QEMU is installed:

```powershell
cd kernel-lab
powershell -ExecutionPolicy Bypass -File .\scripts\run.ps1
```

## Binary Patch Example

```dbyte
import std.buffer as buf

let image: buffer = buf.load("firmware.bin")
let offset: int = buf.find(image, b"\xDE\xAD\xBE\xEF")

if offset >= 0:
    buf.replace(image, offset, b"\x90\x90\x90\x90")
    buf.save("firmware.patched.bin", image)
```

## Kernel Lab Status

The Kernel Lab is intentionally separate from the host-runnable DByteOS userland. It is a freestanding x86 research sandbox, not a production kernel.

Current kernel research areas include:

- Multiboot-compatible boot path.
- VGA text and graphics output.
- Serial logging.
- IDT and exception handling foundations.
- Page fault smoke surfaces.
- PIC and IRQ planning.
- Controlled IRQ0 and IRQ1 runtime experiments.
- Kernel-side DByte VM probe integration.

The guardrail is deliberate: hardware mutation paths stay controlled, documented, and testable.

## Repository Hygiene

Ignored local artifacts include:

- `target/`
- `kernel-lab/target/`
- release bundles and zip packages
- unpacked release directories
- scratch binaries
- temporary VM logs
- `test_release_v*/`

This keeps the repository focused on source, docs, scripts, and reproducible project state.

## Community Files

This repo includes the core GitHub community surface:
- [Contributing Guide](CONTRIBUTING.md)
- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security Policy](SECURITY.md)
- [Support Notes](SUPPORT.md)
- [Pull Request Template](.github/PULL_REQUEST_TEMPLATE.md)
- [Issue Templates](.github/ISSUE_TEMPLATE/)

## License
DBYTE is released under the [GNU General Public License version 2](LICENSE).
The license applies to the tracked source and documentation in this repository. Generated build output, local scratch files, release bundles, VM logs, and ignored artifacts are not part of the source distribution.

## Project Warning

DBYTE is alpha software. Interfaces can change. Kernel Lab behavior can break. Use a VM for OS experiments and keep real data away from unsafe test runs.

## Repository

- Repository: [Deadbytes101/DBYTE](https://github.com/Deadbytes101/DBYTE)
- Site: [dbytelang.site](https://dbytelang.site)
- Docs: [dbytelang.site/docs](https://dbytelang.site/docs/)
- Discord: [Join Community](https://discord.gg/hWuwUbrujb)

honestly i have a mental illness where i must build everything myselfif i see someone else software and i can not control 100 percent of it i just choose to rewrite the whole damn thing from scratch
