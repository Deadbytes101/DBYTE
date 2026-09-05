param(
    [switch]$Quick,
    [string]$Case = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$manifestPath = Join-Path $repoRoot "benchmarks\native\manifest.json"
$measureScript = Join-Path $repoRoot "scripts\native_perf_measure.py"
$proofRoot = Join-Path $repoRoot "tmp\native_v12.4.0\performance"
$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"

function Assert-True($value, [string]$message) {
    if (-not $value) { throw $message }
}

function Sha256([string]$path) {
    return (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Text-Sha256([string]$text) {
    $bytes = [Text.Encoding]::UTF8.GetBytes([string]$text)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $digest = $sha.ComputeHash([byte[]]$bytes)
        return [BitConverter]::ToString($digest).Replace("-", "").ToLowerInvariant()
    }
    finally { $sha.Dispose() }
}

function Invoke-Checked([string]$file, [string[]]$arguments, [string]$description, [string]$cwd = $repoRoot) {
    $saved = $ErrorActionPreference
    $old = Get-Location
    $ErrorActionPreference = "Continue"
    try {
        Set-Location $cwd
        $output = & $file @arguments 2>&1
        $code = $LASTEXITCODE
    }
    finally {
        Set-Location $old
        $ErrorActionPreference = $saved
    }
    if ($code -ne 0) {
        throw "$description failed ($code):`n$($output -join "`n")"
    }
    return ($output -join "`n")
}

function Wsl-Path([string]$path) {
    $portable = $path.Replace('\', '/')
    return (Invoke-Checked "wsl.exe" @("-d", "Debian", "--", "wslpath", "-a", "-u", $portable) "translate WSL path").Trim()
}

function Assert-Same([string]$left, [string]$right, [string]$name) {
    $a = Sha256 $left
    $b = Sha256 $right
    Assert-True ($a -ceq $b) "$name is not deterministic: $a != $b"
    return $a
}

function Candidate-TreeHash {
    $entries = New-Object System.Collections.Generic.List[string]
    foreach ($root in @("crates", "runtime", "benchmarks\native")) {
        Get-ChildItem -LiteralPath (Join-Path $repoRoot $root) -File -Recurse |
            Sort-Object FullName | ForEach-Object {
                $relative = $_.FullName.Substring($repoRoot.Length + 1).Replace('\', '/')
                $entries.Add("$relative $(Sha256 $_.FullName)")
            }
    }
    foreach ($file in @("Cargo.toml", "Cargo.lock", "scripts\native_perf_measure.py")) {
        $path = Join-Path $repoRoot $file
        $entries.Add("$($file.Replace('\', '/')) $(Sha256 $path)")
    }
    return Text-Sha256 (($entries -join "`n") + "`n")
}

function Build-Compiler([string]$sourceRoot, [string]$targetRoot, [string]$label) {
    New-Item -ItemType Directory -Path $targetRoot -Force | Out-Null
    $savedTarget = $env:CARGO_TARGET_DIR
    try {
        $env:CARGO_TARGET_DIR = $targetRoot
        Invoke-Checked $cargo @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec", "--release") "build $label compiler" $sourceRoot | Out-Null
    }
    finally { $env:CARGO_TARGET_DIR = $savedTarget }
    $compiler = Join-Path $targetRoot "release\dbytec.exe"
    Assert-True (Test-Path -LiteralPath $compiler) "$label compiler was not produced"
    return $compiler
}

function Build-Artifact([string]$compiler, [string]$source, [string]$target, [string]$emit, [string]$output, [int]$opt, [switch]$Dump) {
    New-Item -ItemType Directory -Path (Split-Path -Parent $output) -Force | Out-Null
    $arguments = @("build", $source, "--target", $target, "--emit", $emit, "-o", $output)
    if ($opt -ge 0) { $arguments += @("--opt-level", "$opt") }
    if ($Dump) { $arguments += @("--dump-native-mir", "--dump-machine-code") }
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $text = Invoke-Checked $compiler $arguments "build $(Split-Path -Leaf $output)"
    $watch.Stop()
    return [PSCustomObject]@{
        Milliseconds = [Math]::Round($watch.Elapsed.TotalMilliseconds, 3)
        Output = $text
    }
}

function Export-Baseline([string]$commit, [string]$destination, [string[]]$corpusFiles) {
    if (Test-Path -LiteralPath $destination) { Remove-Item -LiteralPath $destination -Recurse -Force }
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    $archive = "$destination.tar"
    Invoke-Checked "git.exe" @("archive", "--format=tar", "-o", $archive, $commit) "archive baseline"
    Invoke-Checked "tar.exe" @("-xf", $archive, "-C", $destination) "extract baseline"
    Remove-Item -LiteralPath $archive -Force
    $nativeRoot = Join-Path $destination "benchmarks\native"
    New-Item -ItemType Directory -Path $nativeRoot -Force | Out-Null
    foreach ($name in $corpusFiles) {
        Copy-Item -LiteralPath (Join-Path $repoRoot "benchmarks\native\$name") -Destination (Join-Path $nativeRoot $name) -Force
    }
}

function Assert-Windows-VariantParity([string]$linuxPath, [string]$windowsPath, [string]$name) {
    $linux = [IO.File]::ReadAllText($linuxPath).Replace("linux_system_allocator.dby", "TARGET_SYSTEM_ALLOCATOR.dby")
    $windows = [IO.File]::ReadAllText($windowsPath).Replace("windows_system_allocator.dby", "TARGET_SYSTEM_ALLOCATOR.dby")
    Assert-True ($linux -ceq $windows) "$name Windows source differs beyond allocator import"
}

function Measure-LinuxPair([string]$baseline, [string]$candidate, [int]$expected, $manifest) {
    $script = Wsl-Path $measureScript
    $base = Wsl-Path $baseline
    $cand = Wsl-Path $candidate
    $arguments = @(
        "-d", "Debian", "--", "taskset", "-c", "$($manifest.linux_affinity_cpu)", "python3", $script, $base, $cand,
        "--expected-exit", "$expected", "--warmups", "$($manifest.warmups)",
        "--measurements", "$($manifest.measurements)", "--attempts", "$($manifest.maximum_attempts)",
        "--maximum-relative-mad", "$($manifest.maximum_relative_mad)",
        "--cpu-affinity", "$($manifest.linux_affinity_cpu)",
        "--settle-milliseconds", "$($manifest.linux_settle_milliseconds)",
        "--sample-repetitions", "$($manifest.sample_repetitions)"
    )
    $json = Invoke-Checked "wsl.exe" $arguments "measure Linux pair"
    return ($json | ConvertFrom-Json)
}

function Measure-WindowsPair([string]$baseline, [string]$candidate, [int]$expected, $manifest) {
    $arguments = @(
        $measureScript, $baseline, $candidate,
        "--expected-exit", "$expected", "--warmups", "$($manifest.warmups)",
        "--measurements", "$($manifest.measurements)", "--attempts", "$($manifest.maximum_attempts)",
        "--maximum-relative-mad", "$($manifest.maximum_relative_mad)",
        "--cpu-affinity", "$($manifest.windows_affinity_cpu)",
        "--settle-milliseconds", "$($manifest.windows_settle_milliseconds)",
        "--sample-repetitions", "$($manifest.sample_repetitions)"
    )
    $json = Invoke-Checked "python.exe" $arguments "measure Windows pair"
    return ($json | ConvertFrom-Json)
}

function Probe-HardwareCounters {
    $saved = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & wsl.exe -d Debian -- sh -lc "if ! command -v perf >/dev/null 2>&1; then echo PERF_NOT_FOUND; exit 127; fi; perf stat -x, -e cycles,instructions -- true" 2>&1
        $code = $LASTEXITCODE
    }
    finally { $ErrorActionPreference = $saved }
    return [ordered]@{
        status = if ($code -eq 0) { "AVAILABLE" } else { "UNAVAILABLE" }
        exit_code = $code
        probe = ($output -join "`n")
    }
}
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
Assert-True ($manifest.schema -eq 2) "native performance manifest schema must be 2"
Assert-True ($manifest.workloads.Count -eq 14) "native performance corpus must contain exactly 14 workloads"
Assert-True ([int]$manifest.linux_settle_milliseconds -ge 0) "Linux settle interval must be non-negative"
Assert-True ([int]$manifest.windows_settle_milliseconds -ge 0) "Windows settle interval must be non-negative"
Assert-True ([int]$manifest.sample_repetitions -ge 1) "sample repetitions must be positive"
$names = @($manifest.workloads | ForEach-Object { $_.name })
Assert-True ((@($names | Sort-Object -Unique)).Count -eq $names.Count) "native performance workload names are not unique"

$selected = @($manifest.workloads)
if ($Case -ne "") {
    $selected = @($selected | Where-Object { $_.name -ceq $Case })
    Assert-True ($selected.Count -eq 1) "unknown native performance case '$Case'"
}

$corpusFiles = New-Object System.Collections.Generic.HashSet[string] ([StringComparer]::Ordinal)
foreach ($workload in $manifest.workloads) {
    $linux = Join-Path $repoRoot "benchmarks\native\$($workload.source)"
    Assert-True (Test-Path -LiteralPath $linux) "missing Linux workload source: $($workload.source)"
    [void]$corpusFiles.Add([string]$workload.source)
    if ($workload.PSObject.Properties.Name -contains "windows_source") {
        $windows = Join-Path $repoRoot "benchmarks\native\$($workload.windows_source)"
        Assert-True (Test-Path -LiteralPath $windows) "missing Windows workload source: $($workload.windows_source)"
        Assert-Windows-VariantParity $linux $windows $workload.name
        [void]$corpusFiles.Add([string]$workload.windows_source)
    }
}
[void]$corpusFiles.Add("startup_calibration.dby")
if (Test-Path -LiteralPath $proofRoot) { Remove-Item -LiteralPath $proofRoot -Recurse -Force }
New-Item -ItemType Directory -Path $proofRoot -Force | Out-Null

$baselineCommit = (Invoke-Checked "git.exe" @("rev-parse", "$($manifest.baseline_tag)^{commit}") "resolve baseline tag").Trim()
$candidateHead = (Invoke-Checked "git.exe" @("rev-parse", "HEAD") "resolve candidate HEAD").Trim()
$candidateTree = Candidate-TreeHash
$manifestHash = Sha256 $manifestPath
$measureHash = Sha256 $measureScript

$candidateCompiler = Build-Compiler $repoRoot (Join-Path $proofRoot "compiler-candidate") "candidate"
$baselineACompiler = $null
$baselineBCompiler = $null
$baselineARoot = Join-Path $proofRoot "baseline-a-source"
$baselineBRoot = Join-Path $proofRoot "baseline-b-source"

if (-not $Quick) {
    Export-Baseline $baselineCommit $baselineARoot @($corpusFiles)
    Export-Baseline $baselineCommit $baselineBRoot @($corpusFiles)
    $baselineACompiler = Build-Compiler $baselineARoot (Join-Path $proofRoot "compiler-baseline-a") "baseline A"
    $baselineBCompiler = Build-Compiler $baselineBRoot (Join-Path $proofRoot "compiler-baseline-b") "baseline B"
}

$quickMeasure = [PSCustomObject]@{
    warmups = 0
    measurements = 1
    maximum_attempts = 1
    maximum_relative_mad = 1.0
    linux_affinity_cpu = [int]$manifest.linux_affinity_cpu
    windows_affinity_cpu = [int]$manifest.windows_affinity_cpu
    linux_settle_milliseconds = 0
    windows_settle_milliseconds = 0
    sample_repetitions = 1
}
$linuxResults = New-Object System.Collections.Generic.List[object]
$windowsResults = New-Object System.Collections.Generic.List[object]
function Build-CandidateSet($workload, [string]$source, [string]$target, [string]$emit, [string]$root) {
    $extension = if ($emit -ceq "exe") { "exe" } else { "elf" }
    $o0a = Join-Path $root "o0-a.$extension"
    $o0b = Join-Path $root "o0-b.$extension"
    $o1a = Join-Path $root "o1-a.$extension"
    $o1b = Join-Path $root "o1-b.$extension"
    $o0First = Build-Artifact $candidateCompiler $source $target $emit $o0a 0 -Dump
    $o0Second = Build-Artifact $candidateCompiler $source $target $emit $o0b 0 -Dump
    $o1First = Build-Artifact $candidateCompiler $source $target $emit $o1a 1 -Dump
    $o1Second = Build-Artifact $candidateCompiler $source $target $emit $o1b 1 -Dump
    $o0Hash = Assert-Same $o0a $o0b "$($workload.name) $target O0 artifact"
    $o1Hash = Assert-Same $o1a $o1b "$($workload.name) $target O1 artifact"
    Assert-True ($o0First.Output -ceq $o0Second.Output) "$($workload.name) $target O0 dump is not deterministic"
    Assert-True ($o1First.Output -ceq $o1Second.Output) "$($workload.name) $target O1 dump is not deterministic"
    return [PSCustomObject]@{
        O0 = $o0a; O1 = $o1a; O0Sha256 = $o0Hash; O1Sha256 = $o1Hash
        O0DumpSha256 = Text-Sha256 $o0First.Output; O1DumpSha256 = Text-Sha256 $o1First.Output
        O0CompileMs = $o0First.Milliseconds; O1CompileMs = $o1First.Milliseconds
    }
}

function Build-BaselineSet($workload, [string]$relativeSource, [string]$target, [string]$emit, [string]$root) {
    $extension = if ($emit -ceq "exe") { "exe" } else { "elf" }
    $sourceA = Join-Path $baselineARoot "benchmarks\native\$relativeSource"
    $sourceB = Join-Path $baselineBRoot "benchmarks\native\$relativeSource"
    $current = Join-Path $repoRoot "benchmarks\native\$relativeSource"
    Assert-True ((Sha256 $sourceA) -ceq (Sha256 $current)) "$($workload.name) baseline A source bytes drifted"
    Assert-True ((Sha256 $sourceB) -ceq (Sha256 $current)) "$($workload.name) baseline B source bytes drifted"
    $a = Join-Path $root "baseline-a.$extension"
    $b = Join-Path $root "baseline-b.$extension"
    $first = Build-Artifact $baselineACompiler $sourceA $target $emit $a -1
    $second = Build-Artifact $baselineBCompiler $sourceB $target $emit $b -1
    $hash = Assert-Same $a $b "$($workload.name) $target baseline artifact"
    return [PSCustomObject]@{
        A = $a; B = $b; Sha256 = $hash
        ACompileMs = $first.Milliseconds; BCompileMs = $second.Milliseconds
    }
}

function Assert-ObservableParity([string]$platform, $candidateSet, [int]$expected) {
    if ($platform -ceq "linux") {
        $result = Measure-LinuxPair $candidateSet.O0 $candidateSet.O1 $expected $quickMeasure
    }
    else {
        $result = Measure-WindowsPair $candidateSet.O0 $candidateSet.O1 $expected $quickMeasure
    }
    Assert-True $result.stable "$platform O0/O1 runtime parity probe was unstable"
    return $result.signature
}

foreach ($workload in $selected) {
    Write-Host "native-perf: $($workload.name)"
    $caseRoot = Join-Path $proofRoot "workloads\$($workload.name)"
    $linuxSource = Join-Path $repoRoot "benchmarks\native\$($workload.source)"
    $candidateLinux = Build-CandidateSet $workload $linuxSource $manifest.primary_target "elf" (Join-Path $caseRoot "linux")
    $linuxSignature = Assert-ObservableParity "linux" $candidateLinux ([int]$workload.expected_exit)
    $linuxReadelf = Invoke-Checked "wsl.exe" @("-d", "Debian", "--", "readelf", "-l", "-d", (Wsl-Path $candidateLinux.O1)) "inspect $($workload.name) Linux ELF"
    Assert-True (-not $linuxReadelf.Contains("INTERP")) "$($workload.name) Linux O1 has PT_INTERP"
    Assert-True ($linuxReadelf.Contains("There is no dynamic section")) "$($workload.name) Linux O1 has dynamic runtime state"

    if (-not $Quick) {
        $baselineLinux = Build-BaselineSet $workload $workload.source $manifest.primary_target "elf" (Join-Path $caseRoot "linux")
        $measurement = Measure-LinuxPair $baselineLinux.A $candidateLinux.O1 ([int]$workload.expected_exit) $manifest
        Assert-True $measurement.stable "$($workload.name) Linux measurement did not stabilize"
        $attempt = $measurement.attempts[$measurement.selected_attempt - 1]
        Assert-True ($attempt.ratio -ge $manifest.minimum_individual_ratio) "$($workload.name) Linux ratio $($attempt.ratio) is below floor $($manifest.minimum_individual_ratio)"
        $linuxResults.Add([PSCustomObject]@{
            name = $workload.name; source_sha256 = Sha256 $linuxSource; baseline_sha256 = $baselineLinux.Sha256
            o0_sha256 = $candidateLinux.O0Sha256; o1_sha256 = $candidateLinux.O1Sha256
            o0_dump_sha256 = $candidateLinux.O0DumpSha256; o1_dump_sha256 = $candidateLinux.O1DumpSha256
            baseline_a_compile_ms = $baselineLinux.ACompileMs; baseline_b_compile_ms = $baselineLinux.BCompileMs
            o0_compile_ms = $candidateLinux.O0CompileMs; o1_compile_ms = $candidateLinux.O1CompileMs
            runtime_signature = $linuxSignature; measurement = $measurement
        })
    }
    $windowsRelative = if ($workload.PSObject.Properties.Name -contains "windows_source") { [string]$workload.windows_source } else { [string]$workload.source }
    $windowsSource = Join-Path $repoRoot "benchmarks\native\$windowsRelative"
    $candidateWindows = Build-CandidateSet $workload $windowsSource $manifest.windows_target "exe" (Join-Path $caseRoot "windows")
    $windowsSignature = Assert-ObservableParity "windows" $candidateWindows ([int]$workload.expected_exit)

    if (-not $Quick) {
        $baselineWindows = Build-BaselineSet $workload $windowsRelative $manifest.windows_target "exe" (Join-Path $caseRoot "windows")
        $measurement = Measure-WindowsPair $baselineWindows.A $candidateWindows.O1 ([int]$workload.expected_exit) $manifest
        Assert-True $measurement.stable "$($workload.name) Windows measurement did not stabilize"
        $attempt = $measurement.attempts[$measurement.selected_attempt - 1]
        Assert-True ($attempt.ratio -ge $manifest.minimum_individual_ratio) "$($workload.name) Windows ratio $($attempt.ratio) is below floor $($manifest.minimum_individual_ratio)"
        $windowsResults.Add([PSCustomObject]@{
            name = $workload.name; source_sha256 = Sha256 $windowsSource; baseline_sha256 = $baselineWindows.Sha256
            o0_sha256 = $candidateWindows.O0Sha256; o1_sha256 = $candidateWindows.O1Sha256
            o0_dump_sha256 = $candidateWindows.O0DumpSha256; o1_dump_sha256 = $candidateWindows.O1DumpSha256
            baseline_a_compile_ms = $baselineWindows.ACompileMs; baseline_b_compile_ms = $baselineWindows.BCompileMs
            o0_compile_ms = $candidateWindows.O0CompileMs; o1_compile_ms = $candidateWindows.O1CompileMs
            runtime_signature = $windowsSignature; measurement = $measurement
        })
    }
}

if ($Quick) {
    Write-Host "DBYTE NATIVE MACHINE CODE PERFORMANCE QUICK: PASS"
    exit 0
}
$ratios = foreach ($result in $linuxResults) {
    $attempt = $result.measurement.attempts[$result.measurement.selected_attempt - 1]
    [double]$attempt.ratio
}
$geometricMean = [Math]::Exp((($ratios | ForEach-Object { [Math]::Log($_) } | Measure-Object -Sum).Sum) / $ratios.Count)
if ($Case -eq "") {
    Assert-True ($geometricMean -ge $manifest.minimum_geometric_mean_speedup) "Linux geometric mean $geometricMean is below gate $($manifest.minimum_geometric_mean_speedup)"
}
$nmos6502Result = @($linuxResults | Where-Object { $_.name -ceq "nmos6502" })
if ($Case -eq "" -or $Case -ceq "nmos6502") {
    Assert-True ($nmos6502Result.Count -eq 1) "native NMOS 6502 workload result is missing"
    $nmos6502Attempt = $nmos6502Result[0].measurement.attempts[$nmos6502Result[0].measurement.selected_attempt - 1]
    Assert-True ($nmos6502Attempt.ratio -ge $manifest.minimum_nmos6502_ratio) "nmos6502 Linux ratio $($nmos6502Attempt.ratio) is below gate $($manifest.minimum_nmos6502_ratio)"
}

$startupWorkload = [PSCustomObject]@{ name = "startup_calibration" }
$startupSource = Join-Path $repoRoot "benchmarks\native\startup_calibration.dby"
$startupLinuxRoot = Join-Path $proofRoot "startup\linux"
$startupWindowsRoot = Join-Path $proofRoot "startup\windows"
$startupLinuxCandidate = Build-CandidateSet $startupWorkload $startupSource $manifest.primary_target "elf" $startupLinuxRoot
$startupLinuxBaseline = Build-BaselineSet $startupWorkload "startup_calibration.dby" $manifest.primary_target "elf" $startupLinuxRoot
$startupWindowsCandidate = Build-CandidateSet $startupWorkload $startupSource $manifest.windows_target "exe" $startupWindowsRoot
$startupWindowsBaseline = Build-BaselineSet $startupWorkload "startup_calibration.dby" $manifest.windows_target "exe" $startupWindowsRoot
$calibration = [PSCustomObject]@{ warmups = 3; measurements = 9; maximum_attempts = 1; maximum_relative_mad = 100.0; linux_affinity_cpu = [int]$manifest.linux_affinity_cpu; windows_affinity_cpu = [int]$manifest.windows_affinity_cpu; linux_settle_milliseconds = [int]$manifest.linux_settle_milliseconds; windows_settle_milliseconds = [int]$manifest.windows_settle_milliseconds; sample_repetitions = [int]$manifest.sample_repetitions }
$startupLinux = Measure-LinuxPair $startupLinuxBaseline.A $startupLinuxCandidate.O1 42 $calibration
$startupWindows = Measure-WindowsPair $startupWindowsBaseline.A $startupWindowsCandidate.O1 42 $calibration

$statusText = [string](Invoke-Checked "git.exe" @("status", "--porcelain=v1", "--untracked-files=all") "read candidate status")
$statusPayload = $statusText + "`n"
$statusHash = Text-Sha256 $statusPayload
$receipt = [ordered]@{
    schema = 1
    contract = "DByte native machine code performance v12.4.0"
    generated_utc = [DateTime]::UtcNow.ToString("o")
    manifest_sha256 = $manifestHash
    measure_script_sha256 = $measureHash
    baseline_tag = $manifest.baseline_tag
    baseline_commit = $baselineCommit
    candidate_head = $candidateHead
    candidate_tree_sha256 = $candidateTree
    candidate_status_sha256 = $statusHash
    candidate_dirty = -not [String]::IsNullOrWhiteSpace($statusText)
    hardware_counters = Probe-HardwareCounters
    linux = [ordered]@{
        target = $manifest.primary_target
        affinity_cpu = [int]$manifest.linux_affinity_cpu
        settle_milliseconds = [int]$manifest.linux_settle_milliseconds
        sample_repetitions = [int]$manifest.sample_repetitions
        geometric_mean_speedup = $geometricMean
        minimum_geometric_mean_speedup = $manifest.minimum_geometric_mean_speedup
        minimum_individual_ratio = $manifest.minimum_individual_ratio
        startup_calibration = $startupLinux
        workloads = $linuxResults.ToArray()
    }
    windows = [ordered]@{
        target = $manifest.windows_target
        affinity_cpu = [int]$manifest.windows_affinity_cpu
        settle_milliseconds = [int]$manifest.windows_settle_milliseconds
        sample_repetitions = [int]$manifest.sample_repetitions
        minimum_individual_ratio = $manifest.minimum_individual_ratio
        startup_calibration = $startupWindows
        workloads = $windowsResults.ToArray()
    }
}
$receiptPath = Join-Path $proofRoot "receipt.json"
[IO.File]::WriteAllText($receiptPath, ($receipt | ConvertTo-Json -Depth 30) + "`n", [Text.UTF8Encoding]::new($false))

Write-Host "DBYTE NATIVE MACHINE CODE PERFORMANCE: PASS"
Write-Host ("Linux O1 geometric mean: {0:N3}x" -f $geometricMean)
Write-Host "Receipt: $receiptPath"
$global:LASTEXITCODE = 0
