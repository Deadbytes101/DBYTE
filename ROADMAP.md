# DByte Roadmap

## Frozen release lineage

- v13.0.0 remains the historical `VIRTUAL_QUALIFIED_SOFTWARE_FINAL` boundary at commit `3b68536dc280dade3afde1620eeffbeb0309ec97`.
- v13.1.0 is the immutable tagged parent for v13.2.0 at commit `867bebea05d796855590099d2340de09f2ced4d5`.
- v13.2.0 is the GNU/Linux-native host and package qualification line built on the v13.1 portable/native-float baseline.
- v13.2 final status is established by the external qualification receipt for one exact candidate identity. Source mutation creates a new candidate and invalidates prior final evidence.
- `REAL_STORAGE=DISARMED`. A `FULL_OS` claim remains a separate future acceptance boundary.

## v13.2 closure contract

1. Freeze one candidate identity from the intended successor inputs while excluding protected user-owned worktree files.
2. Reproduce the source-transfer package independently and verify its source manifest in a clean extraction.
3. Run the full GNU/Linux host matrix, deterministic A/B package build, clean-extraction smoke, cross-target checks, and glibc-floor inspection.
4. Bind final mode to the accepted BIOSTAR inventory and a fresh live inventory on native Linux; WSL is development evidence only.
5. Rebuild the Linux package A/B on BIOSTAR, require byte-identical package hashes, and execute the packaged compiler/runtime smoke path.
6. Seal an external receipt that binds candidate identity, source package, physical package, host result, package result, machine fingerprint, and `REAL_STORAGE=DISARMED`.
7. Any mutation after qualification returns to step 1 for the new candidate.

## After v13.2

- Commit, tag, push, publication, and external release remain explicit delivery actions; qualification does not authorize them automatically.
- Preserve v13.0 and v13.1 tags and receipts as historical authorities; never move or recreate them.
- UEFI, SMP, GUI, networking, real-storage writes, and a full-OS claim remain separate milestones unless a later approved scope brings them in.
- Hardware implementation follows observed topology. QEMU evidence never substitutes for physical qualification, and physical qualification never authorizes disk writes.

## Release discipline

- One heavy verifier at a time.
- Every mutation invalidates dependent evidence; rerun the narrow affected gate before broader verification.
- Candidate identity, package hashes, clean-extraction behavior, physical-machine binding, and final receipt must agree before a FINAL classification.
- Release documentation must describe the actual platform package and must not carry stale version, installer, or physical-status claims.
