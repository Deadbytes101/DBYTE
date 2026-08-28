# Modern Native Target Matrix (v12.2)

Status: normative release contract for `v12.2.0`.

All modern targets consume the same monomorphized `ConcreteHirProject`. Target
code may select representation, ABI, object format, executable format, and
entry sequence; it must not resolve names, infer types, rediscover ownership,
or synthesize Drop semantics.

| Target | Layout | DByte ABI | C ABI | Object | Executable |
| --- | --- | --- | --- | --- | --- |
| `i686-dbyte-none` | I686 | DBYTE32 | i686 cdecl | ELF32 | Multiboot ELF32/BIN |
| `i686-dbyte-user` | I686 | DBYTE32 | i686 cdecl | ELF32 | compact user ELF32 |
| `x86_64-dbyte-none` | X86_64 | DBYTE64 | none | ELF64 | Multiboot2 ELF64/BIN |
| `x86_64-dbyte-linux` | X86_64 | DBYTE64 | SysV AMD64 | ELF64 | static ELF64 |
| `x86_64-dbyte-windows` | X86_64 | DBYTE64 | Win64 | AMD64 COFF | PE32+ console |

On X86_64, pointers and `usize` are eight bytes. Canonical type identities do
not change with the target: only layout does. `Array[T]` and `String` are five
machine words (20 bytes on I686, 40 bytes on X86_64); slices, `StringView`,
closures, and `AllocatorRef` are two words.

X86_64 objects carry `.dbyte.target`. The object parser and internal linker
validate architecture, OS, profile, DByte ABI, C ABI, and formats before
mutation. External C objects are accepted only with an explicit compatible
Linux or Windows target. Bare-metal rejects C objects.
