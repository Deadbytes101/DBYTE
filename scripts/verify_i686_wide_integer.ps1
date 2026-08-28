param([string]$MilestoneVersion = "12.2.0")

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$proofRoot = Join-Path $repoRoot "tmp\native_v$MilestoneVersion\i686-wide-verifier"
$source = Join-Path $repoRoot "examples\native_v12_2_i686_wide\proof.dby"
$compiler = Join-Path $repoRoot "target\debug\dbytec.exe"
$qemu = "C:\Program Files\qemu\qemu-system-i386.exe"
$expected = "WIDE64 OK`r`n"
$initialQemu = @(Get-Process qemu-system-i386 -ErrorAction SilentlyContinue | ForEach-Object Id)

function Assert-True($value, [string]$message) { if (-not $value) { throw $message } }
function Invoke-Checked([string]$file, [string[]]$arguments, [string]$description) {
    $saved = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    try { $output = & $file @arguments 2>&1; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    if ($code -ne 0) { throw "$description failed ($code):`n$($output -join "`n")" }
    return ($output -join "`n")
}
function Read-SharedText([string]$path) {
    $stream = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
    try { $reader = [IO.StreamReader]::new($stream); try { return $reader.ReadToEnd() } finally { $reader.Dispose() } }
    finally { $stream.Dispose() }
}
function Build-Set([string]$root) {
    New-Item -ItemType Directory -Path $root -Force | Out-Null
    foreach ($emit in @("obj", "elf", "bin")) {
        Invoke-Checked $compiler @("build", $source, "--target", "i686-dbyte-none", "--emit", $emit, "-o", (Join-Path $root "proof.$emit")) "build i686 wide $emit" | Out-Null
    }
}
function Run-Qemu([int]$ram, [string]$elf) {
    $root = Join-Path $proofRoot "${ram}m"; New-Item -ItemType Directory -Path $root -Force | Out-Null
    $serial = Join-Path $root "serial.log"; $trace = Join-Path $root "trace.log"
    $process = Start-Process $qemu -ArgumentList @("-kernel", $elf, "-m", "${ram}M", "-display", "none", "-monitor", "none", "-serial", "file:$serial", "-d", "int,cpu_reset,guest_errors", "-D", $trace, "-no-reboot", "-no-shutdown") -PassThru -WindowStyle Hidden
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(12); $complete = $false
        do { Start-Sleep -Milliseconds 100; if (Test-Path $serial) { $actual = Read-SharedText $serial; if ($actual -ceq $expected) { $complete = $true; break }; Assert-True ($expected.StartsWith($actual)) "i686 wide serial diverged" } } while (-not $process.HasExited -and [DateTime]::UtcNow -lt $deadline)
        Assert-True $complete "i686 wide proof timed out"
    } finally { if (-not $process.HasExited) { Stop-Process $process.Id -Force; $process.WaitForExit() } }
    $log = [IO.File]::ReadAllText($trace)
    Assert-True (-not $log.Contains("Triple fault")) "i686 wide proof triple-faulted"
    Assert-True (([regex]::Matches($log, 'v=0[0de6]')).Count -eq 0) "i686 wide proof faulted"
}

Push-Location $repoRoot
try {
    if (Test-Path $proofRoot) { Remove-Item $proofRoot -Recurse -Force }
    New-Item -ItemType Directory -Path $proofRoot -Force | Out-Null
    Invoke-Checked "cargo.exe" @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build dbytec" | Out-Null
    Build-Set (Join-Path $proofRoot "first"); Build-Set (Join-Path $proofRoot "second")
    $hashes = foreach ($name in @("proof.obj", "proof.elf", "proof.bin")) { $a=(Get-FileHash (Join-Path $proofRoot "first\$name") -Algorithm SHA256).Hash; $b=(Get-FileHash (Join-Path $proofRoot "second\$name") -Algorithm SHA256).Hash; Assert-True ($a -ceq $b) "$name is not deterministic"; "$a  $name" }
    $dump = Invoke-Checked "objdump.exe" @("-f", "-h", "-t", "-r", "-d", "-Mintel", (Join-Path $proofRoot "first\proof.elf")) "inspect i686 wide ELF"
    Assert-True ($dump.Contains("__dbyte_proof_signed_quotient")) "i686 signed wide path is absent"
    Assert-True ($dump.Contains("__dbyte_proof_unsigned_remainder")) "i686 unsigned wide path is absent"
    Assert-True ($dump -match '\b(idiv|div)\b') "i686 wide division instructions are absent"
    Assert-True ($dump -match '\bedx\b') "i686 wide EDX:EAX path is absent"
    [IO.File]::WriteAllText((Join-Path $proofRoot "objdump.txt"), $dump)
    Run-Qemu 32 (Join-Path $proofRoot "first\proof.elf"); Run-Qemu 64 (Join-Path $proofRoot "first\proof.elf")
    [IO.File]::WriteAllLines((Join-Path $proofRoot "SHA256SUMS"), $hashes)
    "DBYTE I686 WIDE INTEGER VERIFIER: PASS"; $hashes
} finally {
    Pop-Location
    $after = @(Get-Process qemu-system-i386 -ErrorAction SilentlyContinue | ForEach-Object Id)
    $orphans = @($after | Where-Object { $_ -notin $initialQemu })
    if ($orphans.Count -ne 0) { throw "wide verifier left QEMU processes: $($orphans -join ', ')" }
}
