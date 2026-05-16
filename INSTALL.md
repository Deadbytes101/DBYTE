# DByte Installation Guide

DByte public alpha ships as a single Windows executable plus docs and examples.

## Install From Release Zip

1. Download `dbyte-v4.6.1-windows-x64.zip` from the release page.
2. Extract it to a stable folder.
3. Run the installer from the extracted folder:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

The installer copies `dbyte.exe` to:

```txt
%USERPROFILE%\.dbyte\bin
```

It also adds that folder to the user `PATH` if needed.

For CI or smoke tests where you do not want to change `PATH`, use:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1 -InstallDir .\target\verify-install -NoPathUpdate
```

Open a new PowerShell window and verify:

```powershell
dbyte --version
dbyte run --vm examples\hello.dby
```

Expected version for this release:

```txt
DByte 4.6.1
```

## Install From Source

Requirements:

- Windows PowerShell
- Rust toolchain from https://rustup.rs/
- Python 3.12 or later only if you want `bench --compare-python`

Build:

```powershell
cargo build --release
.\target\release\dbyte.exe --version
```

Install the source-built binary:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

## Manual PATH Setup

You can also copy `dbyte.exe` manually:

```powershell
mkdir $env:USERPROFILE\.dbyte\bin -Force
copy .\dbyte.exe $env:USERPROFILE\.dbyte\bin\dbyte.exe
```

Then add this folder to your user `PATH`:

```txt
%USERPROFILE%\.dbyte\bin
```

## Smoke Tests

After installation, run:

```powershell
dbyte --version
dbyte run --vm examples\hello.dby
dbyte run --vm examples\binary_patcher.dby
dbyte repl --no-rc
dbyte shell --no-rc
dbyte bench --compare-python
```

`bench --compare-python` requires `python` in `PATH`.

For interactive smoke tests, type `.quit` in `dbyte repl` or `quit` in
`dbyte shell`. Use `--no-rc` when testing a clean install so a local `.dbyterc`
cannot affect the result.

## Troubleshooting

- If `dbyte` is not found, restart PowerShell after install.
- If the installer cannot find `dbyte.exe`, run it from the extracted release zip
  or from the repository root after `cargo build --release`.
- If benchmark comparison fails, verify Python with `python --version`.
