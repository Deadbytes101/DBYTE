$kernelPath = "target\i686-unknown-linux-gnu\debug\dbyte_kernel"
if (-not (Test-Path $kernelPath)) {
    Write-Host "[ERROR] Kernel binary not found! Run build.ps1 first." -ForegroundColor Red
    exit 1
}

Write-Host "Checking for QEMU emulator installation..." -ForegroundColor Green
$qemu = Get-Command qemu-system-i386 -ErrorAction SilentlyContinue
if (-not $qemu) {
    Write-Host "[WARNING] qemu-system-i386 is not found in your PATH." -ForegroundColor Yellow
    Write-Host "Please install QEMU or run the built ELF kernel inside a virtualized x86 emulator." -ForegroundColor Yellow
    Write-Host "Kernel ELF is located at: $kernelPath" -ForegroundColor Green
    return
}

Write-Host "Launching freestanding DByteOS Kernel Lab under QEMU..." -ForegroundColor Green
& qemu-system-i386 -kernel $kernelPath
