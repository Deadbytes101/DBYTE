param([string]$MilestoneVersion = "12.2.0")

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$proofRoot = Join-Path $repoRoot "tmp\native_v$MilestoneVersion\windows-verifier"
$source = Join-Path $repoRoot "examples\native_v12_2_windows\runtime_foundation.dby"
$cSource = Join-Path $repoRoot "examples\native_v12_2_windows\c_fixture.c"
$dbyteCSource = Join-Path $repoRoot "examples\native_v12_2_windows\c_fixture.dby"
$compiler = Join-Path $repoRoot "target\debug\dbytec.exe"
$clang = "C:\Program Files\LLVM\bin\clang.exe"
$llvmReadObj = "C:\Program Files\LLVM\bin\llvm-readobj.exe"
$expected = "DBYTE X86_64 WINDOWS`nPE32+ ONLINE`nWIN64 ABI ONLINE`nALLOCATOR ONLINE`nDROP ONLINE`nARRAY ONLINE`nSTRING ONLINE`nC ABI ONLINE`nWINDOWS CHECK OK`n"
$allowedImports = @("ExitProcess", "GetStdHandle", "ReadFile", "WriteFile", "CloseHandle", "VirtualAlloc", "VirtualFree", "QueryPerformanceCounter")

function Assert-True($value, [string]$message) { if (-not $value) { throw $message } }
function Invoke-Checked([string]$file, [string[]]$arguments, [string]$description) {
    $saved = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    try { $output = & $file @arguments 2>&1; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    if ($code -ne 0) { throw "$description failed ($code):`n$($output -join "`n")" }
    return ($output -join "`n")
}
function Assert-Same([string]$left, [string]$right, [string]$name) {
    $a = (Get-FileHash $left -Algorithm SHA256).Hash; $b = (Get-FileHash $right -Algorithm SHA256).Hash
    Assert-True ($a -ceq $b) "$name is not deterministic"; return $a
}
function Build-Set([string]$root) {
    New-Item -ItemType Directory -Path $root -Force | Out-Null
    Invoke-Checked $compiler @("build", $source, "--target", "x86_64-dbyte-windows", "--emit", "obj", "-o", (Join-Path $root "runtime.obj"), "--dump-ir", "--dump-machine-code") "build Windows object" | Set-Content (Join-Path $root "build.txt")
    Invoke-Checked $compiler @("build", $source, "--target", "x86_64-dbyte-windows", "--emit", "staticlib", "-o", (Join-Path $root "runtime.a")) "build Windows archive" | Out-Null
    Invoke-Checked $compiler @("build", $source, "--target", "x86_64-dbyte-windows", "--emit", "exe", "-o", (Join-Path $root "runtime.exe")) "build Windows PE" | Out-Null
    Invoke-Checked $compiler @("link", "--target", "x86_64-dbyte-windows", (Join-Path $root "runtime.obj"), "-o", (Join-Path $root "linked.exe")) "link Windows object" | Out-Null
    Invoke-Checked $compiler @("link", "--target", "x86_64-dbyte-windows", (Join-Path $root "runtime.a"), "-o", (Join-Path $root "archive-linked.exe")) "link Windows archive" | Out-Null
}
function Assert-Executable([string]$path, [string]$name) {
    $output = Invoke-Checked $path @() "execute $name"
    Assert-True (($output + "`n") -ceq $expected) "$name output diverged"
    [IO.File]::WriteAllText((Join-Path $proofRoot "$name-output.txt"), $output + "`n")
}

Push-Location $repoRoot
try {
    if (Test-Path $proofRoot) { Remove-Item -LiteralPath $proofRoot -Recurse -Force }
    New-Item -ItemType Directory -Path $proofRoot -Force | Out-Null
    Assert-True (Test-Path $clang) "pinned LLVM clang is unavailable"
    Assert-True (Test-Path $llvmReadObj) "llvm-readobj is unavailable"
    Invoke-Checked "cargo.exe" @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build dbytec" | Out-Null
    Build-Set (Join-Path $proofRoot "first"); Build-Set (Join-Path $proofRoot "second")
    $hashes = foreach ($name in @("runtime.obj", "runtime.a", "runtime.exe", "linked.exe", "archive-linked.exe")) { "$(Assert-Same (Join-Path $proofRoot "first\$name") (Join-Path $proofRoot "second\$name") $name)  $name" }
    Assert-True ((Get-FileHash (Join-Path $proofRoot "first\runtime.exe") -Algorithm SHA256).Hash -ceq (Get-FileHash (Join-Path $proofRoot "first\archive-linked.exe") -Algorithm SHA256).Hash) "Windows archive extraction changed the final image"
    $pe = Join-Path $proofRoot "first\runtime.exe"
    $inspection = Invoke-Checked $llvmReadObj @("--file-headers", "--sections", "--coff-imports", "--coff-basereloc", $pe) "inspect Windows PE"
    Assert-True ($inspection.Contains("IMAGE_FILE_MACHINE_AMD64")) "PE is not AMD64"
    Assert-True ($inspection.Contains("Name: KERNEL32.dll")) "PE does not import Kernel32"
    $dlls = [regex]::Matches($inspection, 'Name: ([^\r\n]+\.dll)') | ForEach-Object { $_.Groups[1].Value }
    Assert-True (@($dlls | Where-Object { $_ -cne "KERNEL32.dll" }).Count -eq 0) "PE imports a non-allowlisted DLL"
    $symbols = [regex]::Matches($inspection, 'Symbol: ([A-Za-z0-9_]+)') | ForEach-Object { $_.Groups[1].Value }
    Assert-True (@($symbols | Where-Object { $_ -notin $allowedImports }).Count -eq 0) "PE imports a non-allowlisted function"
    [IO.File]::WriteAllText((Join-Path $proofRoot "llvm-readobj.txt"), $inspection)
    $objectInspection = Invoke-Checked $llvmReadObj @("--file-headers", "--sections", "--symbols", "--relocations", (Join-Path $proofRoot "first\runtime.obj")) "inspect Windows object"
    Assert-True ($objectInspection.Contains(".dbyte.target")) "Windows object lacks target metadata"
    Assert-True ($objectInspection.Contains("IMAGE_REL_AMD64")) "Windows object lacks AMD64 relocations"
    [IO.File]::WriteAllText((Join-Path $proofRoot "object-llvm-readobj.txt"), $objectInspection)
    Assert-Executable $pe "runtime"; Assert-Executable (Join-Path $proofRoot "first\linked.exe") "linked"; Assert-Executable (Join-Path $proofRoot "first\archive-linked.exe") "archive-linked"

    $cObject = Join-Path $proofRoot "fixture-c.obj"; $dbyteObject = Join-Path $proofRoot "fixture-dbyte.obj"; $cExe = Join-Path $proofRoot "fixture.exe"
    Invoke-Checked $clang @("--target=x86_64-pc-windows-msvc", "-O2", "-fno-omit-frame-pointer", "-ffreestanding", "-fno-stack-protector", "-fno-builtin", "-c", $cSource, "-o", $cObject) "compile Win64 fixture" | Out-Null
    Invoke-Checked $compiler @("build", $dbyteCSource, "--target", "x86_64-dbyte-windows", "--emit", "obj", "-o", $dbyteObject) "build DByte Win64 fixture" | Out-Null
    Invoke-Checked $compiler @("link", "--target", "x86_64-dbyte-windows", $dbyteObject, $cObject, "-o", $cExe) "link Win64 fixture internally" | Out-Null
    $cOutput = Invoke-Checked $cExe @() "execute Win64 fixture"
    Assert-True (($cOutput -join "`r`n") -ceq "DBYTE WIN64 C ABI OK") "Win64 C ABI output diverged"
    $cInspection = Invoke-Checked $llvmReadObj @("--symbols", "--relocations", $cObject) "inspect Win64 C fixture"
    Assert-True ($cInspection.Contains("dbyte_callback")) "Win64 fixture does not reference the nested DByte callback"
    [IO.File]::WriteAllText((Join-Path $proofRoot "fixture-c-inspection.txt"), $cInspection)
    [IO.File]::WriteAllLines((Join-Path $proofRoot "SHA256SUMS"), $hashes)
    "DBYTE MODERN NATIVE WINDOWS VERIFIER: PASS"; $hashes
} finally { Pop-Location }
