# DByteOS Boot Lifecycle

DByteOS uses a structured boot sequence to initialize the userland environment.

## Sequence of Events

1. **Host Launch**: The user runs `dbyte run boot.dby` or launches the shell with the OS profile.
2. **System Setup**: `boot.dby` initializes system variables and resets session logs.
3. **Core Handover**: Control is passed to `sys/init.dby`.
4. **Service Registry**: `init.dby` reads the service registry and executes all `AUTOSTART` services.
5. **System Readiness**: The system writes a session marker (`tmp/.dbyteos_boot_touch`) and displays the version banner plus a first-run guide.

## Initialization Logic

The initialization logic is encapsulated in `sys/init.dby`. It ensures that the system environment is consistent every time it starts.

### Service Registry
Services are registered in `sys/init.dby`. Current autostart services include:
- `notes`: Personal notes manager.
- `journal`: Event logging system.

## Customization

Users can customize their environment via the `.dbyterc` file in the OS root. This file is executed by the shell upon startup and can be used to set aliases, toggle autopath, or run custom startup scripts.

## First-run guide

After boot, DByteOS suggests the shortest discovery path:

```txt
help
status
man <topic>
which <cmd>
```

---
[Home](../README.md) | [Alpha Status](DBYTEOS_ALPHA.md) | [Commands](DBYTEOS_COMMANDS.md) | [Security](DBYTEOS_SECURITY.md)
