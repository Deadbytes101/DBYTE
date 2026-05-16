# DByteOS Alpha Userland (v4.4.0)

Welcome to the Alpha release of DByteOS.

## What is DByteOS?

DByteOS is a **Personal Computing Userland** built on top of the DByte programming language and runtime. It provides a cohesive environment for file management, productivity, and system exploration, all within a deterministic and host-runnable framework.

> [!IMPORTANT]
> DByteOS v4.x is **NOT** a standalone operating system. It does not contain a kernel, bootloader, or hardware drivers. It is a "simulated OS" environment designed to run on top of Windows, Linux, or macOS.

## Core Philosophy

- **Determinism**: Every system event, from boot logs to security denials, is reproducible.
- **Translucency**: The system is designed to be inspected. Userland tools like `inspect` and `perm` allow you to see exactly how the system handles your data.
- **Portability**: DByteOS lives in a single directory (`examples/dbyteos/`) and can be ported across any host running the DByte VM.

## Current Status: Alpha

As an Alpha release, DByteOS features a stable core command set and a robust security model. Future releases will focus on expanding the userland ecosystem, improving the developer experience, and refining the "HolyC-style" interaction model.

---
[Home](../README.md) | [Commands](DBYTEOS_COMMANDS.md) | [Security](DBYTEOS_SECURITY.md) | [Boot](DBYTEOS_BOOT.md)
