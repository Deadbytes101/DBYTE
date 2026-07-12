# Security Policy

DBYTE is alpha software. Treat the language runtime, DByteOS userland, and Kernel Lab as experimental.

## Supported Scope

Security reports are most useful when they affect:

- DByte parser, type checker, interpreter, compiler, or VM behavior.
- File, buffer, binary patching, or module loading workflows.
- DByteOS userland scripts that model permissions, diagnostics, workspace state, or shell behavior.
- Kernel Lab code paths that affect boot, exceptions, IDT, IRQ, PIC planning, VGA, serial, or VM probe behavior.

## Out of Scope

- Build artifacts under `target/` or `kernel-lab/target/`.
- Old release bundles, zip packages, unpacked release folders, or local scratch files.
- Reports requiring real hardware execution outside the documented Kernel Lab flow.

## Reporting

Open a private security advisory on GitHub when possible, or contact the maintainer through the repository profile.

Please include:

- A short impact summary.
- Exact files or commands involved.
- Reproduction steps.
- Expected behavior and observed behavior.
- Whether the issue affects host DByte, DByteOS userland, Kernel Lab, or release packaging.

## Handling

The maintainer will prioritize reports that are reproducible, scoped, and tied to tracked source files.

