# DByte String (v12.1)

Status: normative release contract for `v12.1.0`.

`String` is an owned UTF-8 byte buffer with the same 20-byte i686 layout as
Array: data, byte length, byte capacity, allocator context, and allocator
vtable. Embedded NUL bytes are permitted; length and capacity always count
bytes.

Empty strings use the allocator-independent zero-size dangling contract.
Capacity grows from 0 to 16 and then doubles. `from_view` and `append_view`
consume valid canonical `StringView` bytes, and reserve is transactional.
Clear retains capacity. Destruction releases the byte buffer through the
allocator stored in the value.

`StringView` remains borrowed and immutable. A literal materializes the
canonical two-word pointer/length representation, and `len(view)` reads its byte
length. `string.from_utf8(slice, allocator)` explicitly validates ASCII and
well-formed two-, three-, and four-byte UTF-8 sequences, rejecting overlong
encodings, surrogate code points, truncation, and values above U+10FFFF before
constructing an owned String. The result distinguishes invalid UTF-8 from an
allocator failure through `StringError`.
Owned codepoint/grapheme APIs are outside v12.1.

Hosted and VM projects may import the versioned `hosted_string` facade as their
`string` module and call `string.new()`. Freestanding construction remains
allocator-explicit.
