# DByteOS Personal Alpha (v5.4.0)

Welcome to the Personal Alpha milestone of DByteOS.

## What is DByteOS?

DByteOS is a **Personal Computing Userland** built on top of the DByte programming language and runtime. It provides a cohesive environment for file management, productivity, preferences, diagnostics, command discovery, and system exploration, all within a deterministic and host-runnable framework.

> [!IMPORTANT]
> DByteOS Personal Alpha is **NOT** a standalone operating system. It does not contain a kernel, bootloader, hardware drivers, or OS passthrough. It is a deterministic userland environment designed to run on top of Windows, Linux, or macOS through the DByte runtime.

## Core Philosophy

- **Determinism**: Every system event, from boot logs to security denials, is reproducible.
- **Translucency**: The system is designed to be inspected. Userland tools like `inspect` and `perm` allow you to see exactly how the system handles your data.
- **Portability**: DByteOS lives in a single directory (`examples/dbyteos/`) and can be ported across any host running the DByte VM.

## Current Status: Personal Alpha

As the v5.4.0 Personal Alpha milestone, DByteOS includes the shell, package flow, manual pages, onboarding, profile/config/preferences, prompt integration, notes, journal, workspace projects, project tasks, services, diagnostics, snapshot, and simulated permission model. Future releases can build on this baseline without redefining the userland boundary.

---
[Home](../README.md) | [Personal Alpha](DBYTEOS_PERSONAL_ALPHA.md) | [Commands](DBYTEOS_COMMANDS.md) | [Projects](DBYTEOS_PROJECTS.md) | [Tasks](DBYTEOS_TASKS.md) | [Security](DBYTEOS_SECURITY.md) | [Boot](DBYTEOS_BOOT.md)

