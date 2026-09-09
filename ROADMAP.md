# DByte Roadmap

## Frozen software baseline
- v13.0.0 is sealed as `VIRTUAL_QUALIFIED_SOFTWARE_FINAL`.
- Release commit: `3b68536dc280dade3afde1620eeffbeb0309ec97`.
- Annotated tag: `v13.0.0`, peeled to the release commit.
- Canonical Windows package SHA-256: `40468ef823543b23638d633fbfc5e7e82b6da23ce4659238a3c427f2916337d9`.
- G1-G5 virtual acceptance: PASS under the v13 final verifier.
- Frozen v12.4.0 authority remains `a1a5c2458065f6f295d1c3e77df62a34846226e3`.
- `REAL_BIOSTAR=PENDING`; `REAL_STORAGE=DISARMED`; `FULL_OS=PENDING`.

## Immediate next: physical qualification
1. Run the read-only machine inventory collector on the actual spare target and return the unmodified receipt. Do not use the development PC as substitute evidence.
2. Validate the returned receipt with `scripts/verify_machine_inventory_receipt.py`; fixture, fingerprint mismatch, altered acceptance state, or missing topology must fail closed before selection.
3. Select `PRIMARY_REAL_HW_TARGET`, boot path, storage family, and input family only from observed target evidence. Unknown topology stays unselected.
4. Boot the exact v13 artifact on the selected target and capture serial/fault/discovery evidence before claiming real-hardware qualification.
5. Keep storage writes disarmed. A dedicated test disk may be selected only after its model, serial, path, capacity, controller relation, and non-system status are revalidated.
6. Any real disk write remains a separate explicit authorization gate and must never be inferred from inventory or prior virtual persistence proof.

## Software continuation
- Never move or recreate `v13.0.0`; post-release work continues from `main`.
- Preserve the G1-G5 QEMU receipts as virtual oracles while adding real-machine evidence separately.
- Do not guess hardware drivers. Storage/input implementation follows the selected physical topology.
- UEFI, SMP, GUI, and networking remain separate future boundaries unless a later approved milestone explicitly brings them into scope.
- A future full-OS claim requires its own acceptance contract; v13.0.0 intentionally does not make that claim.

## Release discipline
- One heavy verifier at a time.
- Every mutation invalidates dependent evidence; rerun the narrow affected gate before broader verification.
- Exact commit, tag, artifact hash, package smoke, and remote state must agree before any future FINAL classification.
- Real BIOSTAR evidence never waives storage-write authorization, and QEMU evidence never waives physical qualification.
