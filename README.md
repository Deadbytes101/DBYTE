<p align="center">
<img src="assets/logo/dbyte-logo.png" width="180" alt="DBYTEOS Logo" />
</p>

<h1 align="center">THE DBYTE PROGRAMMING LANGUAGE</h1>

<p align="center">
<b>[ <a href="https://dbytelang.site">Official Site</a> ]</b> 
<b>[ <a href="https://dbytelang.site/about">About</a> ]</b> 
<b>[ <a href="#features">Features</a> ]</b> 
<b>[ <a href="benchmarks/BENCHMARKS.md#public-alpha-baseline-v192--v200">BENCHMARKS</a> ]</b> 
<b>[ <a href="https://dbytelang.site/docs/">DOCUMENTATION</a> ]</b>
</p>

<p align="center">
<a href="https://github.com/Deadbytes101/DByte/releases/ISO">
<img src="https://img.shields.io/badge/DBYTE-TRY%20ISO%20NOW-178da5?style=flat&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyOTMgMzA5Ij4KPHBhdGggZmlsbD0iIzAwMCIgZD0iTTAgMGgyMjB2MjZoMjl2MjdoMjl2MjdoMTV2MTQ1aC0yNXYyNmgtMjh2MjhoLTIwdjMwSDB6Ii8%2BCjxwYXRoIGZpbGw9IiMwMDU3ZDkiIGQ9Ik0xMCAxMGgyMDB2MjdoMjl2MjdoMjl2MjhoMTV2MTIyaC0yNXYyNmgtMjh2MjhoLTIwdjMxSDEweiIvPgo8cGF0aCBmaWxsPSIjMDAwIiBkPSJNMjggNTVoNTF2NTVIMjh6bTAgNzJoNTF2NTVIMjh6bTAgNzNoNTF2NTVIMjh6bTcwLTE0NWg4MXYzMGgyN3Y0MGgtMjd2MjloMjd2NDBoLTI3djU0SDk4eiIvPgo8cGF0aCBmaWxsPSIjZmZlNDVjIiBkPSJNMzcgNjRoMzJ2MzdIMzd6bTAgNzJoMzJ2MzdIMzd6bTAgNzNoMzJ2MzdIMzd6bTcwLTE0NWg2MnYzMWgyN3YyMWgtMjd2NDdoMjd2MjJoLTI3djU0aC02MnoiLz4KPC9zdmc%2B" alt="Try the DByte ISO" />
</a>

<a href="https://discord.gg/hWuwUbrujb">
<img src="https://img.shields.io/discord/1505230512820588746?label=DISCORD&logo=discord&logoColor=white&color=5865F2" alt="Discord" />
</a>

<a href="https://github.com/Deadbytes101/DByte/stargazers">
<img src="https://img.shields.io/github/stars/Deadbytes101/DByte?style=flat&color=yellow" alt="⭐ STARS" />
</a>

<a href="https://github.com/Deadbytes101/DByte/blob/main/LICENSE">
<img src="https://img.shields.io/github/license/Deadbytes101/DByte?color=green" alt="MIT LICENSE" />
</a>
</p>

**DByte** is a fast low-level scripting language for binary parsing, buffer patching, byte search, typed integer work, and automation scripts that need simple syntax with predictable performance.

Built for byte-level jobs.
Not for hype. Not for framework circus. Just open the data, hit the buffer, patch what needs patching, and ship.

> **Public Alpha** — Expect breaking changes before stable release.

> [!CAUTION]
> **Warning:** This OS is experimental. Run it in a VM if you value your data.

## Features

- Low-level scripting focused on **binary parsing**, buffer patching, byte search, and typed integer work
- Statically checked, Python-like syntax with a bytecode VM
- `bytes` and mutable `buffer` data types with powerful stdlib
- DByteOS userland experiments (host-runnable, not a full OS)
- Handmade, minimal, direct — no framework bloat

## Highlights

- Project workflow with `Dbyte.toml`
- Binary stdlib for endian-aware operations
- Buffer stdlib (`load`, `save`, `find`, `replace`, `slice`, etc.)
- Built-in test runner: `dbyte test`
- Interactive REPL + real DByte-native shell
- Personal tools for hexdump, patching, binary inspection

## Getting Started

1. **Try the ISO** (VM recommended): [Download Latest ISO](https://github.com/Deadbytes101/DByte/releases/ISO)
2. **Read the docs**: [https://dbytelang.site/docs/](https://dbytelang.site/docs/)
3. **Clone & explore**: [github.com/Deadbytes101/DByte](https://github.com/Deadbytes101/DByte)

### Quick Start

```powershell
dbyte --version
dbyte repl
dbyte shell
dbyte run examples/hello.dby
dbyte test
```

### Launch DByteOS Shell

```powershell
dbyte shell --rc examples/dbyteos/.dbyterc
```

## Example: Binary Patch

```dbyte
import std.buffer as buf
import std.fs as fs

let b: buffer = buf.load("sample.bin")
let pos: int = buf.find(b, b"\xDE\xAD\xBE\xEF")

if pos >= 0:
buf.replace(b, pos, b"\x90\x90\x90\x90")
buf.save("sample.patched.bin", b)
```

## Personal Tools

`personal_tools/` — hexdump, bininfo, find-bytes, patch-bytes, u32-table

Run with shortcuts inside the shell: `hexdump`, `patch-bytes`, etc.

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

## Repository & Contact

- **Repository**: [Deadbytes101/DByte](https://github.com/Deadbytes101/DByte)
- **Creator**: [About DEADBYTE](https://dbytelang.site/about)
- **Discord**: [Join Community](https://discord.gg/hWuwUbrujb)

---

**License**: MIT. See [LICENSE](LICENSE).

**This is alpha software. Run in VM if you value your data.**
