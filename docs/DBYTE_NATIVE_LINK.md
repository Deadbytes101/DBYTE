# DByte native objects and static linking

The `i686-dbyte-none` systems path compiles every source module independently:

```text
.dby -> shared lexer/parser/AST -> systems type checking -> typed IR
     -> bounded i686 encoder -> ELF32 ET_REL object
```

The in-process DByte linker reads those objects, resolves their symbols and
relocations, lays out the final image, adds the Multiboot entry and bounded
bootstrap stack, and writes an ELF32 `ET_EXEC`. It does not invoke `ld`, `lld`,
an assembler, a C compiler, LLVM, or Cranelift.

## Module and symbol model

`import "relative.dby" as name` creates a compile-time dependency. Resolution
is relative to the importing source file, filesystem-only, cycle checked, and
ordered deterministically. There is no network, package-manager, or runtime
module loading.

`pub fn`, `pub static`, and `pub const` are visible to importing modules.
Unqualified definitions remain object-local. Duplicate strong exports and
undefined final symbols are link errors; weak and dynamic symbols are not part
of this profile.

## ELF32 object contract

Objects are ELFCLASS32, little-endian, `ET_REL`, `EM_386` files. Depending on
content they carry `.text`, `.rodata`, `.data`, `.bss`, `.symtab`, `.strtab`,
`.shstrtab`, and the relevant `.rel.*` sections. Symbols and relocations use a
canonical, source-independent order.

The supported i386 REL relocations are deliberately small:

- `R_386_PC32` for cross-object relative calls.
- `R_386_32` for string, static/global, and other absolute addresses required
  by generated code.

REL addends live in the relocated bytes. The linker validates the object class,
machine, sections, string and symbol tables, relocation indices and offsets,
arithmetic, and final layout bounds before it writes output.

## DBYTE32 cross-object ABI

- Arguments are evaluated and pushed left to right.
- Every scalar argument occupies one 32-bit stack word.
- The callee copies parameters to deterministic EBP-negative slots.
- The caller removes arguments after the call.
- Scalar results are returned in EAX.
- EAX, ECX, and EDX are caller-saved.
- EBX, ESI, EDI, and EBP are preserved whenever generated code uses them.
- EBP is the frame pointer and each return restores ESP exactly.

## Systems layouts

| Type | Size | Alignment | Signed |
|---|---:|---:|---|
| `bool` | 1 | 1 | no |
| `i8` / `u8` | 1 | 1 | type-dependent |
| `i16` / `u16` | 2 | 2 | type-dependent |
| `i32` / `u32` | 4 | 4 | type-dependent |
| `usize` | 4 | 4 | no |
| `ptr[T]` | 4 | 4 | no |

Arithmetic wraps at the declared integer width. Mixed-width arithmetic and
implicit narrowing are rejected. Explicit `as` casts are required; only
`usize` may convert explicitly to or from a pointer.

`array[T, N]` has exactly `N` declaration-order elements and the alignment of
`T`. Literal out-of-range indices are compile errors. Dynamic indexing in this
low-level profile is intentionally unchecked at runtime.

Normal structs place fields in declaration order with natural alignment and
tail padding to the maximum field alignment. `packed struct` is an explicit
opt-in with alignment one and no inserted padding. `size_of(Type)` and
`align_of(Type)` are compile-time constants.

## Commands

```powershell
dbytec build serial.dby --target i686-dbyte-none --emit obj -o serial.o
dbytec link arch_x86.o serial.o numbers.o memory.o kernel.o -o kernel.elf
dbytec build kernel.dby --target i686-dbyte-none --emit elf -o kernel.elf
```

The integrated ELF build resolves the module graph, emits deterministic
in-memory objects, and invokes the same linker library used by `dbytec link`.
`--emit obj` writes only the requested root module object; its imports remain
undefined ELF symbols for the final link. Compile and link failures use a
temporary sibling and leave an existing output unchanged.

## Deliberate exclusions

There is no heap, dynamic array, 64-bit arithmetic, dynamic linking, weak
symbol, runtime module loading, scheduler, filesystem, driver framework, or
self-hosted compiler in this foundation release.
