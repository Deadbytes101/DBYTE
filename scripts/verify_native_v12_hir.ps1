param([string]$MilestoneVersion = "12.0.0")

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$sourceRoot = Join-Path $repoRoot "examples\native_v12_hir"
$proofRoot = Join-Path $repoRoot "tmp\native_v$MilestoneVersion\hir-smoke"
$firstRoot = Join-Path $proofRoot "first"
$secondRoot = Join-Path $proofRoot "second"
$userFirstRoot = Join-Path $proofRoot "user-first"
$userSecondRoot = Join-Path $proofRoot "user-second"
$compiler = Join-Path $repoRoot "target\debug\dbytec.exe"
$modules = @("arch_x86", "serial", "kernel")
$clang = "C:\Program Files\LLVM\bin\clang.exe"
$userCore = Join-Path $repoRoot "examples\native_user_process_core"
$userModules = @(
    "arch_x86", "ata", "block_cache", "boot", "byte_ring", "dbfs", "dbfs_bitmap",
    "dbfs_dir", "dbfs_inode", "dbfs_layout", "file_handles", "filesystem_runtime",
    "frames", "gdt", "heap", "idt_normal", "keyboard", "memory_ops", "paging", "path",
    "pic", "pit", "process_context", "processes", "proof", "scheduler", "serial", "sha256",
    "tasks", "user_bundle", "kernel"
)
$legacyUserPrograms = @("hostile", "isoa", "isob", "pagefault", "survive")

function Assert-True($condition, [string]$message) {
    if (-not $condition) { throw $message }
}

function Invoke-Checked([string]$executable, [string[]]$arguments, [string]$description) {
    $old = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $executable @arguments 2>&1
        $exitCode = $LASTEXITCODE
    }
    finally { $ErrorActionPreference = $old }
    if ($exitCode -ne 0) { throw "$description failed (${exitCode}):`n$($output -join "`n")" }
    return ($output -join "`n")
}

function Assert-Same([string]$left, [string]$right, [string]$description) {
    $leftHash = (Get-FileHash -LiteralPath $left -Algorithm SHA256).Hash
    $rightHash = (Get-FileHash -LiteralPath $right -Algorithm SHA256).Hash
    Assert-True ($leftHash -eq $rightHash) "$description is not deterministic"
    return $leftHash
}

function Read-SharedBytes([string]$path) {
    $stream = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
    try {
        [byte[]]$bytes = New-Object byte[] $stream.Length
        [void]$stream.Read($bytes, 0, $bytes.Length)
        return ,$bytes
    } finally { $stream.Dispose() }
}

function Patch-Hostile([string]$inputPath, [string]$outputPath) {
    [byte[]]$bytes = [IO.File]::ReadAllBytes($inputPath)
    $offset = -1
    for ($index = 0; $index -lt $bytes.Length - 1; $index++) {
        if ($bytes[$index] -eq 0xcd -and $bytes[$index + 1] -eq 0x80) { $offset = $index; break }
    }
    Assert-True ($offset -ge 0) "hostile user fixture lacks INT 80"
    $bytes[$offset] = 0xee
    $bytes[$offset + 1] = 0x90
    [IO.File]::WriteAllBytes($outputPath, $bytes)
}

function Build-UserSet([string]$destination) {
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    foreach ($module in $userModules) {
        Invoke-Checked $compiler @(
            "build", (Join-Path $userCore "$module.dby"), "--target", "i686-dbyte-none",
            "--emit", "obj", "-o", (Join-Path $destination "$module.o")
        ) "build user-kernel $module.o" | Out-Null
    }
    $kernelObjects = @($userModules | ForEach-Object { Join-Path $destination "$_.o" })
    Invoke-Checked $compiler (@("link") + $kernelObjects + @("-o", (Join-Path $destination "user-kernel.elf"))) "link user kernel" | Out-Null

    Invoke-Checked $compiler @(
        "build", (Join-Path $sourceRoot "user.dby"), "--target", "i686-dbyte-user",
        "--emit", "obj", "-o", (Join-Path $destination "hello.user.o"), "--dump-ir"
    ) "build v12 user object" | Set-Content -LiteralPath (Join-Path $destination "hello.ir.txt")
    Invoke-Checked $compiler @(
        "build", (Join-Path $sourceRoot "user.dby"), "--target", "i686-dbyte-user",
        "--emit", "elf", "-o", (Join-Path $destination "hello.elf")
    ) "build v12 user ELF" | Out-Null
    foreach ($program in $legacyUserPrograms) {
        Invoke-Checked $compiler @(
            "build", (Join-Path $userCore "programs\$program.dby"), "--target", "i686-dbyte-user",
            "--emit", "elf", "-o", (Join-Path $destination "$program.elf")
        ) "build user program $program" | Out-Null
    }
    Patch-Hostile (Join-Path $destination "hostile.elf") (Join-Path $destination "hostile-fixture.elf")
    $bundleInputs = @(
        "HELLO.ELF=$(Join-Path $destination 'hello.elf')",
        "HOSTILE.ELF=$(Join-Path $destination 'hostile-fixture.elf')",
        "ISOA.ELF=$(Join-Path $destination 'isoa.elf')",
        "ISOB.ELF=$(Join-Path $destination 'isob.elf')",
        "PAGEFAULT.ELF=$(Join-Path $destination 'pagefault.elf')",
        "SURVIVE.ELF=$(Join-Path $destination 'survive.elf')"
    )
    Invoke-Checked $compiler (@("pack-user") + $bundleInputs + @("-o", (Join-Path $destination "programs.dbup"))) "pack v12 user bundle" | Out-Null
}

function Invoke-UserProof([int]$ram, [string]$root) {
    $runRoot = Join-Path $proofRoot "user-${ram}m"
    New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
    $disk = Join-Path $runRoot "disk.raw"
    if (Test-Path -LiteralPath $disk) { Remove-Item -LiteralPath $disk -Force }
    $stream = [IO.File]::Open($disk, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write)
    try { $stream.SetLength(8MB) } finally { $stream.Dispose() }
    $serial = Join-Path $runRoot "serial.log"
    $trace = Join-Path $runRoot "qemu.log"
    foreach ($path in @($serial, $trace)) { if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force } }
    $arguments = @(
        "-kernel", (Join-Path $root "user-kernel.elf"), "-initrd", (Join-Path $root "programs.dbup"),
        "-drive", "file=$disk,format=raw,if=ide,index=0,media=disk,cache=directsync", "-m", "${ram}M",
        "-display", "none", "-monitor", "none", "-serial", "file:$serial",
        "-d", "int,cpu_reset,guest_errors", "-D", $trace, "-no-reboot", "-no-shutdown"
    )
    [IO.File]::WriteAllText((Join-Path $runRoot "qemu-command.txt"), "$qemu $($arguments -join ' ')")
    $process = Start-Process -FilePath $qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(22)
        $matched = $false
        while ([DateTime]::UtcNow -lt $deadline -and -not $process.HasExited) {
            Start-Sleep -Milliseconds 100
            if (Test-Path -LiteralPath $serial) {
                $current = [Text.Encoding]::ASCII.GetString((Read-SharedBytes $serial))
                if ($current -ceq $script:userExpected) { $matched = $true; break }
                Assert-True ($script:userExpected.StartsWith($current, [StringComparison]::Ordinal)) "${ram}M v12 user serial diverged"
            }
        }
        Assert-True $matched "${ram}M v12 user program did not complete"
    } finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
    }
    $traceText = Get-Content -LiteralPath $trace -Raw
    Assert-True (([regex]::Matches($traceText, 'v=80')).Count -eq 19) "${ram}M syscall count changed"
    Assert-True (([regex]::Matches($traceText, 'v=0d')).Count -eq 1) "${ram}M controlled #GP count changed"
    Assert-True (([regex]::Matches($traceText, 'v=0e')).Count -eq 1) "${ram}M controlled #PF count changed"
    Assert-True (-not $traceText.Contains("Triple fault")) "${ram}M v12 user proof triple-faulted"
}

function Build-Set([string]$destination) {
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    foreach ($module in $modules) {
        $dump = Invoke-Checked $compiler @(
            "build", (Join-Path $sourceRoot "$module.dby"),
            "--target", "i686-dbyte-none", "--emit", "obj",
            "-o", (Join-Path $destination "$module.o"), "--dump-ir"
        ) "build $module.o"
        [System.IO.File]::WriteAllText((Join-Path $destination "$module.ir.txt"), $dump)
    }
    Invoke-Checked $compiler @(
        "build", (Join-Path $sourceRoot "c_archive.dby"),
        "--target", "i686-dbyte-none", "--emit", "staticlib",
        "-o", (Join-Path $destination "libdbyte.a")
    ) "build DByte static archive" | Out-Null
    Invoke-Checked $compiler @(
        "link",
        (Join-Path $destination "arch_x86.o"),
        (Join-Path $destination "serial.o"),
        (Join-Path $destination "kernel.o"),
        (Join-Path $destination "c_fixture.o"),
        (Join-Path $destination "libdbyte.a"),
        "-o", (Join-Path $destination "kernel.elf")
    ) "link C ABI proof ELF" | Out-Null
}

$qemu = (Get-Command qemu-system-i386.exe -ErrorAction SilentlyContinue).Source
if ([string]::IsNullOrWhiteSpace($qemu)) { $qemu = "C:\Program Files\qemu\qemu-system-i386.exe" }
$objdump = (Get-Command objdump.exe -ErrorAction SilentlyContinue).Source
Assert-True (Test-Path -LiteralPath $qemu) "qemu-system-i386.exe is unavailable"
Assert-True (-not [string]::IsNullOrWhiteSpace($objdump)) "objdump.exe is unavailable"
Assert-True (Test-Path -LiteralPath $clang) "pinned LLVM clang.exe is unavailable"

New-Item -ItemType Directory -Path $proofRoot -Force | Out-Null
Invoke-Checked "$env:USERPROFILE\.cargo\bin\cargo.exe" @("build", "-p", "dbyte_cli", "--bin", "dbytec") "build dbytec" | Out-Null

$source = Get-Content (Join-Path $sourceRoot "kernel.dby") -Raw
Assert-True (-not $source.Contains('"42"')) 'native v12 proof embeds forbidden string literal "42"'
foreach ($root in @($firstRoot, $secondRoot)) {
    New-Item -ItemType Directory -Path $root -Force | Out-Null
    Invoke-Checked $clang @(
        "--target=i686-none-elf", "-m32", "-ffreestanding", "-fno-pic", "-fno-pie",
        "-fno-stack-protector", "-fno-builtin", "-c",
        (Join-Path $sourceRoot "c_fixture.c"), "-o", (Join-Path $root "c_fixture.o")
    ) "compile C ABI fixture" | Out-Null
    $cFormat = Invoke-Checked $objdump @("-f", (Join-Path $root "c_fixture.o")) "inspect C fixture"
    Assert-True ($cFormat.Contains("elf32-i386")) "clang capability probe did not produce elf32-i386"
}
Build-Set $firstRoot
Build-Set $secondRoot

$hashLines = @()
foreach ($name in @("arch_x86.o", "serial.o", "kernel.o", "c_fixture.o", "libdbyte.a", "kernel.elf")) {
    $hash = Assert-Same (Join-Path $firstRoot $name) (Join-Path $secondRoot $name) $name
    $hashLines += "$name $hash"
}
[System.IO.File]::WriteAllLines((Join-Path $proofRoot "sha256.txt"), $hashLines)

$inspection = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", (Join-Path $firstRoot "kernel.elf")) "inspect v12 ELF"
[System.IO.File]::WriteAllText((Join-Path $proofRoot "objdump.txt"), $inspection)
$objectInspection = Invoke-Checked $objdump @("-h", "-t", (Join-Path $firstRoot "kernel.o")) "inspect generic metadata"
[System.IO.File]::WriteAllText((Join-Path $proofRoot "kernel-object.txt"), $objectInspection)
Assert-True ($objectInspection.Contains(".dbyte.generics")) "public generic metadata section is missing"
Assert-True ($objectInspection.Contains("__dbyte_generic:fn:id") -and $objectInspection.Contains("__dbyte_generic:type:Pair")) "public generic definitions are missing from metadata"
Assert-True (-not $inspection.Contains(".dbyte.generics")) "generic metadata leaked into final executable"
Assert-True ($inspection.Contains("elf32-i386")) "v12 proof is not ELF32 i386"
Assert-True ($inspection.Contains("kernel.o::__dbyte_mono_id_t")) "generic specialization symbol is missing or not module-scoped"
Assert-True ($inspection.Contains("rep movs")) "aggregate Result copy is missing"
Assert-True ($inspection.Contains("c_roundtrip")) "DByte to C symbol is missing"
Assert-True ($inspection.Contains("dbyte_c_callback")) "C to DByte callback symbol is missing"
Assert-True ($inspection.Contains("archive_bias")) "DByte archive member was not extracted"
Assert-True (-not $inspection.Contains("INTERP")) "v12 proof has a runtime interpreter dependency"

$ir = Get-Content (Join-Path $firstRoot "kernel.ir.txt") -Raw
foreach ($required in @("callee: Indirect", "callee: Closure", "AggregateCopy", "__dbyte_closure_", "AddressOffset")) {
    Assert-True ($ir.Contains($required)) "canonical/native IR evidence is missing $required"
}
Assert-True ($ir.Contains("index: Some")) "canonical/native IR evidence is missing indexed address lowering"
Assert-True ($ir.Contains("aggregate_result: Some")) "canonical/native IR evidence is missing aggregate Result lowering"

$expectedText = "DBYTE GENERAL PURPOSE V12`r`nCANONICAL HIR ONLINE`r`nGENERICS ONLINE`r`nENUM MATCH ONLINE`r`nOWNERSHIP ONLINE`r`nCLOSURES ONLINE`r`nDEFER ONLINE`r`nC ABI ONLINE`r`nV12 CHECK OK`r`n"
$script:userExpected = "DBYTEOS USER MODE`r`nTSS ONLINE`r`nUSER ADDRESS SPACE ONLINE`r`nELF LOADER ONLINE`r`nRING3 ONLINE`r`nSYSCALL ABI ONLINE`r`nHELLO FROM DBYTE USER`r`nCONFIG VALUE 42`r`nUSER FS ONLINE`r`nUSER ISOLATION OK`r`nUSER #GP TERMINATED`r`nUSER #PF TERMINATED`r`nPROCESS EXIT OK`r`nPROCESS RECLAIM OK`r`nKERNEL SURVIVED`r`nUSERMODE CHECK OK`r`n"
[byte[]]$expected = [System.Text.Encoding]::ASCII.GetBytes($expectedText)
$serial = Join-Path $proofRoot "serial.log"
$debug = Join-Path $proofRoot "qemu.log"
if (Test-Path -LiteralPath $serial) { Remove-Item -LiteralPath $serial -Force }
if (Test-Path -LiteralPath $debug) { Remove-Item -LiteralPath $debug -Force }
$arguments = @(
    "-kernel", (Join-Path $firstRoot "kernel.elf"),
    "-display", "none", "-monitor", "none",
    "-serial", "file:$serial", "-no-reboot", "-no-shutdown",
    "-d", "int,cpu_reset,guest_errors", "-D", $debug
)
$process = $null
try {
    $process = Start-Process -FilePath $qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    while ($timer.Elapsed.TotalSeconds -lt 10) {
        if (Test-Path -LiteralPath $serial) {
            try {
                [byte[]]$observed = [System.IO.File]::ReadAllBytes($serial)
                if ([System.Linq.Enumerable]::SequenceEqual($observed, $expected)) { break }
            }
            catch { }
        }
        if ($process.HasExited) { break }
        Start-Sleep -Milliseconds 100
    }
}
finally {
    if ($null -ne $process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
        $process.WaitForExit()
    }
}
Assert-True (Test-Path -LiteralPath $serial) "QEMU did not create v12 serial evidence"
[byte[]]$actual = [System.IO.File]::ReadAllBytes($serial)
Assert-True ([System.Linq.Enumerable]::SequenceEqual($actual, $expected)) "v12 QEMU serial bytes differ"
$trace = Get-Content $debug -Raw
Assert-True (-not $trace.Contains("Triple fault")) "v12 QEMU proof triple-faulted"

Build-UserSet $userFirstRoot
Build-UserSet $userSecondRoot
foreach ($name in @($userModules | ForEach-Object { "$_.o" }) + @("user-kernel.elf", "hello.user.o", "hello.elf", "hostile-fixture.elf", "isoa.elf", "isob.elf", "pagefault.elf", "survive.elf", "programs.dbup")) {
    [void](Assert-Same (Join-Path $userFirstRoot $name) (Join-Path $userSecondRoot $name) "user $name")
}
$userSource = Get-Content -LiteralPath (Join-Path $sourceRoot "user.dby") -Raw
Assert-True (-not $userSource.Contains('native.')) "i686 user proof uses privileged native capability"
Assert-True (-not $userSource.Contains('"42"')) 'i686 user proof embeds forbidden string literal "42"'
$userInspection = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", (Join-Path $userFirstRoot "hello.user.o")) "inspect v12 user object"
Assert-True ($userInspection.Contains(".dbyte.profile") -and $userInspection.Contains("int    0x80")) "v12 user object lacks profile/syscall machine evidence"
Assert-True ((Get-Item (Join-Path $userFirstRoot "hello.elf")).Length -le 4096) "v12 user ELF exceeds compact bound"
Invoke-UserProof 32 $userFirstRoot
Invoke-UserProof 64 $userFirstRoot

Write-Host "DBYTE V12 CANONICAL HIR NATIVE: PASS"
$hashLines | ForEach-Object { Write-Host $_ }
Write-Host ($expectedText.TrimEnd())
