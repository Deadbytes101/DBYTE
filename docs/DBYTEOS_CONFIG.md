# DByteOS Config

DByteOS config support is a deterministic, read-only preferences foundation for
the Alpha userland. It centralizes existing system/profile values without
adding mutable settings.

## Config keys

```txt
system.mode = alpha-userland
system.prompt = dbyte-shell>
user.name = deadbyte
user.home = home/deadbyte
ui.theme = default
security.mode = simulated
```

## Commands

```txt
config
config show
config keys
config get system.prompt
```

`config` and `config show` print all values in stable order. `config keys`
prints the available keys. `config get <key>` prints one value for scripts and
package smoke tests.

The config layer is read-only in v4.4.1. It does not write config files,
persist settings, change security policy, or invoke the host OS.

---
[Home](../README.md) | [Commands](DBYTEOS_COMMANDS.md) | [Onboarding](DBYTEOS_ONBOARDING.md) | [Profile](DBYTEOS_PROFILE.md) | [Security](DBYTEOS_SECURITY.md)
