# DByte Array[T] (v12.1)

Status: normative release contract for `v12.1.0`.

`Array[T]` is an owned, allocator-bound contiguous buffer. On i686 its layout
is exactly 20 bytes: `data`, `len`, `capacity`, allocator context, and allocator
vtable. Empty arrays are non-null dangling views with zero length and capacity.
Zero-sized element types are rejected.

Capacity grows deterministically from 0 to 4 and then doubles until sufficient.
Every element-count and byte-count calculation is checked. Reserve allocates or
resizes before publishing data/capacity, so failure preserves pointer, length,
capacity, and initialized elements.

Push, pop, insert, remove, clear, and explicit reserve operate through borrowed
array references. Bounds-sensitive operations return Option/Result rather than
performing unchecked safe access. Destruction drops initialized elements in
reverse index order before releasing the backing buffer through the array's
original allocator.

v12.1 cloning is restricted to Copy element types. `clone_in` is the only way
to choose a different allocator; ordinary moves preserve allocator identity.
Hosted and VM projects may import the versioned `hosted_array` facade as their
`array` module and call `array.new[T]()`. Freestanding profiles expose only the
explicit `with_allocator` and `with_capacity` constructors.
