# DByte Sanctum

## Overview
A **DByte Sanctum** is a personal, programmable computing workspace built on top of the DByte Shell and Runtime. It represents a "Personal Environment" where the user has absolute control over their tools, configs, and automation.

## Spirit
Inspired by the personal computing movement, the Sanctum is:
- **Private**: Your logic, your tools, your data.
- **Instant**: Low latency, immediate script execution.
- **Integrated**: The shell and the language are one.

## Workspace Convention
A standard Sanctum workspace follows this layout:
- `.dbyterc`: Environment initialization, aliases, and imports.
- `boot.dby`: The entry point script for the workspace.
- `tools/`: Low-level binary and system manipulation tools.
- `scripts/`: High-level automation and helper scripts.
- `workspace/`: Data, binaries, and scratch files.

## Usage
To enter a sanctum, navigate to the sanctum directory and run:
```powershell
dbyte shell
```
The specialized `.dbyterc` will automatically load your personalized environment.

---
*DByte Sanctum: It is not TempleOS, not HolyC, and not a kernel. It is your own system.*
