# DByteOS Personal Workspace Beta Foundation Package Smoke Guide

DByteOS release packages include the DByte executable, examples, documentation, and the Personal Workspace Beta Foundation userland so a zip download can be smoke-tested without a source checkout.

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
welcome
check-system
doctor
prefs set system.prompt dbyteos>
snapshot
project reset-demo
task reset-demo
task list demo
task add demo write tests
task done demo 1
task status demo
task summary demo
task open demo
task doctor demo
task snapshot demo
task clear-done demo
project status demo
project notes demo
project snapshot demo
project doctor demo
prefs reset-demo
profile show
config show
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
- `boot` initializes the userland and writes deterministic session logs.
- `welcome` prints the onboarding entry point.
- `check-system` verifies the package is ready for interactive use.
- `doctor` prints the full system health report.
- `prefs set system.prompt dbyteos>` changes the next DByteOS shell prompt.
- `prefs reset-demo` restores the default DByteOS shell prompt.
- `snapshot` prints the read-only system summary.
- `project reset-demo` restores a deterministic workspace project.
- `task reset-demo`, `task list demo`, `task add demo write tests`, `task done demo 1`, `task status demo`, `task summary demo`, `task open demo`, `task doctor demo`, `task snapshot demo`, and `task clear-done demo` verify project task state and UX.
- `project status demo`, `project notes demo`, `project snapshot demo`, and `project doctor demo` verify project state.
- `profile show` prints the deterministic profile summary.
- `config show` prints read-only preferences.
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
- User data such as `home/deadbyte/journal.txt` and `home/deadbyte/projects/` is preserved by clean.
- v8.12.1 disabled path foundation keeps workspace project names path-like safe and reports missing projects as `error: project not found: missing`.

