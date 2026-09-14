param(
    [string]$HoppysRoot = "D:\TECHNICAL\hoppysOS\HOPPYSOS_SOLO_GPT_20260903"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Invoke-Checked([string]$FilePath, [string[]]$Arguments) {
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "command failed ($LASTEXITCODE): $FilePath $($Arguments -join ' ')"
    }
}

function Invoke-ExpectedFailure(
    [string]$FilePath,
    [string[]]$Arguments,
    [string]$EvidencePrefix
) {
    $stdout = "$EvidencePrefix.stdout.log"
    $stderr = "$EvidencePrefix.stderr.log"
    $process = Start-Process -FilePath $FilePath -ArgumentList $Arguments -Wait -PassThru -NoNewWindow `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    Assert-True ($process.ExitCode -ne 0) "expected command unexpectedly succeeded: $FilePath $($Arguments -join ' ')"
    return $process.ExitCode
}

function Get-Sha256([string]$Path) {
    (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}

function Write-BootImage(
    [string]$Path,
    [string]$BootPath,
    [string]$Stage2Path,
    [string]$KernelPath
) {
    $image = New-Object byte[] (16 * 1024 * 1024)
    $boot = [IO.File]::ReadAllBytes($BootPath)
    $stage2 = [IO.File]::ReadAllBytes($Stage2Path)
    $kernel = [IO.File]::ReadAllBytes($KernelPath)
    Assert-True ($boot.Length -eq 512) "boot sector is not 512 bytes"
    Assert-True ($boot[510] -eq 0x55 -and $boot[511] -eq 0xAA) "boot signature is invalid"
    Assert-True ($stage2.Length -eq 1024) "stage2 is not two sectors"
    [Array]::Copy($boot, 0, $image, 0, $boot.Length)
    [Array]::Copy($stage2, 0, $image, 512, $stage2.Length)
    [Array]::Copy($kernel, 0, $image, 1536, $kernel.Length)
    [IO.File]::WriteAllBytes($Path, $image)
}

function Invoke-QemuProof(
    [string]$Qemu,
    [string]$Image,
    [string]$Evidence,
    [int]$MemoryMiB,
    [string[]]$Markers
) {
    $debug = Join-Path $Evidence "debugcon-$MemoryMiB.log"
    $fault = Join-Path $Evidence "fault-$MemoryMiB.log"
    $arguments = @(
        "-machine", "pc", "-m", "${MemoryMiB}M",
        "-drive", "format=raw,file=$Image",
        "-display", "none", "-serial", "none", "-monitor", "none",
        "-debugcon", "file:$debug", "-global", "isa-debugcon.iobase=0xe9",
        "-d", "int,cpu_reset", "-D", $fault,
        "-no-reboot", "-no-shutdown"
    )
    $process = Start-Process -FilePath $Qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        Start-Sleep -Seconds 3
    } finally {
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force
            $process.WaitForExit()
        }
    }
    $text = [IO.File]::ReadAllText($debug)
    foreach ($marker in $Markers) {
        $markerCount = ([regex]::Matches($text, [regex]::Escape($marker))).Count
        Assert-True ($markerCount -eq 1) "marker '$marker' count was $markerCount instead of exactly one at ${MemoryMiB}M"
    }
    Assert-True (-not $text.Contains("DBYTE HOPPYSOS FLAT SEAM FAIL")) "DByte call receipt failed at ${MemoryMiB}M"
    $postHandoff = $text.Substring($text.IndexOf("HOPPYSOS64:LONG_MODE_OK"))
    Assert-True (-not ($postHandoff -match "Triple fault|#UD|#GP|#PF")) "unexpected post-handoff CPU fault at ${MemoryMiB}M"
    $faultText = [IO.File]::ReadAllText($fault)
    Assert-True (-not ($faultText -match "Triple fault")) "unexpected QEMU triple fault at ${MemoryMiB}M"
    $expectsUserFaults = $Markers -contains "HOPPYSOS64:DBYRT64_FAULT_UD_OK"
    if ($expectsUserFaults) {
        $udCount = ([regex]::Matches($faultText, "(?m)^\s+\d+: v=06 e=0000 i=0 cpl=3 .*IP=0023:0000000000400000")).Count
        $gpCount = ([regex]::Matches($faultText, "(?m)^\s+\d+: v=0d e=0000 i=0 cpl=3 .*IP=0023:0000000000400000")).Count
        $pfCount = ([regex]::Matches($faultText, "(?m)^\s+\d+: v=0e e=0004 i=0 cpl=3 .*CR2=0000000000900000")).Count
        Assert-True ($udCount -eq 1) "expected one contained CPL3 #UD, got $udCount at ${MemoryMiB}M"
        Assert-True ($gpCount -eq 1) "expected one contained CPL3 #GP, got $gpCount at ${MemoryMiB}M"
        Assert-True ($pfCount -eq 1) "expected one contained CPL3 #PF, got $pfCount at ${MemoryMiB}M"
        $userFaultCount = ([regex]::Matches($faultText, "(?m)^\s+\d+: v=(06|0d|0e) ")).Count
        Assert-True ($userFaultCount -eq 3) "unexpected CPL3 fault count $userFaultCount at ${MemoryMiB}M"
    } else {
        Assert-True (-not ($faultText -match "v=06|v=0d|v=0e")) "unexpected QEMU CPU exception trace at ${MemoryMiB}M"
    }
    $startupResetCount = ([regex]::Matches($faultText, "CPU Reset \(CPU 0\)")).Count
    Assert-True ($startupResetCount -eq 2) "unexpected QEMU reset count $startupResetCount at ${MemoryMiB}M"
    [ordered]@{
        memory_mib = $MemoryMiB
        debug_sha256 = Get-Sha256 $debug
        fault_sha256 = Get-Sha256 $fault
        startup_reset_count = $startupResetCount
        markers = $Markers
    }
}

function Invoke-QemuRejectProof(
    [string]$Qemu,
    [string]$Image,
    [string]$Evidence,
    [string]$Case
) {
    $debug = Join-Path $Evidence "debugcon.log"
    $fault = Join-Path $Evidence "fault.log"
    $arguments = @(
        "-machine", "pc", "-m", "32M",
        "-drive", "format=raw,file=$Image",
        "-display", "none", "-serial", "none", "-monitor", "none",
        "-debugcon", "file:$debug", "-global", "isa-debugcon.iobase=0xe9",
        "-d", "int,cpu_reset", "-D", $fault,
        "-no-reboot", "-no-shutdown"
    )
    $process = Start-Process -FilePath $Qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        Start-Sleep -Seconds 3
    } finally {
        if (-not $process.HasExited) {
            Stop-Process -Id $process.Id -Force
            $process.WaitForExit()
        }
    }
    $text = [IO.File]::ReadAllText($debug)
    $rejectCount = ([regex]::Matches($text, [regex]::Escape("HOPPYSOS64:DBYRT64_REJECTED"))).Count
    $liveCount = ([regex]::Matches($text, [regex]::Escape("HOPPYSOS64:DBYRT64_LIVE"))).Count
    Assert-True ($rejectCount -eq 1) "DBYRT64 rejection marker count was $rejectCount for $Case"
    Assert-True ($liveCount -eq 0) "DBYRT64 live marker appeared for rejected case $Case"
    $faultText = [IO.File]::ReadAllText($fault)
    Assert-True (-not ($faultText -match "Triple fault|v=06|v=0d|v=0e")) "unexpected QEMU CPU exception trace for $Case"
    $startupResetCount = ([regex]::Matches($faultText, "CPU Reset \(CPU 0\)")).Count
    Assert-True ($startupResetCount -eq 2) "unexpected QEMU reset count $startupResetCount for $Case"
    [ordered]@{
        case = $Case
        debug_sha256 = Get-Sha256 $debug
        fault_sha256 = Get-Sha256 $fault
        startup_reset_count = $startupResetCount
    }
}

$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
$nasm = (Get-Command nasm -ErrorAction Stop).Source
$qemu = (Get-Command qemu-system-x86_64 -ErrorAction Stop).Source
$python = (Get-Command python -ErrorAction Stop).Source
$liveBootSource = Join-Path $HoppysRoot "src64\boot\boot.asm"
$liveStage2Source = Join-Path $HoppysRoot "src64\boot\stage2.asm"
$authoritativeKernelSource = Join-Path $HoppysRoot "src64\kernel\kernel.asm"
$authoritativeStateSource = Join-Path $HoppysRoot "STATE.txt"
$legacyKernelReference = Join-Path $HoppysRoot "src\kernel\kernel.asm"
$frozenAuthorityManifestSource = Join-Path $repo "examples\hoppysos_flat_seam\frozen_authority_v1.json"
$protectedSmokeSource = Join-Path $repo "examples\smoke.dby"
$protectedCubeStateSource = Join-Path $repo "examples\.cube-state.bin"
$boundarySource = Join-Path $repo "examples\hoppysos_flat_seam\boundary.dbtasm"
$dbyteSource = Join-Path $repo "examples\hoppysos_flat_seam\probe.dby"
$pmmSource = Join-Path $repo "examples\hoppysos_flat_seam\pmm.dby"
$hoppysPmmSource = Join-Path $repo "examples\hoppysos_flat_seam\pmm_hoppysos.dby"
$taskRuntimeSource = Join-Path $repo "examples\hoppysos_flat_seam\task_runtime.dby"
$runtimeModuleSource = Join-Path $repo "examples\hoppysos_flat_seam\runtime_module.dby"
$runtimeHostAbiSource = Join-Path $repo "examples\hoppysos_flat_seam\runtime_host_abi.dby"
$runtimeUserSource = Join-Path $repo "examples\hoppysos_flat_seam\runtime_worker.dby"
$runtimeModulePacker = Join-Path $repo "scripts\pack_dbyrt64.py"
$runtimeHopfsFixtureBuilder = Join-Path $repo "scripts\build_hoppysos_hopfs1_fixture.py"
foreach ($required in @($liveBootSource, $liveStage2Source, $authoritativeKernelSource, $authoritativeStateSource, $legacyKernelReference, $frozenAuthorityManifestSource, $protectedSmokeSource, $protectedCubeStateSource, $boundarySource, $dbyteSource, $pmmSource, $hoppysPmmSource, $taskRuntimeSource, $runtimeModuleSource, $runtimeHostAbiSource, $runtimeUserSource, $runtimeModulePacker, $runtimeHopfsFixtureBuilder)) {
    Assert-True (Test-Path -LiteralPath $required -PathType Leaf) "missing required source: $required"
}

$expectedFrozenAuthorityManifestSha256 = "35E05DA88CE056A83C320AC160373AE0D1911215661B408544D4892185B70D3B"
$frozenAuthorityManifestSha256 = Get-Sha256 $frozenAuthorityManifestSource
Assert-True ($frozenAuthorityManifestSha256 -eq $expectedFrozenAuthorityManifestSha256) "frozen authority manifest hash mismatch"
$frozenAuthority = Get-Content -LiteralPath $frozenAuthorityManifestSource -Raw | ConvertFrom-Json
Assert-True ($frozenAuthority.schema -eq "hoppysos-x64-frozen-authority-v1") "unexpected frozen authority schema"
$frozenKernelCheckpointSource = Join-Path $HoppysRoot ($frozenAuthority.checkpoint_rel -replace '/', '\')
Assert-True (Test-Path -LiteralPath $frozenKernelCheckpointSource -PathType Leaf) "missing frozen HOPPYSOS checkpoint kernel"
Assert-True ((Get-Sha256 $frozenKernelCheckpointSource) -eq $frozenAuthority.checkpoint_sha256) "frozen checkpoint kernel hash mismatch"
Assert-True ((Get-Sha256 $liveBootSource) -eq $frozenAuthority.boot.sha256) "boot source drifted from frozen authority"
Assert-True ((Get-Sha256 $liveStage2Source) -eq $frozenAuthority.stage2.sha256) "stage2 source drifted from frozen authority"
foreach ($include in $frozenAuthority.includes) {
    $includeSource = Join-Path $HoppysRoot ($include.path -replace '/', '\')
    Assert-True (Test-Path -LiteralPath $includeSource -PathType Leaf) "missing frozen include source: $($include.path)"
    Assert-True ((Get-Sha256 $includeSource) -eq $include.sha256) "include drifted from frozen authority: $($include.path)"
}

$expectedV131Commit = "867bebea05d796855590099d2340de09f2ced4d5"
$headBefore = (& git rev-parse HEAD).Trim()
$tagObjectBefore = (& git rev-parse "refs/tags/v13.1.0").Trim()
$tagCommitBefore = (& git rev-parse "v13.1.0^{}").Trim()
Assert-True ($headBefore -eq $expectedV131Commit) "HEAD moved from sealed v13.1.0 commit before verification"
Assert-True ($tagCommitBefore -eq $expectedV131Commit) "v13.1.0 tag no longer peels to sealed commit"
$authoritativeKernelHashBefore = Get-Sha256 $authoritativeKernelSource
$authoritativeStateHashBefore = Get-Sha256 $authoritativeStateSource
$legacyKernelReferenceHash = Get-Sha256 $legacyKernelReference
$protectedSmokeHashBefore = Get-Sha256 $protectedSmokeSource
$protectedCubeStateHashBefore = Get-Sha256 $protectedCubeStateSource
$verifierHashBefore = Get-Sha256 $PSCommandPath
$gitStatusBefore = ((& git status --porcelain=v1) -join "`n")

$runId = [Guid]::NewGuid().ToString("N")
$evidenceRoot = Join-Path $repo "tmp\hoppysos_flat_seam\$runId"
New-Item -ItemType Directory -Path $evidenceRoot | Out-Null

# Materialize an immutable verifier-owned authority bundle. The live HOPPYSOS
# tree is read only long enough to hash-check/copy the frozen transitive inputs.
$authorityRoot = Join-Path $evidenceRoot "frozen-authority"
New-Item -ItemType Directory -Path $authorityRoot | Out-Null
$authorityCopies = @(
    [ordered]@{ source = $frozenKernelCheckpointSource; relative = "src64/kernel/kernel.asm"; sha256 = $frozenAuthority.checkpoint_sha256 },
    [ordered]@{ source = $liveBootSource; relative = $frozenAuthority.boot.path; sha256 = $frozenAuthority.boot.sha256 },
    [ordered]@{ source = $liveStage2Source; relative = $frozenAuthority.stage2.path; sha256 = $frozenAuthority.stage2.sha256 }
)
foreach ($include in $frozenAuthority.includes) {
    $authorityCopies += [ordered]@{
        source = Join-Path $HoppysRoot ($include.path -replace '/', '\')
        relative = $include.path
        sha256 = $include.sha256
    }
}
foreach ($copy in $authorityCopies) {
    Assert-True ((Get-Sha256 $copy.source) -eq $copy.sha256) "authority source drifted before copy: $($copy.relative)"
    $destination = Join-Path $authorityRoot ($copy.relative -replace '/', '\')
    $destinationParent = Split-Path -Parent $destination
    New-Item -ItemType Directory -Path $destinationParent -Force | Out-Null
    Copy-Item -LiteralPath $copy.source -Destination $destination
    Assert-True ((Get-Sha256 $destination) -eq $copy.sha256) "authority copy hash mismatch: $($copy.relative)"
}
$kernelSource = Join-Path $authorityRoot "src64\kernel\kernel.asm"
$bootSource = Join-Path $authorityRoot "src64\boot\boot.asm"
$stage2Source = Join-Path $authorityRoot "src64\boot\stage2.asm"

Invoke-Checked "cargo" @("build", "-p", "dbyte_cli", "--bin", "dbytec", "--bin", "dbtasm")
$compiler = Join-Path $repo "target\debug\dbytec.exe"
$assembler = Join-Path $repo "target\debug\dbtasm.exe"

$receipts = @()
foreach ($opt in @(0, 1)) {
    $variants = @()
    foreach ($variant in @("a", "b")) {
        $root = Join-Path $evidenceRoot "o$opt-$variant"
        New-Item -ItemType Directory -Path $root | Out-Null
        $boundary = Join-Path $root "boundary.o"
        $body = Join-Path $root "probe.o"
        $kernel = Join-Path $root "kernel.bin"
        Invoke-Checked $assembler @($boundarySource, "--target", "x86_64-dbyte-none", "-o", $boundary)
        Invoke-Checked $compiler @(
            "build", $dbyteSource, "--target", "x86_64-dbyte-none",
            "--emit", "obj", "--opt-level", "$opt", "-o", $body
        )
        Invoke-Checked $compiler @(
            "link", "--target", "x86_64-dbyte-none",
            "--flat-load-address", "0x10000", "--flat-max-size", "129024",
            "--flat-section-alignment", "16", "--entry", "hoppys_entry",
            $boundary, $body, "-o", $kernel
        )
        $kernelBytes = (Get-Item -LiteralPath $kernel).Length
        $kernelSectors = [int][Math]::Ceiling($kernelBytes / 512.0)
        Assert-True ($kernelSectors -ge 1 -and $kernelSectors -le 252) "kernel sector ceiling violated"
        $boot = Join-Path $root "boot.bin"
        $stage2 = Join-Path $root "stage2.bin"
        $image = Join-Path $root "seam.img"
        Invoke-Checked $nasm @("-f", "bin", "-DHOS64_KERNEL_SECTORS=$kernelSectors", $stage2Source, "-o", $stage2)
        Invoke-Checked $nasm @("-f", "bin", $bootSource, "-o", $boot)
        Write-BootImage $image $boot $stage2 $kernel
        $variants += [ordered]@{
            root = $root
            kernel = $kernel
            image = $image
            kernel_bytes = $kernelBytes
            kernel_sectors = $kernelSectors
            kernel_sha256 = Get-Sha256 $kernel
            image_sha256 = Get-Sha256 $image
        }
    }
    Assert-True ($variants[0].kernel_sha256 -eq $variants[1].kernel_sha256) "O$opt kernel A/B mismatch"
    Assert-True ($variants[0].image_sha256 -eq $variants[1].image_sha256) "O$opt image A/B mismatch"
    $qemuReceipts = @()
    foreach ($memory in @(32, 64)) {
        $qemuEvidence = Join-Path $variants[0].root "qemu-$memory"
        New-Item -ItemType Directory -Path $qemuEvidence | Out-Null
        $qemuReceipts += Invoke-QemuProof $qemu $variants[0].image $qemuEvidence $memory @(
            "HOPPYSOS64:LONG_MODE_OK",
            "DBYTE HOPPYSOS FLAT SEAM PASS",
            "DBYTE HOPPYSOS PMM MIGRATION PASS"
        )
    }
    $receipts += [ordered]@{
        opt_level = $opt
        kernel_bytes = $variants[0].kernel_bytes
        kernel_sectors = $variants[0].kernel_sectors
        kernel_sha256 = $variants[0].kernel_sha256
        image_sha256 = $variants[0].image_sha256
        qemu = $qemuReceipts
    }
}

$negative = Join-Path $evidenceRoot "must-not-exist.bin"
$negativeExit = Invoke-ExpectedFailure $compiler @(
    "link", "--target", "x86_64-dbyte-none", "--flat-load-address", "0x10000",
    "--flat-max-size", "64", "--entry", "hoppys_entry",
    (Join-Path $evidenceRoot "o0-a\boundary.o"),
    (Join-Path $evidenceRoot "o0-a\probe.o"), "-o", $negative
) (Join-Path $evidenceRoot "negative-flat-size")
Assert-True ($negativeExit -ne 0) "undersized flat output unexpectedly succeeded"
Assert-True (-not (Test-Path -LiteralPath $negative)) "failed flat link published output"

$fullRoot = Join-Path $evidenceRoot "full-hoppysos-relink"
New-Item -ItemType Directory -Path $fullRoot | Out-Null
$kernelText = [IO.File]::ReadAllText($kernelSource)
$orgLine = "ORG 0x00010000"
Assert-True (([regex]::Matches($kernelText, [regex]::Escape($orgLine))).Count -eq 1) "full kernel must contain one reviewed ORG declaration"
$relocatableText = $kernelText.Replace($orgLine, "section .text align=16`r`nglobal kernel64_entry")
$relocatableSource = Join-Path $fullRoot "kernel-rel.asm"
[IO.File]::WriteAllText($relocatableSource, $relocatableText)
$canonicalKernel = Join-Path $fullRoot "canonical.bin"
Invoke-Checked $nasm @("-I", "$authorityRoot\", "-f", "bin", $kernelSource, "-o", $canonicalKernel)
$fullVariants = @()
foreach ($variant in @("a", "b")) {
    $variantRoot = Join-Path $fullRoot $variant
    New-Item -ItemType Directory -Path $variantRoot | Out-Null
    $object = Join-Path $variantRoot "kernel-rel.o"
    $kernel = Join-Path $variantRoot "kernel.bin"
    Invoke-Checked $nasm @("-I", "$authorityRoot\", "-f", "elf64", $relocatableSource, "-o", $object)
    Invoke-Checked $compiler @(
        "link", "--target", "x86_64-dbyte-none",
        "--flat-load-address", "0x10000", "--flat-max-size", "129024",
        "--flat-section-alignment", "16", "--entry", "kernel64_entry",
        $object, "-o", $kernel
    )
    $fullVariants += [ordered]@{
        root = $variantRoot
        object_sha256 = Get-Sha256 $object
        kernel_sha256 = Get-Sha256 $kernel
        kernel_bytes = (Get-Item -LiteralPath $kernel).Length
        kernel = $kernel
    }
}
$canonicalKernelHash = Get-Sha256 $canonicalKernel
$canonicalKernelBytes = (Get-Item -LiteralPath $canonicalKernel).Length
Assert-True ($canonicalKernelHash -eq $frozenAuthority.canonical_kernel_sha256) "frozen canonical kernel hash mismatch"
Assert-True ($canonicalKernelBytes -eq [int64]$frozenAuthority.canonical_kernel_bytes) "frozen canonical kernel byte count mismatch"
Assert-True ($fullVariants[0].object_sha256 -eq $fullVariants[1].object_sha256) "full external ELF64 A/B mismatch"
Assert-True ($fullVariants[0].kernel_sha256 -eq $fullVariants[1].kernel_sha256) "full linked kernel A/B mismatch"
Assert-True ($fullVariants[0].kernel_sha256 -eq $canonicalKernelHash) "DByte-linked full kernel differs from canonical flat bytes"
$fullSectors = [int][Math]::Ceiling($fullVariants[0].kernel_bytes / 512.0)
Assert-True ($fullSectors -le 252) "full linked kernel exceeds the HOPPYSOS sector ceiling"
$fullBoot = Join-Path $fullRoot "boot.bin"
$fullStage2 = Join-Path $fullRoot "stage2.bin"
$fullImage = Join-Path $fullRoot "full-relinked.img"
Invoke-Checked $nasm @("-f", "bin", "-DHOS64_KERNEL_SECTORS=$fullSectors", $stage2Source, "-o", $fullStage2)
Invoke-Checked $nasm @("-f", "bin", $bootSource, "-o", $fullBoot)
Write-BootImage $fullImage $fullBoot $fullStage2 $fullVariants[0].kernel
$fullQemuRoot = Join-Path $fullRoot "qemu-64"
New-Item -ItemType Directory -Path $fullQemuRoot | Out-Null
$fullQemu = Invoke-QemuProof $qemu $fullImage $fullQemuRoot 64 @(
    "HOPPYSOS64:LONG_MODE_OK",
    "HOPPYSOS64:KERNEL64_ENTER",
    "HOPPYSOS64:BOOTINFO_OK",
    "HOPPYSOS64:PAGING_4G_OK",
    "HOPPYSOS64:PMM_OK",
    "HOPPYSOS64:PAGE_ALLOC_OK",
    "HOPPYSOS64:PMM_GUARD_OK",
    "HOPPYSOS64:READY"
)

# Replace only the reviewed HOPPYSOS PMM implementation and its two owned data
# symbols in a generated proof source. The frozen HOPPYSOS tree is never
# mutated. All original call sites remain, so a link without the DByte PMM
# object must fail and a successful boot necessarily traverses the DByte code.
$migrationRoot = Join-Path $evidenceRoot "full-hoppysos-dbyte-pmm"
New-Item -ItemType Directory -Path $migrationRoot | Out-Null
$pmmStartMarker = "pmm_init64:"
$pmmEndMarker = "setup_idt64:"
$pmmStart = $kernelText.IndexOf($pmmStartMarker, [StringComparison]::Ordinal)
$pmmEnd = $kernelText.IndexOf($pmmEndMarker, [StringComparison]::Ordinal)
Assert-True ($pmmStart -ge 0 -and $pmmEnd -gt $pmmStart) "reviewed HOPPYSOS PMM block was not found"
Assert-True ($kernelText.IndexOf($pmmStartMarker, $pmmStart + 1, [StringComparison]::Ordinal) -lt 0) "duplicate PMM start marker"
Assert-True ($kernelText.IndexOf($pmmEndMarker, $pmmEnd + 1, [StringComparison]::Ordinal) -lt 0) "duplicate PMM end marker"
$externPmm = @"
extern pmm_init64
extern pmm_alloc_page64
extern pmm_free_page64
extern pmm_selftest64
extern pmm_guard_selftest64
extern pmm_free_pages
extern pmm_usable_regions

"@
$migratedText = $kernelText.Substring(0, $pmmStart) + $externPmm + $kernelText.Substring($pmmEnd)
foreach ($ownedData in @("pmm_free_pages: dq 0", "pmm_usable_regions: dq 0")) {
    Assert-True (([regex]::Matches($migratedText, [regex]::Escape($ownedData))).Count -eq 1) "reviewed PMM data symbol '$ownedData' was not unique"
    $migratedText = $migratedText.Replace($ownedData, "")
}
Assert-True (-not $migratedText.Contains($pmmStartMarker)) "ASM PMM implementation remains in generated source"
Assert-True (-not $migratedText.Contains("pmm_free_pages: dq 0")) "ASM PMM free-page storage remains"
Assert-True (-not $migratedText.Contains("pmm_usable_regions: dq 0")) "ASM PMM region storage remains"
$migratedText = $migratedText.Replace($orgLine, "section .text align=16`r`nglobal kernel64_entry")
$migratedSource = Join-Path $migrationRoot "kernel-dbyte-pmm.asm"
[IO.File]::WriteAllText($migratedSource, $migratedText)

# Keep the pre-runtime PMM proof inside the frozen 252-sector transport. The
# successor-only reservation entry point is linked later with the live runtime
# kernel, where production O1 is the bounded transport authority.
$pmmRuntimeText = [IO.File]::ReadAllText($hoppysPmmSource)
$pmmReserveMarker = "pub fn pmm_reserve_range64("
$pmmReserveStart = $pmmRuntimeText.IndexOf($pmmReserveMarker, [StringComparison]::Ordinal)
Assert-True ($pmmReserveStart -gt 0) "successor PMM reservation entry point is missing"
$pmmCoreSource = Join-Path $migrationRoot "pmm-hoppysos-core.dby"
[IO.File]::WriteAllText($pmmCoreSource, $pmmRuntimeText.Substring(0, $pmmReserveStart).TrimEnd() + "`r`n")

$migrationReceipts = @()
foreach ($opt in @(0, 1)) {
    $variants = @()
    foreach ($variant in @("a", "b")) {
        $variantRoot = Join-Path $migrationRoot "o$opt-$variant"
        New-Item -ItemType Directory -Path $variantRoot | Out-Null
        $asmObject = Join-Path $variantRoot "kernel-boundary.o"
        $pmmObject = Join-Path $variantRoot "pmm-dbyte.o"
        $kernel = Join-Path $variantRoot "kernel.bin"
        Invoke-Checked $nasm @("-I", "$authorityRoot\", "-f", "elf64", $migratedSource, "-o", $asmObject)
        Invoke-Checked $compiler @(
            "build", $pmmCoreSource, "--target", "x86_64-dbyte-none",
            "--emit", "obj", "--opt-level", "$opt", "-o", $pmmObject
        )
        Invoke-Checked $compiler @(
            "link", "--target", "x86_64-dbyte-none",
            "--flat-load-address", "0x10000", "--flat-max-size", "129024",
            "--flat-section-alignment", "16", "--entry", "kernel64_entry",
            $asmObject, $pmmObject, "-o", $kernel
        )
        $kernelBytes = (Get-Item -LiteralPath $kernel).Length
        $kernelSectors = [int][Math]::Ceiling($kernelBytes / 512.0)
        Assert-True ($kernelSectors -le 252) "DByte PMM kernel exceeds the HOPPYSOS sector ceiling"
        $variants += [ordered]@{
            root = $variantRoot
            asm_object_sha256 = Get-Sha256 $asmObject
            pmm_object_sha256 = Get-Sha256 $pmmObject
            kernel = $kernel
            kernel_sha256 = Get-Sha256 $kernel
            kernel_bytes = $kernelBytes
            kernel_sectors = $kernelSectors
        }
    }
    Assert-True ($variants[0].asm_object_sha256 -eq $variants[1].asm_object_sha256) "O$opt PMM boundary object A/B mismatch"
    Assert-True ($variants[0].pmm_object_sha256 -eq $variants[1].pmm_object_sha256) "O$opt DByte PMM object A/B mismatch"
    Assert-True ($variants[0].kernel_sha256 -eq $variants[1].kernel_sha256) "O$opt DByte PMM kernel A/B mismatch"

    $missingPmm = Join-Path $migrationRoot "must-not-link-o$opt.bin"
    $boundaryOnly = Join-Path $variants[0].root "kernel-boundary.o"
    $missingPmmExit = Invoke-ExpectedFailure $compiler @(
        "link", "--target", "x86_64-dbyte-none", "--flat-load-address", "0x10000",
        "--flat-max-size", "129024", "--flat-section-alignment", "16",
        "--entry", "kernel64_entry", $boundaryOnly, "-o", $missingPmm
    ) (Join-Path $variantRoot "negative-missing-pmm")
    Assert-True ($missingPmmExit -ne 0) "HOPPYSOS boundary linked without the DByte PMM object"
    Assert-True (-not (Test-Path -LiteralPath $missingPmm)) "failed missing-PMM link published output"

    $boot = Join-Path $variants[0].root "boot.bin"
    $stage2 = Join-Path $variants[0].root "stage2.bin"
    $image = Join-Path $variants[0].root "hoppysos-dbyte-pmm.img"
    Invoke-Checked $nasm @("-f", "bin", "-DHOS64_KERNEL_SECTORS=$($variants[0].kernel_sectors)", $stage2Source, "-o", $stage2)
    Invoke-Checked $nasm @("-f", "bin", $bootSource, "-o", $boot)
    Write-BootImage $image $boot $stage2 $variants[0].kernel
    $qemuReceipts = @()
    foreach ($memory in @(32, 64)) {
        $qemuEvidence = Join-Path $variants[0].root "qemu-$memory"
        New-Item -ItemType Directory -Path $qemuEvidence | Out-Null
        $qemuReceipts += Invoke-QemuProof $qemu $image $qemuEvidence $memory @(
            "HOPPYSOS64:LONG_MODE_OK",
            "HOPPYSOS64:KERNEL64_ENTER",
            "HOPPYSOS64:BOOTINFO_OK",
            "HOPPYSOS64:PAGING_4G_OK",
            "HOPPYSOS64:PMM_OK",
            "HOPPYSOS64:PAGE_ALLOC_OK",
            "HOPPYSOS64:PMM_GUARD_OK",
            "HOPPYSOS64:READY"
        )
    }
    $migrationReceipts += [ordered]@{
        opt_level = $opt
        asm_object_sha256 = $variants[0].asm_object_sha256
        pmm_object_sha256 = $variants[0].pmm_object_sha256
        kernel_sha256 = $variants[0].kernel_sha256
        kernel_bytes = $variants[0].kernel_bytes
        kernel_sectors = $variants[0].kernel_sectors
        image_sha256 = Get-Sha256 $image
        missing_pmm_rejected = $true
        qemu = $qemuReceipts
    }
}

# Add a bounded cooperative task-runtime consumer after the DByte PMM gate.
# O0 is kept as a deterministic object oracle. The live HOPPYSOS payload uses
# production O1 because the frozen stage2 contract allows at most 252 sectors.
$runtimeRoot = Join-Path $evidenceRoot "full-hoppysos-dbyte-runtime"
New-Item -ItemType Directory -Path $runtimeRoot | Out-Null
$runtimeDataPath = Join-Path $runtimeRoot "DATA.TXT"
$runtimeDataBytes = New-Object byte[] 64
for ($index = 0; $index -lt $runtimeDataBytes.Length; $index++) {
    $runtimeDataBytes[$index] = [byte](($index * 29 + 0xA7) -band 0xFF)
}
$runtimeDataBytes[0] = 0x44
$runtimeDataBytes[1] = 0x42
$runtimeDataBytes[2] = 0x59
$runtimeDataBytes[3] = 0x54
[IO.File]::WriteAllBytes($runtimeDataPath, $runtimeDataBytes)
$runtimeText = $migratedText
$externAnchor = "extern pmm_init64"
Assert-True (([regex]::Matches($runtimeText, [regex]::Escape($externAnchor))).Count -eq 1) "PMM extern anchor is not unique"
$runtimeText = $runtimeText.Replace($externAnchor, "extern dbyte_task_selftest64`r`nextern dbyte_scheduler_init64`r`nextern dbyte_scheduler_run_once64`r`nextern dbyte_scheduler_run_count64`r`nextern dbyte_irq0_init64`r`nextern dbyte_irq0_tick64`r`nextern dbyte_irq0_count64`r`nextern dbyte_privilege_init64`r`nextern dbyte_syscall_enter64`r`nextern dbyte_syscall_count64`r`nextern dbyte_syscall_last64`r`nextern pmm_reserve_range64`r`n$externAnchor")
$callAnchor = "    mov rsi, dbg_pmm_guard_ok`r`n    call dbg_puts64"
Assert-True (([regex]::Matches($runtimeText, [regex]::Escape($callAnchor))).Count -eq 1) "PMM completion call anchor is not unique"
$runtimeCall = @"
$callAnchor
    mov r11, [pmm_free_pages]
    mov [dbyrt64_pmm_free_before], r11
    mov rax, 0x01000000
    mov rdx, 0x00040000
    call pmm_reserve_range64
    test eax, eax
    jz .fail
    mov r10, [pmm_free_pages]
    add r10, 64
    cmp r10, [dbyrt64_pmm_free_before]
    jne .fail
    mov rax, 0x01000000
    mov rdx, 0x00040000
    call pmm_reserve_range64
    test eax, eax
    jnz .fail
    call dbyte_task_selftest64
    test eax, eax
    jz .fail
    mov rsi, dbg_dbyte_task_ok
    call dbg_puts64
    call dbyte_scheduler_init64
    test eax, eax
    jz .fail
    call dbyte_privilege_init64
    call setup_privilege64
    test eax, eax
    jz .fail
    mov rsi, dbg_dbyte_tss_ok
    call dbg_puts64
    call privilege_probe64
    test eax, eax
    jz .fail
    call dbyte_syscall_count64
    cmp rax, 1
    jne .fail
    call dbyte_syscall_last64
    cmp rax, 1
    jne .fail
    mov rsi, dbg_dbyte_ring3_syscall_ok
    call dbg_puts64
    mov rsi, dbg_dbyte_ring0_return_ok
    call dbg_puts64
"@
$runtimeText = $runtimeText.Replace($callAnchor, $runtimeCall.TrimEnd("`r", "`n"))
$hopfsMountAnchor = "    call hopfs1_detect_readonly64`r`n    mov [hopfs_mount_state64], eax"
Assert-True (([regex]::Matches($runtimeText, [regex]::Escape($hopfsMountAnchor))).Count -eq 1) "HOPFS1 mount anchor is not unique"
$hopfsModuleCall = @"
$hopfsMountAnchor
    cmp eax, 2
    jne .fail
    call dbyrt64_load_call64
    test eax, eax
    jz .fail
    mov rsi, dbg_dbyrt64_live
    call dbg_puts64
    mov rsi, dbg_dbyrt64_host_abi_ok
    call dbg_puts64
    mov rsi, dbg_dbyrt64_user_elf_ok
    call dbg_puts64
    mov rsi, dbg_dbyrt64_private_cr3_ok
    call dbg_puts64
    mov rsi, dbg_dbyrt64_user_exec_ok
    call dbg_puts64
"@
$runtimeText = $runtimeText.Replace($hopfsMountAnchor, $hopfsModuleCall.TrimEnd("`r", "`n"))
$dataAnchor = "dbg_pmm_guard_ok: db 'HOPPYSOS64:PMM_GUARD_OK', 10, 0"
Assert-True (([regex]::Matches($runtimeText, [regex]::Escape($dataAnchor))).Count -eq 1) "PMM diagnostic anchor is not unique"
$runtimeText = $runtimeText.Replace($dataAnchor, "$dataAnchor`r`ndbg_dbyte_task_ok: db 'HOPPYSOS64:DBYTE_TASK_OK', 10, 0`r`ndbg_dbyte_irq0_live: db 'HOPPYSOS64:DBYTE_IRQ0_LIVE', 10, 0`r`ndbg_dbyte_tss_ok: db 'HOPPYSOS64:DBYTE_TSS_OK', 10, 0`r`ndbg_dbyte_ring3_syscall_ok: db 'HOPPYSOS64:DBYTE_RING3_SYSCALL_OK', 10, 0`r`ndbg_dbyte_ring0_return_ok: db 'HOPPYSOS64:DBYTE_RING0_RETURN_OK', 10, 0`r`ndbg_dbyrt64_live: db 'HOPPYSOS64:DBYRT64_LIVE', 10, 0`r`ndbg_dbyrt64_host_abi_ok: db 'HOPPYSOS64:DBYRT64_HOST_ABI_OK', 10, 0`r`ndbg_dbyrt64_user_elf_ok: db 'HOPPYSOS64:DBYRT64_USER_ELF_OK', 10, 0`r`ndbg_dbyrt64_private_cr3_ok: db 'HOPPYSOS64:DBYRT64_PRIVATE_CR3_OK', 10, 0`r`ndbg_dbyrt64_user_exec_ok: db 'HOPPYSOS64:DBYRT64_USER_EXEC_OK', 10, 0`r`ndbg_dbyrt64_rejected: db 'HOPPYSOS64:DBYRT64_REJECTED', 10, 0`r`ndbyrt64_name: db 'DBYRT641'`r`ndbyrt64_data_name: db 'DATA.TXT'`r`ndbg_dbyte_sched_live: db 'HOPPYSOS64:DBYTE_SCHED_LIVE', 10, 0")
$eventStateAnchor = "event_mouse_seen: db 0"
Assert-True (([regex]::Matches($runtimeText, [regex]::Escape($eventStateAnchor))).Count -eq 1) "event state anchor is not unique"
$runtimeText = $runtimeText.Replace($eventStateAnchor, "$eventStateAnchor`r`ndbyte_scheduler_live_seen: db 0`r`ndbyrt64_user_active: db 0`r`ndbyrt64_fd_open: db 0`r`nalign 4`r`ndbyrt64_user_owner: dd 0`r`ndbyrt64_fd_owner: dd 0`r`ndbyrt64_write_count: dd 0`r`ndbyrt64_open_count: dd 0`r`ndbyrt64_read_count: dd 0`r`ndbyrt64_close_count: dd 0`r`ndbyrt64_exit_count: dd 0`r`ndbyrt64_efault_count: dd 0`r`ndbyrt64_user_fault_reason: dd 0`r`ndbyrt64_fault_ud_count: dd 0`r`ndbyrt64_fault_gp_count: dd 0`r`ndbyrt64_fault_pf_count: dd 0`r`nalign 8`r`ndbyrt64_pmm_free_before: dq 0`r`ndbyrt64_user_kernel_cr3: dq 0`r`ndbyrt64_user_kernel_rsp: dq 0`r`ndbyrt64_user_exit_code: dd 0`r`nalign 8`r`ndbyrt64_host_services:`r`n    dd 2`r`n    dd 48`r`n    dq pmm_alloc_page64`r`n    dq pmm_free_page64`r`n    dq dbyrt64_host_read_file64`r`n    dq dbyrt64_host_enter_user64`r`n    dq dbyrt64_host_abort_user64")
$irqBootAnchor = "    mov qword [irq0_count], 0`r`n    sti`r`n.wait_irq0:`r`n    hlt`r`n    cmp qword [irq0_count], 3`r`n    jb .wait_irq0`r`n    cli`r`n    mov rsi, dbg_irq0_ok`r`n    call dbg_puts64"
Assert-True (([regex]::Matches($runtimeText, [regex]::Escape($irqBootAnchor))).Count -eq 1) "IRQ0 boot gate anchor is not unique"
$irqBoot = @"
    call dbyte_irq0_init64
    sti
.wait_irq0:
    hlt
    call dbyte_irq0_count64
    cmp rax, 3
    jb .wait_irq0
    cli
    mov rsi, dbg_irq0_ok
    call dbg_puts64
    mov rsi, dbg_dbyte_irq0_live
    call dbg_puts64
"@
$runtimeText = $runtimeText.Replace($irqBootAnchor, $irqBoot.TrimEnd("`r", "`n"))
$eventResumeAnchor = ".event_resume:`r`n    sti"
Assert-True (([regex]::Matches($runtimeText, [regex]::Escape($eventResumeAnchor))).Count -eq 1) "event-resume anchor is not unique"
$eventResume = @"
.event_resume:
    call dbyte_scheduler_run_once64
    test eax, eax
    jz .fail
    call dbyte_scheduler_run_count64
    cmp rax, 3
    jb .scheduler_resume
    cmp byte [dbyte_scheduler_live_seen], 0
    jne .scheduler_resume
    mov byte [dbyte_scheduler_live_seen], 1
    mov rsi, dbg_dbyte_sched_live
    call dbg_puts64
.scheduler_resume:
    sti
"@
$runtimeText = $runtimeText.Replace($eventResumeAnchor, $eventResume.TrimEnd("`r", "`n"))
$irqHandlerAnchor = "irq0_handler:`r`n    push rax`r`n    inc qword [irq0_count]`r`n    mov al, 0x20`r`n    out 0x20, al`r`n    pop rax`r`n    iretq"
Assert-True (([regex]::Matches($runtimeText, [regex]::Escape($irqHandlerAnchor))).Count -eq 1) "IRQ0 handler anchor is not unique"
$irqHandler = @"
irq0_handler:
    push rax
    push rcx
    push rdx
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    call dbyte_irq0_tick64
    mov al, 0x20
    out 0x20, al
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rax
    iretq
"@
$runtimeText = $runtimeText.Replace($irqHandlerAnchor, $irqHandler.TrimEnd("`r", "`n"))
Assert-True (([regex]::Matches($runtimeText, [regex]::Escape("irq0_count: dq 0"))).Count -eq 1) "IRQ0 ASM storage is not unique"
$runtimeText = $runtimeText.Replace("irq0_count: dq 0", "")

$privilegeCodeAnchor = "default_fault_handler:"
Assert-True (([regex]::Matches($runtimeText, [regex]::Escape($privilegeCodeAnchor))).Count -eq 1) "privilege code anchor is not unique"
$privilegeCode = @"
dbyrt64_load_call64:
    cmp dword [hopfs_mount_state64], 2
    jne .fail
    mov rsi, dbyrt64_name
    mov rdi, 0x01000000
    mov edx, 0x00040000
    call hopfs1_read_file64
    test eax, eax
    jz .fail
    cmp ecx, 64
    jb .fail
    mov rbx, 0x01000000
    mov r10, 0x3134365452594244
    cmp qword [rbx], r10
    jne .fail
    cmp dword [rbx + 8], 1
    jne .fail
    cmp dword [rbx + 12], 64
    jne .fail
    cmp dword [rbx + 16], ecx
    jne .fail
    mov eax, [rbx + 20]
    cmp eax, 64
    jb .fail
    test eax, 3
    jnz .fail
    cmp eax, ecx
    jae .fail
    mov eax, [rbx + 24]
    test eax, eax
    jz .fail
    test eax, 3
    jnz .fail
    mov edx, eax
    add edx, 64
    jc .fail
    cmp edx, ecx
    jne .fail
    cmp dword [rbx + 28], 0
    jne .fail
    cmp qword [rbx + 36], 0
    jne .fail
    cmp qword [rbx + 44], 0
    jne .fail
    cmp qword [rbx + 52], 0
    jne .fail
    cmp dword [rbx + 60], 0
    jne .fail
    mov r11d, [rbx + 32]
    lea rsi, [rbx + 64]
    mov ecx, eax
    shr ecx, 2
    xor edx, edx
.checksum:
    xor edx, [rsi]
    add rsi, 4
    loop .checksum
    cmp edx, r11d
    jne .fail
    mov edx, [rbx + 20]
    lea r11, [rbx + rdx]
    sub rsp, 8
    lea rax, [rel dbyrt64_host_services]
    call r11
    add rsp, 8
    mov r10, 0x444259525436344f
    cmp rax, r10
    jne .fail
    cmp dword [dbyrt64_fault_ud_count], 1
    jne .fail
    cmp dword [dbyrt64_fault_gp_count], 1
    jne .fail
    cmp dword [dbyrt64_fault_pf_count], 1
    jne .fail
    mov eax, 1
    ret
.fail:
    mov rsi, dbg_dbyrt64_rejected
    call dbg_puts64
    xor eax, eax
    ret

dbyrt64_host_read_file64:
    push rbx
    mov rsi, rax
    mov rdi, rdx
    mov edx, ecx
    call hopfs1_read_file64
    test eax, eax
    jz .failed
    mov [r8], rcx
    mov eax, 1
    pop rbx
    ret
.failed:
    xor ecx, ecx
    mov [r8], rcx
    xor eax, eax
    pop rbx
    ret

dbyrt64_host_abort_user64:
    cmp byte [dbyrt64_user_active], 1
    jne .fatal
    mov [dbyrt64_user_fault_reason], eax
    cmp eax, 6
    je .invalid_opcode
    cmp eax, 13
    je .general_protection
    cmp eax, 14
    je .page_fault
    jmp .fatal
.invalid_opcode:
    cmp rdx, 0x00400000
    jne .fatal
    test rcx, rcx
    jne .fatal
    inc dword [dbyrt64_fault_ud_count]
    jmp .recover
.general_protection:
    cmp rdx, 0x00400000
    jne .fatal
    test rcx, rcx
    jne .fatal
    inc dword [dbyrt64_fault_gp_count]
    jmp .recover
.page_fault:
    cmp rdx, 0x00900000
    jne .fatal
    cmp rcx, 4
    jne .fatal
    inc dword [dbyrt64_fault_pf_count]
.recover:
    mov [dbyrt64_user_exit_code], eax
    mov rax, [dbyrt64_user_kernel_cr3]
    mov cr3, rax
    mov rsp, [dbyrt64_user_kernel_rsp]
    jmp dbyrt64_host_enter_user64.returned
.fatal:
    cli
.fatal_halt:
    hlt
    jmp .fatal_halt

dbyrt64_host_enter_user64:
    cmp byte [dbyrt64_user_active], 0
    jne .busy
    test r8d, r8d
    jz .busy
    mov [dbyrt64_user_owner], r8d
    push rbx
    push rbp
    push r12
    push r13
    push r14
    push r15
    mov [dbyrt64_user_kernel_rsp], rsp
    mov r8, cr3
    mov [dbyrt64_user_kernel_cr3], r8
    mov byte [dbyrt64_user_active], 1
    mov byte [dbyrt64_fd_open], 0
    mov dword [dbyrt64_fd_owner], 0
    mov dword [dbyrt64_write_count], 0
    mov dword [dbyrt64_open_count], 0
    mov dword [dbyrt64_read_count], 0
    mov dword [dbyrt64_close_count], 0
    mov dword [dbyrt64_exit_count], 0
    mov dword [dbyrt64_efault_count], 0
    mov dword [dbyrt64_user_fault_reason], 0
    mov cr3, rax
    push qword 0x2B
    push rcx
    push qword 0x202
    push qword 0x23
    push rdx
    iretq
.returned:
    cmp byte [dbyrt64_fd_open], 0
    jne .lifecycle_failed
    cmp dword [dbyrt64_user_owner], 1
    jne .lifecycle_failed
    cmp dword [dbyrt64_user_fault_reason], 0
    jne .fault_returned
    cmp dword [dbyrt64_efault_count], 3
    jne .lifecycle_failed
    cmp dword [dbyrt64_write_count], 2
    jne .lifecycle_failed
    cmp dword [dbyrt64_open_count], 1
    jne .lifecycle_failed
    cmp dword [dbyrt64_read_count], 1
    jne .lifecycle_failed
    cmp dword [dbyrt64_close_count], 1
    jne .lifecycle_failed
    cmp dword [dbyrt64_exit_count], 1
    jne .lifecycle_failed
    jmp .restore
.fault_returned:
    mov eax, [dbyrt64_user_fault_reason]
    cmp [dbyrt64_user_exit_code], eax
    jne .lifecycle_failed
    jmp .restore
.lifecycle_failed:
    mov dword [dbyrt64_user_exit_code], -2
.restore:
    mov byte [dbyrt64_user_active], 0
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbp
    pop rbx
    mov eax, [dbyrt64_user_exit_code]
    ret
.busy:
    mov eax, -1
    ret

dbyrt64_validate_user_range64:
    ; RAX=user address, R9=length, R10D=required PTE flags (5=read, 7=write).
    ; Preserve user-visible RBX/RDX; the syscall ABI permits RAX/R9-R11 scratch.
    push rbx
    push rdx
    test r9, r9
    jz .ok
    cmp rax, 0x00400000
    jb .bad
    cmp rax, 0x00800000
    jae .bad
    mov r11, rax
    add r11, r9
    jc .bad
    cmp r11, rax
    jbe .bad
    cmp r11, 0x00800000
    ja .bad
    mov rbx, rax
.page:
    mov rdx, cr3
    and rdx, -4096
    mov rdx, [rdx]
    mov r9, rdx
    and r9d, r10d
    cmp r9d, r10d
    jne .bad
    and rdx, -4096
    mov rdx, [rdx]
    mov r9, rdx
    and r9d, r10d
    cmp r9d, r10d
    jne .bad
    and rdx, -4096
    mov r9, rbx
    shr r9, 21
    and r9d, 0x1FF
    mov rdx, [rdx + r9 * 8]
    mov r9, rdx
    and r9d, r10d
    cmp r9d, r10d
    jne .bad
    test dl, 0x80
    jnz .bad
    and rdx, -4096
    mov r9, rbx
    shr r9, 12
    and r9d, 0x1FF
    mov rdx, [rdx + r9 * 8]
    mov r9, rdx
    and r9d, r10d
    cmp r9d, r10d
    jne .bad
    and rbx, -4096
    add rbx, 4096
    cmp rbx, r11
    jb .page
.ok:
    mov eax, 1
    pop rdx
    pop rbx
    ret
.bad:
    xor eax, eax
    pop rdx
    pop rbx
    ret

setup_privilege64:
    mov qword [tss64_priv + 4], 0x00078000
    mov word [tss64_priv + 102], 104
    lea rax, [rel tss64_priv]
    mov word [gdt64_tss + 2], ax
    shr rax, 16
    mov byte [gdt64_tss + 4], al
    shr rax, 8
    mov byte [gdt64_tss + 7], al
    shr rax, 8
    mov dword [gdt64_tss + 8], eax
    lea rax, [rel gdt64_priv]
    mov qword [gdtr64_priv + 2], rax
    lgdt [rel gdtr64_priv]
    mov ax, 0x30
    ltr ax
    xor eax, eax
    str ax
    cmp ax, 0x30
    jne .fail
    mov eax, 0x80
    mov rdx, syscall80_handler64
    call set_idt_gate64
    mov byte [IDT_ADDR + (0x80 * 16) + 5], 0xEE
    mov eax, 1
    ret
.fail:
    xor eax, eax
    ret

privilege_probe64:
    or qword [0x1000], 4
    or qword [0x2000], 4
    or qword [0x3008], 4
    mov rax, cr3
    mov cr3, rax
    lea rsi, [rel user64_stub]
    mov rdi, 0x00200000
    mov ecx, user64_stub_end - user64_stub
    rep movsb
    mov [privilege_kernel_rsp], rsp
    push qword 0x2B
    push qword 0x003FF000
    push qword 2
    push qword 0x23
    push qword 0x00200000
    iretq

privilege_ring0_return64:
    cli
    and qword [0x3008], -5
    and qword [0x2000], -5
    and qword [0x1000], -5
    mov rax, cr3
    mov cr3, rax
    mov eax, 1
    ret

privilege_ring0_fail64:
    cli
    and qword [0x3008], -5
    and qword [0x2000], -5
    and qword [0x1000], -5
    mov rax, cr3
    mov cr3, rax
    xor eax, eax
    ret

syscall80_handler64:
    mov r10, [rsp + 8]
    and r10d, 3
    cmp r10d, 3
    jne .bad
    cmp byte [dbyrt64_user_active], 0
    jne .dbyrt64
    sub rsp, 8
    call dbyte_syscall_enter64
    add rsp, 8
    test eax, eax
    jz .bad
    mov rsp, [privilege_kernel_rsp]
    jmp privilege_ring0_return64
.bad:
    mov rsp, [privilege_kernel_rsp]
    jmp privilege_ring0_fail64
.dbyrt64:
    cmp eax, 1
    je .dbyrt64_write
    cmp eax, 3
    je .dbyrt64_open
    cmp eax, 4
    je .dbyrt64_read
    cmp eax, 5
    je .dbyrt64_close
    test eax, eax
    jz .dbyrt64_exit
    mov rax, -4
    iretq
.dbyrt64_write:
    cmp rdx, 1
    jne .dbyrt64_bad
    cmp r8, 4096
    ja .dbyrt64_bad
    mov rax, rcx
    mov r9, r8
    mov r10d, 5
    call dbyrt64_validate_user_range64
    test eax, eax
    jz .dbyrt64_efault
    mov r9, r8
.dbyrt64_write_loop:
    test r8, r8
    jz .dbyrt64_write_done
    mov al, [rcx]
    out 0xE9, al
    inc rcx
    dec r8
    jmp .dbyrt64_write_loop
.dbyrt64_write_done:
    inc dword [dbyrt64_write_count]
    mov rax, r9
    iretq
.dbyrt64_open:
    cmp rcx, 8
    jne .dbyrt64_bad
    cmp r8, 1
    jne .dbyrt64_bad
    cmp byte [dbyrt64_fd_open], 0
    jne .dbyrt64_bad
    mov rax, rdx
    mov r9, rcx
    mov r10d, 5
    call dbyrt64_validate_user_range64
    test eax, eax
    jz .dbyrt64_efault
    mov r10, 0x5458542e41544144
    cmp qword [rdx], r10
    jne .dbyrt64_bad
    mov byte [dbyrt64_fd_open], 1
    mov r10d, [dbyrt64_user_owner]
    mov [dbyrt64_fd_owner], r10d
    inc dword [dbyrt64_open_count]
    mov eax, 3
    iretq
.dbyrt64_read:
    cmp rdx, 3
    jne .dbyrt64_bad
    cmp byte [dbyrt64_fd_open], 1
    jne .dbyrt64_bad
    mov r10d, [dbyrt64_user_owner]
    cmp [dbyrt64_fd_owner], r10d
    jne .dbyrt64_bad
    cmp r8, 64
    jne .dbyrt64_bad
    mov rax, rcx
    mov r9, r8
    mov r10d, 7
    call dbyrt64_validate_user_range64
    test eax, eax
    jz .dbyrt64_efault
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    mov rdi, rcx
    mov rsi, dbyrt64_data_name
    mov edx, r8d
    sub rsp, 8
    call hopfs1_read_file64
    add rsp, 8
    test eax, eax
    jz .dbyrt64_read_failed
    inc dword [dbyrt64_read_count]
    mov rax, rcx
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    iretq
.dbyrt64_read_failed:
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    jmp .dbyrt64_bad
.dbyrt64_close:
    cmp rdx, 3
    jne .dbyrt64_bad
    cmp byte [dbyrt64_fd_open], 1
    jne .dbyrt64_bad
    mov r10d, [dbyrt64_user_owner]
    cmp [dbyrt64_fd_owner], r10d
    jne .dbyrt64_bad
    mov byte [dbyrt64_fd_open], 0
    inc dword [dbyrt64_close_count]
    xor eax, eax
    iretq
.dbyrt64_exit:
    inc dword [dbyrt64_exit_count]
    mov [dbyrt64_user_exit_code], edx
    mov rax, [dbyrt64_user_kernel_cr3]
    mov cr3, rax
    mov rsp, [dbyrt64_user_kernel_rsp]
    jmp dbyrt64_host_enter_user64.returned
.dbyrt64_efault:
    inc dword [dbyrt64_efault_count]
    mov rax, -3
    iretq
.dbyrt64_bad:
    mov rax, -5
    iretq

user64_stub:
    mov eax, 1
    int 0x80
    ud2
user64_stub_end:

$privilegeCodeAnchor
"@
$runtimeText = $runtimeText.Replace($privilegeCodeAnchor, $privilegeCode.TrimEnd("`r", "`n"))

$privilegeDataAnchor = "idtr64:"
Assert-True (([regex]::Matches($runtimeText, [regex]::Escape($privilegeDataAnchor))).Count -eq 1) "privilege data anchor is not unique"
$privilegeData = @"
align 8
gdt64_priv:
    dq 0x0000000000000000
    dq 0x00CF9A000000FFFF
    dq 0x00CF92000000FFFF
    dq 0x00AF9A000000FFFF
    dq 0x00AFFA000000FFFF
    dq 0x00CFF2000000FFFF
gdt64_tss:
    dw 0x0067
    dw 0
    db 0
    db 0x89
    db 0
    db 0
    dd 0
    dd 0
gdt64_priv_end:
gdtr64_priv:
    dw gdt64_priv_end - gdt64_priv - 1
    dq 0
align 16
tss64_priv:
    times 104 db 0
privilege_kernel_rsp:
    dq 0

$privilegeDataAnchor
"@
$runtimeText = $runtimeText.Replace($privilegeDataAnchor, $privilegeData.TrimEnd("`r", "`n"))
$runtimeAsmSource = Join-Path $runtimeRoot "kernel-dbyte-runtime.asm"
[IO.File]::WriteAllText($runtimeAsmSource, $runtimeText)

$runtimeObjectReceipts = @()
foreach ($opt in @(0, 1)) {
    $objects = @()
    foreach ($variant in @("a", "b")) {
        $variantRoot = Join-Path $runtimeRoot "object-o$opt-$variant"
        New-Item -ItemType Directory -Path $variantRoot | Out-Null
        $object = Join-Path $variantRoot "task-runtime.o"
        Invoke-Checked $compiler @(
            "build", $taskRuntimeSource, "--target", "x86_64-dbyte-none",
            "--emit", "obj", "--opt-level", "$opt", "-o", $object
        )
        $objects += [ordered]@{ path = $object; sha256 = Get-Sha256 $object }
    }
    Assert-True ($objects[0].sha256 -eq $objects[1].sha256) "O$opt task-runtime object A/B mismatch"
    $runtimeObjectReceipts += [ordered]@{
        opt_level = $opt
        object_sha256 = $objects[0].sha256
    }
}

$runtimeVariants = @()
foreach ($variant in @("a", "b")) {
    $variantRoot = Join-Path $runtimeRoot "live-o1-$variant"
    New-Item -ItemType Directory -Path $variantRoot | Out-Null
    $asmObject = Join-Path $variantRoot "kernel-boundary.o"
    $pmmObject = Join-Path $variantRoot "pmm-dbyte.o"
    $taskObject = Join-Path $variantRoot "task-runtime.o"
    $kernel = Join-Path $variantRoot "kernel.bin"
    Invoke-Checked $nasm @("-I", "$authorityRoot\", "-f", "elf64", $runtimeAsmSource, "-o", $asmObject)
    Invoke-Checked $compiler @("build", $hoppysPmmSource, "--target", "x86_64-dbyte-none", "--emit", "obj", "--opt-level", "1", "-o", $pmmObject)
    Invoke-Checked $compiler @("build", $taskRuntimeSource, "--target", "x86_64-dbyte-none", "--emit", "obj", "--opt-level", "1", "-o", $taskObject)
    Invoke-Checked $compiler @(
        "link", "--target", "x86_64-dbyte-none", "--flat-load-address", "0x10000",
        "--flat-max-size", "129024", "--flat-section-alignment", "16", "--entry", "kernel64_entry",
        $asmObject, $pmmObject, $taskObject, "-o", $kernel
    )
    $kernelBytes = (Get-Item -LiteralPath $kernel).Length
    $runtimeVariants += [ordered]@{
        root = $variantRoot
        asm_object_sha256 = Get-Sha256 $asmObject
        pmm_object_sha256 = Get-Sha256 $pmmObject
        task_object_sha256 = Get-Sha256 $taskObject
        kernel = $kernel
        kernel_sha256 = Get-Sha256 $kernel
        kernel_bytes = $kernelBytes
        kernel_sectors = [int][Math]::Ceiling($kernelBytes / 512.0)
    }
}
Assert-True ($runtimeVariants[0].asm_object_sha256 -eq $runtimeVariants[1].asm_object_sha256) "runtime boundary object A/B mismatch"
Assert-True ($runtimeVariants[0].pmm_object_sha256 -eq $runtimeVariants[1].pmm_object_sha256) "runtime PMM object A/B mismatch"
Assert-True ($runtimeVariants[0].task_object_sha256 -eq $runtimeVariants[1].task_object_sha256) "task-runtime object A/B mismatch"
Assert-True ($runtimeVariants[0].kernel_sha256 -eq $runtimeVariants[1].kernel_sha256) "runtime kernel A/B mismatch"
Assert-True ($runtimeVariants[0].kernel_sectors -le 252) "runtime kernel exceeds the HOPPYSOS sector ceiling"

$missingRuntime = Join-Path $runtimeRoot "must-not-link-without-runtime.bin"
$missingRuntimeExit = Invoke-ExpectedFailure $compiler @(
    "link", "--target", "x86_64-dbyte-none", "--flat-load-address", "0x10000",
    "--flat-max-size", "129024", "--flat-section-alignment", "16", "--entry", "kernel64_entry",
    (Join-Path $runtimeVariants[0].root "kernel-boundary.o"),
    (Join-Path $runtimeVariants[0].root "pmm-dbyte.o"), "-o", $missingRuntime
) (Join-Path $runtimeRoot "negative-missing-runtime")
Assert-True ($missingRuntimeExit -ne 0) "HOPPYSOS runtime boundary linked without DByte task runtime"
Assert-True (-not (Test-Path -LiteralPath $missingRuntime)) "failed missing-runtime link published output"

$runtimeBoot = Join-Path $runtimeRoot "boot.bin"
$runtimeStage2 = Join-Path $runtimeRoot "stage2.bin"
$runtimeBaseImage = Join-Path $runtimeRoot "hoppysos-dbyte-runtime-base.img"
$runtimeImage = Join-Path $runtimeRoot "hoppysos-dbyte-runtime.img"
Invoke-Checked $nasm @("-f", "bin", "-DHOS64_KERNEL_SECTORS=$($runtimeVariants[0].kernel_sectors)", $stage2Source, "-o", $runtimeStage2)
Invoke-Checked $nasm @("-f", "bin", $bootSource, "-o", $runtimeBoot)
Write-BootImage $runtimeBaseImage $runtimeBoot $runtimeStage2 $runtimeVariants[0].kernel

$moduleVariants = @()
foreach ($variant in @("a", "b")) {
    $variantRoot = Join-Path $runtimeRoot "module-o1-$variant"
    New-Item -ItemType Directory -Path $variantRoot | Out-Null
    $object = Join-Path $variantRoot "runtime-module.o"
    $payload = Join-Path $variantRoot "runtime-module.bin"
    $package = Join-Path $variantRoot "DBYRT641"
    Invoke-Checked $compiler @(
        "build", $runtimeModuleSource, "--target", "x86_64-dbyte-none",
        "--emit", "obj", "--opt-level", "1", "-o", $object
    )
    Invoke-Checked $compiler @(
        "link", "--target", "x86_64-dbyte-none",
        "--flat-load-address", "0x01000040", "--flat-max-size", "196544",
        "--flat-section-alignment", "16", "--entry", "dbyrt64_entry",
        $object, "-o", $payload
    )
    Invoke-Checked $python @(
        $runtimeModulePacker, $payload, $package,
        "--entry-object", $object, "--entry-symbol", "dbyrt64_entry"
    )
    $packageBytes = [IO.File]::ReadAllBytes($package)
    $entryOffset = [BitConverter]::ToUInt32($packageBytes, 20)
    Assert-True ($entryOffset -ge 64 -and $entryOffset -lt $packageBytes.Length) "DBYRT64 package entry is out of bounds"
    Assert-True (($entryOffset % 4) -eq 0) "DBYRT64 package entry is not four-byte aligned"
    $moduleVariants += [ordered]@{
        object_sha256 = Get-Sha256 $object
        payload_sha256 = Get-Sha256 $payload
        payload_bytes = (Get-Item -LiteralPath $payload).Length
        package = $package
        package_sha256 = Get-Sha256 $package
        package_bytes = (Get-Item -LiteralPath $package).Length
        entry_offset = $entryOffset
    }
}
Assert-True ($moduleVariants[0].object_sha256 -eq $moduleVariants[1].object_sha256) "DBYRT64 object A/B mismatch"
Assert-True ($moduleVariants[0].payload_sha256 -eq $moduleVariants[1].payload_sha256) "DBYRT64 payload A/B mismatch"
Assert-True ($moduleVariants[0].package_sha256 -eq $moduleVariants[1].package_sha256) "DBYRT64 package A/B mismatch"
Assert-True ($moduleVariants[0].entry_offset -eq $moduleVariants[1].entry_offset) "DBYRT64 entry offset A/B mismatch"

$runtimeUserVariants = @()
foreach ($variant in @("a", "b")) {
    $userRoot = Join-Path $runtimeRoot "user-o1-$variant"
    New-Item -ItemType Directory -Path $userRoot | Out-Null
    $userElf = Join-Path $userRoot "USER.ELF"
    Invoke-Checked $compiler @(
        "build", $runtimeUserSource, "--target", "x86_64-dbyte-user",
        "--emit", "elf", "--opt-level", "1", "-o", $userElf
    )
    Assert-True ((Get-Item -LiteralPath $userElf).Length -le 4096) "DBYRT64 user ELF exceeds bounded read capacity"
    $runtimeUserVariants += [ordered]@{
        path = $userElf
        sha256 = Get-Sha256 $userElf
        bytes = (Get-Item -LiteralPath $userElf).Length
    }
}
Assert-True ($runtimeUserVariants[0].sha256 -eq $runtimeUserVariants[1].sha256) "DBYRT64 user ELF A/B mismatch"

Invoke-Checked $python @($runtimeHopfsFixtureBuilder, $runtimeBaseImage, $moduleVariants[0].package, $runtimeImage, "--user", $runtimeUserVariants[0].path, "--data", $runtimeDataPath)
$runtimeImageSha256 = Get-Sha256 $runtimeImage
$runtimeQemuReceipts = @()
foreach ($memory in @(32, 64)) {
    $qemuEvidence = Join-Path $runtimeRoot "qemu-$memory"
    New-Item -ItemType Directory -Path $qemuEvidence | Out-Null
    $runtimeQemuReceipts += Invoke-QemuProof $qemu $runtimeImage $qemuEvidence $memory @(
        "HOPPYSOS64:LONG_MODE_OK",
        "HOPPYSOS64:PMM_OK",
        "HOPPYSOS64:PMM_GUARD_OK",
        "HOPPYSOS64:DBYTE_TASK_OK",
        "HOPPYSOS64:DBYTE_IRQ0_LIVE",
        "HOPPYSOS64:DBYTE_TSS_OK",
        "HOPPYSOS64:DBYTE_RING3_SYSCALL_OK",
        "HOPPYSOS64:DBYTE_RING0_RETURN_OK",
            "HOPPYSOS64:DBYRT64_LIVE",
            "HOPPYSOS64:DBYRT64_HOST_ABI_OK",
            "HOPPYSOS64:DBYRT64_USER_ELF_OK",
            "HOPPYSOS64:DBYRT64_PRIVATE_CR3_OK",
            "DBYRT64 WORKER START",
            "DBYRT64 WORKER OK",
            "HOPPYSOS64:DBYRT64_USER_EXEC_OK",
            "HOPPYSOS64:DBYRT64_FAULT_UD_OK",
            "HOPPYSOS64:DBYRT64_FAULT_GP_OK",
            "HOPPYSOS64:DBYRT64_FAULT_PF_OK",
        "HOPPYSOS64:DBYTE_SCHED_LIVE",
        "HOPPYSOS64:READY"
    )
}

$runtimeNegativeRoot = Join-Path $runtimeRoot "module-negatives"
New-Item -ItemType Directory -Path $runtimeNegativeRoot | Out-Null
$runtimeNegativeReceipts = @()

$missingImage = Join-Path $runtimeNegativeRoot "missing-module.img"
Invoke-Checked $python @($runtimeHopfsFixtureBuilder, $runtimeBaseImage, $moduleVariants[0].package, $missingImage, "--empty", "--user", $runtimeUserVariants[0].path, "--data", $runtimeDataPath)
$missingEvidence = Join-Path $runtimeNegativeRoot "missing-module"
New-Item -ItemType Directory -Path $missingEvidence | Out-Null
$runtimeNegativeReceipts += Invoke-QemuRejectProof $qemu $missingImage $missingEvidence "missing-module"

$missingUserImage = Join-Path $runtimeNegativeRoot "missing-user.img"
Invoke-Checked $python @($runtimeHopfsFixtureBuilder, $runtimeBaseImage, $moduleVariants[0].package, $missingUserImage, "--data", $runtimeDataPath)
$missingUserEvidence = Join-Path $runtimeNegativeRoot "missing-user"
New-Item -ItemType Directory -Path $missingUserEvidence | Out-Null
$runtimeNegativeReceipts += Invoke-QemuRejectProof $qemu $missingUserImage $missingUserEvidence "missing-user"

foreach ($case in @(
    [ordered]@{ name = "corrupt-magic"; offset = 0 },
    [ordered]@{ name = "corrupt-xor"; offset = 32 }
)) {
    $package = Join-Path $runtimeNegativeRoot "$($case.name).DBYRT641"
    $bytes = [IO.File]::ReadAllBytes($moduleVariants[0].package)
    $bytes[$case.offset] = $bytes[$case.offset] -bxor 1
    [IO.File]::WriteAllBytes($package, $bytes)
    $image = Join-Path $runtimeNegativeRoot "$($case.name).img"
    Invoke-Checked $python @($runtimeHopfsFixtureBuilder, $runtimeBaseImage, $package, $image, "--user", $runtimeUserVariants[0].path, "--data", $runtimeDataPath)
    $caseEvidence = Join-Path $runtimeNegativeRoot $case.name
    New-Item -ItemType Directory -Path $caseEvidence | Out-Null
    $runtimeNegativeReceipts += Invoke-QemuRejectProof $qemu $image $caseEvidence $case.name
}

$invalidEntryPackage = Join-Path $runtimeNegativeRoot "invalid-entry-offset.DBYRT641"
$invalidEntryBytes = [IO.File]::ReadAllBytes($moduleVariants[0].package)
[Array]::Copy([BitConverter]::GetBytes([uint32]65), 0, $invalidEntryBytes, 20, 4)
[IO.File]::WriteAllBytes($invalidEntryPackage, $invalidEntryBytes)
$invalidEntryImage = Join-Path $runtimeNegativeRoot "invalid-entry-offset.img"
Invoke-Checked $python @($runtimeHopfsFixtureBuilder, $runtimeBaseImage, $invalidEntryPackage, $invalidEntryImage, "--user", $runtimeUserVariants[0].path, "--data", $runtimeDataPath)
$invalidEntryEvidence = Join-Path $runtimeNegativeRoot "invalid-entry-offset"
New-Item -ItemType Directory -Path $invalidEntryEvidence | Out-Null
$runtimeNegativeReceipts += Invoke-QemuRejectProof $qemu $invalidEntryImage $invalidEntryEvidence "invalid-entry-offset"

$oversizePayload = Join-Path $runtimeNegativeRoot "oversize-payload.bin"
$oversizePackage = Join-Path $runtimeNegativeRoot "must-not-exist.DBYRT641"
[IO.File]::WriteAllBytes($oversizePayload, (New-Object byte[] 196545))
$oversizeExit = Invoke-ExpectedFailure $python @(
    $runtimeModulePacker, $oversizePayload, $oversizePackage
) (Join-Path $runtimeNegativeRoot "oversize-package")
Assert-True ($oversizeExit -ne 0) "oversize DBYRT64 package unexpectedly succeeded"
Assert-True (-not (Test-Path -LiteralPath $oversizePackage)) "oversize DBYRT64 packer published output"

$missingEntryPackage = Join-Path $runtimeNegativeRoot "must-not-exist-missing-entry.DBYRT641"
$missingEntryExit = Invoke-ExpectedFailure $python @(
    $moduleVariants[0].package, $missingEntryPackage,
    "--entry-object", (Join-Path $runtimeVariants[0].root "task-runtime.o"),
    "--entry-symbol", "dbyrt64_entry"
) (Join-Path $runtimeNegativeRoot "missing-entry-symbol")
Assert-True ($missingEntryExit -ne 0) "missing DBYRT64 entry symbol unexpectedly succeeded"
Assert-True (-not (Test-Path -LiteralPath $missingEntryPackage)) "missing-entry packer published output"

$headAfter = (& git rev-parse HEAD).Trim()
$tagObjectAfter = (& git rev-parse "refs/tags/v13.1.0").Trim()
$tagCommitAfter = (& git rev-parse "v13.1.0^{}").Trim()
$authoritativeKernelHashAfter = Get-Sha256 $authoritativeKernelSource
$authoritativeStateHashAfter = Get-Sha256 $authoritativeStateSource
$legacyKernelReferenceHashAfter = Get-Sha256 $legacyKernelReference
$protectedSmokeHashAfter = Get-Sha256 $protectedSmokeSource
$protectedCubeStateHashAfter = Get-Sha256 $protectedCubeStateSource
$verifierHashAfter = Get-Sha256 $PSCommandPath
$gitStatusAfter = ((& git status --porcelain=v1) -join "`n")
$runtimeHeadroomBytes = 129024 - $runtimeVariants[0].kernel_bytes
$runtimeHeadroomSectors = 252 - $runtimeVariants[0].kernel_sectors
Assert-True ($headAfter -eq $headBefore) "HEAD changed during verification"
Assert-True ($tagObjectAfter -eq $tagObjectBefore) "v13.1.0 tag object changed during verification"
Assert-True ($tagCommitAfter -eq $tagCommitBefore) "v13.1.0 peeled commit changed during verification"
Assert-True ($authoritativeKernelHashAfter -eq $authoritativeKernelHashBefore) "authoritative HOPPYSOS x64 kernel source changed during verification"
Assert-True ($authoritativeStateHashAfter -eq $authoritativeStateHashBefore) "authoritative HOPPYSOS STATE.txt changed during verification"
Assert-True ($legacyKernelReferenceHashAfter -eq $legacyKernelReferenceHash) "legacy reference kernel changed during verification"
Assert-True ($protectedSmokeHashAfter -eq $protectedSmokeHashBefore) "protected examples/smoke.dby changed during verification"
Assert-True ($protectedCubeStateHashAfter -eq $protectedCubeStateHashBefore) "protected examples/.cube-state.bin changed during verification"
Assert-True ($verifierHashAfter -eq $verifierHashBefore) "verifier script changed during verification"
Assert-True ($gitStatusAfter -eq $gitStatusBefore) "Git working-tree status changed during verification"
Assert-True ($runtimeHeadroomBytes -ge 0 -and $runtimeHeadroomSectors -ge 0) "runtime kernel exceeded frozen loader capacity"
foreach ($copy in $authorityCopies) {
    $destination = Join-Path $authorityRoot ($copy.relative -replace '/', '\')
    Assert-True ((Get-Sha256 $destination) -eq $copy.sha256) "frozen authority bundle changed during verification: $($copy.relative)"
    Assert-True ((Get-Sha256 $copy.source) -eq $copy.sha256) "frozen authority source changed during verification: $($copy.relative)"
}
$liveKernelDriftFromFrozen = $authoritativeKernelHashBefore -ne $frozenAuthority.checkpoint_sha256

$result = [ordered]@{
    schema = "dbyte-hoppysos-flat-seam-v7"
    result = "PASS"
    head = $headAfter
    verifier_sha256 = $verifierHashAfter
    compiler_sha256 = Get-Sha256 $compiler
    assembler_sha256 = Get-Sha256 $assembler
    sources = [ordered]@{
        dbyte = Get-Sha256 $dbyteSource
        pmm = Get-Sha256 $pmmSource
        hoppysos_pmm = Get-Sha256 $hoppysPmmSource
        hoppysos_pmm_core = Get-Sha256 $pmmCoreSource
        task_runtime = Get-Sha256 $taskRuntimeSource
        runtime_module = Get-Sha256 $runtimeModuleSource
        runtime_host_abi = Get-Sha256 $runtimeHostAbiSource
        runtime_user = Get-Sha256 $runtimeUserSource
        runtime_data = Get-Sha256 $runtimeDataPath
        runtime_module_packer = Get-Sha256 $runtimeModulePacker
        runtime_hopfs_fixture_builder = Get-Sha256 $runtimeHopfsFixtureBuilder
        dbtasm_boundary = Get-Sha256 $boundarySource
        boot = Get-Sha256 $bootSource
        stage2 = Get-Sha256 $stage2Source
    }
    load_address = "0x00010000"
    maximum_bytes = 129024
    real_storage = "DISARMED"
    hoppysos_tree_mutated = $false
    frozen_identity = [ordered]@{
        expected_v13_1_commit = $expectedV131Commit
        head_before = $headBefore
        head_after = $headAfter
        tag_object_before = $tagObjectBefore
        tag_object_after = $tagObjectAfter
        tag_commit_before = $tagCommitBefore
        tag_commit_after = $tagCommitAfter
        git_status_unchanged = ($gitStatusAfter -eq $gitStatusBefore)
    }
    protected_state = [ordered]@{
        live_kernel_sha256_before = $authoritativeKernelHashBefore
        live_kernel_sha256_after = $authoritativeKernelHashAfter
        live_kernel_drift_from_frozen = $liveKernelDriftFromFrozen
        authoritative_state_sha256_before = $authoritativeStateHashBefore
        authoritative_state_sha256_after = $authoritativeStateHashAfter
        smoke_sha256_before = $protectedSmokeHashBefore
        smoke_sha256_after = $protectedSmokeHashAfter
        cube_state_sha256_before = $protectedCubeStateHashBefore
        cube_state_sha256_after = $protectedCubeStateHashAfter
    }
    frozen_authority = [ordered]@{
        manifest_sha256 = $frozenAuthorityManifestSha256
        manifest_schema = $frozenAuthority.schema
        checkpoint_path = $frozenAuthority.checkpoint_rel
        checkpoint_sha256 = $frozenAuthority.checkpoint_sha256
        canonical_kernel_expected_sha256 = $frozenAuthority.canonical_kernel_sha256
        canonical_kernel_observed_sha256 = $canonicalKernelHash
        canonical_kernel_expected_bytes = [int64]$frozenAuthority.canonical_kernel_bytes
        canonical_kernel_observed_bytes = $canonicalKernelBytes
        transitive_include_count = $frozenAuthority.includes.Count
        verifier_owned_bundle = $true
        live_top_level_kernel_used_as_input = $false
    }
    evidence_provenance = [ordered]@{
        current_x86_64 = "generated proof source from hash-bound frozen HOPPYSOS x64 checkpoint authority"
        live_tree = "observed for drift only; live top-level kernel is not a proof input"
        legacy_compat = "reference only; never counted as current x86-64 implementation evidence"
        legacy_kernel_reference_sha256 = $legacyKernelReferenceHash
        legacy_promoted_to_current = $false
    }
    receipts = $receipts
    full_hoppysos_relink = [ordered]@{
        canonical_kernel_sha256 = $canonicalKernelHash
        linked_kernel_sha256 = $fullVariants[0].kernel_sha256
        linked_kernel_bytes = $fullVariants[0].kernel_bytes
        linked_kernel_sectors = $fullSectors
        exact_byte_match = $true
        image_sha256 = Get-Sha256 $fullImage
        qemu = $fullQemu
    }
    full_hoppysos_dbyte_pmm = [ordered]@{
        generated_source_sha256 = Get-Sha256 $migratedSource
        asm_pmm_implementation_removed = $true
        asm_pmm_storage_removed = $true
        receipts = $migrationReceipts
    }
    full_hoppysos_dbyte_runtime = [ordered]@{
        generated_source_sha256 = Get-Sha256 $runtimeAsmSource
        object_oracles = $runtimeObjectReceipts
        live_opt_level = 1
        kernel_sha256 = $runtimeVariants[0].kernel_sha256
        kernel_bytes = $runtimeVariants[0].kernel_bytes
        kernel_sectors = $runtimeVariants[0].kernel_sectors
        kernel_headroom_bytes = $runtimeHeadroomBytes
        kernel_headroom_sectors = $runtimeHeadroomSectors
        image_sha256 = $runtimeImageSha256
        missing_runtime_rejected = $true
        irq0_asm_counter_removed = $true
        irq0_dbyte_interrupt_path = $true
        scheduler_dbyte_event_loop_path = $true
        privilege_foundation_new_x86_64 = $true
        privilege_legacy_promoted = $false
        tss_selector = "0x30"
        syscall_vector = "0x80"
        user_code_va = "0x00200000"
        user_stack_va = "0x003FF000"
        user_mapping_window = "0x00200000-0x003FFFFF"
        low_kernel_mapping_remains_supervisor = $true
        user_mapping_restored_supervisor_after_return = $true
        expected_syscall_count = 1
        expected_last_syscall = 1
        dbyrt64 = [ordered]@{
            object_sha256 = $moduleVariants[0].object_sha256
            payload_sha256 = $moduleVariants[0].payload_sha256
            payload_bytes = $moduleVariants[0].payload_bytes
            package_sha256 = $moduleVariants[0].package_sha256
            package_bytes = $moduleVariants[0].package_bytes
            fixture_sha256 = $runtimeImageSha256
            load_address = "0x01000000"
            payload_link_address = "0x01000040"
            entry_offset = $moduleVariants[0].entry_offset
            entry_symbol_derived_from_object = $true
            host_abi_version = 2
            host_abi_table_bytes = 48
            host_abi_alloc_free_round_trip = $true
            user_elf_sha256 = $runtimeUserVariants[0].sha256
            user_elf_bytes = $runtimeUserVariants[0].bytes
            user_elf_deterministic = $true
            user_elf_hopfs_read = $true
            user_elf_header_and_load_segments_validated = $true
            private_cr3_page_tables_constructed = $true
            private_cr3_user_code_and_stack_mapped = $true
            private_cr3_lifecycles = 4
            private_cr3_frames_reclaimed_each = 8
            private_cr3_frames_reclaimed_total = 32
            private_cr3_activated = $true
            ring3_user_executed = $true
            ring3_syscalls = [ordered]@{
                total = 9
                write = 2
                open = 1
                read = 1
                close = 1
                exit = 1
                efault_rejections = 3
            }
            user_pointer_ranges_validated = $true
            user_write_requires_writable_mapping = $true
            owner_qualified_fd = $true
            process_owner = 1
            process_lifecycle = "worker-plus-three-contained-fault-lifecycles"
            cpl3_fault_containment = [ordered]@{
                invalid_opcode = [ordered]@{ vector = 6; error = 0; rip = "0x00400000"; count = 1 }
                general_protection = [ordered]@{ vector = 13; error = 0; rip = "0x00400000"; count = 1 }
                page_fault = [ordered]@{ vector = 14; error = 4; cr2 = "0x00900000"; count = 1 }
                kernel_cr3_restored = $true
                kernel_stack_restored = $true
            }
            deterministic_object = $true
            deterministic_payload = $true
            deterministic_package = $true
            rejection_cases = $runtimeNegativeReceipts
            oversize_package_rejected = $true
            missing_entry_symbol_rejected = $true
        }
        qemu = $runtimeQemuReceipts
    }
}
$resultPath = Join-Path $evidenceRoot "RESULT.json"
$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $resultPath -Encoding utf8
Write-Host "DBYTE HOPPYSOS FLAT SEAM: PASS"
Write-Host "EVIDENCE_ROOT=$evidenceRoot"
Write-Host "RESULT_SHA256=$(Get-Sha256 $resultPath)"
