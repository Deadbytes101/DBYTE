param(
    [Parameter(Mandatory = $true)][string]$ExpectedHead,
    [switch]$RequireTag
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$ExpectedHead = $ExpectedHead.Trim().ToLowerInvariant()
if ($ExpectedHead -notmatch '^[0-9a-f]{40}$') { throw 'V13_1_FINAL_HEAD_INVALID' }

$frozenV13Head = '3b68536dc280dade3afde1620eeffbeb0309ec97'
$root = Join-Path $repo ('tmp/v13_1_final/final-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root -Force | Out-Null
$script:sequence = 0

function Hash([string]$Path) {
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Run([string]$Exe, [string[]]$Arguments, [string]$Description) {
    $script:sequence++
    $log = Join-Path $root ('command-{0:D3}.txt' -f $script:sequence)
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $output = & $Exe @Arguments 2>&1
        $exitCode = $LASTEXITCODE
    }
    finally { $ErrorActionPreference = $saved }
    [IO.File]::WriteAllText($log, "COMMAND=$Exe $($Arguments -join ' ')`nEXIT=$exitCode`n$($output -join "`n")")
    if ($exitCode -ne 0) { throw "$Description failed ($exitCode): $log" }
    return ($output -join "`n")
}

function Evidence-Root([string]$Output, [string]$Label) {
    $line = @($Output -split "`r?`n" | Where-Object { $_.StartsWith('Evidence root:', [StringComparison]::OrdinalIgnoreCase) }) | Select-Object -Last 1
    if (-not $line) { throw "$Label evidence root missing" }
    $path = $line.Substring('Evidence root:'.Length).Trim()
    $result = Join-Path $path 'RESULT.json'
    if (-not (Test-Path -LiteralPath $result -PathType Leaf)) { throw "$Label RESULT.json missing: $path" }
    return $path
}

Push-Location $repo
try {
    $head = (Run 'git.exe' @('rev-parse', 'HEAD') 'read HEAD').Trim().ToLowerInvariant()
    if ($head -cne $ExpectedHead) { throw "V13_1_FINAL_HEAD_MISMATCH: $head" }

    $parentType = (Run 'git.exe' @('cat-file', '-t', 'v13.0.0') 'read v13.0.0 tag type').Trim()
    if ($parentType -cne 'tag') { throw 'V13_1_FROZEN_PARENT_NOT_ANNOTATED' }
    $parentHead = (Run 'git.exe' @('rev-parse', 'v13.0.0^{}') 'read v13.0.0 tag target').Trim().ToLowerInvariant()
    if ($parentHead -cne $frozenV13Head) { throw 'V13_1_FROZEN_PARENT_DRIFT' }

    Run 'git.exe' @('diff', '--check') 'diff check' | Out-Null
    $dirtyBefore = (Run 'git.exe' @('status', '--porcelain=v1', '--untracked-files=all', '--', '.', ':(exclude)examples/smoke.dby', ':(exclude)examples/.cube-state.bin') 'release cleanliness').Trim()
    if ($dirtyBefore.Length -ne 0) { throw 'V13_1_FINAL_WORKTREE_DIRTY' }
    Run 'cargo.exe' @('fmt', '--all', '--', '--check') 'format check' | Out-Null
    Run 'cargo.exe' @('clippy', '--workspace', '--all-targets', '--', '-D', 'warnings') 'clippy warnings-as-errors' | Out-Null
    Run 'cargo.exe' @('test', '-q', '--workspace') 'workspace tests' | Out-Null
    $version = (Run 'cargo.exe' @('run', '-q', '-p', 'dbyte_cli', '--', '--version') 'version probe').Trim()
    if ($version -cne 'DByte 13.1.0') { throw "V13_1_FINAL_VERSION_MISMATCH: $version" }

    Run 'cargo.exe' @('test', '-q', '-p', 'dbyte_native', 'check_portable') 'portable semantic tests' | Out-Null
    $portableOutput = Run 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $PSScriptRoot 'verify_portable_runtime_parity.ps1')) 'portable runtime parity'
    $portableRoot = Evidence-Root $portableOutput 'portable runtime parity'
    $floatOutput = Run 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $PSScriptRoot 'verify_native_explicit_float.ps1')) 'native explicit float'
    $floatRoot = Evidence-Root $floatOutput 'native explicit float'

    $portableResult = Get-Content -LiteralPath (Join-Path $portableRoot 'RESULT.json') -Raw | ConvertFrom-Json
    if ($portableResult.status -cne 'PASS') { throw 'V13_1_PORTABLE_RESULT_NOT_PASS' }
    if ($portableResult.candidate_dirty -ne $false) { throw 'V13_1_PORTABLE_RESULT_DIRTY' }
    $floatResult = Get-Content -LiteralPath (Join-Path $floatRoot 'RESULT.json') -Raw | ConvertFrom-Json
    if ($floatResult.status -cne 'PASS') { throw 'V13_1_FLOAT_RESULT_NOT_PASS' }
    if ($floatResult.candidate_dirty -ne $false) { throw 'V13_1_FLOAT_RESULT_DIRTY' }


    $tagState = 'NOT_REQUIRED'
    if ($RequireTag) {
        $dirty = (Run 'git.exe' @('status', '--porcelain', '--untracked-files=all', '--', '.', ':(exclude)examples/smoke.dby', ':(exclude)examples/.cube-state.bin') 'release cleanliness').Trim()
        if ($dirty.Length -ne 0) { throw 'V13_1_FINAL_WORKTREE_DIRTY' }
        $tagType = (Run 'git.exe' @('cat-file', '-t', 'v13.1.0') 'v13.1.0 tag type').Trim()
        if ($tagType -cne 'tag') { throw 'V13_1_FINAL_TAG_NOT_ANNOTATED' }
        $tagHead = (Run 'git.exe' @('rev-parse', 'v13.1.0^{}') 'v13.1.0 tag target').Trim().ToLowerInvariant()
        if ($tagHead -cne $ExpectedHead) { throw 'V13_1_FINAL_TAG_HEAD_MISMATCH' }
        $tagState = 'ANNOTATED_MATCH'
    }

    $receipt = [ordered]@{
        status = 'PASS'
        version = '13.1.0'
        candidate_head = $ExpectedHead
        frozen_v13_head = $frozenV13Head
        portable_runtime_parity = 'PASS'
        portable_result_sha256 = Hash (Join-Path $portableRoot 'RESULT.json')
        native_explicit_float = 'PASS'
        float_result_sha256 = Hash (Join-Path $floatRoot 'RESULT.json')
        real_storage = 'DISARMED'
        tag = $tagState
    }
    $receiptPath = Join-Path $root 'RESULT.json'
    $receipt | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $receiptPath
    Write-Output 'DBYTE V13.1 FINAL VERIFIER: PASS'
    Write-Output "EVIDENCE ROOT: $root"
    Write-Output "RESULT SHA256: $(Hash $receiptPath)"
}
finally { Pop-Location }
