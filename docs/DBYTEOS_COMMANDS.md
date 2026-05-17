# DByteOS Command Reference

DByteOS provides a set of userland tools accessible via the shell autopath.

## Start Here

Use `welcome` for the first screen, `profile show` for identity, `config show` for read-only preferences, `snapshot` for a read-only system summary, `project list` for workspace projects, `task list demo` for project tasks, `getting-started` for the first-run checklist, `commands` for grouped discovery, `man-index` for manual topics, and `which <command>` to inspect autopath resolution.

## Stable Command Set

| Command | Purpose | Enforcement |
| :--- | :--- | :--- |
| `cat` | View file contents | **Yes (Read)** |
| `touch` | Create/update files | **Yes (Write)** |
| `inspect` | View file metadata | **Yes (Read)** |
| `ls` | List directory contents | No |
| `pwd` | Print working directory | No |
| `man` | View manual pages | No |
| `help` | Show system help | No |
| `welcome` | Show onboarding entry point | No |
| `getting-started` | Show first-run checklist | No |
| `commands` | Browse commands by category | No |
| `man-index` | List manual topics | No |
| `notes` | Manage personal notes | No |
| `journal` | Personal event logger | No |
| `services` | Manage system services | No |
| `log` | View system logs | No |
| `perm` | Check security policy | No |
| `clean` | Purge temporary artifacts | No |
| `boot` | Re-run system boot | No |
| `status` | Show system summary | No |
| `profile` | Show profile identity | No |
| `config` | Show read-only preferences | No |
| `snapshot` | Summarize subsystem state | No |
| `project` | Manage workspace projects | No |
| `task` | Manage project tasks and task UX | No |

## Discovery Flow

```txt
welcome
profile show
config show
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
getting-started
commands
man-index
help
status
which read
man index
man perm
path which notes
```

## Shell Interaction

DByteOS uses a simulated autopath. When you type a command, the shell looks in `bin/` and other registered paths.

### Autopath Configuration
The autopath is managed in `sys/session.dby` and typically enabled in `.dbyterc`:
```dbyte
@shell dbyteos_autopath on
```

### Aliases
Common aliases are defined in `.dbyterc` to provide a familiar experience:
```dbyte
@shell alias help = run bin/help.dby
@shell alias dir = ls
@shell alias whereami = pwd
```

---
[Home](../README.md) | [Alpha Status](DBYTEOS_ALPHA.md) | [Onboarding](DBYTEOS_ONBOARDING.md) | [Profile](DBYTEOS_PROFILE.md) | [Config](DBYTEOS_CONFIG.md) | [Snapshot](DBYTEOS_SNAPSHOT.md) | [Projects](DBYTEOS_PROJECTS.md) | [Security](DBYTEOS_SECURITY.md) | [Boot](DBYTEOS_BOOT.md)

