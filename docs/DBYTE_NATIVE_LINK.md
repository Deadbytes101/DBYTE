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

## Modern x86_64 machine-boundary linking

The current `x86_64-dbyte-none` linker can ingest canonical DByte ELF64 objects
and a checked subset of metadata-free external `EM_X86_64` `ET_REL` objects
when the target is explicit. External objects are a transitional machine
boundary only: they cannot select a target, enter a user profile, contain
dynamic dependencies, or bypass relocation and symbol validation.

For a fixed-address firmware or bootloader payload, the linker can materialize
a deterministic flat image while applying the same symbol and relocation
rules used by its ELF64 executable path:

```powershell
dbytec link --target x86_64-dbyte-none `
  --flat-load-address 0x10000 --flat-max-size 129024 `
  --flat-section-alignment 16 --entry kernel64_entry `
  boundary.o runtime.o -o kernel.bin
```

The flat writer validates the entry, load range, section alignment and
non-overlap, checked address arithmetic, BSS materialization, and maximum
payload size before transactional publication. It does not extract arbitrary
`.text`, patch bytes after linking, or invoke an external linker.

The constrained DBTASM frontend uses the same bounded x86_64 encoder and ELF64
writer as the compiler:

```powershell
dbtasm boundary.dbtasm --target x86_64-dbyte-none -o boundary.o
```

Its current instruction/directive surface is deliberately consumer-driven;
it is not a NASM, MASM, or historical Borland TASM compatibility layer. The
HOPPYSOS transition and remaining NASM boundary are documented in
[HOPPYSOS_DBYTE_MIGRATION.md](HOPPYSOS_DBYTE_MIGRATION.md).
