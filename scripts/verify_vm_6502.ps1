param([switch]$Quick)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$BaselineCommit = "3843f7b3cf29951f10fe732754434b41ac53fc7f"
$AllowedRoot = [IO.Path]::GetFullPath((Join-Path $RepoRoot "tmp\vm_v12.3.0"))
$EvidenceRoot = [IO.Path]::GetFullPath((Join-Path $AllowedRoot "6502"))
$ArraySource = Join-Path $RepoRoot "benchmarks\vm\6502\array_driver.dby"
$BufferSource = Join-Path $RepoRoot "benchmarks\vm\6502\buffer_driver.dby"
$MatrixSource = Join-Path $RepoRoot "benchmarks\vm\6502\matrix.dby"
$TraceSource = Join-Path $RepoRoot "benchmarks\vm\6502\trace.dby"
$ManifestPath = Join-Path $RepoRoot "benchmarks\vm\corpus.json"
$MeasureScript = Join-Path $RepoRoot "scripts\vm_perf_pair.py"

& (Join-Path $PSScriptRoot "verify_vm_architecture.ps1") -SkipTests
if ($LASTEXITCODE -ne 0) { throw "VM_6502_ARCHITECTURE_GATE_FAILED" }

function Assert-UnderEvidenceRoot([string]$Path) {
    $resolved = [IO.Path]::GetFullPath($Path)
    if (-not $resolved.StartsWith($AllowedRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "VM_6502_PATH_OUTSIDE_EVIDENCE_ROOT: $resolved"
    }
}

function Invoke-Checked([string]$Command, [string[]]$Arguments, [string]$WorkingDirectory) {
    Push-Location $WorkingDirectory
    try {
        $output = & $Command @Arguments
        if ($LASTEXITCODE -ne 0) {
            throw "command failed ($LASTEXITCODE): $Command $($Arguments -join ' ')"
        }
        return ($output | Out-String).Replace("`r`n", "`n")
    }
    finally {
        Pop-Location
    }
}

function Invoke-Receipt([string]$Executable, [string]$Source, [int]$Warmups, [int]$Measurements) {
    $output = & $Executable $Source --warmups $Warmups --measurements $Measurements --iterations 1
    if ($LASTEXITCODE -ne 0) {
        throw "VM_6502_RUNNER_FAILED: $Executable $Source"
    }
    return ($output | Out-String | ConvertFrom-Json)
}

function Get-CorpusHash {
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $files = @(
            "benchmarks\vm\6502\opcodes.csv",
            "benchmarks\vm\6502\core.dby",
            "benchmarks\vm\6502\workload.dby",
            "benchmarks\vm\6502\array_driver.dby"
        )
        $stream = [IO.MemoryStream]::new()
        try {
            foreach ($relative in $files) {
                $name = [Text.Encoding]::UTF8.GetBytes($relative.Replace('\', '/') + "`n")
                $bytes = [IO.File]::ReadAllBytes((Join-Path $RepoRoot $relative))
                $stream.Write($name, 0, $name.Length)
                $stream.Write($bytes, 0, $bytes.Length)
            }
            return (($sha.ComputeHash($stream.ToArray()) | ForEach-Object { $_.ToString("x2") }) -join "")
        }
        finally { $stream.Dispose() }
    }
    finally { $sha.Dispose() }
}

if ((git -C $RepoRoot rev-parse "v12.2.1^{}") -ne $BaselineCommit) {
    throw "VM_6502_BASELINE_TAG_MISMATCH"
}
$manifest = Get-Content -Raw -LiteralPath $ManifestPath | ConvertFrom-Json
if ([int]$manifest.windows_affinity_cpu -lt 0) { throw "VM_6502_AFFINITY_CPU_INVALID" }
if (-not (Test-Path -LiteralPath $MeasureScript -PathType Leaf)) { throw "VM_6502_MEASURE_HELPER_MISSING" }
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
if ([int]$manifest.windows_affinity_cpu -ge [int]$cpu.NumberOfLogicalProcessors) {
    throw "VM_6502_AFFINITY_CPU_UNAVAILABLE: $($manifest.windows_affinity_cpu)"
}

$rows = @(Import-Csv -LiteralPath (Join-Path $RepoRoot "benchmarks\vm\6502\opcodes.csv"))
if ($rows.Count -ne 151) { throw "VM_6502_OPCODE_COUNT: $($rows.Count)" }
$unique = @($rows.opcode | Sort-Object -Unique)
if ($unique.Count -ne 151) { throw "VM_6502_OPCODE_DUPLICATE" }
$corpusHash = Get-CorpusHash

$matrixTree = Invoke-Checked "cargo" @("run", "-q", "-p", "dbyte_cli", "--", "run", $MatrixSource) $RepoRoot
$matrixVm = Invoke-Checked "cargo" @("run", "-q", "-p", "dbyte_cli", "--", "run", "--vm", $MatrixSource) $RepoRoot
$matrixExpected = "151`n151`n0`n"
if ($matrixTree -ne $matrixExpected -or $matrixVm -ne $matrixExpected) {
    throw "VM_6502_MATRIX_DIVERGENCE: tree=[$matrixTree] vm=[$matrixVm]"
}
$traceTree = Invoke-Checked "cargo" @("run", "-q", "-p", "dbyte_cli", "--", "run", $TraceSource) $RepoRoot
$traceVm = Invoke-Checked "cargo" @("run", "-q", "-p", "dbyte_cli", "--", "run", "--vm", $TraceSource) $RepoRoot
$traceExpected = "true`ntrue`ntrue`ntrue`ntrue`ntrue`n1`n"
if ($traceTree -ne $traceExpected -or $traceVm -ne $traceExpected) {
    throw "VM_6502_TRACE_DIVERGENCE"
}
Invoke-Checked "cargo" @("test", "-q", "-p", "dbyte_vm", "nmos_6502_matrix_and_trace_match_tree_contract_at_o0_and_o1") $RepoRoot | Out-Null

Assert-UnderEvidenceRoot $EvidenceRoot
if (Test-Path -LiteralPath $EvidenceRoot) { Remove-Item -Recurse -Force -LiteralPath $EvidenceRoot }
New-Item -ItemType Directory -Force -Path $EvidenceRoot | Out-Null
$baselineRoot = Join-Path $EvidenceRoot "baseline"
$baselineSource = Join-Path $baselineRoot "source"
New-Item -ItemType Directory -Force -Path $baselineSource | Out-Null
$archive = Join-Path $baselineRoot "v12.2.1.tar"
Invoke-Checked "git" @("archive", "--format=tar", "--output=$archive", $BaselineCommit) $RepoRoot | Out-Null
Invoke-Checked "tar" @("-xf", $archive, "-C", $baselineSource) $RepoRoot | Out-Null

$runnerSource = Join-Path $RepoRoot "tools\vm_benchmark_runner"
$runnerTarget = Join-Path $baselineSource "tools\vm_benchmark_runner"
New-Item -ItemType Directory -Force -Path (Split-Path $runnerTarget -Parent) | Out-Null
Copy-Item -Recurse -LiteralPath $runnerSource -Destination $runnerTarget
$baselineCargo = Join-Path $baselineSource "Cargo.toml"
$cargoText = Get-Content -Raw -LiteralPath $baselineCargo
$needle = '    "crates/dbyte_kernel_vm",'
if (-not $cargoText.Contains($needle)) { throw "VM_6502_BASELINE_WORKSPACE_SHAPE_CHANGED" }
$cargoText = $cargoText.Replace($needle, "$needle`r`n    `"tools/vm_benchmark_runner`",")
Set-Content -NoNewline -LiteralPath $baselineCargo -Value $cargoText

$baselineTarget = Join-Path $baselineRoot "target"
$candidateTarget = Join-Path $EvidenceRoot "candidate-target"
$env:CARGO_TARGET_DIR = $baselineTarget
Invoke-Checked "cargo" @("build", "--release", "--offline", "-p", "dbyte_vm_benchmark_runner", "--manifest-path", $baselineCargo) $baselineSource | Out-Null
$baselineExe = Join-Path $baselineTarget "release\dbyte_vm_benchmark_runner.exe"
$env:CARGO_TARGET_DIR = $candidateTarget
Invoke-Checked "cargo" @("build", "--release", "--locked", "-p", "dbyte_vm_benchmark_runner", "--features", "candidate-profile") $RepoRoot | Out-Null
$candidateExe = Join-Path $candidateTarget "release\dbyte_vm_benchmark_runner.exe"
Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue

$warmups = if ($Quick) { 1 } else { 3 }
$measurements = if ($Quick) { 3 } else { 9 }
$attemptLimit = if ($Quick) { 1 } else { 3 }
$maximumMad = if ($Quick) { 1.0 } else { 0.05 }
$arguments = @(
    $MeasureScript, $baselineExe, $candidateExe, $ArraySource,
    "--iterations", "1", "--warmups", "$warmups",
    "--measurements", "$measurements", "--attempts", "$attemptLimit",
    "--maximum-relative-mad", "$maximumMad",
    "--cpu-affinity", "$([int]$manifest.windows_affinity_cpu)"
)
$pairResult = ((Invoke-Checked "python.exe" $arguments $RepoRoot) | Out-String).Trim() | ConvertFrom-Json
if (-not $pairResult.stable) { throw "VM_6502_UNSTABLE" }
$selected = $pairResult.attempts[[int]$pairResult.selected_attempt - 1]
$pair = [ordered]@{
    attempt = [int]$pairResult.selected_attempt
    baseline = $selected.baseline
    candidate = $selected.candidate
    speedup = [double]$selected.ratio
    cpu_affinity = [int]$pairResult.cpu_affinity
    priority_class = [string]$pairResult.priority_class
    launch_control = [string]$pairResult.launch_control
    sample_order = [string]$pairResult.sample_order
}
if (-not $Quick -and $pair.speedup -lt 1.20) { throw "VM_6502_SPEEDUP_BELOW_GATE: $($pair.speedup)" }

$buffer = Invoke-Receipt $candidateExe $BufferSource 0 1
if ($buffer.output_sha256 -ne $pair.candidate.output_sha256) {
    throw "VM_6502_ARRAY_BUFFER_DIVERGENCE"
}

$receipt = [ordered]@{
    schema = 1
    baseline_commit = $BaselineCommit
    candidate_commit = (git -C $RepoRoot rev-parse HEAD)
    candidate_dirty = [bool](git -C $RepoRoot status --porcelain)
    corpus_sha256 = $corpusHash
    measure_script_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $MeasureScript).Hash.ToLowerInvariant()
    measurement = [ordered]@{
        windows_affinity_cpu = [int]$manifest.windows_affinity_cpu
        launch_control = "suspended_affinity_high_then_resume"
        sample_order = "alternating"
    }
    opcode_matrix = [ordered]@{ expected = 151; implemented = 151; tested = 151; missing = 0; duplicates = 0 }
    instructions = 1000000
    performance_memory_bytes = 4096
    correctness_memory_bytes = 65536
    quick = [bool]$Quick
    machine = [ordered]@{
        computer = $env:COMPUTERNAME
        os = [Environment]::OSVersion.VersionString
        cpu = $cpu.Name
        rustc = ((rustc -Vv) -join "`n")
    }
    performance = $pair
    buffer_equivalence = $buffer
    tree_matrix_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $MatrixSource).Hash.ToLowerInvariant()
    trace_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $TraceSource).Hash.ToLowerInvariant()
}
$receiptPath = Join-Path $EvidenceRoot "receipt.json"
$receipt | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $receiptPath
Write-Host ("VM 6502: PASS 151/151/151 speedup {0:N3}x" -f $pair.speedup)
Write-Host "receipt: $receiptPath"
