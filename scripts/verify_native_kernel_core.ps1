param(
    [string]$MilestoneVersion = "11.2.0"
)

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$tmpRoot = (Resolve-Path (Join-Path $repoRoot "tmp")).Path
$proofRoot = Join-Path $tmpRoot "native_v$MilestoneVersion"
$repeatRoot = Join-Path $tmpRoot "native_v$MilestoneVersion.repeat"
$sourceRoot = Join-Path $repoRoot "examples\native_kernel_core"
$modules = @("arch_x86", "serial", "gdt", "exceptions", "pic", "pit", "idt", "kernel")
$expectedSerial = "DBYTEOS NATIVE CORE`r`nGDT ONLINE`r`nIDT ONLINE`r`nEXCEPTIONS ONLINE`r`nPIC ONLINE`r`nIRQ0 TIMER ONLINE`r`nTICKS 1`r`nTICKS 8`r`n"

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
}

$rootSource = Join-Path $sourceRoot "kernel.dby"
$elf = Join-Path $proofRoot "dbyte_native_core_v$MilestoneVersion.elf"
$elfRepeat = Join-Path $repeatRoot "dbyte_native_core_v$MilestoneVersion.elf"
$bin = Join-Path $proofRoot "dbyte_native_core_v$MilestoneVersion.bin"
$binRepeat = Join-Path $repeatRoot "dbyte_native_core_v$MilestoneVersion.bin"
Invoke-Checked $compiler @("build", $rootSource, "--target", "i686-dbyte-none", "--emit", "elf", "-o", $elf) "build native core ELF" | Out-Null
Invoke-Checked $compiler @("build", $rootSource, "--target", "i686-dbyte-none", "--emit", "elf", "-o", $elfRepeat) "repeat native core ELF" | Out-Null
Invoke-Checked $compiler @("build", $rootSource, "--target", "i686-dbyte-none", "--emit", "bin", "-o", $bin) "build native core BIN" | Out-Null
Invoke-Checked $compiler @("build", $rootSource, "--target", "i686-dbyte-none", "--emit", "bin", "-o", $binRepeat) "repeat native core BIN" | Out-Null
Assert-SameFile $elf $elfRepeat "linked native core ELF"
Assert-SameFile $bin $binRepeat "linked native core BIN"

$finalInspection = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", $elf) "inspect final native core ELF"
[System.IO.File]::WriteAllText((Join-Path $proofRoot "final.objdump.txt"), $finalInspection)
foreach ($needle in @("lgdt", "lidt", "int3", "pusha", "popa", "iret", "sti", "hlt", "out", "in")) {
    Assert-Contains $finalInspection $needle "final $needle machine instruction"
}
Assert-Contains $finalInspection "<breakpoint>" "breakpoint entry symbol"
Assert-Contains $finalInspection "<irq0>" "IRQ0 entry symbol"
Assert-Contains $finalInspection "__dbyte_interrupt_breakpoint_body" "breakpoint DBYTE32 body"
Assert-Contains $finalInspection "__dbyte_interrupt_irq0_body" "IRQ0 DBYTE32 body"
Assert-True (-not $finalInspection.Contains("__dbyte_interrupt:")) "non-loadable interrupt metadata leaked into final symbols"
Assert-True (-not $finalInspection.Contains(".interp")) "final ELF contains PT_INTERP evidence"
Assert-True (-not $finalInspection.Contains(".dynamic")) "final ELF contains a dynamic section"

$exceptionsObject = Join-Path $proofRoot "exceptions.o"
$idtObject = Join-Path $proofRoot "idt.o"
$pitObject = Join-Path $proofRoot "pit.o"
$exceptionsInspection = Get-Content -LiteralPath (Join-Path $proofRoot "exceptions.objdump.txt") -Raw
$idtInspection = Get-Content -LiteralPath (Join-Path $proofRoot "idt.objdump.txt") -Raw
$pitInspection = Get-Content -LiteralPath (Join-Path $proofRoot "pit.objdump.txt") -Raw
Assert-Contains $exceptionsInspection "R_386_PC32" "interrupt stub-to-body relocation"
Assert-Contains $idtInspection "R_386_32" "IDT handler address relocation"
Assert-Contains $pitInspection "R_386_PC32" "IRQ0 cross-object EOI relocation"
$metadataDump = Invoke-Checked $objdump @("-s", "-j", ".dbyte.interrupts", $idtObject) "dump interrupt metadata"
[System.IO.File]::WriteAllText((Join-Path $proofRoot "idt.interrupts.txt"), $metadataDump)
Assert-Contains $metadataDump "DBYTEINT" "interrupt metadata magic"

$sourceText = ($modules | ForEach-Object { Get-Content -LiteralPath (Join-Path $sourceRoot "$_.dby") -Raw }) -join "`n"
foreach ($forbidden in @("core::arch", "global_asm", "nasm", "gcc", "clang", "rustc", "llvm", "std::process")) {
    Assert-True (-not $sourceText.Contains($forbidden)) "native core source contains forbidden external runtime/generator token '$forbidden'"
}

$serialPath = Join-Path $proofRoot "qemu.serial.log"
$debugPath = Join-Path $proofRoot "qemu.debug.log"
$qemuArguments = @(
    "-kernel", $elf,
    "-display", "none",
    "-serial", "file:$serialPath",
    "-monitor", "none",
    "-no-reboot",
    "-no-shutdown",
    "-d", "int,cpu_reset,guest_errors",
    "-D", $debugPath
)
$qemuProcess = Start-Process -FilePath $qemu -ArgumentList $qemuArguments -PassThru -WindowStyle Hidden
try {
    if (-not $qemuProcess.WaitForExit(12000)) {
        Stop-Process -Id $qemuProcess.Id -Force
        $qemuProcess.WaitForExit()
    }
}
finally {
    if (-not $qemuProcess.HasExited) { Stop-Process -Id $qemuProcess.Id -Force }
}
Assert-True (Test-Path -LiteralPath $serialPath) "QEMU serial log was not created"
$serial = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($serialPath))
Assert-True ($serial -ceq $expectedSerial) "QEMU serial output mismatch:`n$serial"
$debug = Get-Content -LiteralPath $debugPath -Raw
Assert-Contains $debug "v=03" "controlled INT3 delivery"
Assert-Contains $debug "Servicing hardware INT=0x20" "hardware IRQ0 delivery"
Assert-Contains $debug "GDT=" "loaded GDT state"
Assert-Contains $debug "IDT=" "loaded IDT state"
Assert-True (-not $debug.Contains("Triple fault")) "QEMU trace contains a triple fault"
$irq0Deliveries = ([regex]::Matches($debug, "Servicing hardware INT=0x20")).Count
Assert-True ($irq0Deliveries -ge 8) "expected at least 8 IRQ0 deliveries, found $irq0Deliveries"
Start-Sleep -Milliseconds 100
$orphan = Get-Process -Name "qemu-system-i386" -ErrorAction SilentlyContinue | Where-Object { $_.Id -eq $qemuProcess.Id }
Assert-True ($null -eq $orphan) "QEMU process $($qemuProcess.Id) remains alive"

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

Write-Host "DBYTE NATIVE KERNEL CORE: PASS"
Write-Host "objects=$($modules.Count) irq0_deliveries=$irq0Deliveries"
Write-Host "ELF_SHA256=$elfHash"
Write-Host "BIN_SHA256=$binHash"
Write-Host "QEMU=$qemu $($qemuArguments -join ' ')"
Write-Host $serial
