param([switch]$Quick)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$proofRoot = Join-Path $repoRoot "tmp\native_v12.4.0\code-quality"
$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
$objdump = "objdump.exe"
$baselineTag = "v12.3.0"
$maximumRatio = 1.10

function Assert-True($value, [string]$message) {
    if (-not $value) { throw $message }
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
    if ($code -ne 0) { throw "$description failed ($code):`n$($output -join "`n")" }
    return ($output -join "`n")
}

function Sha256([string]$path) {
    return (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Export-Source([string]$commit, [string]$destination, [string[]]$benchmarkFiles) {
    if (Test-Path -LiteralPath $destination) { Remove-Item -LiteralPath $destination -Recurse -Force }
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    $archive = "$destination.tar"
    Invoke-Checked "git.exe" @("archive", "--format=tar", "-o", $archive, $commit) "archive $commit" | Out-Null
    Invoke-Checked "tar.exe" @("-xf", $archive, "-C", $destination) "extract $commit" | Out-Null
    Remove-Item -LiteralPath $archive -Force
    $target = Join-Path $destination "benchmarks\native"
    New-Item -ItemType Directory -Path $target -Force | Out-Null
    foreach ($file in $benchmarkFiles) {
        Copy-Item -LiteralPath (Join-Path $repoRoot "benchmarks\native\$file") -Destination (Join-Path $target $file) -Force
    }
}

function Build-Compiler([string]$source, [string]$target, [string]$label) {
    $saved = $env:CARGO_TARGET_DIR
    try {
        $env:CARGO_TARGET_DIR = $target
        Invoke-Checked $cargo @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec", "--release") "build $label compiler" $source | Out-Null
    }
    finally { $env:CARGO_TARGET_DIR = $saved }
    $compiler = Join-Path $target "release\dbytec.exe"
    Assert-True (Test-Path -LiteralPath $compiler) "$label compiler is absent"
    return $compiler
}

function Build-Object([string]$compiler, [string]$source, [string]$target, [string]$output, [bool]$candidate) {
    New-Item -ItemType Directory -Path (Split-Path -Parent $output) -Force | Out-Null
    $arguments = @("build", $source, "--target", $target, "--emit", "obj", "-o", $output)
    if ($candidate) { $arguments += @("--opt-level", "1", "--dump-machine-code") }
    return Invoke-Checked $compiler $arguments "build $(Split-Path -Leaf $output)"
}

function Read-Metrics([string]$path, [string]$arch) {
    $headers = Invoke-Checked $objdump @("-h", $path) "inspect sections"
    $disassembly = Invoke-Checked $objdump @("-d", "-Mintel", $path) "inspect instructions"
    $textLine = @($headers -split "`r?`n" | Where-Object { $_ -match '^\s*\d+\s+\.text\s+' })[0]
    $textBytes = [Convert]::ToUInt32((($textLine -split '\s+')[3]), 16)
    $instructions = @($disassembly -split "`r?`n" | Where-Object { $_ -match '^\s*[0-9a-f]+:\s+(?:[0-9a-f]{2}\s+)+' })
    $base = if ($arch -ceq "i686") { "ebp" } else { "rbp" }
    $stack = if ($arch -ceq "i686") { "esp" } else { "rsp" }
    $loads = @($instructions | Where-Object { $_ -match '\bmov\w*\s+[^,]+,.*PTR' }).Count
    $stores = @($instructions | Where-Object { $_ -match '\bmov\w*\s+.*PTR[^,]*,' }).Count
    $spillAccesses = @($instructions | Where-Object { $_ -match "\[$base-" }).Count
    $frameBytes = 0L
    $inPrologue = $false
    foreach ($line in ($disassembly -split "`r?`n")) {
        if ($line -match '^\s*[0-9a-f]+ <[^>]+>:$') { $inPrologue = $false }
        if ($line -match "\bmov\s+$base,$stack") { $inPrologue = $true; continue }
        if ($inPrologue -and $line -match "\bsub\s+$stack,0x([0-9a-f]+)") {
            $frameBytes += [Convert]::ToUInt32($Matches[1], 16)
            $inPrologue = $false
        }
        elseif ($inPrologue -and $line -match '^\s*[0-9a-f]+:' -and $line -notmatch '\bpush\s+') {
            $inPrologue = $false
        }
    }
    return [ordered]@{
        text_bytes = [long]$textBytes
        instructions = [long]$instructions.Count
        loads = [long]$loads
        stores = [long]$stores
        spill_accesses = [long]$spillAccesses
        frame_bytes = [long]$frameBytes
    }
}

function Assert-Metric([string]$case, [string]$metric, [long]$baseline, [long]$candidate) {
    if ($baseline -eq 0) {
        Assert-True ($candidate -eq 0) "$case $metric introduced from zero to $candidate"
        return 1.0
    }
    $ratio = [double]$candidate / [double]$baseline
    Assert-True ($ratio -le $maximumRatio) "$case $metric ratio $ratio exceeds $maximumRatio ($candidate/$baseline)"
    return $ratio
}

$i686 = @("integer_loop.dby", "branch_loop.dby", "direct_calls.dby", "indirect_calls.dby", "recursion.dby", "scalar_struct.dby", "wide_arithmetic.dby")
$none = @(
    [ordered]@{ name = "bootstrap"; path = "examples\native_v12_2_none\bootstrap.dby" },
    [ordered]@{ name = "runtime"; path = "examples\native_v12_2_none\runtime.dby" },
    [ordered]@{ name = "collections"; path = "examples\native_v12_3_collections\none.dby" }
)
$baselineCommit = (Invoke-Checked "git.exe" @("rev-parse", "$baselineTag^{commit}") "resolve baseline").Trim()
if (Test-Path -LiteralPath $proofRoot) { Remove-Item -LiteralPath $proofRoot -Recurse -Force }
New-Item -ItemType Directory -Path $proofRoot -Force | Out-Null
$sourceA = Join-Path $proofRoot "baseline-a-source"
$sourceB = Join-Path $proofRoot "baseline-b-source"
Export-Source $baselineCommit $sourceA $i686
Export-Source $baselineCommit $sourceB $i686
$candidateCompiler = Build-Compiler $repoRoot (Join-Path $proofRoot "candidate-compiler") "candidate"
$baselineACompiler = Build-Compiler $sourceA (Join-Path $proofRoot "baseline-a-compiler") "baseline A"
$baselineBCompiler = Build-Compiler $sourceB (Join-Path $proofRoot "baseline-b-compiler") "baseline B"

$cases = New-Object System.Collections.Generic.List[object]
foreach ($file in $i686) {
    $name = [IO.Path]::GetFileNameWithoutExtension($file)
    $currentSource = Join-Path $repoRoot "benchmarks\native\$file"
    $aSource = Join-Path $sourceA "benchmarks\native\$file"
    $bSource = Join-Path $sourceB "benchmarks\native\$file"
    Assert-True ((Sha256 $currentSource) -ceq (Sha256 $aSource)) "$name baseline A source drifted"
    Assert-True ((Sha256 $currentSource) -ceq (Sha256 $bSource)) "$name baseline B source drifted"
    $root = Join-Path $proofRoot "i686\$name"
    $a = Join-Path $root "baseline-a.o"; $b = Join-Path $root "baseline-b.o"
    $ca = Join-Path $root "candidate-a.o"; $cb = Join-Path $root "candidate-b.o"
    Build-Object $baselineACompiler $aSource "i686-dbyte-none" $a $false | Out-Null
    Build-Object $baselineBCompiler $bSource "i686-dbyte-none" $b $false | Out-Null
    $dumpA = Build-Object $candidateCompiler $currentSource "i686-dbyte-none" $ca $true
    $dumpB = Build-Object $candidateCompiler $currentSource "i686-dbyte-none" $cb $true
    Assert-True ((Sha256 $a) -ceq (Sha256 $b)) "$name baseline object is not deterministic"
    Assert-True ((Sha256 $ca) -ceq (Sha256 $cb)) "$name candidate object is not deterministic"
    Assert-True ($dumpA -ceq $dumpB) "$name candidate machine dump is not deterministic"
    $baseMetrics = Read-Metrics $a "i686"; $candidateMetrics = Read-Metrics $ca "i686"
    $ratios = [ordered]@{}
    foreach ($metric in $baseMetrics.Keys) { $ratios[$metric] = Assert-Metric "i686/$name" $metric $baseMetrics[$metric] $candidateMetrics[$metric] }
    $cases.Add([ordered]@{ target = "i686-dbyte-none"; name = $name; source_sha256 = Sha256 $currentSource; baseline_sha256 = Sha256 $a; candidate_sha256 = Sha256 $ca; baseline = $baseMetrics; candidate = $candidateMetrics; ratios = $ratios; machine_dump_sha256 = (Get-FileHash -InputStream ([IO.MemoryStream]::new([Text.Encoding]::UTF8.GetBytes($dumpA))) -Algorithm SHA256).Hash.ToLowerInvariant() })
}

foreach ($item in $none) {
    $currentSource = Join-Path $repoRoot $item.path
    $aSource = Join-Path $sourceA $item.path
    $bSource = Join-Path $sourceB $item.path
    $gitPath = $item.path.Replace('\', '/')
    $currentBlob = (Invoke-Checked "git.exe" @("hash-object", "--", $gitPath) "hash current $gitPath").Trim()
    $baselineBlob = (Invoke-Checked "git.exe" @("rev-parse", "$baselineCommit`:$gitPath") "resolve baseline $gitPath").Trim()
    Assert-True ($currentBlob -ceq $baselineBlob) "$($item.name) tracked source differs from v12.3.0"
    Assert-True ((Sha256 $aSource) -ceq (Sha256 $bSource)) "$($item.name) baseline source exports differ"
    $root = Join-Path $proofRoot "none\$($item.name)"
    $a = Join-Path $root "baseline-a.o"; $b = Join-Path $root "baseline-b.o"
    $ca = Join-Path $root "candidate-a.o"; $cb = Join-Path $root "candidate-b.o"
    Build-Object $baselineACompiler $aSource "x86_64-dbyte-none" $a $false | Out-Null
    Build-Object $baselineBCompiler $bSource "x86_64-dbyte-none" $b $false | Out-Null
    $dumpA = Build-Object $candidateCompiler $currentSource "x86_64-dbyte-none" $ca $true
    $dumpB = Build-Object $candidateCompiler $currentSource "x86_64-dbyte-none" $cb $true
    Assert-True ((Sha256 $a) -ceq (Sha256 $b)) "$($item.name) baseline object is not deterministic"
    Assert-True ((Sha256 $ca) -ceq (Sha256 $cb)) "$($item.name) candidate object is not deterministic"
    Assert-True ($dumpA -ceq $dumpB) "$($item.name) candidate machine dump is not deterministic"
    $baseMetrics = Read-Metrics $a "x86_64"; $candidateMetrics = Read-Metrics $ca "x86_64"
    $ratios = [ordered]@{}
    foreach ($metric in $baseMetrics.Keys) { $ratios[$metric] = Assert-Metric "none/$($item.name)" $metric $baseMetrics[$metric] $candidateMetrics[$metric] }
    $cases.Add([ordered]@{ target = "x86_64-dbyte-none"; name = $item.name; source_sha256 = Sha256 $currentSource; baseline_sha256 = Sha256 $a; candidate_sha256 = Sha256 $ca; baseline = $baseMetrics; candidate = $candidateMetrics; ratios = $ratios })
}

foreach ($metric in @("text_bytes", "instructions", "loads", "stores", "spill_accesses", "frame_bytes")) {
    $baseline = ($cases | ForEach-Object { $_.baseline[$metric] } | Measure-Object -Sum).Sum
    $candidate = ($cases | ForEach-Object { $_.candidate[$metric] } | Measure-Object -Sum).Sum
    Assert-True ($candidate -le $baseline) "aggregate $metric regressed ($candidate > $baseline)"
}

$receipt = [ordered]@{
    schema = 1
    contract = "DByte native code quality v12.4.0"
    generated_utc = [DateTime]::UtcNow.ToString("o")
    baseline_tag = $baselineTag
    baseline_commit = $baselineCommit
    candidate_head = (Invoke-Checked "git.exe" @("rev-parse", "HEAD") "resolve candidate").Trim()
    maximum_individual_ratio = $maximumRatio
    cases = $cases.ToArray()
}
[IO.File]::WriteAllText((Join-Path $proofRoot "receipt.json"), ($receipt | ConvertTo-Json -Depth 12) + "`n", [Text.UTF8Encoding]::new($false))
Write-Host "DBYTE NATIVE CODE QUALITY: PASS"
Write-Host "Receipt: $(Join-Path $proofRoot 'receipt.json')"
