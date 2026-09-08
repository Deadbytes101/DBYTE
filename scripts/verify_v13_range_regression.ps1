param([ValidateSet('0','1')][string]$OptLevel = '1')
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$root = Join-Path $repo ('tmp/v13_g1/range-' + [Guid]::NewGuid().ToString('N'))
$grub = Join-Path $root 'iso/boot/grub'
New-Item -ItemType Directory -Path $grub -Force | Out-Null
function Run([string]$exe, [string[]]$arguments) {
    $saved = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { $output = & $exe @arguments 2>&1; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    Add-Content -LiteralPath (Join-Path $root 'commands.txt') -Value "$exe $($arguments -join ' ')`nEXIT=$code`n$($output -join "`n")"
    if ($code -ne 0) { throw "command failed ($code); evidence: $root" }
    return ($output -join "`n")
}
function LinuxPath([string]$path) { (Run wsl.exe @('-d', 'Debian', '--', 'wslpath', '-a', '-u', $path.Replace('\','/'))).Trim() }
Push-Location $repo
try {
    Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_value_formation.ps1')) 'value formation prerequisite' | Out-Null
    $inputs = @(Get-ChildItem -LiteralPath (Join-Path $repo 'examples/native_v13_machine') -Filter '*.dby' -File)
    $inputs += Get-Item -LiteralPath (Join-Path $repo 'runtime/v12.1.0/serial.dby'), (Join-Path $repo 'target/debug/dbytec.exe')
    $inputs | ForEach-Object { [pscustomobject]@{ path=$_.FullName; bytes=$_.Length; sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash } } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'input-hashes.json')
    $elf = Join-Path $root 'iso/boot/kernel.elf'
    Run (Join-Path $repo 'target/debug/dbytec.exe') @('build', 'examples/native_v13_machine/range_regression.dby', '--target', 'x86_64-dbyte-none', '--opt-level', $OptLevel, '--emit', 'elf', '-o', $elf) | Out-Null
    [IO.File]::WriteAllText((Join-Path $grub 'grub.cfg'), "set timeout=0`nset default=0`nmenuentry 'DByte range regression' {`n multiboot2 /boot/kernel.elf`n boot`n}`n")
    Run wsl.exe @('-d', 'Debian', '--', 'grub-file', '--is-x86-multiboot2', (LinuxPath $elf)) | Out-Null
    $iso = Join-Path $root 'range.iso'
    Run wsl.exe @('-d', 'Debian', '--', 'grub-mkrescue', '-o', (LinuxPath $iso), (LinuxPath (Join-Path $root 'iso'))) | Out-Null
    $serial = Join-Path $root 'serial.log'; $trace = Join-Path $root 'trace.log'
    $qemu = 'C:/Program Files/qemu/qemu-system-x86_64.exe'
    $arguments = @('-cdrom', $iso, '-m', '32M', '-display', 'none', '-monitor', 'none', '-serial', "file:$serial", '-d', 'int,cpu_reset,guest_errors', '-D', $trace, '-no-reboot', '-no-shutdown')
    Add-Content -LiteralPath (Join-Path $root 'commands.txt') -Value "$qemu $($arguments -join ' ')"
    $owned = Start-Process -FilePath $qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(10)
        while (-not $owned.HasExited -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
    } finally {
        if (-not $owned.HasExited) { Stop-Process -Id $owned.Id; $owned.WaitForExit() }
        $owned.Dispose()
    }
    $text = [IO.File]::ReadAllText($serial)
    $faults = [IO.File]::ReadAllText($trace)
    @($elf,$iso) | ForEach-Object { [pscustomobject]@{ path=$_; bytes=(Get-Item -LiteralPath $_).Length; sha256=(Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash } } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'hashes.json')
    if ($text -cne "RANGE REJECT OK`r`n" -or $faults -match 'Triple fault|v=0[0de6]') {
        throw "RANGE REGRESSION FAILED: $root"
    }
    "RANGE REGRESSION PASS: $root"
} finally { Pop-Location }
