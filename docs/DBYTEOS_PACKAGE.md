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
boot
help
status
sysinfo
which read
man perm
quit
```

## Expected success signals

- `dbyte.exe --version` prints the packaged version.
- `boot` prints the DByteOS banner and first-run guide.
- `help` shows grouped command discovery.
- `status` shows the system summary and filesystem integrity.
- `which read` resolves through DByteOS autopath.
- `man perm` opens the permission command manual.

## Determinism notes

- Temporary session artifacts live under `examples/dbyteos/tmp/`.
- `clean` removes session logs such as `tmp/security.log`.
- User data such as `home/deadbyte/journal.txt` is preserved by clean.
