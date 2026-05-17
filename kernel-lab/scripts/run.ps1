param(
    [switch]$Serial
)

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$kernelLabRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path

$kernelPath = Join-Path $kernelLabRoot "target\i686-unknown-linux-gnu\debug\dbyte_kernel"
if (-not (Test-Path $kernelPath)) {
    Write-Host "[ERROR] Kernel binary not found! Run build.ps1 first." -ForegroundColor Red
    return
}

Write-Host "Checking for QEMU emulator installation..." -ForegroundColor Green
$qemu = Get-Command qemu-system-i386 -ErrorAction SilentlyContinue
if (-not $qemu) {
    $qemu = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
}

if (-not $qemu) {
    Write-Host "[WARNING] Neither qemu-system-i386 nor qemu-system-x86_64 was found in your PATH." -ForegroundColor Yellow
    Write-Host "Please install QEMU or run the built ELF kernel inside a virtualized x86 emulator." -ForegroundColor Yellow
    Write-Host "Kernel ELF is located at: $kernelPath" -ForegroundColor Green
    return
}

$qemuExeName = $qemu.Name

if ($Serial) {
    Write-Host "========================================================================" -ForegroundColor Green
    Write-Host "Launching freestanding DByteOS Kernel Lab in HEADLESS SERIAL mode..." -ForegroundColor Green
    Write-Host "Executing: $qemuExeName -kernel `"$kernelPath`" -serial stdio -display none" -ForegroundColor Cyan
    Write-Host "Note: Headless Serial Mode initiated. QEMU is running in the background." -ForegroundColor Green
    Write-Host "Press [Ctrl + C] in this terminal to terminate the simulation." -ForegroundColor Yellow
    Write-Host "========================================================================" -ForegroundColor Green
    & $qemuExeName -kernel $kernelPath -serial stdio -display none
} else {
    Write-Host "========================================================================" -ForegroundColor Green
    Write-Host "Launching freestanding DByteOS Kernel Lab under QEMU graphical display..." -ForegroundColor Green
    Write-Host "Executing: $qemuExeName -kernel `"$kernelPath`"" -ForegroundColor Cyan
    Write-Host "Note: Left-click inside the QEMU graphical display window to redirect keyboard focus!" -ForegroundColor Green
    Write-Host "Press keys (e.g. 'A') and observe raw make/break scancodes print to the VGA frame buffer." -ForegroundColor Yellow
    Write-Host "========================================================================" -ForegroundColor Green
    & $qemuExeName -kernel $kernelPath
}
