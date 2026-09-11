$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$helper = Join-Path $PSScriptRoot 'verify_git_boundary.ps1'
if (-not (Test-Path -LiteralPath $helper -PathType Leaf)) {
    throw "verify Git boundary helper is missing: $helper"
}
. $helper
Set-Location $repoRoot

$resolvedGitExe = @(Get-Command git.exe -CommandType Application)[0].Source
$resolvedGitNormalized = ([string]$resolvedGitExe).Trim('"').TrimEnd('\\')
$boundaryGitNormalized = ([string]$global:DbyteVerifyGitExecutable).Trim('"').TrimEnd('\\')
if ($resolvedGitNormalized -ine $boundaryGitNormalized) {
    throw "git.exe did not resolve to bounded boundary executable: $resolvedGitExe"
}

function Assert-Equal([object]$actual, [object]$expected, [string]$name) {
    $a = @($actual) -join "`n"
    $e = @($expected) -join "`n"
    if ($a -cne $e) { throw "$name mismatch: '$a' != '$e'" }
}

$directHead = & git.exe rev-parse HEAD
$wrappedHead = & git rev-parse HEAD
Assert-Equal $wrappedHead $directHead 'rev-parse HEAD'
if ($LASTEXITCODE -ne 0) { throw 'wrapped rev-parse did not preserve exit 0' }

$directTag = & git.exe rev-list -n 1 v9.1.0
$wrappedTag = & git rev-list -n 1 v9.1.0
Assert-Equal $wrappedTag $directTag 'rev-list tag'
$directType = & git.exe cat-file -t HEAD
$wrappedType = & git cat-file -t HEAD
Assert-Equal $wrappedType $directType 'cat-file HEAD'

$directNames = & git.exe diff --name-only v10.59.0 v10.60.0 --
$wrappedNames = & git diff --name-only v10.59.0 v10.60.0 --
Assert-Equal $wrappedNames $directNames 'diff --name-only'

$directStatus = & git.exe -C $repoRoot status --porcelain
$wrappedStatus = & git -C $repoRoot status --porcelain
Assert-Equal $wrappedStatus $directStatus 'git -C status'

$savedPreference = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
try {
    $directMissing = & git.exe rev-parse refs/tags/__dbyte_missing_ref__ 2>$null
    $directMissingExit = $LASTEXITCODE
    $wrappedMissing = & git rev-parse refs/tags/__dbyte_missing_ref__ 2>$null
    $wrappedMissingExit = $LASTEXITCODE
}
finally { $ErrorActionPreference = $savedPreference }
Assert-Equal $wrappedMissing $directMissing 'missing-ref stdout'
if ($wrappedMissingExit -ne $directMissingExit -or $wrappedMissingExit -eq 0) {
    throw 'missing ref did not preserve nonzero exit semantics'
}

$nestedScript = Join-Path $env:TEMP ("dbyte-verify-git-child-" + [guid]::NewGuid().ToString('N') + '.ps1')
try {
    [IO.File]::WriteAllText($nestedScript, '& git rev-parse HEAD' + [Environment]::NewLine)
    $nestedHead = & $nestedScript
    Assert-Equal $nestedHead $directHead 'nested script rev-parse HEAD'
    if ($LASTEXITCODE -ne 0) { throw 'nested script Git boundary did not preserve exit 0' }
}
finally { Remove-Item -LiteralPath $nestedScript -Force -ErrorAction SilentlyContinue }

$oldExe = $global:DbyteVerifyGitExecutable
$oldTimeout = $global:DbyteVerifyGitTimeoutMs
try {
    $global:DbyteVerifyGitExecutable = (Get-Command powershell.exe -CommandType Application).Source
    $global:DbyteVerifyGitTimeoutMs = 100
    $timedOut = $false
    try { & git -NoProfile -Command 'Start-Sleep -Milliseconds 750' | Out-Null }
    catch { $timedOut = $_.Exception.Message -like 'verify git timeout*' }
    if (-not $timedOut -or $LASTEXITCODE -ne 124) { throw 'bounded timeout contract failed' }
}
finally {
    $global:DbyteVerifyGitExecutable = $oldExe
    $global:DbyteVerifyGitTimeoutMs = $oldTimeout
}

$global:LASTEXITCODE = 0
Write-Host 'VERIFY GIT BOUNDARY TESTS: PASS'
