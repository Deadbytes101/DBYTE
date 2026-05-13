# DByteOS Userland (v3.3.0)

Welcome to **DByteOS**: a simulated operating-system-style userland that runs on the host DByte runtime.

## Directory Structure
- `/bin`: System utilities and commands.
- `/etc`: System configuration files.
- `/home`: User directories (`home/deadbyte` is the simulated user; `notes.txt` is git-ignored).
- `/sys`: System internal profile and logic.
- `/tmp`: Temporary files (ephemeral; ignored by git except layout markers).

## Path rules (v3.3.0)
- Commands **`read`**, **`write`**, **`append`**, and **`touch`** only accept paths under `tmp/` or `home/deadbyte/` (relative to the DByteOS tree). Paths containing `..` are rejected (no repo escape).
- **`cat`** still uses the broader `resolve_os_file` helper for inspecting arbitrary project paths (e.g. `boot.dby`); use **`read`** when you want the sandboxed user-data view.

## Session environment (v3.3.0)
- **`sys/session.dby`** defines the simulated `PATH`, resolved command-search roots (`COMMAND_ROOT=...` lines from `env` / `path`), and `lookup_command` for `path which <name>`.
- Search roots are listed in **`etc/cmd_path_roots.txt`** (mirrored by the `dbyte shell` **`@shell dbyteos_autopath on`** resolver in `dbyte_cli`).
- With autopath enabled, shell commands such as `cat` or `mkdir-demo` resolve to `bin/*.dby` without per-command aliases (hyphenated names map to underscore script names).

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
   - `home` / `tmp` — logical path labels in the simulated tree.
   - `ls-sys` — layout of top-level virtual mounts.
   - `write-demo` — idempotent demo write under `tmp/` (ignored by git).
   - `read` / `write` / `append` / `touch` — sandboxed file I/O under `tmp/` or `home/deadbyte/`.
   - `mkdir-demo` — creates `tmp/mkdir_demo/` and a fixed marker file (idempotent).
   - `notes` — resets `home/deadbyte/notes.txt` to a deterministic seed body.
   - `profile` — user + logical home + `os_version`.
   - `env` / `path` — simulated environment (not host OS env).
   - `cat` — read a file path (broader resolution than `read`).
   - `status` — system health; `inspect <file>` — existence check; `clean` — deterministic sweep of **known** `tmp/` artifacts only.

## Philosophy
DByteOS aims to provide a personal computing environment where the entire userland is scriptable in DByte. This prototype demonstrates the directory layout and command execution flow that will eventually be implemented in the native kernel.
