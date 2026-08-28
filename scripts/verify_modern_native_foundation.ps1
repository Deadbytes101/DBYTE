param([string]$MilestoneVersion = "12.2.0")

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$proofRoot = Join-Path $repoRoot "tmp\native_v$MilestoneVersion\foundation-verifier"
$compiler = Join-Path $repoRoot "target\debug\dbytec.exe"

function Assert-True($value, [string]$message) { if (-not $value) { throw $message } }
function Invoke-Checked([string]$file, [string[]]$arguments, [string]$description) {
    $saved = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    try { $output = & $file @arguments 2>&1; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    if ($code -ne 0) { throw "$description failed ($code):`n$($output -join "`n")" }
    return ($output -join "`n")
}
function Invoke-ExpectedFailure([string[]]$arguments, [string]$output, [string]$code) {
    [IO.File]::WriteAllText($output, "PRESERVE")
    $saved = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    try { $text = & $compiler @arguments 2>&1; $exit = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    Assert-True ($exit -ne 0) "mixed-target link unexpectedly succeeded"
    Assert-True (($text -join "`n").Contains($code)) "mixed-target diagnostic omitted $code"
    Assert-True ([IO.File]::ReadAllText($output) -ceq "PRESERVE") "failed link changed the existing output"
}

Push-Location $repoRoot
try {
    if (Test-Path $proofRoot) { Remove-Item -LiteralPath $proofRoot -Recurse -Force }
    New-Item -ItemType Directory -Path $proofRoot -Force | Out-Null
    Invoke-Checked "cargo.exe" @("test", "-q", "-p", "dbyte_hir", "tests::v122_target_layout_changes_only_target_sized_representations", "--", "--exact") "run target-layout parity regression" | Set-Content (Join-Path $proofRoot "layout-test.txt")
    Invoke-Checked "cargo.exe" @("test", "-q", "-p", "dbyte_native", "modern_native::tests::dbyte64_capturing_closure_uses_hidden_environment", "--", "--exact") "run DBYTE64 closure regression" | Set-Content (Join-Path $proofRoot "closure-test.txt")
    Invoke-Checked "cargo.exe" @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build dbytec" | Out-Null
    $none = Join-Path $proofRoot "none.o"; $linux = Join-Path $proofRoot "linux.o"
    Invoke-Checked $compiler @("build", (Join-Path $repoRoot "examples\native_v12_2_none\bootstrap.dby"), "--target", "x86_64-dbyte-none", "--emit", "obj", "-o", $none) "build none rejection object" | Out-Null
    Invoke-Checked $compiler @("build", (Join-Path $repoRoot "examples\native_v12_2_linux\exit42.dby"), "--target", "x86_64-dbyte-linux", "--emit", "obj", "-o", $linux) "build Linux rejection object" | Out-Null
    Invoke-ExpectedFailure @("link", "--target", "x86_64-dbyte-linux", $none, $linux, "-o", (Join-Path $proofRoot "mixed.elf")) (Join-Path $proofRoot "mixed.elf") "E_TARGET_MISMATCH"

    $wideResult = & (Join-Path $PSScriptRoot "verify_i686_wide_integer.ps1") -MilestoneVersion $MilestoneVersion
    $noneResult = & (Join-Path $PSScriptRoot "verify_modern_native_none.ps1") -MilestoneVersion $MilestoneVersion
    $linuxResult = & (Join-Path $PSScriptRoot "verify_modern_native_linux.ps1") -MilestoneVersion $MilestoneVersion
    $windowsResult = & (Join-Path $PSScriptRoot "verify_modern_native_windows.ps1") -MilestoneVersion $MilestoneVersion
    Assert-True (($wideResult -join "`n").Contains("VERIFIER: PASS")) "i686 wide verifier did not pass"
    Assert-True (($noneResult -join "`n").Contains("VERIFIER: PASS")) "none verifier did not pass"
    Assert-True (($linuxResult -join "`n").Contains("VERIFIER: PASS")) "Linux verifier did not pass"
    Assert-True (($windowsResult -join "`n").Contains("VERIFIER: PASS")) "Windows verifier did not pass"
    [IO.File]::WriteAllText((Join-Path $proofRoot "targets.txt"), (($wideResult + $noneResult + $linuxResult + $windowsResult) -join "`n"))
    "DBYTE MODERN NATIVE FOUNDATION VERIFIER: PASS"
} finally { Pop-Location }
