# DByte Native Systems Toolchain

`i686-dbyte-none` is DByte's freestanding 32-bit systems target. It reuses the
shared lexer, parser, and AST, then follows a target-owned pipeline:

```text
DByte source modules -> native systems checking -> typed systems IR
                     -> bounded i686 encoder -> ELF32 ET_REL objects
                     -> DByte static linker -> Multiboot ELF32 ET_EXEC
```

This path does not invoke the interpreter, bytecode VM, LLVM, Cranelift, a C or
Rust transpiler, an assembler, `ld`, or `lld`. Generated programs contain only
DByte-owned machine code and static data.

## Build and inspect

An integrated build resolves the import graph, emits deterministic in-memory
objects, and calls the same linker library exposed by `dbytec link`:

```powershell
dbytec build examples\native_kernel_multimodule\kernel.dby `
    --target i686-dbyte-none --emit elf -o kernel.elf
```

Objects can also be produced and linked independently:

```powershell
dbytec build examples\native_kernel_multimodule\serial.dby `
    --target i686-dbyte-none --emit obj -o serial.o
dbytec link arch_x86.o serial.o numbers.o memory.o kernel.o -o kernel.elf
```

`--emit bin` writes the final file-backed load image. `--dump-ir` and
`--dump-machine-code` are inspection outputs only. Compile and link failures
leave an existing output unchanged.

The original v11.0.0 i32/bool single-image source profile remains accepted for
existing kernels and applications. `--emit obj` and any source that opts into
modules or the systems type syntax use the v11.1 object/linker pipeline; the
legacy compatibility path remains DByte-owned native code and does not invoke
the interpreter, VM, or an external toolchain.

## Systems types and integer rules

| Type | Size | Alignment | Signed |
| --- | ---: | ---: | --- |
| `bool` | 1 | 1 | no |
| `i8` / `u8` | 1 | 1 | yes / no |
| `i16` / `u16` | 2 | 2 | yes / no |
| `i32` / `u32` | 4 | 4 | yes / no |
| `usize` | 4 | 4 | no |
| `ptr[T]` | 4 | 4 | no |

Arithmetic wraps at the declared integer width. Comparisons use the declared
signedness. Literals are checked against their contextual type before bytes are
emitted. Mixed-width arithmetic, implicit narrowing, and implicit
integer-pointer conversion are rejected. Explicit `as` casts are required;
only `usize` has an explicit pointer conversion.

The native operator set includes `+`, `-`, `*`, `&`, `|`, `^`, `~`, `<<`, and
`>>`. Shift counts must be integer values and the i686 low-five-bit count rule
is the documented wrapping systems behavior.

`&value` takes an address and `*pointer` loads or stores the pointee type.
Pointers are typed; arbitrary integer-pointer mixing is not accepted.

`array[T, N]` has a compile-time length, declaration-order elements, and the
alignment of `T`. Constant out-of-range indices are rejected. Dynamic native
indexing is intentionally unchecked in this low-level profile; it does not
claim runtime bounds safety.

Normal structs use declaration-order natural layout with tail padding to their
maximum field alignment. `packed struct` is an explicit alignment-one layout
with no inserted padding. `size_of(Type)` and `align_of(Type)` are compile-time
constants. Aggregate values are local/static layout facilities in this release,
not a cross-function aggregate ABI.

## Constants, statics, modules, and symbols

`const` is evaluated at compile time. Immutable `static` data is emitted in
`.rodata`, initialized `static mut` data in `.data`, and zero-initialized
storage in `.bss`; there are no runtime constructors.

`import "relative.dby" as name` resolves only on the local filesystem relative
to the importer. Traversal is canonical and deterministic. Cycles, duplicate
aliases/modules/exports, unresolved imports, and URL or network resolution are
rejected. `pub fn`, `pub static`, and `pub const` are visible across modules;
unqualified definitions remain object-local.

Standalone ET_REL modules do not need an entry point. A final executable must
contain exactly `fn kernel_main() -> int`.

## DBYTE32 ABI

DBYTE32 is the native binary contract across object boundaries:

- scalar arguments are evaluated and pushed left to right in 32-bit words;
- callee parameter `i` arrives at `[EBP + 8 + 4 * (N - 1 - i)]`;
- the callee copies each argument to a deterministic width-correct local slot;
- the caller removes all argument words after `call`;
- scalar results are returned in `EAX`;
- `EAX`, `ECX`, and `EDX` are caller-saved;
- `EBX`, `ESI`, `EDI`, and `EBP` are preserved when used;
- every generated return restores `ESP` and `EBP` exactly.

The DByte linker synthesizes `_start`, installs a bounded 16 KiB bootstrap
stack, calls `kernel_main`, and enters a `cli` / `hlt` loop after it returns.

## Freestanding intrinsics

The deliberately small native boundary lowers directly to x86:

```text
native.volatile_load8(address)       native.volatile_store8(address, value)
native.volatile_load16(address)      native.volatile_store16(address, value)
native.volatile_load32(address)      native.volatile_store32(address, value)
native.io_in8(port)                  native.io_out8(port, value)
native.io_in16(port)                 native.io_out16(port, value)
native.cpu_hlt()
```

Widths are checked statically. There is no host callback or hidden runtime.

## Object and executable contract

Objects are little-endian ELFCLASS32 `ET_REL` / `EM_386` files with `.text`,
`.rodata`, `.data`, `.bss`, `.symtab`, `.strtab`, `.shstrtab`, and relevant
`.rel.*` sections. The supported i386 REL relocations are `R_386_32` and
`R_386_PC32`. The in-process linker validates all object bounds and symbols,
applies relocations, and emits deterministic `ET_EXEC` output with no dynamic
interpreter, dynamic section, libc, or DLL imports. See
[DByte native objects and static linking](DBYTE_NATIVE_LINK.md) for details.

## Executable proof

`examples/native_kernel_multimodule/` contains five meaningful DByte modules.
They initialize COM1, cross object boundaries, exercise narrow and address-width
integers, pointers, arrays, structs, packed layout, bitwise operations, shifts,
and static storage, then compute and print 42 without embedding the string
`"42"`.

Run the byte inspection and QEMU acceptance proof with:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\verify_native.ps1
```

The exact serial result is:

```text
DBYTE NATIVE SYSTEM
MULTI MODULE ONLINE
LINKER ONLINE
SYSTEM TYPES ONLINE
42
```

There is no heap, dynamic array, 64-bit arithmetic, division, dynamic linking,
weak symbol, runtime module loading, scheduler, filesystem, driver framework,
or self-hosted compiler in this foundation release.
