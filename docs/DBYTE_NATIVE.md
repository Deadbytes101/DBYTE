# DByte Native Compiler

The `i686-dbyte-none` target is DByte's freestanding native profile. It reuses
the normal DByte lexer, parser, and AST, then follows a separate audited path:

```text
DByte source -> native validation -> typed native IR -> i686 lowering
             -> DByte machine-code encoder -> DByte ELF32 writer
```

The native path does not invoke the tree interpreter, bytecode VM, Python,
Rust runtime, LLVM, Cranelift, a C compiler, a linker, or an assembler. The
emitted ELF contains instructions encoded directly by `dbyte_native`.

## Build

```powershell
cargo run -p dbyte_cli --bin dbytec -- build examples/native_kernel/kernel.dby `
    --target i686-dbyte-none --emit elf -o tmp/dbyte_native_kernel_v11.0.0.elf
```

The existing CLI exposes the same operation as `dbyte build`. `--emit elf`
produces the bootable ELF32 executable. `--emit bin` produces the corresponding
flat load image. `--dump-ir` and `--dump-machine-code` are inspection options;
neither dump is used to generate the binary.

The final output path is touched only after lexing, parsing, validation,
lowering, fixup resolution, and ELF validation succeed.

## Freestanding native profile

Supported:

- `int`, represented as a signed wrapping i32 at runtime; literal bounds are
  checked at compile time;
- `bool`;
- immutable ASCII string literals in `.rodata` and byte indexing;
- lexical local variables and assignment;
- `+`, `-`, `*`, unary `-`, and `!`;
- `==`, `!=`, `<`, `<=`, `>`, and `>=`;
- `if` / `else`, `while`, and indented blocks;
- functions, typed arguments, calls, and typed returns.

Every native function needs explicit parameter and return types. The boot entry
is exactly `fn kernel_main() -> int`. All return paths must produce the
function's declared type; the entry therefore returns an i32.
Integer literals outside `-2147483648..=2147483647` are rejected before code
emission.

Rejected with deterministic native diagnostics:

- hosted top-level execution and globals;
- floats, bytes, buffers, lists, formatted strings, and modules/imports;
- `for`, division, nested functions, and hosted built-ins;
- filesystem, process, Windows, interpreter, VM, and host callback access;
- unknown symbols, duplicate definitions, wrong call arity, invalid operands,
  invalid intrinsics, unresolved calls, and invalid entry points.

## Native intrinsic boundary

Privileged operations exist only under the native target namespace:

```text
native.volatile_load8(address)
native.volatile_store8(address, value)
native.volatile_load32(address)
native.volatile_store32(address, value)
native.io_out8(port, value)
native.io_in8(port)
native.cpu_hlt()
```

Arguments are statically required to be i32. Byte stores/output use the low 8
bits, byte loads/input zero-extend to i32, and ports use the low 16 bits. Calls
lower directly to x86; hosted type checking and runtimes do not expose them.

## DBYTE32 ABI

DBYTE32 is the only native calling convention in this milestone:

- one 32-bit stack slot per argument and local;
- arguments are evaluated and pushed left to right;
- for `N` arguments, callee parameter `i` is copied from
  `[EBP + 8 + 4 * (N - 1 - i)]` into its deterministic local slot;
- the caller removes all argument slots after `call`;
- i32, bool, and string-pointer returns use `EAX`;
- `EAX`, `ECX`, and `EDX` are caller-saved;
- emitted functions do not use `EBX`, `ESI`, or `EDI`;
- `EBP` is the frame pointer and every return restores `ESP` exactly.

This ordering preserves DByte's existing left-to-right argument evaluation.

## Executable image

The ELF writer emits deterministic ELF32 little-endian `ET_EXEC` / `EM_386`
output at the 1 MiB load address. It includes a Multiboot v1 header, `.text`,
`.rodata`, `.data`, `.bss`, a bounded bootstrap stack, and symbol/string tables.
The synthetic `_start` sets the stack, calls generated `kernel_main`, then enters
a `cli` / `hlt` loop after return. There is no dynamic interpreter, dynamic
section, operating-system loader, libc dependency, DLL import, or host runtime.

`examples/native_kernel/kernel.dby` initializes COM1 and writes serial bytes in
DByte using `native.io_out8` and `native.io_in8`. Its arithmetic, locals,
function calls, branch, and loops compute and print `42`.

Run the complete native proof with:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify_native.ps1
```

The verifier performs a double build and SHA-256 comparison, parses ELF bounds,
uses GNU `objdump` only after DByte has emitted the image, boots it with QEMU,
and requires these serial lines in order:

```text
DBYTE NATIVE KERNEL
MACHINE CODE ONLINE
42
```
