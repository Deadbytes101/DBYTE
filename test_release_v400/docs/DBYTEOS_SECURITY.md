# DByteOS Security Policy

DByteOS implements a centralized, deterministic security layer to protect the userland environment.

## Policy Overview

The security policy is defined in `sys/security.dby`. It governs all file-level operations performed by enforced commands (`cat`, `touch`, `inspect`, `read`, `write`, `append`).

### Directory Rules

| Path | Read | Write | Rationale |
| :--- | :--- | :--- | :--- |
| `bin/` | Allowed | Denied | System binaries are immutable. |
| `etc/` | Allowed | Denied | System configuration is read-only. |
| `sys/` | Allowed | Denied | Core system logic is protected. |
| `tmp/` | Allowed | Allowed | Temporary workspace for scripts. |
| `home/deadbyte/` | Allowed | Allowed | Primary user data zone. |

### Global Guards

1. **Path Traversal**: Any path containing `..` is strictly blocked to prevent escaping the OS root.
2. **Absolute Paths**: Paths starting with `/`, `\`, or drive letters (e.g., `C:`) are blocked.
3. **Unknown Roots**: Access to directories not explicitly defined in the policy is denied.

## Security Logging

Unauthorized access attempts are logged to `tmp/security.log` in a deterministic format:
```txt
DENY cat tmp/../etc/system.dby (path escape)
DENY touch etc/config.txt (policy)
```

## Policy Exceptions

Selected files in the root directory (`boot.dby`, `.dbyterc`) are granted **Read-Only** access to allow for system initialization and inspection without compromising integrity.

---
[Home](file:///C:/Users/DEADBYTE/Downloads/ProgramingLangPJ/README.md) | [Alpha Status](file:///C:/Users/DEADBYTE/Downloads/ProgramingLangPJ/docs/DBYTEOS_ALPHA.md) | [Commands](file:///C:/Users/DEADBYTE/Downloads/ProgramingLangPJ/docs/DBYTEOS_COMMANDS.md) | [Boot](file:///C:/Users/DEADBYTE/Downloads/ProgramingLangPJ/docs/DBYTEOS_BOOT.md)
