# DByteOS Mutable Preferences

**Version:** 6.8.0
**Subsystem:** User Configuration

## Overview
DByteOS `v6.8.0` introduces **Mutable Preferences**, an overlay configuration system that allows users to persist safe configuration changes across sessions.

Because DByteOS emphasizes determinism and strict security boundaries, standard configurations (`etc/config.dby`, `etc/system.dby`) remain read-only. The Mutable Preferences subsystem is strictly sandboxed.

## Storage
User preferences are stored as a DByte-native module at:
`home/deadbyte/preferences.dby`

This file is part of the user's workspace and is preserved during `clean` operations. It is written dynamically by the `sys/preferences.dby` API.

## Safe Writable Keys
To prevent arbitrary configuration injection and maintain system health, preferences are restricted to a strict allowlist of keys and values.

| Key | Description | Allowed Values |
| --- | --- | --- |
| `ui.theme` | Visual theme | `default`, `dark`, `light` |
| `system.prompt` | DByteOS shell prompt | `dbyte-shell>`, `dbyteos>`, `deadbyte>` |
| `user.display_name` | Name shown in profile | `deadbyte`, `guest`, `operator` |

When the shell is launched with the DByteOS rc file, `system.prompt` controls
the interactive shell prompt. The shell falls back to `dbyte-shell>` if the
preferences file is missing, malformed, or contains an unsupported prompt.

## Command Line Interface
The `prefs` command manages these settings:
- `prefs show` - View all preferences
- `prefs get <key>` - Get a specific preference
- `prefs set <key> <value>` - Set a safe preference
- `prefs status` - Check the health of the preference subsystem and backup
- `prefs doctor` - Validate the schema of the current preference file
- `prefs allowed` - List all safe mutable keys and their permitted values
- `prefs backup-demo` - Copy the current preferences to a `.bak` file
- `prefs restore-demo` - Restore preferences from a `.bak` file
- `prefs reset-demo` - Reset preferences to their default state

## Security and Diagnostics
- **Anti-Injection:** `sys/preferences.dby` validates all sets against the strict allowlist before generating the `preferences.dby` file.
- **Diagnostics:** `doctor` and `check-system` now verify the existence and integrity of the preferences file.

