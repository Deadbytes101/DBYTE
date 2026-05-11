# DByte

DByte is a fast low-level scripting language for binary parsing, buffer patching,
byte search, typed integer workloads, and automation scripts that need simple
syntax with predictable performance.

Public alpha status: DByte is usable for experiments and small tools, but the
language and standard library may still change before a stable 2.x release.

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

## Quick Start

```powershell
dbyte --version
dbyte run --vm examples\hello.dby
dbyte test --engine vm
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
dbyte new <name>
```

## Documentation

- [INSTALL.md](INSTALL.md)
- [LANGUAGE_SPEC.md](LANGUAGE_SPEC.md)
- [benchmarks/BENCHMARKS.md](benchmarks/BENCHMARKS.md)

## Release Checklist

See [INSTALL.md](INSTALL.md) for install verification and release smoke tests.

## License

MIT. See [LICENSE](LICENSE).
