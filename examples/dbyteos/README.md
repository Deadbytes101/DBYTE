# DByteOS Userland (v3.1.0)

Welcome to **DByteOS**: a simulated operating-system-style userland that runs on the host DByte runtime.

## Directory Structure
- `/bin`: System utilities and commands.
- `/etc`: System configuration files.
- `/home`: User directories.
- `/sys`: System internal profile and logic.
- `/tmp`: Temporary files (ephemeral; ignored by git except layout markers).

## How to use
1. Launch the DByte shell with the DByteOS configuration:
   ```powershell
   ./target/release/dbyte.exe shell --rc examples/dbyteos/.dbyterc
   ```
2. Run the boot sequence:
   ```txt
   dbyte> boot
   ```
3. **Command set** (also runnable as `dbyte run examples/dbyteos/bin/<name>.dby` from the repo root):
   - `whoami` — current user (simulated).
   - `sysinfo` — prototype banner and DByte version string.
   - `home` / `tmp` — logical paths in the simulated tree.
   - `ls-sys` — layout of top-level virtual mounts.
   - `write-demo` — writes only under `tmp/` (ignored); `cat` reads a path; `clean` removes known tmp artifacts.
   - `status` — system health; `inspect <file>` — existence check; `clean` — deterministic tmp sweep.

## Philosophy
DByteOS aims to provide a personal computing environment where the entire userland is scriptable in DByte. This prototype demonstrates the directory layout and command execution flow that will eventually be implemented in the native kernel.
