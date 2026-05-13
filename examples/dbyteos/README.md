# DByteOS Userland Prototype (v3.0.0)

Welcome to the first prototype of **DByteOS**. This is a simulated operating system environment that runs directly on your host machine using the DByte runtime.

## Directory Structure
- `/bin`: System utilities and commands.
- `/etc`: System configuration files.
- `/home`: User directories.
- `/sys`: System internal profile and logic.
- `/tmp`: Temporary files.

## How to use
1. Launch the DByte shell with the DByteOS configuration:
   ```powershell
   ./target/release/dbyte.exe shell --rc examples/dbyteos/.dbyterc
   ```
2. Run the boot sequence:
   ```txt
   dbyte> boot
   ```
3. Use system commands:
   - `status`: View system health.
   - `ls`: List root directories (simulated).
   - `whoami`: Check current user (simulated).
   - `inspect <file>`: Inspect a file.
   - `clean`: Clean temporary files.

## Philosophy
DByteOS aims to provide a personal computing environment where the entire userland is scriptable in DByte. This prototype demonstrates the directory layout and command execution flow that will eventually be implemented in the native kernel.
