param(
    [switch]$Quick,
    [string]$Case
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$EvidenceRoot = [IO.Path]::GetFullPath((Join-Path $RepoRoot "tmp\vm_v12.3.0\performance"))
$AllowedRoot = [IO.Path]::GetFullPath((Join-Path $RepoRoot "tmp\vm_v12.3.0"))
$BaselineCommit = "3843f7b3cf29951f10fe732754434b41ac53fc7f"
$ManifestPath = Join-Path $RepoRoot "benchmarks\vm\corpus.json"

& (Join-Path $PSScriptRoot "verify_vm_architecture.ps1") -SkipTests
if ($LASTEXITCODE -ne 0) { throw "VM_PERF_ARCHITECTURE_GATE_FAILED" }

function Assert-UnderAllowedRoot([string]$Path) {
    $resolved = [IO.Path]::GetFullPath($Path)
    $prefix = $AllowedRoot.TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "VM_PERF_PATH_OUTSIDE_EVIDENCE_ROOT: $resolved"
    }
}

function Reset-EvidenceDirectory([string]$Path) {
    Assert-UnderAllowedRoot $Path
    if (Test-Path -LiteralPath $Path) {
        Remove-Item -Recurse -Force -LiteralPath $Path
    }
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
}

function Invoke-Checked([string]$Command, [string[]]$Arguments, [string]$WorkingDirectory) {
    Push-Location $WorkingDirectory
    try {
        & $Command @Arguments
        if ($LASTEXITCODE -ne 0) {
            throw "command failed ($LASTEXITCODE): $Command $($Arguments -join ' ')"
        }
    }
    finally {
        Pop-Location
    }
}

function Invoke-Receipt(
    [string]$Executable,
    [string]$Source,
    [int]$Warmups,
    [int]$Measurements,
    [int]$Iterations
) {
    $output = & $Executable $Source --warmups $Warmups --measurements $Measurements --iterations $Iterations
    if ($LASTEXITCODE -ne 0) {
        throw "benchmark runner failed ($LASTEXITCODE): $Executable $Source"
    }
    return ($output | Out-String | ConvertFrom-Json)
}

function Invoke-StablePair($Case, $Manifest, [string]$BaselineExe, [string]$CandidateExe) {
    $measurements = if ($Quick) { 3 } else { [int]$Manifest.measurements }
    $warmups = if ($Quick) { 1 } else { [int]$Manifest.warmups }
    $attemptLimit = if ($Quick) { 1 } else { [int]$Manifest.maximum_attempts }
    $source = [IO.Path]::GetFullPath((Join-Path $RepoRoot ([string]$Case.source)))
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "VM_PERF_SOURCE_MISSING: $source"
    }
    for ($attempt = 1; $attempt -le $attemptLimit; $attempt++) {
        $baseline = Invoke-Receipt $BaselineExe $source $warmups $measurements ([int]$Case.iterations)
        $candidate = Invoke-Receipt $CandidateExe $source $warmups $measurements ([int]$Case.iterations)
        if ($baseline.source_sha256 -ne $candidate.source_sha256) {
            throw "VM_PERF_SOURCE_DIVERGENCE: $($Case.name)"
        }
        if ($baseline.output_sha256 -ne $candidate.output_sha256) {
            throw "VM_PERF_OUTPUT_DIVERGENCE: $($Case.name)"
        }
        $stable = $Quick -or ($baseline.relative_mad -le $Manifest.maximum_relative_mad -and
            $candidate.relative_mad -le $Manifest.maximum_relative_mad)
        if ($stable) {
            return [ordered]@{
                name = [string]$Case.name
                attempt = $attempt
                iterations = [int]$Case.iterations
                baseline = $baseline
                candidate = $candidate
                speedup = [double]$baseline.median_ns / [double]$candidate.median_ns
            }
        }
    }
    throw "VM_PERF_UNSTABLE: $($Case.name) remained above relative MAD $($Manifest.maximum_relative_mad)"
}

if ((git -C $RepoRoot rev-parse "v12.2.1^{}") -ne $BaselineCommit) {
    throw "VM_PERF_BASELINE_TAG_MISMATCH"
}
$manifest = Get-Content -Raw -LiteralPath $ManifestPath | ConvertFrom-Json
if ($manifest.baseline_commit -ne $BaselineCommit) {
    throw "VM_PERF_MANIFEST_BASELINE_MISMATCH"
}

Reset-EvidenceDirectory $EvidenceRoot
$baselineRoot = Join-Path $EvidenceRoot "baseline"
$baselineSource = Join-Path $baselineRoot "source"
New-Item -ItemType Directory -Force -Path $baselineSource | Out-Null
$archive = Join-Path $baselineRoot "v12.2.1.tar"
Invoke-Checked "git" @("archive", "--format=tar", "--output=$archive", $BaselineCommit) $RepoRoot
Invoke-Checked "tar" @("-xf", $archive, "-C", $baselineSource) $RepoRoot

$runnerSource = Join-Path $RepoRoot "tools\vm_benchmark_runner"
$runnerTarget = Join-Path $baselineSource "tools\vm_benchmark_runner"
New-Item -ItemType Directory -Force -Path (Split-Path $runnerTarget -Parent) | Out-Null
Copy-Item -Recurse -LiteralPath $runnerSource -Destination $runnerTarget
$baselineCargo = Join-Path $baselineSource "Cargo.toml"
$cargoText = Get-Content -Raw -LiteralPath $baselineCargo
$needle = '    "crates/dbyte_kernel_vm",'
if (-not $cargoText.Contains($needle)) {
    throw "VM_PERF_BASELINE_WORKSPACE_SHAPE_CHANGED"
}
$cargoText = $cargoText.Replace($needle, "$needle`r`n    `"tools/vm_benchmark_runner`",")
Set-Content -NoNewline -LiteralPath $baselineCargo -Value $cargoText

$baselineTarget = Join-Path $baselineRoot "target"
$candidateTarget = Join-Path $EvidenceRoot "candidate-target"
$env:CARGO_TARGET_DIR = $baselineTarget
Invoke-Checked "cargo" @("build", "--release", "--offline", "-p", "dbyte_vm_benchmark_runner", "--manifest-path", $baselineCargo) $baselineSource
$baselineExe = Join-Path $baselineTarget "release\dbyte_vm_benchmark_runner.exe"
$env:CARGO_TARGET_DIR = $candidateTarget
Invoke-Checked "cargo" @("build", "--release", "--locked", "-p", "dbyte_vm_benchmark_runner", "--features", "candidate-profile") $RepoRoot
$candidateExe = Join-Path $candidateTarget "release\dbyte_vm_benchmark_runner.exe"
Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue

$selectedCases = @($manifest.cases)
if ($Case) {
    $selectedCases = @($selectedCases | Where-Object { $_.name -eq $Case })
    if ($selectedCases.Count -ne 1) {
        throw "VM_PERF_UNKNOWN_CASE: $Case"
    }
}
$results = @()
foreach ($benchmarkCase in $selectedCases) {
    $result = Invoke-StablePair $benchmarkCase $manifest $baselineExe $candidateExe
    $results += $result
    Write-Host ("{0,-30} {1,7:N3}x" -f $result.name, $result.speedup)
}

$logSum = 0.0
foreach ($result in $results) {
    $logSum += [Math]::Log([double]$result.speedup)
    if ($result.speedup -lt (1.0 - [double]$manifest.maximum_workload_regression)) {
        throw "VM_PERF_WORKLOAD_REGRESSION: $($result.name) speedup=$($result.speedup)"
    }
}
$geometricMean = [Math]::Exp($logSum / $results.Count)
if (-not $Quick -and $geometricMean -lt [double]$manifest.minimum_geometric_mean_speedup) {
    throw "VM_PERF_GEOMEAN_BELOW_GATE: $geometricMean"
}

$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$receipt = [ordered]@{
    schema = 1
    baseline_commit = $BaselineCommit
    candidate_commit = (git -C $RepoRoot rev-parse HEAD)
    candidate_dirty = [bool](git -C $RepoRoot status --porcelain)
    manifest_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $ManifestPath).Hash.ToLowerInvariant()
    quick = [bool]$Quick
    machine = [ordered]@{
        computer = $env:COMPUTERNAME
        os = [Environment]::OSVersion.VersionString
        cpu = $cpu.Name
        logical_processors = $cpu.NumberOfLogicalProcessors
        rustc = ((rustc -Vv) -join "`n")
    }
    geometric_mean_speedup = $geometricMean
    cases = $results
}
$receiptPath = Join-Path $EvidenceRoot "core-receipt.json"
$receipt | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $receiptPath
Write-Host ("VM PERFORMANCE: PASS geometric mean {0:N3}x" -f $geometricMean)
Write-Host "receipt: $receiptPath"
