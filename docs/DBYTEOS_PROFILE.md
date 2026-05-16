# DByteOS Profile

DByteOS profile support is a deterministic, read-only identity layer for the
Alpha userland.

## Profile fields

```txt
user: deadbyte
home: home/deadbyte
shell: dbyte shell
mode: alpha-userland
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

The profile is read-only in v4.3.1. It does not write config files, persist
settings, change security policy, or invoke the host OS.

---
[Home](../README.md) | [Commands](DBYTEOS_COMMANDS.md) | [Onboarding](DBYTEOS_ONBOARDING.md) | [Security](DBYTEOS_SECURITY.md)
