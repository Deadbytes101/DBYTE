param([string]$BaselineCommit = "74e4eaca8d8e1086770e8804c3707d301996afeb")

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$proofRoot = Join-Path $repoRoot "tmp\windows_package_correction_v12.2.1\frozen-v12.2-compat"
$baselineSource = Join-Path $proofRoot "baseline-source"
$baselineOutFirst = Join-Path $proofRoot "baseline-first"
$baselineOutSecond = Join-Path $proofRoot "baseline-second"
$candidateOut = Join-Path $proofRoot "candidate"
$archive = Join-Path $proofRoot "baseline.zip"

function Assert-True($value, [string]$message) { if (-not $value) { throw $message } }
function Invoke-Checked([string]$file, [string[]]$arguments, [string]$description, [string]$working = $repoRoot) {
    $saved = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    try { Push-Location $working; try { $output = & $file @arguments 2>&1; $code = $LASTEXITCODE } finally { Pop-Location } }
    finally { $ErrorActionPreference = $saved }
    if ($code -ne 0) { throw "$description failed ($code):`n$($output -join "`n")" }
}
function Build-Set([string]$compiler, [string]$sourceRoot, [string]$root) {
    New-Item -ItemType Directory -Path $root -Force | Out-Null
    $cases = @(
        @("i686-runtime.o", (Join-Path $sourceRoot "runtime\v12.1.0\kernel.dby"), "i686-dbyte-none", "obj"),
        @("i686-runtime.elf", (Join-Path $sourceRoot "runtime\v12.1.0\kernel.dby"), "i686-dbyte-none", "elf"),
        @("none.o", (Join-Path $sourceRoot "examples\native_v12_2_none\bootstrap.dby"), "x86_64-dbyte-none", "obj"),
        @("none.elf", (Join-Path $sourceRoot "examples\native_v12_2_none\bootstrap.dby"), "x86_64-dbyte-none", "elf"),
        @("linux.o", (Join-Path $sourceRoot "examples\native_v12_2_linux\exit42.dby"), "x86_64-dbyte-linux", "obj"),
        @("linux.elf", (Join-Path $sourceRoot "examples\native_v12_2_linux\exit42.dby"), "x86_64-dbyte-linux", "elf"),
        @("windows.o", (Join-Path $sourceRoot "examples\native_v12_2_windows\runtime_foundation.dby"), "x86_64-dbyte-windows", "obj"),
        @("windows.exe", (Join-Path $sourceRoot "examples\native_v12_2_windows\runtime_foundation.dby"), "x86_64-dbyte-windows", "exe")
    )
    foreach ($case in $cases) {
        Invoke-Checked $compiler @("build", $case[1], "--target", $case[2], "--emit", $case[3], "-o", (Join-Path $root $case[0])) "build frozen v12.2 artifact $($case[0])"
    }
}

if (Test-Path $proofRoot) { Remove-Item -LiteralPath $proofRoot -Recurse -Force }
New-Item -ItemType Directory -Path $baselineSource,$baselineOutFirst,$baselineOutSecond,$candidateOut -Force | Out-Null
Assert-True ((git rev-parse "$BaselineCommit^{commit}").Trim() -eq $BaselineCommit) "frozen v12.2 commit is unavailable"
& git.exe -c core.autocrlf=false archive --format=zip -o $archive $BaselineCommit
if ($LASTEXITCODE -ne 0) { throw "archive frozen v12.2 compiler failed" }
Expand-Archive -LiteralPath $archive -DestinationPath $baselineSource
Invoke-Checked "cargo.exe" @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build frozen v12.2 compiler" $baselineSource
Invoke-Checked "cargo.exe" @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build candidate compiler"
$baselineCompiler = Join-Path $baselineSource "target\debug\dbytec.exe"
Build-Set $baselineCompiler $baselineSource $baselineOutFirst
Build-Set $baselineCompiler $baselineSource $baselineOutSecond
Build-Set (Join-Path $repoRoot "target\debug\dbytec.exe") $repoRoot $candidateOut
$baselineHashes = @(); $candidateHashes = @()
foreach ($name in @("i686-runtime.o", "i686-runtime.elf", "none.o", "none.elf", "linux.o", "linux.elf", "windows.o", "windows.exe")) {
    $firstHash = (Get-FileHash (Join-Path $baselineOutFirst $name) -Algorithm SHA256).Hash
    $secondHash = (Get-FileHash (Join-Path $baselineOutSecond $name) -Algorithm SHA256).Hash
    Assert-True ($firstHash -ceq $secondHash) "frozen v12.2 self-reproduction drift: $name"
    $baselineHashes += "$firstHash  $name"
    $candidateHash = (Get-FileHash (Join-Path $candidateOut $name) -Algorithm SHA256).Hash
    $candidateHashes += "$candidateHash  $name"
}
[IO.File]::WriteAllLines((Join-Path $proofRoot "BASELINE_SHA256SUMS"), $baselineHashes)
[IO.File]::WriteAllLines((Join-Path $proofRoot "CANDIDATE_SHA256SUMS"), $candidateHashes)
$runtimeResult = & (Join-Path $PSScriptRoot "verify_native_runtime_foundation.ps1") -MilestoneVersion "12.1.0"
$modernResult = & (Join-Path $PSScriptRoot "verify_modern_native_foundation.ps1") -MilestoneVersion "12.2.0"
Assert-True (($runtimeResult -join "`n").Contains("VERIFIER: PASS")) "candidate failed historical v12.1 runtime semantics"
Assert-True (($modernResult -join "`n").Contains("VERIFIER: PASS")) "candidate failed historical v12.2 native semantics"
[IO.File]::WriteAllText((Join-Path $proofRoot "semantic-compat.txt"), (($runtimeResult + $modernResult) -join "`n"))
"DBYTE V12.2 FROZEN SELF-REPRO + CANDIDATE SEMANTIC COMPAT: PASS"
$baselineHashes
$candidateHashes
