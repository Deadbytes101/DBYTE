param([string]$BaselineCommit = "97994da26fa05af729f179dfc8fa2861330249fc")

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$tmpRoot = (Resolve-Path (Join-Path $repoRoot "tmp")).Path
$proofRoot = Join-Path $tmpRoot "native_v12.1.0\frozen-v12-compat"
$baselineSource = Join-Path $proofRoot "baseline-source"
$baselineOut = Join-Path $proofRoot "baseline"
$candidateOut = Join-Path $proofRoot "candidate"
$archive = Join-Path $proofRoot "baseline.tar"

function Assert-True($condition, [string]$message) {
    if (-not $condition) { throw $message }
}

function Assert-ProofPath([string]$path) {
    $full = [IO.Path]::GetFullPath($path)
    $allowed = [IO.Path]::GetFullPath($proofRoot).TrimEnd('\') + '\'
    Assert-True ($full.StartsWith($allowed, [StringComparison]::OrdinalIgnoreCase) -or $full -eq $proofRoot) "compatibility path escapes proof root: $full"
}

function Reset-ProofDirectory([string]$path) {
    Assert-ProofPath $path
    if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Recurse -Force }
    New-Item -ItemType Directory -Path $path -Force | Out-Null
}

function Invoke-Checked([string]$executable, [string[]]$arguments, [string]$description, [string]$workingDirectory = $repoRoot) {
    $saved = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        Push-Location $workingDirectory
        try { $output = & $executable @arguments 2>&1; $code = $LASTEXITCODE }
        finally { Pop-Location }
    } finally { $ErrorActionPreference = $saved }
    if ($code -ne 0) { throw "$description failed (${code}):`n$($output -join "`n")" }
    return ($output -join "`n")
}

New-Item -ItemType Directory -Path $proofRoot -Force | Out-Null
Reset-ProofDirectory $baselineSource
Reset-ProofDirectory $baselineOut
Reset-ProofDirectory $candidateOut
Assert-ProofPath $archive
if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }

Assert-True ((git rev-parse "$BaselineCommit^{commit}").Trim() -eq $BaselineCommit) "frozen v12.0.0 commit is unavailable"
Invoke-Checked "git" @("archive", "--format=tar", "-o", $archive, $BaselineCommit) "archive frozen v12 compiler" | Out-Null
Invoke-Checked "tar.exe" @("-xf", $archive, "-C", $baselineSource) "extract frozen v12 compiler" | Out-Null
Invoke-Checked "cargo.exe" @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build frozen v12 compiler" $baselineSource | Out-Null
Invoke-Checked "cargo.exe" @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build candidate compiler" $repoRoot | Out-Null

$baselineCompiler = Join-Path $baselineSource "target\debug\dbytec.exe"
$candidateCompiler = Join-Path $repoRoot "target\debug\dbytec.exe"
$sources = @(
    @{ Path = "examples\native_v12_hir\arch_x86.dby"; Target = "i686-dbyte-none" },
    @{ Path = "examples\native_v12_hir\serial.dby"; Target = "i686-dbyte-none" },
    @{ Path = "examples\native_v12_hir\kernel.dby"; Target = "i686-dbyte-none" },
    @{ Path = "examples\native_v12_hir\user.dby"; Target = "i686-dbyte-user" }
)
$hashLines = @()
$mismatches = @()
foreach ($source in $sources) {
    $name = [IO.Path]::GetFileNameWithoutExtension($source.Path)
    $baselineObject = Join-Path $baselineOut "$name.o"
    $candidateObject = Join-Path $candidateOut "$name.o"
    $absoluteSource = Join-Path $repoRoot $source.Path
    Invoke-Checked $baselineCompiler @("build", $absoluteSource, "--target", $source.Target, "--emit", "obj", "-o", $baselineObject) "frozen v12 object $($source.Path)" | Out-Null
    Invoke-Checked $candidateCompiler @("build", $absoluteSource, "--target", $source.Target, "--emit", "obj", "-o", $candidateObject) "candidate object $($source.Path)" | Out-Null
    $baselineHash = (Get-FileHash -LiteralPath $baselineObject -Algorithm SHA256).Hash
    $candidateHash = (Get-FileHash -LiteralPath $candidateObject -Algorithm SHA256).Hash
    $hashLines += "$($source.Path) baseline=$baselineHash candidate=$candidateHash"
    if ($baselineHash -cne $candidateHash) { $mismatches += $hashLines[-1] }
}
[IO.File]::WriteAllLines((Join-Path $proofRoot "sha256.txt"), $hashLines)
Assert-True ($mismatches.Count -eq 0) "frozen v12.0 artifact drift ($($mismatches.Count)):`n$($mismatches -join "`n")"
Write-Host "DBYTE V12 FROZEN COMPILER COMPAT: PASS"
Write-Host "Objects compared: $($sources.Count)"
