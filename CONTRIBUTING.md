# Contributing to DBYTE

DBYTE is currently a fast-moving working snapshot. Contributions are welcome when they keep the project direct, testable, and aligned with byte-level tooling.

## Project Priorities

- Keep DByte focused on binary parsing, buffer patching, typed integer work, automation scripts, and low-level experiments.
- Keep DByteOS userland host-runnable and reproducible.
- Keep Kernel Lab work isolated under `kernel-lab/` and clearly marked as experimental.
- Avoid adding framework weight or unrelated abstractions.

## Before Sending Changes

Run the core checks from the repository root:

```powershell
cargo check
```

For Kernel Lab changes:

```powershell
cd kernel-lab
powershell -ExecutionPolicy Bypass -File .\scripts\build.ps1
```

## Commit Style

Use short, direct commit messages. Do not use colon separators in commit text.

Good examples:

- `Polish DBYTE docs`
- `Harden buffer patch checks`
- `Add kernel lab status note`

## Pull Request Checklist

- The change is scoped to one purpose.
- Build or verification commands are listed in the PR.
- Generated build output, release bundles, VM logs, and scratch binaries are not committed.
- User-facing docs are updated when behavior changes.
- Kernel Lab changes explain whether they touch boot, IRQ, IDT, PIC, VGA, serial, or VM probe behavior.

