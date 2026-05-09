$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")

Set-Location $repoRoot

$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"

& $cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cargo clippy --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cargo test
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cargo run -q -p dbyte_cli -- test
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "verify passed"
