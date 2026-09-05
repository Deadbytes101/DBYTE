param([string]$MilestoneVersion = "12.2.0")

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$proofRoot = Join-Path $repoRoot "tmp\native_v$MilestoneVersion\none-verifier"
$source = Join-Path $repoRoot "examples\native_v12_2_none\runtime.dby"
$compiler = Join-Path $repoRoot "target\debug\dbytec.exe"
$qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe"
$expected = "DBYTE X86_64 NONE`r`nELF64 ONLINE`r`nDBYTE64 ABI ONLINE`r`nALLOCATOR ONLINE`r`nDROP ONLINE`r`nARRAY ONLINE`r`nSTRING ONLINE`r`nNONE CHECK OK`r`n"
$ownedQemu = [System.Collections.Generic.List[int]]::new()

function Assert-True($value, [string]$message) { if (-not $value) { throw $message } }
function Invoke-Checked([string]$file, [string[]]$arguments, [string]$description) {
    $saved = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    try { $output = & $file @arguments 2>&1; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    if ($code -ne 0) { throw "$description failed ($code):`n$($output -join "`n")" }
    return ($output -join "`n")
}
function Assert-Same([string]$left, [string]$right, [string]$name) {
    $a = (Get-FileHash -LiteralPath $left -Algorithm SHA256).Hash
    $b = (Get-FileHash -LiteralPath $right -Algorithm SHA256).Hash
    Assert-True ($a -ceq $b) "$name is not deterministic"
    return $a
}
function Read-SharedText([string]$path) {
    $stream = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
    try {
        $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::ASCII, $false, 1024, $true)
        try { return $reader.ReadToEnd() } finally { $reader.Dispose() }
    } finally { $stream.Dispose() }
}
function Wsl-Path([string]$path) {
    $portable = $path.Replace('\', '/')
    return (Invoke-Checked "wsl.exe" @("-d", "Debian", "--", "wslpath", "-a", "-u", $portable) "translate WSL path").Trim()
}
function Build-Set([string]$root) {
    New-Item -ItemType Directory -Path $root -Force | Out-Null
    Invoke-Checked $compiler @("build", $source, "--target", "x86_64-dbyte-none", "--emit", "obj", "-o", (Join-Path $root "runtime.o"), "--dump-ir", "--dump-machine-code") "build none object" | Set-Content (Join-Path $root "build.txt")
    Invoke-Checked $compiler @("build", $source, "--target", "x86_64-dbyte-none", "--emit", "staticlib", "-o", (Join-Path $root "runtime.a")) "build none archive" | Out-Null
    Invoke-Checked $compiler @("build", $source, "--target", "x86_64-dbyte-none", "--emit", "elf", "-o", (Join-Path $root "runtime.elf")) "build none ELF" | Out-Null
    Invoke-Checked $compiler @("build", $source, "--target", "x86_64-dbyte-none", "--emit", "bin", "-o", (Join-Path $root "runtime.bin")) "build none BIN" | Out-Null
    Invoke-Checked $compiler @("link", "--target", "x86_64-dbyte-none", (Join-Path $root "runtime.o"), "-o", (Join-Path $root "linked.elf")) "link none object" | Out-Null
    Invoke-Checked $compiler @("link", "--target", "x86_64-dbyte-none", (Join-Path $root "runtime.a"), "-o", (Join-Path $root "archive-linked.elf")) "link none archive" | Out-Null
}
function Run-Qemu([int]$ram, [string]$elf) {
    $runRoot = Join-Path $proofRoot "${ram}m"
    $isoRoot = Join-Path $runRoot "iso"
    New-Item -ItemType Directory -Path (Join-Path $isoRoot "boot\grub") -Force | Out-Null
    Copy-Item -LiteralPath $elf -Destination (Join-Path $isoRoot "boot\kernel.elf") -Force
    [IO.File]::WriteAllText((Join-Path $isoRoot "boot\grub\grub.cfg"), "set timeout=0`nset default=0`nmenuentry 'DByte v12.2' {`n multiboot2 /boot/kernel.elf`n boot`n}`n")
    $iso = Join-Path $runRoot "runtime.iso"
    Invoke-Checked "wsl.exe" @("-d", "Debian", "--", "grub-mkrescue", "-o", (Wsl-Path $iso), (Wsl-Path $isoRoot)) "create verifier GRUB ISO" | Set-Content (Join-Path $runRoot "grub-mkrescue.txt")
    $serial = Join-Path $runRoot "serial.log"; $trace = Join-Path $runRoot "qemu.log"
    $arguments = @("-cdrom", $iso, "-m", "${ram}M", "-display", "none", "-monitor", "none", "-serial", "file:$serial", "-d", "int,cpu_reset,guest_errors", "-D", $trace, "-no-reboot", "-no-shutdown")
    [IO.File]::WriteAllText((Join-Path $runRoot "qemu-command.txt"), "$qemu $($arguments -join ' ')")
    $process = Start-Process -FilePath $qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    $ownedQemu.Add($process.Id)
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(15); $complete = $false
        do {
            Start-Sleep -Milliseconds 100
            if (Test-Path $serial) {
                $actual = Read-SharedText $serial
                Assert-True ($expected.StartsWith($actual, [StringComparison]::Ordinal)) "${ram}M serial diverged"
                if ($actual -ceq $expected) { $complete = $true; break }
            }
        } while (-not $process.HasExited -and [DateTime]::UtcNow -lt $deadline)
        Assert-True $complete "${ram}M none proof timed out"
    } finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
    }
    $traceText = [IO.File]::ReadAllText($trace)
    Assert-True (-not $traceText.Contains("Triple fault")) "${ram}M triple fault"
    Assert-True (([regex]::Matches($traceText, 'v=0[0de6]')).Count -eq 0) "${ram}M unexpected CPU fault"
}

Push-Location $repoRoot
try {
    if (Test-Path $proofRoot) { Remove-Item -LiteralPath $proofRoot -Recurse -Force }
    New-Item -ItemType Directory -Path $proofRoot -Force | Out-Null
    Invoke-Checked "cargo.exe" @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build dbytec" | Out-Null
    Build-Set (Join-Path $proofRoot "first"); Build-Set (Join-Path $proofRoot "second")
    $hashes = foreach ($name in @("runtime.o", "runtime.a", "runtime.elf", "runtime.bin", "linked.elf", "archive-linked.elf")) { "$(Assert-Same (Join-Path $proofRoot "first\$name") (Join-Path $proofRoot "second\$name") $name)  $name" }
    Assert-True ((Get-FileHash (Join-Path $proofRoot "first\runtime.elf") -Algorithm SHA256).Hash -ceq (Get-FileHash (Join-Path $proofRoot "first\runtime.bin") -Algorithm SHA256).Hash) "ELF/BIN load-image contract drifted"
    Assert-True ((Get-FileHash (Join-Path $proofRoot "first\runtime.elf") -Algorithm SHA256).Hash -ceq (Get-FileHash (Join-Path $proofRoot "first\archive-linked.elf") -Algorithm SHA256).Hash) "none archive extraction changed the final image"
    $objectDump = Invoke-Checked "objdump.exe" @("-f", "-h", "-t", "-r", "-d", "-Mintel", (Join-Path $proofRoot "first\runtime.o")) "inspect none object"
    Assert-True ($objectDump.Contains(".dbyte.target")) "none object lacks target metadata"
    Assert-True ($objectDump.Contains("R_X86_64_")) "none object lacks real x86_64 relocations"
    [IO.File]::WriteAllText((Join-Path $proofRoot "object-objdump.txt"), $objectDump)
    $elfWsl = Wsl-Path (Join-Path $proofRoot "first\runtime.elf")
    Invoke-Checked "wsl.exe" @("-d", "Debian", "--", "grub-file", "--is-x86-multiboot2", $elfWsl) "validate Multiboot2" | Out-Null
    $readelf = Invoke-Checked "wsl.exe" @("-d", "Debian", "--", "readelf", "-h", "-l", "-r", $elfWsl) "inspect none ELF"
    Assert-True ($readelf.Contains("Machine:")) "readelf omitted ELF machine"
    Assert-True (-not $readelf.Contains("INTERP")) "none ELF has an interpreter"
    [IO.File]::WriteAllText((Join-Path $proofRoot "readelf.txt"), $readelf)
    Run-Qemu 32 (Join-Path $proofRoot "first\runtime.elf"); Run-Qemu 64 (Join-Path $proofRoot "first\runtime.elf")
    [IO.File]::WriteAllLines((Join-Path $proofRoot "SHA256SUMS"), $hashes)
    "DBYTE MODERN NATIVE NONE VERIFIER: PASS"; $hashes
} finally {
    Pop-Location
    $reapDeadline = [DateTime]::UtcNow.AddSeconds(2)
    do {
        $orphans = @($ownedQemu | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue })
        if ($orphans.Count -eq 0 -or [DateTime]::UtcNow -ge $reapDeadline) { break }
        Start-Sleep -Milliseconds 100
    } while ($true)
    if ($orphans.Count -ne 0) { throw "none verifier left owned QEMU processes: $($orphans -join ', ')" }
}
