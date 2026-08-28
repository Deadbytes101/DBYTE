param([string]$MilestoneVersion = "12.2.0")

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$proofRoot = Join-Path $repoRoot "tmp\native_v$MilestoneVersion\linux-verifier"
$source = Join-Path $repoRoot "examples\native_v12_2_linux\runtime_foundation.dby"
$cSource = Join-Path $repoRoot "examples\native_v12_2_linux\c_fixture.c"
$dbyteCSource = Join-Path $repoRoot "examples\native_v12_2_linux\c_fixture.dby"
$compiler = Join-Path $repoRoot "target\debug\dbytec.exe"
$clang = "C:\Program Files\LLVM\bin\clang.exe"
$expected = "DBYTE X86_64 LINUX`nELF64 ONLINE`nLINUX SYSCALL ONLINE`nALLOCATOR ONLINE`nDROP ONLINE`nARRAY ONLINE`nSTRING ONLINE`nC ABI ONLINE`nLINUX CHECK OK`n"

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
function Wsl-Path([string]$path) {
    return (Invoke-Checked "wsl.exe" @("-d", "Debian", "--", "wslpath", "-a", "-u", $path.Replace('\', '/')) "translate WSL path").Trim()
}
function Build-Set([string]$root) {
    New-Item -ItemType Directory -Path $root -Force | Out-Null
    Invoke-Checked $compiler @("build", $source, "--target", "x86_64-dbyte-linux", "--emit", "obj", "-o", (Join-Path $root "runtime.o"), "--dump-ir", "--dump-machine-code") "build Linux object" | Set-Content (Join-Path $root "build.txt")
    Invoke-Checked $compiler @("build", $source, "--target", "x86_64-dbyte-linux", "--emit", "staticlib", "-o", (Join-Path $root "runtime.a")) "build Linux archive" | Out-Null
    Invoke-Checked $compiler @("build", $source, "--target", "x86_64-dbyte-linux", "--emit", "elf", "-o", (Join-Path $root "runtime.elf")) "build Linux ELF" | Out-Null
    Invoke-Checked $compiler @("link", "--target", "x86_64-dbyte-linux", (Join-Path $root "runtime.o"), "-o", (Join-Path $root "linked.elf")) "link Linux object" | Out-Null
    Invoke-Checked $compiler @("link", "--target", "x86_64-dbyte-linux", (Join-Path $root "runtime.a"), "-o", (Join-Path $root "archive-linked.elf")) "link Linux archive" | Out-Null
}

Push-Location $repoRoot
try {
    if (Test-Path $proofRoot) { Remove-Item -LiteralPath $proofRoot -Recurse -Force }
    New-Item -ItemType Directory -Path $proofRoot -Force | Out-Null
    Assert-True (Test-Path $clang) "pinned LLVM clang is unavailable"
    Invoke-Checked "cargo.exe" @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build dbytec" | Out-Null
    Build-Set (Join-Path $proofRoot "first"); Build-Set (Join-Path $proofRoot "second")
    $hashes = foreach ($name in @("runtime.o", "runtime.a", "runtime.elf", "linked.elf", "archive-linked.elf")) { "$(Assert-Same (Join-Path $proofRoot "first\$name") (Join-Path $proofRoot "second\$name") $name)  $name" }
    Assert-True ((Get-FileHash (Join-Path $proofRoot "first\runtime.elf") -Algorithm SHA256).Hash -ceq (Get-FileHash (Join-Path $proofRoot "first\archive-linked.elf") -Algorithm SHA256).Hash) "Linux archive extraction changed the final image"
    Assert-True ((Get-FileHash (Join-Path $proofRoot "first\runtime.elf") -Algorithm SHA256).Hash -ceq (Get-FileHash (Join-Path $proofRoot "first\linked.elf") -Algorithm SHA256).Hash) "Linux object path changed the final image"
    $objectDump = Invoke-Checked "objdump.exe" @("-f", "-h", "-t", "-r", "-d", "-Mintel", (Join-Path $proofRoot "first\runtime.o")) "inspect Linux object"
    Assert-True ($objectDump.Contains(".dbyte.target")) "Linux object lacks target metadata"
    Assert-True ($objectDump.Contains(".dbyte.entry")) "Linux object lacks entry metadata"
    Assert-True ($objectDump.Contains("R_X86_64_")) "Linux object lacks real x86_64 relocations"
    [IO.File]::WriteAllText((Join-Path $proofRoot "object-objdump.txt"), $objectDump)
    $elf = Join-Path $proofRoot "first\runtime.elf"; $elfWsl = Wsl-Path $elf
    $inspection = Invoke-Checked "wsl.exe" @("-d", "Debian", "--", "readelf", "-h", "-l", "-r", "-d", $elfWsl) "inspect Linux ELF"
    Assert-True ($inspection.Contains("Advanced Micro Devices X86-64")) "Linux ELF is not EM_X86_64"
    Assert-True (-not $inspection.Contains("INTERP")) "Linux ELF has PT_INTERP"
    Assert-True ($inspection.Contains("There is no dynamic section")) "Linux ELF has a dynamic runtime"
    Assert-True (-not $inspection.Contains(".dbyte.entry")) "Linux final ELF retained entry metadata"
    [IO.File]::WriteAllText((Join-Path $proofRoot "readelf.txt"), $inspection)
    $output = Invoke-Checked "wsl.exe" @("-d", "Debian", "--", $elfWsl) "execute Linux runtime"
    Assert-True ((($output + "`n") -ceq $expected)) "Linux runtime output diverged"
    [IO.File]::WriteAllText((Join-Path $proofRoot "runtime-output.txt"), $output + "`n")
    $linkedOutput = Invoke-Checked "wsl.exe" @("-d", "Debian", "--", (Wsl-Path (Join-Path $proofRoot "first\linked.elf"))) "execute linked Linux runtime"
    Assert-True ((($linkedOutput + "`n") -ceq $expected)) "linked Linux runtime output diverged"
    $archiveOutput = Invoke-Checked "wsl.exe" @("-d", "Debian", "--", (Wsl-Path (Join-Path $proofRoot "first\archive-linked.elf"))) "execute archive-linked Linux runtime"
    Assert-True ((($archiveOutput + "`n") -ceq $expected)) "archive-linked Linux runtime output diverged"

    $cObject = Join-Path $proofRoot "fixture-c.o"; $dbyteObject = Join-Path $proofRoot "fixture-dbyte.o"; $cElf = Join-Path $proofRoot "fixture.elf"
    Invoke-Checked $clang @("--target=x86_64-none-elf", "-O2", "-fno-omit-frame-pointer", "-ffreestanding", "-fno-pic", "-fno-pie", "-fno-stack-protector", "-fno-builtin", "-c", $cSource, "-o", $cObject) "compile SysV fixture" | Out-Null
    Invoke-Checked $compiler @("build", $dbyteCSource, "--target", "x86_64-dbyte-linux", "--emit", "obj", "-o", $dbyteObject) "build DByte SysV fixture" | Out-Null
    Invoke-Checked $compiler @("link", "--target", "x86_64-dbyte-linux", $dbyteObject, $cObject, "-o", $cElf) "link SysV fixture internally" | Out-Null
    $cOutput = Invoke-Checked "wsl.exe" @("-d", "Debian", "--", (Wsl-Path $cElf)) "execute SysV fixture"
    Assert-True (($cOutput + "`n") -ceq "DBYTE SYSV C ABI OK`n") "SysV C ABI output diverged"
    $cDisassembly = Invoke-Checked "objdump.exe" @("-d", "-Mintel", $cObject) "disassemble SysV fixture"
    Assert-True ($cDisassembly.Contains("call")) "SysV fixture does not contain the nested C-to-DByte callback"
    [IO.File]::WriteAllText((Join-Path $proofRoot "fixture-c-disassembly.txt"), $cDisassembly)
    $entryElf = Join-Path $proofRoot "entry-args.elf"
    $entryObject = Join-Path $proofRoot "entry-args.o"
    $entryLinked = Join-Path $proofRoot "entry-args-linked.elf"
    Invoke-Checked $compiler @("build", (Join-Path $repoRoot "examples\native_v12_2_linux\entry_args.dby"), "--target", "x86_64-dbyte-linux", "--emit", "elf", "-o", $entryElf) "build Linux argc entry" | Out-Null
    Invoke-Checked $compiler @("build", (Join-Path $repoRoot "examples\native_v12_2_linux\entry_args.dby"), "--target", "x86_64-dbyte-linux", "--emit", "obj", "-o", $entryObject) "build Linux argc object" | Out-Null
    Invoke-Checked $compiler @("link", "--target", "x86_64-dbyte-linux", $entryObject, "-o", $entryLinked) "link Linux argc object" | Out-Null
    $saved = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    try { & wsl.exe -d Debian -- (Wsl-Path $entryElf) alpha beta | Out-Null; $entryExit = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    Assert-True ($entryExit -eq 3) "Linux argc/argv/envp entry returned $entryExit instead of argc 3"
    $saved = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    try { & wsl.exe -d Debian -- (Wsl-Path $entryLinked) alpha beta | Out-Null; $linkedEntryExit = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    Assert-True ($linkedEntryExit -eq 3) "linked Linux argc metadata returned $linkedEntryExit instead of argc 3"
    [IO.File]::WriteAllLines((Join-Path $proofRoot "SHA256SUMS"), $hashes)
    "DBYTE MODERN NATIVE LINUX VERIFIER: PASS"; $hashes
} finally { Pop-Location }
