# DByteOS Command Reference

DByteOS provides a set of userland tools accessible via the shell autopath.

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
| `notes` | Manage personal notes | No |
| `journal` | Personal event logger | No |
| `services` | Manage system services | No |
| `log` | View system logs | No |
| `perm` | Check security policy | No |
| `clean` | Purge temporary artifacts | No |
| `boot` | Re-run system boot | No |
| `status` | Show system information | No |

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
[Home](file:///C:/Users/DEADBYTE/Downloads/ProgramingLangPJ/README.md) | [Alpha Status](file:///C:/Users/DEADBYTE/Downloads/ProgramingLangPJ/docs/DBYTEOS_ALPHA.md) | [Security](file:///C:/Users/DEADBYTE/Downloads/ProgramingLangPJ/docs/DBYTEOS_SECURITY.md) | [Boot](file:///C:/Users/DEADBYTE/Downloads/ProgramingLangPJ/docs/DBYTEOS_BOOT.md)
