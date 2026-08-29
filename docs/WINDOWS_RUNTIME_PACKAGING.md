# Windows Runtime Packaging Correction

Status: normative packaging contract for `v12.2.1`.

`v12.2.1` changes release packaging only. The compiler, Concrete HIR,
runtime semantics, DBYTE32/DBYTE64 ABIs, object formats, linkers, platform
runtimes, and frozen native proofs remain the `v12.2.0` implementations.

## Source authority

Every Windows package is derived from one exact peeled release commit. The
packager accepts separate source/export and output roots, verifies the source
tree byte-for-byte against Git blobs, selects package content only from the
tracked manifest, and fails closed on a version or commit mismatch. Historical
ZIP files are evidence, not source authority.

The canonical package surface is:

- `dbyte.exe` and `dbytec.exe` built from the verified source tree;
- `README.md`, `INSTALL.md`, `LANGUAGE_SPEC.md`, and `LICENSE`;
- `scripts/install.ps1` and `benchmarks/BENCHMARKS.md`;
- tracked files under `docs/`, `examples/`, `kernel-lab/`, and `runtime/`.

`runtime/` is mandatory. The `v12.2.1` package contains the eleven files in
`runtime/v12.1.0/` and the two platform allocator providers in
`runtime/v12.2.0/`. There is no synthetic `runtime/v12.2.1/` tree.

## Deterministic archive contract

ZIP members use ordinal path order, `/` separators, fixed timestamps and
attributes, and no wrapper directory or embedded Git bundle. Windows release
binaries are linked with reproducible PE metadata and source paths are remapped
before packaging. A package is published only after two independent tag exports
produce the same SHA-256.

The verifier rejects missing or empty runtime trees, future or nested runtime
paths, duplicate or unsafe ZIP members, manifest drift, wrong source commits,
binary-version drift, and any per-file hash mismatch. Output replacement occurs
only after the candidate ZIP passes the complete manifest verification.

## Historical correction policy

Corrected `v12.1.0` and `v12.2.0` ZIPs preserve their immutable commits and
annotated tags. Their contents are rebuilt from the exact corresponding peeled
tag. Content absent from those tags is contamination and is removed; tag-valid
members are sourced from the tag bytes, and the original ZIP bytes and checksum
receipts remain preserved as ignored local evidence.
