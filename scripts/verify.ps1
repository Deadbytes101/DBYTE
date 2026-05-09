$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")

Set-Location $repoRoot

$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"

& $cargo fmt --all -- --check
& $cargo clippy --all-targets -- -D warnings
& $cargo test
& $cargo run -q -p dbyte_cli -- test

Write-Host "verify passed"
