$ErrorActionPreference = "Stop"

$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"

& $cargo fmt --all -- --check
& $cargo clippy --all-targets -- -D warnings
& $cargo test
& $cargo run -q -p dbyte_cli -- test

Write-Host "verify passed"
