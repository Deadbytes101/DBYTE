# DByteOS Personal Workspace Beta Foundation

DByteOS Personal Workspace Beta Foundation is the v6.2.2 milestone for the host-runnable DByteOS
userland. It marks the point where DByte, the interactive shell, packaged
examples, DByteOS commands, preferences, diagnostics, and documentation form a
cohesive personal computing environment.

## What is included

- DByte language runtime, REPL, shell, project workflow, and release package.
- DByteOS userland under `examples/dbyteos/`.
- Manual pages, onboarding, command discovery, and package smoke guides.
- Profile, config, mutable preferences, and shell prompt integration.
- Notes, journal, workspace projects, logger, services, diagnostics, and system snapshot commands.
- Simulated permission checks for protected and writable userland paths.

## First journey

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
```

The journey initializes the userland, shows onboarding, checks readiness,
verifies diagnostics, demonstrates a safe prompt preference, summarizes the
system, restores and inspects a deterministic workspace project, exercises
project tasks, and restores demo state.

## Boundaries

DByteOS Personal Workspace Beta Foundation is not a standalone operating system. It does not add a
kernel, bootloader, hardware drivers, OS passthrough, new language syntax, or
new security semantics. It remains a deterministic userland that runs through
the DByte runtime on the host system.

---
[Home](../README.md) | [Commands](DBYTEOS_COMMANDS.md) | [Onboarding](DBYTEOS_ONBOARDING.md) | [Projects](DBYTEOS_PROJECTS.md) | [Tasks](DBYTEOS_TASKS.md) | [Package Smoke](DBYTEOS_PACKAGE.md) | [Security](DBYTEOS_SECURITY.md)

