# DByte Linux native host portability

Status: v13.2.0 release contract. The annotated `v13.1.0` tag is immutable.
Final status is established only by the external qualification receipt bound to
the exact candidate identity; any source mutation requires requalification.
This work does not claim that Linux code generation is new: DByte has
emitted and internally linked static x86-64 Linux ELF images since v12.2.0.

The new acceptance boundary is that the DByte toolchain itself is built and run
as GNU/Linux ELF executables:

```text
Linux Cargo/Rust -> dbyte + dbytec + dbtasm
Linux dbytec -> Concrete HIR -> Native MIR -> DByte ELF object/linker
Linux kernel -> generated static DByte ELF
```

The canonical host ABI is `x86_64-unknown-linux-gnu`. The release receipt records
the actual ELF interpreter, `DT_NEEDED` set, and maximum required GLIBC symbol
version from the final host binaries. A musl package is outside v13.2.0.

The host Rust compiler may use its normal GNU linker. This does not relax the
DByte production contract: generated DByte objects, archives, and executables
are written and linked by DByte without invoking GNU ld, lld, or a C runtime.
Clang or GCC may produce a verifier-owned C fixture object only. Linux external
ELF64 imports may carry .note.gnu.property, but DByte accepts only structurally
valid GNU_PROPERTY_X86_ISA_1_USED and GNU_PROPERTY_X86_FEATURE_2_USED
records as informational metadata. NEEDED, CET/AND, unknown, duplicate, or
malformed GNU properties remain fail-closed.

Final evidence must come from native Linux and the physical BIOSTAR host. WSL,
Wine, and Windows-hosted DByte binaries are rejected as final evidence. Raw
storage remains disarmed; BIOSTAR tests may create ordinary files only inside a
verifier-owned temporary directory.

Final mode is fail-closed and cannot be enabled by an environment marker. It
requires the reviewed, unmodified machine-inventory receipt, its receipt hash,
the expected machine fingerprint, and the candidate identity established before
transfer. The verifier collects a fresh inventory on the running host and
requires both validated identities to match exactly:

```sh
scripts/verify_v13_2_linux_host.sh --mode full --final \
  --machine-inventory /path/to/accepted-biostar-inventory.json \
  --expected-inventory-sha256 <receipt-sha256> \
  --expected-machine-fingerprint <identity-sha256> \
  --expected-candidate-identity <candidate-sha256>
```

Both receipts must retain `disk_write = NO WRITE`; validation never promotes or
authorizes a storage device. A fixture receipt, incomplete identity, changed
machine, changed candidate, WSL kernel, or hash mismatch blocks final status.
Final package mode independently requires the same expected candidate identity;
it cannot promote a different dirty tree by version string alone.

The Linux release package contains one top-level directory with the three
executables, versioned runtime, canonical examples and documentation, manifest,
and SHA-256 list. Tar headers, ownership, modes, ordering, and gzip timestamps
are fixed so two independent builds of the same candidate are byte-identical.
