# v13.0.0 software release boundary

Classification: `VIRTUAL_QUALIFIED_SOFTWARE_FINAL`.

Sealed release: v13.0.0 at commit `3b68536dc280dade3afde1620eeffbeb0309ec97`; annotated tag `v13.0.0` peels to that exact commit. The canonical Windows package SHA-256 is `40468ef823543b23638d633fbfc5e7e82b6da23ce4659238a3c427f2916337d9`, and the final release receipt SHA-256 is `bf177552f4c75e70d6c7dd38b7de6a48f5520d71b3fed6f43aadb08556f5412f`. The release closure also verified `origin/main` and the remote peeled tag against the same commit. The qualification procedure below is retained as the exact gate contract that produced this sealed state.

`REAL_BIOSTAR=PENDING` and `REAL_STORAGE=DISARMED` remain independent of
software release qualification. QEMU execution does not certify a physical PC,
physical media durability, or a general-purpose production OS. Physical provisioning
and disk writes require separate authorization.

## Immutable parent

The frozen v12.4.0 commit is `a1a5c2458065f6f295d1c3e77df62a34846226e3`;
its annotated tag object is `28c074777f46bd9e8885f42c49f903ba34e19254`.
Historical proof sources, release artifacts and tags must not be rewritten.
The v13 candidate commit is a separate identity; set `DBYTE_V13_EXPECTED_HEAD`
to that exact commit for post-commit verifiers.

## Qualification gates

1. Review canonical source, documentation and verifier inputs; exclude user-local
   examples, generated caches, receipts, disk images and secrets from the commit.
2. Run serialized format, warnings-as-errors Clippy, Cargo regression and the full
   historical/behavioral verifier on the final pre-commit tree.
3. Commit the reviewed candidate, then rerun those checks and
   `scripts/verify_v13_final.ps1 -ExpectedHead <commit>` on the clean exact commit.
   Pre-commit receipts do not establish the new release identity.
4. Require G4 hardware-injected input, virtual DBFS write/flush/reboot persistence,
   and G5 aggregate (including transitive G1-G3 and userspace/worker/fault/shell
   gates) to pass. Inspect child receipt identities and hashes, not just PASS text.
5. Only after post-commit gates pass, create the annotated `v13.0.0` tag, verify its
   object type and peeled commit, and run the final verifier with `-RequireTag`.
   Never force-move or recreate an existing tag.
6. Independently export the exact tagged commit twice. Build Windows packages
   A/B with the deterministic packager, compare ZIP SHA-256 and manifests, then
   extract and smoke-test the binaries and runtime capability surface.
7. Seal an ignored evidence receipt binding commit, tag object, command exits,
   child results, compiler/artifact/package hashes, deterministic comparison and
   package smoke. State the classification and physical exclusions explicitly.

## Execution discipline

Run only one heavy release job at a time. Do not mutate source while it is active.
A failed gate stops release progression; classify the failure and make only the
smallest necessary repair, then invalidate dependent evidence and rerun it.
No feature additions, unrelated refactors, physical writes or pushes are part of
release closure. Once the bound final receipt is complete, the release-closure
watchdog must perform no repository mutations or duplicate verification builds.
