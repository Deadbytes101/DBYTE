# DByteOS Alpha Userland (v4.0.1)

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
| `notes` | Manage personal notes |
| `journal` | Personal event logger |
| `services` | Manage system services |
| `status` | Show system information |
| `clean` | Purge temporary artifacts |
| `boot` | Re-run system boot |

## Quick Start

1. Launch the shell with the OS profile:
   ```powershell
   ./dbyte.exe shell --rc examples/dbyteos/.dbyterc
   ```
2. The system will autostart core services and provide the Alpha Userland banner.
3. Type `help` or `man <topic>` to explore the environment.

## Security & Persistence
- **Enforcement**: File operations on `etc/`, `bin/`, and `sys/` are read-only.
- **Persistence**: User data in `home/deadbyte/journal.txt` is persistent.
- **Determinism**: All system logs (`boot.log`, `security.log`) are reproducible.

---
[Alpha Positioning](file:///C:/Users/DEADBYTE/Downloads/ProgramingLangPJ/docs/DBYTEOS_ALPHA.md) | [Security Policy](file:///C:/Users/DEADBYTE/Downloads/ProgramingLangPJ/docs/DBYTEOS_SECURITY.md)
