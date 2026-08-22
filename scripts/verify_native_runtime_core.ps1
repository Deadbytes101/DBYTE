param(
    [string]$MilestoneVersion = "11.4.0"
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$tmpRoot = (Resolve-Path (Join-Path $repoRoot "tmp")).Path
$proofRoot = Join-Path $tmpRoot "native_v$MilestoneVersion"
$repeatRoot = Join-Path $tmpRoot "native_v$MilestoneVersion.repeat"
$sourceRoot = Join-Path $repoRoot "examples\native_runtime_core"
$modules = @("arch_x86", "serial", "gdt", "exceptions", "boot", "frames", "paging", "page_fault", "idt", "memory_ops", "heap", "proof", "kernel")
$expectedSerial = "DBYTEOS NATIVE RUNTIME`r`nVIRTUAL ARENA ONLINE`r`nKERNEL HEAP ONLINE`r`nALLOC 001 OK`r`nALLOC 002 OK`r`nFREE REUSE OK`r`nALIGNMENT OK`r`nMEMORY OPS OK`r`nRUNTIME CHECK OK`r`n"

function Assert-True($Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Assert-Contains([string]$Text, [string]$Needle, [string]$Description) {
    Assert-True ($Text.Contains($Needle)) "missing ${Description}: $Needle"
}

function Invoke-Checked([string]$Executable, [string[]]$Arguments, [string]$Description) {
    $savedPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $Executable @Arguments 2>&1
        $exitCode = $LASTEXITCODE
    }
    finally { $ErrorActionPreference = $savedPreference }
    if ($exitCode -ne 0) {
        throw "$Description failed with exit code ${exitCode}:`n$($output -join "`n")"
    }
    return ($output -join "`n")
}

function Assert-SameFile([string]$Left, [string]$Right, [string]$Description) {
    [byte[]]$leftBytes = [System.IO.File]::ReadAllBytes($Left)
    [byte[]]$rightBytes = [System.IO.File]::ReadAllBytes($Right)
    Assert-True ([System.Linq.Enumerable]::SequenceEqual($leftBytes, $rightBytes)) "$Description is not byte-for-byte deterministic"
}

function Reset-ProofDirectory([string]$Path) {
    $full = [System.IO.Path]::GetFullPath($Path)
    $prefix = $tmpRoot.TrimEnd('\') + '\'
    Assert-True ($full.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) "proof path escapes tmp: $full"
    if (Test-Path -LiteralPath $full) { Remove-Item -LiteralPath $full -Recurse -Force }
    New-Item -ItemType Directory -Path $full | Out-Null
}

function Read-Telemetry([string]$Path) {
    $values = @{}
    $text = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($Path))
    foreach ($line in ($text -split "`r`n")) {
        if ($line -match '^([A-Z0-9_]+)=([0-9A-F]{8})$') {
            Assert-True (-not $values.ContainsKey($Matches[1])) "duplicate debugcon telemetry '$($Matches[1])'"
            $values[$Matches[1]] = [Convert]::ToUInt32($Matches[2], 16)
        }
    }
    return $values
}

function Invoke-QemuProof([int]$RamMiB, [string]$Elf, [string]$Qemu) {
    $serialPath = Join-Path $proofRoot "qemu.${RamMiB}m.serial.log"
    $tracePath = Join-Path $proofRoot "qemu.${RamMiB}m.trace.log"
    $debugconPath = Join-Path $proofRoot "qemu.${RamMiB}m.debugcon.log"
    $arguments = @(
        "-m", "${RamMiB}M", "-kernel", $Elf, "-display", "none",
        "-serial", "file:$serialPath", "-monitor", "none", "-no-reboot", "-no-shutdown",
        "-d", "int,cpu_reset,guest_errors", "-D", $tracePath,
        "-chardev", "file,id=dbyte_debugcon,path=$debugconPath",
        "-device", "isa-debugcon,iobase=0xe9,chardev=dbyte_debugcon"
    )
    $command = "$Qemu $($arguments -join ' ')"
    [System.IO.File]::WriteAllText((Join-Path $proofRoot "qemu.${RamMiB}m.command.txt"), $command)
    $process = Start-Process -FilePath $Qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        if (-not $process.WaitForExit(12000)) {
            Stop-Process -Id $process.Id -Force
            $process.WaitForExit()
        }
    }
    finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    }
    Assert-True (Test-Path -LiteralPath $serialPath) "QEMU ${RamMiB} MiB serial log is missing"
    Assert-True (Test-Path -LiteralPath $tracePath) "QEMU ${RamMiB} MiB trace is missing"
    Assert-True (Test-Path -LiteralPath $debugconPath) "QEMU ${RamMiB} MiB debugcon log is missing"
    $serial = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($serialPath))
    Assert-True ($serial -ceq $expectedSerial) "QEMU ${RamMiB} MiB serial output mismatch:`n$serial"
    $trace = Get-Content -LiteralPath $tracePath -Raw
    Assert-True (-not [regex]::IsMatch($trace, 'v=0e', [System.Text.RegularExpressions.RegexOptions]::IgnoreCase)) "QEMU ${RamMiB} MiB has an unexpected page fault"
    Assert-True (-not $trace.Contains("Triple fault")) "QEMU ${RamMiB} MiB trace contains a triple fault"
    $telemetry = Read-Telemetry $debugconPath
    $required = @("ARENA_START", "ARENA_END", "ALLOC_ONE", "ALLOC_TWO", "REUSE", "ALIGNED_4096", "LARGE", "PAGES_BEFORE", "PAGES_LARGE", "PAGES_FINAL", "HIGH_WATER", "MAP_COUNT", "UNMAP_COUNT", "INVLPG_COUNT", "FRAME_ALLOCS", "FRAME_RELEASES", "COALESCE", "LAST_ERROR", "CR3", "CR0", "FRAME_1", "FRAME_2", "FRAME_3", "FRAME_4", "FRAME_5", "FRAME_6", "FRAME_7")
    foreach ($key in $required) { Assert-True ($telemetry.ContainsKey($key)) "QEMU ${RamMiB} MiB is missing telemetry '$key'" }
    Assert-True ([uint64]$telemetry.ARENA_START -eq 3238002688) "arena start is not locked to 0xC1000000"
    Assert-True ([uint64]$telemetry.ARENA_END -eq 3242196992) "arena end is not locked to 0xC1400000"
    Assert-True ($telemetry.REUSE -eq $telemetry.ALLOC_ONE) "freed block was not reused at the exact address"
    Assert-True (($telemetry.ALIGNED_4096 -band 0xfff) -eq 0) "4096-byte allocation is not aligned"
    Assert-True ($telemetry.PAGES_LARGE -gt $telemetry.PAGES_BEFORE) "large allocation did not grow the arena"
    Assert-True ($telemetry.PAGES_FINAL -lt $telemetry.PAGES_LARGE) "tail free did not decommit pages"
    Assert-True ($telemetry.PAGES_FINAL -ge 1) "heap did not retain its metadata page"
    Assert-True ($telemetry.HIGH_WATER -ge $telemetry.PAGES_LARGE) "heap high-water accounting is wrong"
    Assert-True ($telemetry.MAP_COUNT -eq $telemetry.FRAME_ALLOCS) "mapped pages and owned-frame allocations differ"
    Assert-True ($telemetry.UNMAP_COUNT -eq $telemetry.FRAME_RELEASES) "unmapped pages and frame releases differ"
    Assert-True ($telemetry.INVLPG_COUNT -eq ($telemetry.MAP_COUNT + $telemetry.UNMAP_COUNT)) "TLB invalidation count does not cover every mapping change"
    Assert-True (($telemetry.FRAME_ALLOCS - $telemetry.FRAME_RELEASES) -eq $telemetry.PAGES_FINAL) "live frame ownership does not equal committed pages"
    Assert-True ($telemetry.COALESCE -gt 0) "heap never coalesced adjacent free blocks"
    Assert-True ($telemetry.LAST_ERROR -eq 0) "heap did not finish in a consistent success state"
    Assert-True (($telemetry.CR3 -band 0xfff) -eq 0) "CR3 is not page aligned"
    Assert-True (([uint64]$telemetry.CR0 -band 2147483648) -ne 0) "CR0.PG is not enabled"
    $initialFrames = @($telemetry.FRAME_1, $telemetry.FRAME_2, $telemetry.FRAME_3, $telemetry.FRAME_4, $telemetry.FRAME_5, $telemetry.FRAME_6, $telemetry.FRAME_7)
    Assert-True (($initialFrames | Select-Object -Unique).Count -eq $initialFrames.Count) "concurrent arena mappings reused a physical frame"
    foreach ($frame in $initialFrames) {
        Assert-True (($frame -band 0xfff) -eq 0) "mapped physical frame is not aligned"
        Assert-True ($frame -ge 0x00100000) "mapped physical frame is below 1 MiB"
    }
    Start-Sleep -Milliseconds 100
    $orphan = Get-Process -Name "qemu-system-i386" -ErrorAction SilentlyContinue | Where-Object { $_.Id -eq $process.Id }
    Assert-True ($null -eq $orphan) "QEMU process $($process.Id) remains alive"
    return @{ Command = $command; Serial = $serial; Telemetry = $telemetry }
}

Reset-ProofDirectory $proofRoot
Reset-ProofDirectory $repeatRoot
$cargo = (Get-Command cargo -ErrorAction Stop).Source
$objdump = (Get-Command objdump -ErrorAction Stop).Source
$qemu = (Get-Command qemu-system-i386 -ErrorAction Stop).Source
Invoke-Checked $cargo @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build dbytec" | Out-Null
$compiler = Join-Path $repoRoot "target\debug\dbytec.exe"
Assert-True (Test-Path -LiteralPath $compiler) "dbytec executable is missing"

$pc32Count = 0
$abs32Count = 0
foreach ($module in $modules) {
    $source = Join-Path $sourceRoot "$module.dby"
    $first = Join-Path $proofRoot "$module.o"
    $second = Join-Path $repeatRoot "$module.o"
    Invoke-Checked $compiler @("build", $source, "--target", "i686-dbyte-none", "--emit", "obj", "-o", $first) "build $module object" | Out-Null
    Invoke-Checked $compiler @("build", $source, "--target", "i686-dbyte-none", "--emit", "obj", "-o", $second) "repeat $module object" | Out-Null
    Assert-SameFile $first $second "$module object"
    $inspection = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", $first) "inspect $module object"
    [System.IO.File]::WriteAllText((Join-Path $proofRoot "$module.objdump.txt"), $inspection)
    Assert-Contains $inspection "file format elf32-i386" "$module ELF32 identity"
    Assert-Contains $inspection ".dbyte.interrupts" "$module interrupt metadata section"
    Assert-Contains $inspection ".dbyte.entry" "$module entry metadata section"
    $pc32Count += ([regex]::Matches($inspection, 'R_386_PC32')).Count
    $abs32Count += ([regex]::Matches($inspection, 'R_386_32')).Count
}

$rootSource = Join-Path $sourceRoot "kernel.dby"
$elf = Join-Path $proofRoot "dbyte_native_runtime_v$MilestoneVersion.elf"
$elfRepeat = Join-Path $repeatRoot "dbyte_native_runtime_v$MilestoneVersion.elf"
$bin = Join-Path $proofRoot "dbyte_native_runtime_v$MilestoneVersion.bin"
$binRepeat = Join-Path $repeatRoot "dbyte_native_runtime_v$MilestoneVersion.bin"
Invoke-Checked $compiler @("build", $rootSource, "--target", "i686-dbyte-none", "--emit", "elf", "-o", $elf) "build native runtime ELF" | Out-Null
Invoke-Checked $compiler @("build", $rootSource, "--target", "i686-dbyte-none", "--emit", "elf", "-o", $elfRepeat) "repeat native runtime ELF" | Out-Null
Invoke-Checked $compiler @("build", $rootSource, "--target", "i686-dbyte-none", "--emit", "bin", "-o", $bin) "build native runtime BIN" | Out-Null
Invoke-Checked $compiler @("build", $rootSource, "--target", "i686-dbyte-none", "--emit", "bin", "-o", $binRepeat) "repeat native runtime BIN" | Out-Null
Assert-SameFile $elf $elfRepeat "linked native runtime ELF"
Assert-SameFile $bin $binRepeat "linked native runtime BIN"

$finalInspection = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", $elf) "inspect final native runtime ELF"
[System.IO.File]::WriteAllText((Join-Path $proofRoot "final.objdump.txt"), $finalInspection)
foreach ($needle in @("<_start>", "push   eax", "push   ebx", "mov    eax,cr0", "mov    cr0,eax", "mov    eax,cr3", "mov    cr3,eax", "invlpg", "<map_kernel_page>", "<unmap_kernel_page>", "<alloc>", "<free>", "<memset>", "<memcpy>", "<memmove>", "<page_fault>", "iret")) {
    Assert-Contains $finalInspection $needle "final runtime machine code"
}
Assert-True (-not $finalInspection.Contains(".dbyte.entry")) "entry metadata leaked into final executable"
Assert-True (-not $finalInspection.Contains(".dbyte.interrupts")) "interrupt metadata leaked into final executable"
Assert-True (-not $finalInspection.Contains(".interp")) "final ELF contains PT_INTERP evidence"
Assert-True (-not $finalInspection.Contains(".dynamic")) "final ELF contains a dynamic section"

$sourceText = ($modules | ForEach-Object { Get-Content -LiteralPath (Join-Path $sourceRoot "$_.dby") -Raw }) -join "`n"
foreach ($forbidden in @("core::arch", "global_asm", "nasm", "gcc", "clang", "rustc", "llvm", "std::process", "int 14", "malloc")) {
    Assert-True (-not $sourceText.Contains($forbidden)) "native runtime source contains forbidden external runtime/generator token '$forbidden'"
}

$proof32 = Invoke-QemuProof 32 $elf $qemu
$proof64 = Invoke-QemuProof 64 $elf $qemu
$hashLines = @()
foreach ($module in $modules) {
    $hash = (Get-FileHash -LiteralPath (Join-Path $proofRoot "$module.o") -Algorithm SHA256).Hash
    $hashLines += "$module.o $hash"
}
$elfHash = (Get-FileHash -LiteralPath $elf -Algorithm SHA256).Hash
$binHash = (Get-FileHash -LiteralPath $bin -Algorithm SHA256).Hash
$hashLines += "ELF $elfHash"
$hashLines += "BIN $binHash"
[System.IO.File]::WriteAllLines((Join-Path $proofRoot "sha256.txt"), $hashLines)

Write-Host "DBYTE NATIVE KERNEL RUNTIME: PASS"
Write-Host "objects=$($modules.Count) R_386_PC32=$pc32Count R_386_32=$abs32Count"
Write-Host "arena=$('{0:X8}' -f $proof32.Telemetry.ARENA_START)-$('{0:X8}' -f $proof32.Telemetry.ARENA_END) pages=$($proof32.Telemetry.PAGES_BEFORE)/$($proof32.Telemetry.PAGES_LARGE)/$($proof32.Telemetry.PAGES_FINAL)"
Write-Host "ELF_SHA256=$elfHash"
Write-Host "BIN_SHA256=$binHash"
Write-Host "QEMU32=$($proof32.Command)"
Write-Host "QEMU64=$($proof64.Command)"
Write-Host $proof32.Serial
