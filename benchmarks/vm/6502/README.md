# NMOS 6502 VM corpus

This directory is a tracked, offline correctness and performance authority for
the 151 documented legal NMOS 6502 opcodes. `opcodes.csv` is the explicit
byte/mnemonic/addressing-mode matrix. Undocumented opcodes are intentionally
outside the corpus and must fail deterministically.

The release verifier requires the matrix counts `151/151/151`, exact trace and
final-state parity across the Concrete HIR tree executor, VM O0, and optimized
VM. The historical speed gate compiles the Array-backed driver with the exact
`v12.2.1^{}` compiler and candidate compiler; the Buffer-backed v12.3 driver is
a candidate-only equivalence proof.

No file in this directory is downloaded or generated from an external ROM.
Synthetic programs and expected hashes are repository-owned and deterministic.
The historical performance program uses a bounded 4 KiB Array backing because
its tracked ROM touches only `0x0200..0x0213`; the candidate-only correctness
matrix uses the complete 64 KiB address space, including zero-page and indirect
page-wrap edge cases.
