# DByte Language Specification

Version: Public Alpha v5.3.0

DByte is a statically checked, Python-like scripting language with a bytecode VM
and low-level standard library support for binary parsing and buffer patching.

## Files and Projects

- Source files use `.dby`.
- A project can define `Dbyte.toml`.
- `package.entry` points to the main script.
- The embeddable runtime crate `dbyte_embed` runs the same tested language and
  standard library surface as the CLI tree interpreter.

Minimal `Dbyte.toml`:

```toml
[package]
name = "scanner"
version = "0.1.0"
entry = "src/main.dby"
```

## Types

| Type | Description |
|---|---|
| `int` | 64-bit signed integer |
| `float` | Floating-point number |
| `bool` | `true` or `false` |
| `str` | UTF-8 string |
| `bytes` | Immutable byte array |
| `buffer` | Mutable byte buffer |
| `list[T]` | Homogeneous list |
| `void` | No useful return value |

## Variables

```dbyte
let x: int = 10
let name: str = "DByte"
let data: bytes = b"\xDE\xAD\xBE\xEF"
```

Type annotations can be omitted when the checker can infer the type:

```dbyte
let x = 10
let data = b"abc"
```

## Control Flow

```dbyte
if x > 0:
    print("positive")
else:
    print("zero or negative")

while x > 0:
    x = x - 1
```

## Functions

```dbyte
fn add(a: int, b: int) -> int:
    return a + b

print(add(10, 20))
```

Recursive functions are supported with a deterministic runtime depth guard.

## Modules

```dbyte
import std.buffer as buf
import std.binary as bin
import "./calc.dby" as calc
```

Only `pub` declarations are exported from local modules:

```dbyte
pub fn add(a: int, b: int) -> int:
    return a + b
```

## Built-ins

```dbyte
print("hello")
print(len(b"abc"))
```

`len()` supports `str`, `list`, `bytes`, and `buffer`.

## Interactive Runtime

`dbyte repl` evaluates DByte statements with persistent tree-interpreter state.
Variables, functions, imports, and module state survive until `.reset` or
session exit. Multiline `fn`, `if`, `while`, and `for` blocks are finished with
a blank line.

REPL commands:

```txt
.help
.reset
.quit
.exit
```

`dbyte shell` is a DByte-native shell. It does not execute unknown commands as
operating-system commands. Supported shell commands:

```txt
help
quit
exit
clear
pwd
cd <path>
ls
run <file.dby>
check <file.dby>
test
version
repl
: <dbyte code>
```

`dbyte repl` and `dbyte shell` load `.dbyterc` from the current directory before
interactive input. Pass `--no-rc` to skip it. Other commands, including `run`,
`check`, `test`, `bench`, and `new`, never load `.dbyterc`.

## Standard Modules

### `std.fs`

```dbyte
fs.read_text(path: str) -> str
fs.write_text(path: str, text: str) -> void
fs.read_bytes(path: str) -> bytes
fs.write_bytes(path: str, data: bytes) -> void
fs.exists(path: str) -> int
```

### `std.encoding`

```dbyte
enc.hex_encode(data: bytes) -> str
enc.hex_decode(text: str) -> bytes
```

### `std.hash`

```dbyte
hash.sha256(data: bytes) -> bytes
```

### `std.buffer`

```dbyte
buf.new(size: int) -> buffer
buf.from_bytes(data: bytes) -> buffer
buf.to_bytes(data: buffer) -> bytes
buf.len(data: buffer) -> int
buf.get(data: buffer, offset: int) -> int
buf.set(data: buffer, offset: int, value: int) -> void
buf.slice(data: buffer, offset: int, length: int) -> bytes
buf.load(path: str) -> buffer
buf.save(path: str, data: buffer) -> void
buf.find(data: buffer, pattern: bytes) -> int
buf.replace(data: buffer, offset: int, replacement: bytes) -> void
```

### `std.binary`

Endian-aware byte reads:

```dbyte
bin.u8(data: bytes, offset: int) -> int
bin.i8(data: bytes, offset: int) -> int
bin.u16_le(data: bytes, offset: int) -> int
bin.u16_be(data: bytes, offset: int) -> int
bin.i16_le(data: bytes, offset: int) -> int
bin.i16_be(data: bytes, offset: int) -> int
bin.u32_le(data: bytes, offset: int) -> int
bin.u32_be(data: bytes, offset: int) -> int
bin.i32_le(data: bytes, offset: int) -> int
bin.i32_be(data: bytes, offset: int) -> int
```

Buffer writes:

```dbyte
bin.write_u16_le(data: buffer, offset: int, value: int) -> void
bin.write_u16_be(data: buffer, offset: int, value: int) -> void
bin.write_u32_le(data: buffer, offset: int, value: int) -> void
bin.write_u32_be(data: buffer, offset: int, value: int) -> void
```

### `std.math`

```dbyte
math.abs(x: int) -> int
math.min(a: int, b: int) -> int
math.max(a: int, b: int) -> int
```

### `std.env`

```dbyte
env.args() -> list[str]
```

`env.args()` returns only arguments passed after the script path. It does not
include the `dbyte` executable, subcommand, flags, or script path.

## Errors

Errors include a category, message, file path, line, and column:

```txt
TypeError: expected int, found str
 --> main.dby:1:14
```

Runtime integer overflow is checked and reported as:

```txt
RuntimeError: integer overflow
```
