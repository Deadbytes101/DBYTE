# DByteOS Package Smoke Guide

DByteOS release packages include the DByte executable, examples, documentation, and the Alpha userland so a zip download can be smoke-tested without a source checkout.

## Package contents

A Windows zip package should include:

- `dbyte.exe`
- `README.md`
- `INSTALL.md`
- `docs/`
- `examples/dbyteos/`
- `scripts/install.ps1`
- `benchmarks/BENCHMARKS.md`

## Zip quickstart

From the extracted package root:

```powershell
.\dbyte.exe --version
.\dbyte.exe shell --rc examples/dbyteos/.dbyterc
```

Inside the DByte shell:

```txt
welcome
profile show
getting-started
commands
man-index
boot
help
status
sysinfo
which read
man index
man perm
quit
```

## Expected success signals

- `dbyte.exe --version` prints the packaged version.
- `welcome` prints the onboarding entry point.
- `profile show` prints the deterministic profile summary.
- `getting-started` prints the first-run checklist.
- `commands` prints commands grouped by category.
- `man-index` lists manual topics.
- `boot` prints the DByteOS banner and first-run guide.
- `help` shows grouped command discovery.
- `status` shows the system summary and filesystem integrity.
- `which read` resolves through DByteOS autopath.
- `man index` opens the manual topic index.
- `man perm` opens the permission command manual.

## Determinism notes

- Temporary session artifacts live under `examples/dbyteos/tmp/`.
- `clean` removes session logs such as `tmp/security.log`.
- User data such as `home/deadbyte/journal.txt` is preserved by clean.
