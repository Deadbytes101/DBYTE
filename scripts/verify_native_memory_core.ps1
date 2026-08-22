param(
    [string]$MilestoneVersion = "11.3.0"
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$tmpRoot = (Resolve-Path (Join-Path $repoRoot "tmp")).Path
$proofRoot = Join-Path $tmpRoot "native_v$MilestoneVersion"
$repeatRoot = Join-Path $tmpRoot "native_v$MilestoneVersion.repeat"
$sourceRoot = Join-Path $repoRoot "examples\native_memory_core"
$modules = @("arch_x86", "serial", "gdt", "exceptions", "boot", "frames", "paging", "page_fault", "idt", "proof", "kernel")
$expectedSerial = "DBYTEOS NATIVE MEMORY`r`nMEMORY MAP ONLINE`r`nFRAME ALLOCATOR ONLINE`r`nPAGING ONLINE`r`nPAGE FAULT ABI ONLINE`r`nPAGE ALLOC 001`r`nPAGE ALLOC 002`r`nMEMORY CHECK OK`r`n"

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

function Assert-NoOverlap([uint64]$Start, [uint64]$End, [uint64]$OtherStart, [uint64]$OtherEnd, [string]$Description) {
    Assert-True (-not (($Start -lt $OtherEnd) -and ($OtherStart -lt $End))) "$Description overlaps a reserved range"
}

function Invoke-QemuProof([int]$RamMiB, [string]$Elf, [string]$Qemu) {
    $serialPath = Join-Path $proofRoot "qemu.${RamMiB}m.serial.log"
    $tracePath = Join-Path $proofRoot "qemu.${RamMiB}m.trace.log"
    $debugconPath = Join-Path $proofRoot "qemu.${RamMiB}m.debugcon.log"
    $arguments = @(
        "-m", "${RamMiB}M",
        "-kernel", $Elf,
        "-display", "none",
        "-serial", "file:$serialPath",
        "-monitor", "none",
        "-no-reboot",
        "-no-shutdown",
        "-d", "int,cpu_reset,guest_errors",
        "-D", $tracePath,
        "-chardev", "file,id=dbyte_debugcon,path=$debugconPath",
        "-device", "isa-debugcon,iobase=0xe9,chardev=dbyte_debugcon"
    )
    [System.IO.File]::WriteAllText((Join-Path $proofRoot "qemu.${RamMiB}m.command.txt"), "$Qemu $($arguments -join ' ')")
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
    Assert-True ([regex]::IsMatch($trace, 'v=0e e=0002.*CR2=00400000', [System.Text.RegularExpressions.RegexOptions]::Singleline)) "QEMU ${RamMiB} MiB lacks real page fault v=0e e=0002 with CR2=00400000"
    Assert-True (-not $trace.Contains("Triple fault")) "QEMU ${RamMiB} MiB trace contains a triple fault"
    $telemetry = Read-Telemetry $debugconPath
    foreach ($key in @("MAP_COUNT", "IMAGE_START", "IMAGE_END", "STACK_START", "STACK_END", "MB_INFO", "MB_MAP", "MB_MAP_LENGTH", "FRAME_ONE", "FRAME_TWO", "PAGE_DIRECTORY", "IDENTITY_TABLE", "FAULT_TABLE", "CR3", "CR0", "CR2", "PF_ERROR")) {
        Assert-True ($telemetry.ContainsKey($key)) "QEMU ${RamMiB} MiB is missing telemetry '$key'"
    }
    Assert-True ($telemetry.MAP_COUNT -gt 0) "QEMU ${RamMiB} MiB memory map is empty"
    Assert-True (($telemetry.FRAME_ONE -band 0xfff) -eq 0) "first allocated frame is not 4 KiB aligned"
    Assert-True (($telemetry.FRAME_TWO -band 0xfff) -eq 0) "second allocated frame is not 4 KiB aligned"
    Assert-True ($telemetry.FRAME_ONE -ne $telemetry.FRAME_TWO) "allocated frames are duplicated"
    foreach ($table in @("PAGE_DIRECTORY", "IDENTITY_TABLE", "FAULT_TABLE", "CR3")) {
        Assert-True (($telemetry[$table] -band 0xfff) -eq 0) "$table is not 4 KiB aligned"
    }
    Assert-True ($telemetry.CR3 -eq $telemetry.PAGE_DIRECTORY) "CR3 does not point at the page directory"
    Assert-True (($telemetry.CR0 -band 0x80000000) -ne 0) "CR0.PG was not enabled"
    Assert-True ($telemetry.CR2 -eq 0x00400000) "page-fault CR2 telemetry is wrong"
    Assert-True ($telemetry.PF_ERROR -eq 2) "page-fault error code telemetry is wrong"
    foreach ($frame in @([uint64]$telemetry.FRAME_ONE, [uint64]$telemetry.FRAME_TWO)) {
        Assert-True ($frame -ge 0x00100000) "allocator returned a frame below 1 MiB"
        Assert-NoOverlap $frame ($frame + 4096) $telemetry.IMAGE_START $telemetry.IMAGE_END "allocated frame"
        Assert-NoOverlap $frame ($frame + 4096) $telemetry.STACK_START $telemetry.STACK_END "allocated frame"
        Assert-NoOverlap $frame ($frame + 4096) $telemetry.MB_INFO ($telemetry.MB_INFO + 52) "allocated frame"
        Assert-NoOverlap $frame ($frame + 4096) $telemetry.MB_MAP ($telemetry.MB_MAP + $telemetry.MB_MAP_LENGTH) "allocated frame"
    }
    Start-Sleep -Milliseconds 100
    $orphan = Get-Process -Name "qemu-system-i386" -ErrorAction SilentlyContinue | Where-Object { $_.Id -eq $process.Id }
    Assert-True ($null -eq $orphan) "QEMU process $($process.Id) remains alive"
    return @{ Command = "$Qemu $($arguments -join ' ')"; Serial = $serial; Telemetry = $telemetry }
}

Reset-ProofDirectory $proofRoot
Reset-ProofDirectory $repeatRoot

$cargo = (Get-Command cargo -ErrorAction Stop).Source
$objdump = (Get-Command objdump -ErrorAction Stop).Source
$qemu = (Get-Command qemu-system-i386 -ErrorAction Stop).Source
Invoke-Checked $cargo @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build dbytec" | Out-Null
$compiler = Join-Path $repoRoot "target\debug\dbytec.exe"
Assert-True (Test-Path -LiteralPath $compiler) "dbytec executable is missing"

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
}

$rootSource = Join-Path $sourceRoot "kernel.dby"
$elf = Join-Path $proofRoot "dbyte_native_memory_v$MilestoneVersion.elf"
$elfRepeat = Join-Path $repeatRoot "dbyte_native_memory_v$MilestoneVersion.elf"
$bin = Join-Path $proofRoot "dbyte_native_memory_v$MilestoneVersion.bin"
$binRepeat = Join-Path $repeatRoot "dbyte_native_memory_v$MilestoneVersion.bin"
Invoke-Checked $compiler @("build", $rootSource, "--target", "i686-dbyte-none", "--emit", "elf", "-o", $elf) "build native memory ELF" | Out-Null
Invoke-Checked $compiler @("build", $rootSource, "--target", "i686-dbyte-none", "--emit", "elf", "-o", $elfRepeat) "repeat native memory ELF" | Out-Null
Invoke-Checked $compiler @("build", $rootSource, "--target", "i686-dbyte-none", "--emit", "bin", "-o", $bin) "build native memory BIN" | Out-Null
Invoke-Checked $compiler @("build", $rootSource, "--target", "i686-dbyte-none", "--emit", "bin", "-o", $binRepeat) "repeat native memory BIN" | Out-Null
Assert-SameFile $elf $elfRepeat "linked native memory ELF"
Assert-SameFile $bin $binRepeat "linked native memory BIN"

$finalInspection = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", $elf) "inspect final native memory ELF"
[System.IO.File]::WriteAllText((Join-Path $proofRoot "final.objdump.txt"), $finalInspection)
foreach ($needle in @("<_start>", "push   eax", "push   ebx", "mov    eax,cr0", "mov    cr0,eax", "mov    eax,cr2", "mov    eax,cr3", "mov    cr3,eax", "invlpg", "pusha", "popa", "iret")) {
    Assert-Contains $finalInspection $needle "final paging/entry machine instruction"
}
Assert-Contains $finalInspection "<page_fault>" "page fault entry stub symbol"
Assert-Contains $finalInspection "__dbyte_interrupt_page_fault_body" "local page fault body symbol"
Assert-True (-not $finalInspection.Contains("int    0xe")) "final image contains a fake software INT 14"
Assert-True (-not $finalInspection.Contains(".dbyte.entry")) "entry metadata leaked into final executable"
Assert-True (-not $finalInspection.Contains(".dbyte.interrupts")) "interrupt metadata leaked into final executable"
Assert-True (-not $finalInspection.Contains(".interp")) "final ELF contains PT_INTERP evidence"
Assert-True (-not $finalInspection.Contains(".dynamic")) "final ELF contains a dynamic section"

$kernelInspection = Get-Content -LiteralPath (Join-Path $proofRoot "kernel.objdump.txt") -Raw
$framesInspection = Get-Content -LiteralPath (Join-Path $proofRoot "frames.objdump.txt") -Raw
$idtInspection = Get-Content -LiteralPath (Join-Path $proofRoot "idt.objdump.txt") -Raw
$pageFaultInspection = Get-Content -LiteralPath (Join-Path $proofRoot "page_fault.objdump.txt") -Raw
Assert-Contains $kernelInspection "__dbyte_entry:multiboot_v1" "dual entry ABI marker"
Assert-Contains $framesInspection "R_386_32" "linker-owned image/stack boundary relocation"
Assert-Contains $idtInspection "R_386_32" "vector 14 handler address relocation"
Assert-Contains $pageFaultInspection "R_386_PC32" "page fault stub/body and cross-object relocations"
$entryDump = Invoke-Checked $objdump @("-s", "-j", ".dbyte.entry", (Join-Path $proofRoot "kernel.o")) "dump entry metadata"
[System.IO.File]::WriteAllText((Join-Path $proofRoot "kernel.entry.txt"), $entryDump)
Assert-Contains $entryDump "DBYTEENT" "entry metadata magic"

$sourceText = ($modules | ForEach-Object { Get-Content -LiteralPath (Join-Path $sourceRoot "$_.dby") -Raw }) -join "`n"
foreach ($forbidden in @("core::arch", "global_asm", "nasm", "gcc", "clang", "rustc", "llvm", "std::process", "int 14")) {
    Assert-True (-not $sourceText.Contains($forbidden)) "native memory source contains forbidden external runtime/generator token '$forbidden'"
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

Write-Host "DBYTE NATIVE MEMORY CORE: PASS"
Write-Host "objects=$($modules.Count) map32=$($proof32.Telemetry.MAP_COUNT) map64=$($proof64.Telemetry.MAP_COUNT)"
Write-Host "frames32=$('{0:X8}' -f $proof32.Telemetry.FRAME_ONE),$('{0:X8}' -f $proof32.Telemetry.FRAME_TWO)"
Write-Host "ELF_SHA256=$elfHash"
Write-Host "BIN_SHA256=$binHash"
Write-Host "QEMU32=$($proof32.Command)"
Write-Host "QEMU64=$($proof64.Command)"
Write-Host $proof32.Serial
