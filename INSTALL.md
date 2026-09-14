# DByte Installation Guide

DByte v13.2.0 GNU/Linux packages ship the `dbyte` runtime CLI, the `dbytec`
compiler, the `dbtasm` assembler, versioned runtime files, docs, and examples.
The Linux package targets `x86_64-unknown-linux-gnu` and requires glibc 2.39 or
newer. The external qualification receipt is the authority for the exact
candidate and package hash; this guide does not self-certify a build.

## GNU/Linux Binary Tarball

Requirements:

- x86-64 GNU/Linux
- glibc 2.39 or newer
- no Rust or Python installation for the packaged binaries

Extract the release tarball into a stable directory:

```sh
tar -xzf dbyte-v13.2.0-linux-x86_64.tar.gz
cd dbyte-v13.2.0-linux-x86_64
```

Verify the packaged runtime and compiler directly:

```sh
./dbyte --version
./dbytec build examples/native_v13_2_linux_host/import_main.dby --target x86_64-dbyte-linux --emit elf -o /tmp/dbyte-v13.2-smoke
/tmp/dbyte-v13.2-smoke
printf 'exit=%s
' "$?"
```

Expected results:

```text
DByte 13.2.0
exit=42
```

The release tarball records `0755` mode for `dbyte`, `dbytec`, and `dbtasm`.
If those files are not executable after extraction, check the extraction tool
or filesystem before changing the package.

## Install for the Current User

A system-wide installer is not required. To put the three tools on a normal
user `PATH`:

```sh
mkdir -p "$HOME/.local/bin"
install -m 0755 dbyte dbytec dbtasm "$HOME/.local/bin/"
```

Ensure `$HOME/.local/bin` is on `PATH`, then verify from a new shell with
`dbyte --version`.

## Build From Source on GNU/Linux

Install a Rust toolchain and the normal C development toolchain for your
distribution, then from the source root run:

```sh
cargo build --release --locked --bins
./target/release/dbyte --version
```

Python 3 is required by the release verifiers, not by the packaged DByte
executables themselves.

## Windows

The v13.2.0 GNU/Linux tarball is not a Windows package. Use the Windows package
for the applicable tagged release, or build the source tree with Rust on
Windows and verify `target\release\dbyte.exe --version`. Do not apply the
Linux tarball's executable-mode or glibc requirements to Windows.

## Troubleshooting

- `GLIBC_x.y not found`: the host is older than the package's glibc 2.39 floor.
- `Permission denied` when starting a packaged binary: verify that extraction
  preserved mode `0755`; the release manifest records the expected mode.
- `dbyte: command not found` after a user install: add `$HOME/.local/bin` to
  `PATH` or run the binary by its explicit path.
- A generated DByte Linux executable returning `42` in the smoke test is the
  expected success contract, not an error from the compiler.
