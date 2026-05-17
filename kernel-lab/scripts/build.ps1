Write-Host "Compiling DByteOS Kernel Lab freestanding binary..." -ForegroundColor Green
cargo build --target i686-unknown-linux-gnu
if ($LASTEXITCODE -eq 0) {
    Write-Host "[OK] Freestanding kernel ELF generated successfully." -ForegroundColor Green
} else {
    Write-Host "[ERROR] Freestanding compilation failed." -ForegroundColor Red
    exit $LASTEXITCODE
}
