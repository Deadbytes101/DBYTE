param(
    [Parameter(Mandatory=$true)][string]$ExpectedHead,
    [switch]$RequireTag
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'v13_release_identity.ps1')
$ExpectedHead = $ExpectedHead.Trim().ToLowerInvariant()
if ($ExpectedHead -notmatch '^[0-9a-f]{40}$') { throw 'V13_FINAL_HEAD_INVALID' }
$root = Join-Path $repo ('tmp/v13_final/final-' + [Guid]::NewGuid().ToString('N'))
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
    try { $out = & $Exe @Arguments 2>&1; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    [IO.File]::WriteAllText($log, "COMMAND=$Exe $($Arguments -join ' ')`nEXIT=$code`n$($out -join "`n")")
    if ($code -ne 0) { throw "$Description failed ($code): $log" }
    return ($out -join "`n")
}
function Evidence-Root([string]$Output, [string]$Label) {
    $line = @($Output -split "`r?`n" | Where-Object { $_.StartsWith('EVIDENCE ROOT:') }) | Select-Object -Last 1
    if (-not $line) { throw "$Label evidence root missing" }
    $path = $line.Substring('EVIDENCE ROOT:'.Length).Trim()
    if (-not (Test-Path -LiteralPath (Join-Path $path 'RESULT.json'))) {
        throw "$Label RESULT.json missing: $path"
    }
    return $path
}

Push-Location $repo
$oldExpected = $env:DBYTE_V13_EXPECTED_HEAD
try {
    $head = (Run 'git.exe' @('rev-parse','HEAD') 'read HEAD').Trim().ToLowerInvariant()
    if ($head -cne $ExpectedHead) { throw "V13_FINAL_HEAD_MISMATCH: $head" }
    $frozen = (Run 'git.exe' @('rev-parse','v12.4.0^{}') 'read frozen tag').Trim().ToLowerInvariant()
    if ($frozen -cne $V13FrozenBaseHead) { throw 'V13_FROZEN_BASE_DRIFT' }
    $env:DBYTE_V13_EXPECTED_HEAD = $ExpectedHead
    Run 'git.exe' @('diff','--check') 'diff check' | Out-Null
    $version = (Run 'cargo.exe' @('run','-q','-p','dbyte_cli','--','--version') 'version probe').Trim()
    if ($version -cne 'DByte 13.0.0') { throw "V13_FINAL_VERSION_MISMATCH: $version" }

    $boundary = Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $PSScriptRoot 'verify_v13_historical_boundary.ps1')) 'frozen boundary'
    [IO.File]::WriteAllText((Join-Path $root 'historical-boundary.txt'), $boundary)
    $inputOut = Run 'python.exe' @((Join-Path $PSScriptRoot 'verify_v13_generation4_input.py')) 'G4 input master'
    $inputRoot = Evidence-Root $inputOut 'G4 input'
    $dbfsOut = Run 'python.exe' @((Join-Path $PSScriptRoot 'verify_v13_generation4_dbfs_persistence.py')) 'G4 DBFS persistence master'
    $dbfsRoot = Evidence-Root $dbfsOut 'G4 DBFS persistence'
    $g5Out = Run 'python.exe' @((Join-Path $PSScriptRoot 'verify_v13_generation5.py')) 'G5 aggregate master'
    $g5Root = Evidence-Root $g5Out 'G5 aggregate'

    foreach ($pair in @(
        @('G4 input',$inputRoot),
        @('G4 DBFS persistence',$dbfsRoot),
        @('G5 aggregate',$g5Root)
    )) {
        $result = Get-Content -Raw -LiteralPath (Join-Path $pair[1] 'RESULT.json') | ConvertFrom-Json
        if ($result.status -cne 'PASS') { throw "$($pair[0]) status is not PASS" }
        if ($result.head -cne $ExpectedHead) { throw "$($pair[0]) result HEAD drift" }
        if ($result.real_storage -cne 'DISARMED') { throw "$($pair[0]) armed real storage" }
    }

    $tagState = 'NOT_REQUIRED'
    if ($RequireTag) {
        $dirty = (Run 'git.exe' @('status','--porcelain') 'release cleanliness').Trim()
        if ($dirty.Length -ne 0) { throw 'V13_FINAL_WORKTREE_DIRTY' }
        $tagType = (Run 'git.exe' @('cat-file','-t','v13.0.0') 'v13 tag type').Trim()
        if ($tagType -cne 'tag') { throw 'V13_FINAL_TAG_NOT_ANNOTATED' }
        $tagHead = (Run 'git.exe' @('rev-parse','v13.0.0^{}') 'v13 tag target').Trim().ToLowerInvariant()
        if ($tagHead -cne $ExpectedHead) { throw 'V13_FINAL_TAG_HEAD_MISMATCH' }
        $tagState = 'ANNOTATED_MATCH'
    }
    $receipt = [ordered]@{
        status = 'PASS'
        version = '13.0.0'
        candidate_head = $ExpectedHead
        frozen_v12_4_head = $V13FrozenBaseHead
        historical_boundary = 'PASS'
        g4_input = 'PASS_QEMU'
        g4_input_result_sha256 = Hash (Join-Path $inputRoot 'RESULT.json')
        g4_dbfs_persistence = 'PASS_QEMU'
        g4_dbfs_result_sha256 = Hash (Join-Path $dbfsRoot 'RESULT.json')
        g5_aggregate = 'PASS_QEMU'
        g5_result_sha256 = Hash (Join-Path $g5Root 'RESULT.json')
        tag = $tagState
        real_biostar = 'PENDING'
        real_storage = 'DISARMED'
    }
    $receiptPath = Join-Path $root 'RESULT.json'
    $receipt | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $receiptPath
    Write-Output 'V13 FINAL SOFTWARE BASELINE VERIFIER: PASS'
    Write-Output "EVIDENCE ROOT: $root"
    Write-Output "RESULT SHA256: $(Hash $receiptPath)"
}
finally {
    $env:DBYTE_V13_EXPECTED_HEAD = $oldExpected
    Pop-Location
}
