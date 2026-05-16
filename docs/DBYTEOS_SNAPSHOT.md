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

Snapshot output is read-only in v4.8.0. It does not write config files, persist
settings, change security policy, or invoke the host OS.

---
[Home](../README.md) | [Commands](DBYTEOS_COMMANDS.md) | [Onboarding](DBYTEOS_ONBOARDING.md) | [Profile](DBYTEOS_PROFILE.md) | [Config](DBYTEOS_CONFIG.md) | [Security](DBYTEOS_SECURITY.md)
