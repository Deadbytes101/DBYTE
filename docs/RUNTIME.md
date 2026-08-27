# DByte Runtime Foundation (v12.1)

Status: normative release contract for `v12.1.0`.

The runtime foundation is expressed through canonical DByte types and Concrete
HIR cleanup edges. A backend receives resolved `DropGlueId`-equivalent cleanup
actions; it must not rediscover ownership from source syntax.

## Destruction

`drop fn` is a nominal language item with one `borrow mut T` parameter and a
`unit` result. `T` must be a struct or enum owned by the same module. A type has
at most one such item. Source cannot call it directly; `drop(value)` consumes
the owner and invokes compiler-resolved glue.

Destruction runs the custom destructor first, then active payload or owned
fields in reverse declaration order. Moved sources lose their cleanup
obligation and destinations acquire exactly one obligation. Locals register
cleanup only after successful initialization. Return storage is populated
before local cleanup.

Drop and `defer` share one reverse-activation cleanup graph for fallthrough,
`return`, `break`, `continue`, and `?`. v12.1 has no dynamic drop flags:
partial moves and maybe-live/maybe-moved control-flow joins are rejected.
Architectural faults, aborts, and process termination do not unwind.

Public nominal drop contracts are carried in non-loadable `.dbyte.drop`
metadata. The object parser and linker validate canonical records and remove
the section from final executables.

## Runtime modules

The versioned sources under `runtime/v12.1.0/` implement allocator, Array, and
String behavior in DByte. Native code reaches providers through function
pointers in `AllocatorVTable`; there is no native Rust allocation shim.

The i686 owned collection layout is five words: data, length, capacity,
allocator context, and allocator vtable. The allocator capability moves with
the collection and is used again by its destruction glue.
