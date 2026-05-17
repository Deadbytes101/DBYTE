# DByteOS Onboarding

DByteOS onboarding is a deterministic Personal Alpha first-run flow for users opening a release zip or entering the DByteOS shell for the first time.

## First commands

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
project snapshot demo
prefs reset-demo
profile show
config show
getting-started
commands
man-index
help
status
```

## Discovery path

- `boot` initializes the userland session.
- `welcome` gives the first screen and next commands.
- `check-system` verifies the package is ready.
- `doctor` prints the full subsystem health report.
- `prefs set system.prompt dbyteos>` updates the next DByteOS shell prompt.
- `snapshot` prints a read-only system summary for debugging.
- `project reset-demo` restores a deterministic workspace project.
- `task reset-demo`, `task list demo`, `task add demo write tests`, `task done demo 1`, and `task status demo` exercise project tasks.
- `project status demo` and `project snapshot demo` inspect project state.
- `prefs reset-demo` restores deterministic demo preferences.
- `profile show` prints the deterministic user profile.
- `config show` prints read-only system preferences.
- `getting-started` prints the first-run checklist.
- `commands` lists commands by category.
- `man-index` lists manual topics.
- `man <topic>` opens a manual page.
- `which <command>` shows how autopath resolves a command.

## Boundaries

DByteOS onboarding is userland documentation and command discovery only. It does not add a kernel, bootloader, OS passthrough, language syntax, or security-policy changes.

