# DByteOS Personal Workspace Beta Foundation (v7.7.0)

Welcome to **DByteOS**: a personal computing userland built on the DByte runtime.

## Directory Structure
- `/bin`: System utilities and commands.
- `/etc`: System configuration files.
- `/home`: User data zone (`home/deadbyte`).
- `/sys`: Core system logic and services.
- `/tmp`: Temporary workspace (deterministic sweep).

## Core Commands

| Command | Purpose |
| :--- | :--- |
| `cat` | View file contents |
| `touch` | Create/update files |
| `inspect` | View file metadata |
| `man` | View manual pages |
| `welcome` | Show onboarding entry point |
| `getting-started` | Show first-run checklist |
| `commands` | Browse commands by category |
| `man-index` | List manual topics |
| `notes` | Manage personal notes |
| `journal` | Personal event logger |
| `services` | Manage system services |
| `status` | Show system summary |
| `profile` | Show profile identity |
| `config` | Show read-only preferences |
| `snapshot` | Summarize subsystem state |
| `project` | Manage workspace projects |
| `clean` | Purge temporary artifacts |
| `boot` | Re-run system boot |

## Quick Start

1. Launch the shell with the OS profile:
   ```powershell
   ./dbyte.exe shell --rc examples/dbyteos/.dbyterc
   ```
2. Run `welcome` to see the onboarding entry point.
3. Type `profile show`, `config show`, `snapshot`, `project list`, `task list demo`, `getting-started`, `commands`, `man-index`, or `man <topic>` to explore the environment.
4. Use `prefs set system.prompt dbyteos>` to change the next shell prompt, and `prefs reset-demo` to restore the default demo state.

## Package Smoke

From an extracted zip release:

```powershell
.\dbyte.exe --version
.\dbyte.exe shell --rc examples/dbyteos/.dbyterc
```

Inside the shell, run:

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
boot
help
status
sysinfo
which read
man index
man perm
quit
```

## Security & Persistence
- **Enforcement**: File operations on `etc/`, `bin/`, and `sys/` are read-only.
- **Persistence**: User data in `home/deadbyte/journal.txt` and `home/deadbyte/projects/` is persistent.
- **Determinism**: All system logs (`boot.log`, `security.log`) are reproducible.

---
[Personal Workspace Beta Foundation](../../docs/DBYTEOS_PERSONAL_ALPHA.md) | [Alpha Positioning](../../docs/DBYTEOS_ALPHA.md) | [Onboarding](../../docs/DBYTEOS_ONBOARDING.md) | [Profile](../../docs/DBYTEOS_PROFILE.md) | [Config](../../docs/DBYTEOS_CONFIG.md) | [Snapshot](../../docs/DBYTEOS_SNAPSHOT.md) | [Projects](../../docs/DBYTEOS_PROJECTS.md) | [Tasks](../../docs/DBYTEOS_TASKS.md) | [Security Policy](../../docs/DBYTEOS_SECURITY.md)
