param([string]$BaselineCommit = "66fb51bfcb2f91f11b39ac8b4593d87d8717ad48")

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$proofRoot = Join-Path $repoRoot "tmp\native_v12.2.0\frozen-v12.1-compat"
$baselineSource = Join-Path $proofRoot "baseline-source"
$baselineOut = Join-Path $proofRoot "baseline"
$candidateOut = Join-Path $proofRoot "candidate"
$archive = Join-Path $proofRoot "baseline.tar"
$runtime = Join-Path $repoRoot "runtime\v12.1.0"

function Assert-True($value, [string]$message) { if (-not $value) { throw $message } }
function Invoke-Checked([string]$file, [string[]]$arguments, [string]$description, [string]$working = $repoRoot) {
    $saved = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    try { Push-Location $working; try { $output = & $file @arguments 2>&1; $code = $LASTEXITCODE } finally { Pop-Location } }
    finally { $ErrorActionPreference = $saved }
    if ($code -ne 0) { throw "$description failed ($code):`n$($output -join "`n")" }
}
function Build-LegacySet([string]$compiler, [string]$root) {
    New-Item -ItemType Directory -Path $root -Force | Out-Null
    foreach ($module in @("arch_x86", "serial", "alloc", "array", "string", "kernel")) {
        Invoke-Checked $compiler @("build", (Join-Path $runtime "$module.dby"), "--target", "i686-dbyte-none", "--emit", "obj", "-o", (Join-Path $root "$module.o")) "build frozen-compatible $module"
    }
    foreach ($emit in @("elf", "bin")) {
        Invoke-Checked $compiler @("build", (Join-Path $runtime "kernel.dby"), "--target", "i686-dbyte-none", "--emit", $emit, "-o", (Join-Path $root "runtime.$emit")) "build frozen-compatible runtime $emit"
    }
}

if (Test-Path $proofRoot) { Remove-Item -LiteralPath $proofRoot -Recurse -Force }
New-Item -ItemType Directory -Path $baselineSource,$baselineOut,$candidateOut -Force | Out-Null
Assert-True ((git rev-parse "$BaselineCommit^{commit}").Trim() -eq $BaselineCommit) "frozen v12.1 commit is unavailable"
Invoke-Checked "git.exe" @("archive", "--format=tar", "-o", $archive, $BaselineCommit) "archive frozen v12.1 compiler"
Invoke-Checked "tar.exe" @("-xf", $archive, "-C", $baselineSource) "extract frozen v12.1 compiler"
Invoke-Checked "cargo.exe" @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build frozen v12.1 compiler" $baselineSource
Invoke-Checked "cargo.exe" @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build candidate compiler"
Build-LegacySet (Join-Path $baselineSource "target\debug\dbytec.exe") $baselineOut
Build-LegacySet (Join-Path $repoRoot "target\debug\dbytec.exe") $candidateOut
$hashes = foreach ($name in @("arch_x86.o", "serial.o", "alloc.o", "array.o", "string.o", "kernel.o", "runtime.elf", "runtime.bin")) {
    $baselineHash = (Get-FileHash (Join-Path $baselineOut $name) -Algorithm SHA256).Hash
    $candidateHash = (Get-FileHash (Join-Path $candidateOut $name) -Algorithm SHA256).Hash
    Assert-True ($baselineHash -ceq $candidateHash) "frozen v12.1 artifact drift: $name"
    "$baselineHash  $name"
}
[IO.File]::WriteAllLines((Join-Path $proofRoot "SHA256SUMS"), $hashes)
"DBYTE V12.1 FROZEN COMPILER COMPAT: PASS"; $hashes
