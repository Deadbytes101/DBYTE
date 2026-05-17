# DByte

DByte is a fast low-level scripting language for binary parsing, buffer patching,
byte search, typed integer workloads, and automation scripts that need simple
syntax with predictable performance.

Public alpha status: DByte is usable for experiments and small tools, but the
language and standard library may still change before a stable x.x.x release.

![DByte Logo](assets/logo/dbyte-logo.png)

Website: https://dbytelang.site

DByte — a fast personal low-level scripting language for binary tools, shell
workflows, and system experiments.

Performance claim:

> DByte v1.9.2 outperforms Python 3.12.9 across DByte's measured benchmark
> suite on a Windows release-build test machine.

This is a benchmark-suite claim, not a claim that DByte is faster than Python in
every program or environment. See [benchmarks/BENCHMARKS.md](benchmarks/BENCHMARKS.md).

## Highlights

- Python-like syntax with static type checks.
- Project workflow with `Dbyte.toml`.
- Bytecode VM with disassembly and trace tooling.
- `bytes` and mutable `buffer` data types.
- Binary stdlib for endian-aware reads and writes.
- Buffer stdlib for load, save, find, replace, slice, get, and set.
- File, hash, encoding, math, env, binary, and buffer standard modules.
- Built-in test runner: `dbyte test`.
- Interactive REPL and DByte-native shell for personal scripting sessions.

## DByteOS Personal Alpha (v5.9.1)

DByteOS is a host-runnable personal computing environment built on the DByte runtime.

1. **Launch the DByteOS Shell**:

```powershell
dbyte shell --rc examples/dbyteos/.dbyterc
```

This configures the session environment, activates autopath resolving, and
loads the DByteOS prompt from `system.prompt`.

2. **Explore the System**:
- [DByteOS Personal Alpha](docs/DBYTEOS_PERSONAL_ALPHA.md)
- [DByteOS Alpha Positioning](docs/DBYTEOS_ALPHA.md)
- [Command Reference](docs/DBYTEOS_COMMANDS.md)
- [Onboarding Guide](docs/DBYTEOS_ONBOARDING.md)
- [Profile Guide](docs/DBYTEOS_PROFILE.md)
- [Config Guide](docs/DBYTEOS_CONFIG.md)
- [Snapshot Guide](docs/DBYTEOS_SNAPSHOT.md)
- [Projects Guide](docs/DBYTEOS_PROJECTS.md)
- [Tasks Guide](docs/DBYTEOS_TASKS.md)
- [Security Policy](docs/DBYTEOS_SECURITY.md)
- [Boot Lifecycle](docs/DBYTEOS_BOOT.md)
- [Package Smoke Guide](docs/DBYTEOS_PACKAGE.md)

3. **Initialize & Interact**:

```bash
dbyte-shell> welcome
dbyte-shell> check-system
dbyte-shell> doctor
dbyte-shell> profile show
dbyte-shell> config show
dbyte-shell> prefs set system.prompt dbyteos>
dbyte-shell> snapshot
dbyteos> project reset-demo
dbyteos> task reset-demo
dbyteos> task list demo
dbyteos> task add demo write tests
dbyteos> task done demo 1
dbyteos> task status demo
dbyteos> task summary demo
dbyteos> task open demo
dbyteos> task doctor demo
dbyteos> task snapshot demo
dbyteos> task clear-done demo
dbyteos> project status demo
dbyteos> project snapshot demo
dbyteos> prefs reset-demo
dbyte-shell> getting-started
dbyte-shell> commands
dbyte-shell> man-index
dbyte-shell> boot
dbyte-shell> help
dbyte-shell> status
dbyte-shell> which read
dbyte-shell> notes list
dbyte-shell> journal read
```

4. **Smoke-test a zip release**:

```powershell
.\dbyte.exe --version
.\dbyte.exe shell --rc examples/dbyteos/.dbyterc
```

Expected first commands inside the package shell:

```txt
boot
welcome
check-system
doctor
prefs set system.prompt dbyteos>
snapshot
project reset-demo
task reset-demo
task list demo
task add demo write tests
task done demo 1
task status demo
task summary demo
task open demo
task doctor demo
task snapshot demo
task clear-done demo
project status demo
project snapshot demo
prefs reset-demo
profile show
config show
getting-started
commands
man-index
boot
help
status
sysinfo
which read
man index
man perm
quit
```

## Quick Start

```powershell
dbyte --version
dbyte run --vm examples\hello.dby
dbyte test --engine vm
```

Start an interactive session:

```powershell
dbyte repl
dbyte shell
```

Create a project:

```powershell
dbyte new scanner
cd scanner
dbyte run --vm
dbyte test
```

## Example: Binary Patch

```dbyte
import std.buffer as buf
import std.encoding as enc
import std.fs as fs

fs.write_bytes("sample.bin", b"\x00\xDE\xAD\xBE\xEF\x00")

let b: buffer = buf.load("sample.bin")
let pos: int = buf.find(b, b"\xDE\xAD\xBE\xEF")

if pos >= 0:
    buf.replace(b, pos, b"\x90\x90\x90\x90")
    buf.save("sample.patched.bin", b)

let patched: bytes = fs.read_bytes("sample.patched.bin")
print(enc.hex_encode(patched))
```

Run it:

```powershell
dbyte run --vm examples\binary_patcher.dby
```

## Common Commands

```powershell
dbyte run <file>
dbyte run --vm <file>
dbyte check <file>
dbyte test
dbyte test --engine vm
dbyte disasm <file>
dbyte tokens <file>
dbyte ast <file>
dbyte bench --compare-python
dbyte repl [--no-rc]
dbyte shell [--no-rc]
dbyte new <name>
```

## Interactive Runtime

`dbyte repl` keeps variables, functions, imports, and module state alive across
inputs. Use `.help`, `.reset`, and `.quit` / `.exit` for REPL control.

`dbyte shell` is a DByte-native command shell, not an OS passthrough shell.
Built-ins include `pwd`, `cd`, `ls`, `run`, `check`, `test`, `version`, and
`repl`. Shell help is generated from the built-in command registry, and
`which`, `alias`, `unalias`, and `aliases` are available for personal command
shortcuts. Execute DByte code explicitly with `:`, for example:

```txt
: let x: int = 40
: print(x + 2)
```

Shell aliases are command-level shortcuts, not DByte language syntax:

```txt
alias hello = run examples/hello.dby
hello
which hello
unalias hello
```

Both interactive commands load `.dbyterc` from the current directory unless
`--no-rc` is passed. Non-interactive commands such as `run`, `check`, `test`,
`bench`, and `new` do not load `.dbyterc`.

For `dbyte shell`, `.dbyterc` may include shell-only directives that are stripped
before DByte code is parsed:

```txt
@shell alias hello = run hello.dby

let boot: int = 41
```

See `personal_tools/` for a small personal command environment example.

## Personal Tools

`personal_tools/` contains self-contained DByte scripts for common binary
inspection and patching workflows. They use the existing file, buffer, binary,
and encoding standard modules and write only deterministic scratch files:

```powershell
dbyte run personal_tools\hexdump.dby
dbyte run personal_tools\bininfo.dby
dbyte run personal_tools\find_bytes.dby
dbyte run personal_tools\patch_bytes.dby
dbyte run personal_tools\read_u32_table.dby
```

They also accept script arguments for real files:

```powershell
dbyte run personal_tools\hexdump.dby firmware.bin
dbyte run personal_tools\hexdump.dby firmware.bin 16 64
dbyte run personal_tools\bininfo.dby firmware.bin
dbyte run personal_tools\find_bytes.dby firmware.bin DEADBEEF
dbyte run personal_tools\patch_bytes.dby firmware.bin DEADBEEF CAFEBABE
dbyte run personal_tools\patch_bytes.dby --all firmware.bin DEADBEEF CAFEBABE
dbyte run personal_tools\patch_bytes.dby --offset 128 firmware.bin CAFEBABE
dbyte run personal_tools\patch_bytes.dby firmware.bin DEADBEEF CAFEBABE --out firmware.patched
dbyte run personal_tools\read_u32_table.dby firmware.bin
dbyte run personal_tools\read_u32_table.dby firmware.bin 0 8
```

### Personal Tools Command Reference

| Tool | Usage | Description |
|---|---|---|
| `hexdump.dby` | `hexdump.dby <file> [offset length]` | Hex dump file bytes, 8 bytes per row |
| `bininfo.dby` | `bininfo.dby <file>` | File size, first 8 bytes hex, checksum |
| `find_bytes.dby` | `find_bytes.dby <file> <hex_pattern>` | Find byte pattern, print offset (dec + hex) |
| `patch_bytes.dby` | `patch_bytes.dby <file> <find> <replace>` | Patch first match, output to `<file>.patched` |
| `patch_bytes.dby --all` | `patch_bytes.dby --all <file> <find> <replace>` | Patch all matches |
| `patch_bytes.dby --offset` | `patch_bytes.dby --offset <N> <file> <replace>` | Patch at explicit byte offset |
| `patch_bytes.dby --out` | append `--out <outfile>` to any mode | Write output to explicit path instead of `<file>.patched` |
| `read_u32_table.dby` | `read_u32_table.dby <file> [offset count]` | Dump little-endian u32 table |

All tools support `--help` / `-h`.

From `dbyte shell`, the repo `.dbyterc` exposes shortcuts:

```txt
hexdump
bininfo
find-bytes
patch-bytes
u32-table
```

Script arguments are available to DByte code through `std.env.args()`. The list
contains only arguments after the script path.

## Embedding DByte

Rust host applications can embed the tree runtime through `dbyte_embed`:

```rust
use dbyte_embed::DByteRuntime;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt = DByteRuntime::new();

    rt.run_source("host", "let x: int = 40")?;
    let out = rt.run_source_capture("host", "print(x + 2)")?;

    assert_eq!(out.stdout.trim(), "42");
    Ok(())
}
```

The embed API uses persistent tree-interpreter state and does not auto-load
`.dbyterc`; host applications opt into startup scripts with `load_rc()`.

## Documentation

- [INSTALL.md](INSTALL.md)
- [LANGUAGE_SPEC.md](LANGUAGE_SPEC.md)
- [DByteOS Alpha](docs/DBYTEOS_ALPHA.md)
- [DByteOS Personal Alpha](docs/DBYTEOS_PERSONAL_ALPHA.md)
- [DByteOS Commands](docs/DBYTEOS_COMMANDS.md)
- [DByteOS Profile](docs/DBYTEOS_PROFILE.md)
- [DByteOS Config](docs/DBYTEOS_CONFIG.md)
- [DByteOS Security](docs/DBYTEOS_SECURITY.md)
- [DByteOS Boot](docs/DBYTEOS_BOOT.md)
- [benchmarks/BENCHMARKS.md](benchmarks/BENCHMARKS.md)

## Release Checklist

See [INSTALL.md](INSTALL.md) for install verification and release smoke tests.

## License

MIT. See [LICENSE](LICENSE).

