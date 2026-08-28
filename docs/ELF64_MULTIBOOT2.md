# ELF64 and Multiboot2 (v12.2)

Status: normative release contract for `v12.2.0`.

DByte writes and parses ELFCLASS64 little-endian `EM_X86_64` objects and static
executables internally. Supported relocations are `R_X86_64_64`, `PC32`,
`PLT32`, `32`, and `32S`; every application checks range and overflow before
committing output. A final executable has no unresolved relocations,
`PT_INTERP`, or dynamic runtime.

The bare-metal image uses a Multiboot2 i386 header in the first 32 KiB. Its
deterministic wrapper starts at physical `0x00100000`; transition structures
remain below `0x00200000`; the DBYTE64 payload begins at `0x00200000`. The
bootstrap saves Multiboot registers, installs a temporary GDT, identity maps
1 GiB with 2 MiB pages, enables PAE and EFER.LME, enables paging, enters long
mode, aligns a bounded static stack, and calls `kernel_main` using DBYTE64
`RAX`/`RDX` slots.

The transition stub performs mode entry only. It contains no DByteOS kernel
logic and is synthesized by the internal final-image writer. GRUB ISO creation
is verifier-owned boot transport and contributes no object to the DByte link.
ELF and BIN emissions carry the same checked address-tagged load image, so no
external `objcopy` is required.
