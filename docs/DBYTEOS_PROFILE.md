# DByteOS Profile

DByteOS profile support is a deterministic, read-only identity layer for the
Alpha userland.

## Profile fields

```txt
user: deadbyte
home: home/deadbyte
shell: dbyte shell
mode: beta-userland
theme: default
prompt: dbyte-shell>
```

## Commands

```txt
profile
profile show
profile whoami
profile home
profile theme
profile prompt
```

`profile` and `profile show` print the full summary. The other modes print one
field for scripting and package smoke tests.

Profile values are sourced from the read-only DByteOS config layer in v8.13.1.
The profile command does not write config files, persist settings, change
security policy, or invoke the host OS.

Use `snapshot profile` when you need the same identity values inside a broader
system snapshot. Workspace projects and tasks use the same profile home,
`home/deadbyte`, as their user data root.

---
[Home](../README.md) | [Commands](DBYTEOS_COMMANDS.md) | [Onboarding](DBYTEOS_ONBOARDING.md) | [Config](DBYTEOS_CONFIG.md) | [Snapshot](DBYTEOS_SNAPSHOT.md) | [Projects](DBYTEOS_PROJECTS.md) | [Tasks](DBYTEOS_TASKS.md) | [Security](DBYTEOS_SECURITY.md)

