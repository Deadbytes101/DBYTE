param([string]$BaselineCommit = "d6564f6f4b87bb0e493c4c4359970c66645b962d")

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$tmpRoot = (Resolve-Path (Join-Path $repoRoot "tmp")).Path
$proofRoot = Join-Path $tmpRoot "native_v12.0.0\frozen-v11-compat"
$baselineRoot = Join-Path $proofRoot "baseline-source"
$currentRoot = Join-Path $proofRoot "current"
$baselineOut = Join-Path $proofRoot "baseline"
$archive = Join-Path $proofRoot "baseline.tar"
$currentCompiler = Join-Path $repoRoot "target\debug\dbytec.exe"
$proofDirectories = @(
    "native_kernel", "native_kernel_multimodule", "native_kernel_core", "native_memory_core",
    "native_runtime_core", "native_scheduler_core", "native_blocking_core", "native_irq_io_core",
    "native_block_device_core", "native_filesystem_core", "native_filesystem_runtime",
    "native_user_process_core"
)

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
Reset-ProofDirectory $baselineRoot
Reset-ProofDirectory $currentRoot
Reset-ProofDirectory $baselineOut
Assert-ProofPath $archive
if (Test-Path -LiteralPath $archive) { Remove-Item -LiteralPath $archive -Force }

Assert-True ((git rev-parse "$BaselineCommit^{commit}").Trim() -eq $BaselineCommit) "frozen v11.11 commit is unavailable"
Invoke-Checked "git" @("archive", "--format=tar", "-o", $archive, $BaselineCommit) "archive frozen compiler" | Out-Null
Invoke-Checked "tar.exe" @("-xf", $archive, "-C", $baselineRoot) "extract frozen compiler" | Out-Null
Invoke-Checked "cargo.exe" @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build frozen compiler" $baselineRoot | Out-Null
Invoke-Checked "cargo.exe" @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build candidate compiler" $repoRoot | Out-Null
$baselineCompiler = Join-Path $baselineRoot "target\debug\dbytec.exe"

$sources = foreach ($directory in $proofDirectories) {
    Get-ChildItem -LiteralPath (Join-Path $repoRoot "examples\$directory") -Filter "*.dby" -File -Recurse
}
$sources = @($sources | Sort-Object FullName -Unique)
$hashLines = @()
$mismatches = @()
foreach ($source in $sources) {
    $relative = $source.FullName.Substring($repoRoot.Length).TrimStart('\')
    $safe = ($relative -replace '[\\/:]', '_')
    $target = if ($relative -like '*native_user_process_core\programs\*') { "i686-dbyte-user" } else { "i686-dbyte-none" }
    $emit = if ($relative -eq 'examples\native_kernel\kernel.dby') { 'elf' } else { 'obj' }
    $extension = if ($emit -eq 'elf') { 'elf' } else { 'o' }
    $candidateObject = Join-Path $currentRoot "$safe.$extension"
    $baselineObject = Join-Path $baselineOut "$safe.$extension"
    Invoke-Checked $currentCompiler @("build", $source.FullName, "--target", $target, "--emit", $emit, "-o", $candidateObject) "candidate artifact $relative" | Out-Null
    Invoke-Checked $baselineCompiler @("build", $source.FullName, "--target", $target, "--emit", $emit, "-o", $baselineObject) "baseline artifact $relative" | Out-Null
    $candidateHash = (Get-FileHash -LiteralPath $candidateObject -Algorithm SHA256).Hash
    $baselineHash = (Get-FileHash -LiteralPath $baselineObject -Algorithm SHA256).Hash
    if ($candidateHash -ne $baselineHash) {
        $mismatches += "$relative candidate=$candidateHash baseline=$baselineHash"
    }
    $hashLines += "$relative candidate=$candidateHash baseline=$baselineHash"
}
[IO.File]::WriteAllLines((Join-Path $proofRoot "sha256.txt"), $hashLines)
Assert-True ($mismatches.Count -eq 0) "frozen artifact drift ($($mismatches.Count)):`n$($mismatches -join "`n")"
Write-Host "DBYTE V11 FROZEN COMPILER COMPAT: PASS"
Write-Host "Objects compared: $($sources.Count)"
