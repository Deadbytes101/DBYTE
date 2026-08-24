# DByte 12.0 Language Contract

Status: normative release contract for `v12.0.0`. The implementation and all
backend parity gates must pass before this document may be tagged as released.

## Compatibility and versioning

DByte 12 is additive to the hosted language and the frozen i686 kernel and user
profiles. A source/profile pair that does not use a DByte 12 feature retains its
previous representation and object metadata. DByteOS proofs through `v11.11.0`
are immutable compatibility inputs, not implementation dependencies.

The language has one semantic authority. Parsing produces the shared AST;
resolution and typing produce canonical IDs and typed HIR; ownership analysis
annotates value use; profile validation rejects unavailable capabilities; and a
backend lowers already-resolved HIR. A backend must not reinterpret a canonical
type or infer a different layout.

## Canonical HIR

The stable identity domains are `ModuleId`, `SymbolId`, `TypeId`,
`GenericDefId`, `VariantId`, `FieldId`, and `ClosureId`. IDs are allocated by
canonical local-module traversal and never from hash-map iteration order.

Typed expressions record their `TypeId`, lvalue or rvalue category, and one of
Copy, Move, Borrow, or BorrowMut. Function signatures record their calling
convention. Enum construction and patterns record resolved variants. Closure
captures retain source order. Every active `defer` creates explicit LIFO cleanup
edges for fallthrough, return, break, continue, and `?` propagation.

Pointer size, pointer alignment, `usize` size, and `usize` alignment are target
data-layout inputs. Rust host `usize` is never a target layout input.

## Types and layout

The fixed integer types are `i8`, `u8`, `i16`, `u16`, `i32`, `u32`, and
`usize`. `ptr[T]` is nullable and mutable; `const_ptr[T]` is nullable and
read-only. `ptr[T]` widens implicitly to `const_ptr[T]`; the reverse conversion
is invalid. Integer-pointer conversion is explicit and only `usize` participates.
Pointer arithmetic scales by the pointee layout; byte addressing uses `ptr[u8]`.

`slice[T]` and `mut_slice[T]` are non-null borrowed pointer-length views.
`StringView` is an immutable borrowed UTF-8 pointer and byte length. A string
literal becomes a static `StringView` in a v12 context and retains legacy `str`
representation only when legacy `str` is explicitly expected. Conversion from
`StringView` to `slice[u8]` is explicit. The reverse conversion validates UTF-8
and returns `Result`.

Ordinary structs use declaration-order target layout. `@repr(C)` uses the same
target engine with the i686 C field rules. `@repr(C, packed)` reduces field and
aggregate alignment to one. Struct construction is named and must initialize
every declared field exactly once.

Payload enums use a `u32` tag at offset zero. Discriminants follow declaration
order. Payload offset is `align_up(4, max_payload_alignment)` and final alignment
is `max(4, max_payload_alignment)`. Tuple payloads use aggregate layout. There
is no niche, null, or smallest-tag optimization. Inactive bytes and padding are
unspecified.

Legacy `str`, list, bytes, and buffer remain hosted-compatibility types. New
public APIs must use the v12 types. No backend may assign a different
representation to the same canonical type.

## Generics

Generics use brackets:

```dbyte
fn id[T](value: T) -> T:
    return value

struct Pair[A, B]:
    first: A
    second: B
```

Instantiation is compile-time monomorphization keyed by `GenericDefId` and the
canonical `TypeId` list. Machine-code folding does not merge type identity.
Generic bodies are checked symbolically. Numeric, equality, ordering, and Copy
requirements are inferred from operations and checked for each specialization.
Same-type recursive calls reuse the current specialization. A recursive call
whose type arguments grow fails with
`E_GENERIC_INSTANTIATION_RECURSION`. Exported generic bodies are stored in
non-loadable `.dbyte.generics` metadata, validated by the linker, and omitted
from an executable image.

There is no public trait, constraint, dictionary, reflection, boxing, or runtime
generic metadata in version 12.0.

## Option, Result, match, and propagation

`Option[T]` and `Result[T, E]` are ordinary canonical prelude payload enums.
They obey the public enum layout above.

`match` is exhaustive and accepts qualified variants, tuple bindings, and `_`.
It has no guards or named variant payloads. `match value` moves non-Copy
payloads; `match borrow value` creates immutable borrowed bindings; and
`match borrow mut value` creates exclusive mutable borrowed bindings.

`?` accepts canonical `Result` only. It evaluates its operand once, yields the
`Ok` payload, and returns `Err` through the enclosing function or closure. The
enclosing error `TypeId` must be identical. There is no conversion, exception,
or unwinding.

## Ownership and borrowing

Integers, bool, raw pointers, borrows, slices, `StringView`, function pointers,
and aggregates or enums whose members are all Copy are Copy. Other values move
on assignment, parameter passing, and return. Use after move is invalid.

`borrow value` permits immutable aliases. `borrow mut value` is exclusive.
While borrowed, the owner cannot move, be destroyed, or be assigned in a way
that changes its storage. Lifetimes are lexical. Temporary lifetime extension
and hidden closure capture are invalid. A borrowed return must derive directly
from exactly one borrowed parameter; ambiguous or local borrowed returns fail.

Automatic `Drop` is not part of version 12.0. Explicit `defer` is the only
general cleanup mechanism in this release.

## Functions, closures, and defer

`fn(A) -> R` is a DByte function pointer. `extern "C" fn(A) -> R` is a C
function pointer. A non-capturing closure can coerce to a compatible DByte
function pointer. C coercion additionally requires an explicitly compatible C
calling convention. A capturing closure has a concrete code-plus-environment
type and cannot be erased, allocated, or returned in version 12.0.

Closure and defer capture lists are explicit and evaluated in source order:

```dbyte
let callback = fn [borrow factor, borrow mut total, move token](x: i32) -> i32:
    return x * factor

defer [borrow total]:
    print(total)
```

The environment contains captures in source order. Its call ABI is hidden
environment pointer followed by ordinary arguments. A move capture moves at
creation; a borrow capture remains active until the closure or defer scope ends.
`defer` blocks run in reverse registration order on every scope exit and use no
runtime defer stack.

## Modules and interfaces

Imports are local and deterministic. URL and network imports, cycles, duplicate
aliases, duplicate exports, and unresolved imports are errors. Unqualified
symbols are module-private. Only `pub` symbols cross an object or package
boundary.

An exported interface fingerprint includes canonical public types and layout,
generic definitions and inferred capabilities, symbol visibility, and callable
conventions. Package registries, lockfiles, and third-party resolution are not
part of version 12.0.

## i686 C ABI

The supported convention is i686 cdecl: arguments are laid out right-to-left,
the caller removes the argument area, scalar results use EAX, and EBX, ESI, EDI,
and EBP are callee-saved. DByte normalizes the outgoing stack to 16-byte
alignment without changing the legacy DByte ABI.

Allowed ABI types are unit, integer types no wider than 32 bits, `usize`, raw
pointers, and compatible C function pointers. Aggregate interoperability uses a
pointer to an `@repr(C)` or `@repr(C, packed)` struct. Structs by value, payload
enums, unions, varargs, large returns, C++, and exception interop are rejected.

DByte emits its own ELF32 objects, Unix archives, and final ELF. The verifier may
use an external compiler only to create the C fixture object. Archives normalize
metadata, sort members canonically, carry a symbol index, and use deterministic
fixed-point extraction.

## Grammar additions

```text
generic-params  := '[' IDENT (',' IDENT)* ']'
type-args       := '[' type (',' type)* ']'
struct-init     := type '(' IDENT '=' expr (',' IDENT '=' expr)* ')'
enum-decl       := 'enum' IDENT generic-params? ':' NEWLINE INDENT variant+ DEDENT
match-stmt      := 'match' ('borrow' 'mut'?)? expr ':' match-arm+
try-expr        := postfix-expr '?'
borrow-expr     := 'borrow' 'mut'? expr
closure         := 'fn' capture-list '(' params? ')' ('->' type)? ':' block
capture-list    := '[' capture (',' capture)* ']'
capture         := ('borrow' 'mut'? | 'move') IDENT
defer-stmt      := 'defer' capture-list ':' block
c-function      := 'extern' '"C"' 'fn' IDENT generic-params? signature body?
representation  := '@repr(C)' | '@repr(C, packed)'
```

## Target matrix

| Capability | Hosted | VM | i686 none/kernel | i686 user |
|---|---:|---:|---:|---:|
| Canonical types and HIR | yes | yes | yes | yes |
| Generics/enums/match/Result | yes | yes | yes | yes |
| Borrowed views/closures/defer | yes | yes | yes | yes |
| Raw privileged `native.*` | no | no | kernel profile only | no |
| Interrupt declarations | no | no | kernel profile only | no |
| User syscalls | no | no | no | user profile only |
| i686 C ABI and static archives | inspect | inspect | yes | restricted |
| Owned String/Array/collections/Drop | deferred | deferred | deferred | deferred |

Version 12.0 does not include an owned `String`, owned dynamic `Array[T]`, a
collection library, allocator abstraction, automatic drop glue, dynamic closure,
garbage collector, exception system, reflection, or x86_64 application target.
