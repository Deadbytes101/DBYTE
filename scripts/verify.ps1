$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")

Set-Location $repoRoot

$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"

& $cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cargo clippy --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cargo test
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cargo run -q -p dbyte_cli -- test
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "Running project workflow tests..."

$cli = Join-Path $repoRoot "target\debug\dbyte.exe"

function Normalize-Output($value) {
    return (($value -join "`n") -replace "`r`n", "`n").Trim()
}

function Assert-Equal($actual, $expected, $name) {
    if ($actual -ne $expected) {
        throw "$name failed: expected '$expected', got '$actual'"
    }
}

function Assert-Contains($actual, $expected, $name) {
    if (-not $actual.Contains($expected)) {
        throw "$name failed: expected output to contain '$expected', got '$actual'"
    }
}

function Invoke-Dbyte {
    param([string[]]$Arguments)

    $oldPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $cli @Arguments 2>&1
        $code = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $oldPreference
    }
    return [PSCustomObject]@{
        Code = $code
        Text = Normalize-Output $output
    }
}

Push-Location (Join-Path $repoRoot "tests\project\basic")
try {
    $result = Invoke-Dbyte -Arguments @("run")
    if ($result.Code -ne 0) { throw "basic project run failed: $($result.Text)" }
    $expected = (Get-Content "expected.out" -Raw).Trim()
    Assert-Equal $result.Text $expected "basic project run"
    $checkResult = Invoke-Dbyte -Arguments @("check")
    if ($checkResult.Code -ne 0) { throw "basic project check failed: $($checkResult.Text)" }
    Assert-Contains $checkResult.Text "no type errors found" "basic project check"
}
finally {
    Pop-Location
}

Push-Location (Join-Path $repoRoot "tests\project\missing_manifest")
try {
    $result = Invoke-Dbyte -Arguments @("run")
    if ($result.Code -eq 0) { throw "missing manifest project unexpectedly passed" }
    $expected = (Get-Content "expected.err" -Raw).Trim()
    Assert-Contains $result.Text $expected "missing manifest project"
}
finally {
    Pop-Location
}

Push-Location (Join-Path $repoRoot "tests\project\missing_entry")
try {
    $result = Invoke-Dbyte -Arguments @("run")
    if ($result.Code -eq 0) { throw "missing entry project unexpectedly passed" }
    $expected = (Get-Content "expected.err" -Raw).Trim()
    Assert-Contains $result.Text $expected "missing entry project"
}
finally {
    Pop-Location
}

Push-Location (Join-Path $repoRoot "tests\project\invalid_manifest")
try {
    $result = Invoke-Dbyte -Arguments @("run")
    if ($result.Code -eq 0) { throw "invalid manifest project unexpectedly passed" }
    $expected = (Get-Content "expected.err" -Raw).Trim()
    Assert-Contains $result.Text $expected "invalid manifest project"
}
finally {
    Pop-Location
}

Push-Location (Join-Path $repoRoot "tests\project\nested_run\src\tools")
try {
    $result = Invoke-Dbyte -Arguments @("run")
    if ($result.Code -ne 0) { throw "nested project run failed: $($result.Text)" }
    $expected = (Get-Content "..\..\expected.out" -Raw).Trim()
    Assert-Equal $result.Text $expected "nested project run"
}
finally {
    Pop-Location
}

$newRoot = Join-Path $repoRoot "target\verify-project-new"
if (Test-Path $newRoot) {
    Remove-Item -Recurse -Force $newRoot
}
New-Item -ItemType Directory -Path $newRoot | Out-Null
Push-Location $newRoot
try {
    $result = Invoke-Dbyte -Arguments @("new", "scanner")
    if ($result.Code -ne 0) { throw "dbyte new failed: $($result.Text)" }
    Push-Location "scanner"
    try {
        $runResult = Invoke-Dbyte -Arguments @("run")
        if ($runResult.Code -ne 0) { throw "new project run failed: $($runResult.Text)" }
        Assert-Equal $runResult.Text "hello from scanner" "new project run"
        $testResult = Invoke-Dbyte -Arguments @("test")
        if ($testResult.Code -ne 0) { throw "new project test failed: $($testResult.Text)" }
        Assert-Contains $testResult.Text "Test result: 1 passed, 0 failed" "new project test"
    }
    finally {
        Pop-Location
    }
}
finally {
    Pop-Location
}

Write-Host "verify passed"
