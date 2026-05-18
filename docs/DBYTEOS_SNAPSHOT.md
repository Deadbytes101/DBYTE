# DByteOS Snapshot

DByteOS snapshot support is a deterministic, read-only system summary for the
Alpha userland. It helps users inspect profile, config, security, and session
log state from one command without mutating preferences or files.

## Commands

```txt
snapshot
snapshot system
snapshot profile
snapshot config
snapshot security
snapshot logs
```

`snapshot` and `snapshot system` print the full summary. The focused modes print
one subsystem for debugging and package smoke tests.

Snapshot output is read-only in v8.3.0. It does not write config files, persist
settings, change security policy, or invoke the host OS.

Use `project snapshot <name>` for deterministic workspace project state and
`task status <name>` for project task state.

---
[Home](../README.md) | [Commands](DBYTEOS_COMMANDS.md) | [Onboarding](DBYTEOS_ONBOARDING.md) | [Profile](DBYTEOS_PROFILE.md) | [Config](DBYTEOS_CONFIG.md) | [Projects](DBYTEOS_PROJECTS.md) | [Tasks](DBYTEOS_TASKS.md) | [Security](DBYTEOS_SECURITY.md)

