param([string]$MilestoneVersion = "11.5.0")

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$tmpRoot = (Resolve-Path (Join-Path $repoRoot "tmp")).Path
$evidenceRoot = Join-Path $tmpRoot "native_v$MilestoneVersion"
$repeatRoot = Join-Path $tmpRoot "native_v$MilestoneVersion.repeat"
$sourceRoot = Join-Path $repoRoot "examples\native_scheduler_core"
$normalModules = @("arch_x86", "serial", "gdt", "boot", "frames", "paging", "memory_ops", "heap", "pic", "proof", "page_fault", "scheduler", "tasks", "pit", "idt_normal", "kernel")
$guardModules = @("arch_x86", "serial", "gdt", "boot", "frames", "paging", "memory_ops", "heap", "pic", "proof", "scheduler", "guard_fault", "idt_guard", "guard_task", "guard_kernel")
$normalSerial = "DBYTEOS NATIVE SCHEDULER`r`nTASK CORE ONLINE`r`nTASK STACKS ONLINE`r`nPREEMPTION ONLINE`r`nTASK A RAN`r`nTASK B RAN`r`nTASK EXIT OK`r`nSTACK RECLAIM OK`r`nSCHEDULER CHECK OK`r`n"
$guardSerial = "DBYTEOS TASK GUARD TEST`r`nGUARD ARMED`r`nPAGE FAULT CAUGHT`r`nCR2 C1200FFC`r`nERROR 00000002`r`nGUARD PAGE VERIFIED`r`nHALT`r`n"

function Assert-True($Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Invoke-Checked([string]$Executable, [string[]]$Arguments, [string]$Description) {
    $saved = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try { $output = & $Executable @Arguments 2>&1; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    if ($code -ne 0) { throw "$Description failed with exit code ${code}:`n$($output -join "`n")" }
    return ($output -join "`n")
}

function Reset-Evidence([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    $prefix = $tmpRoot.TrimEnd('\') + '\'
    Assert-True ($full.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) "evidence path escapes tmp: $full"
    if (Test-Path -LiteralPath $full) { Remove-Item -LiteralPath $full -Recurse -Force }
    New-Item -ItemType Directory -Path $full | Out-Null
}

function Assert-SameFile([string]$Left, [string]$Right, [string]$Role) {
    [byte[]]$a = [IO.File]::ReadAllBytes($Left)
    [byte[]]$b = [IO.File]::ReadAllBytes($Right)
    Assert-True ([Linq.Enumerable]::SequenceEqual($a, $b)) "$Role is not deterministic"
}

function Read-Telemetry([string]$Path) {
    $result = @{}
    $text = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($Path))
    foreach ($line in ($text -split "`r`n")) {
        if ($line -match '^([A-Z0-9_]+)=([0-9A-F]{8})$') {
            Assert-True (-not $result.ContainsKey($Matches[1])) "duplicate telemetry $($Matches[1])"
            $result[$Matches[1]] = [Convert]::ToUInt32($Matches[2], 16)
        }
    }
    return $result
}

function Invoke-Qemu([string]$Kind, [int]$Ram, [string]$Elf, [string]$Qemu, [string]$Expected) {
    $dir = Join-Path $evidenceRoot $Kind
    $serial = Join-Path $dir "qemu.${Ram}m.serial.log"
    $trace = Join-Path $dir "qemu.${Ram}m.trace.log"
    $debug = Join-Path $dir "qemu.${Ram}m.debugcon.log"
    $arguments = @(
        "-m", "${Ram}M", "-kernel", $Elf, "-display", "none",
        "-serial", "file:$serial", "-monitor", "none", "-no-reboot", "-no-shutdown",
        "-d", "int,cpu_reset,guest_errors", "-D", $trace,
        "-chardev", "file,id=dbyte_debugcon,path=$debug",
        "-device", "isa-debugcon,iobase=0xe9,chardev=dbyte_debugcon"
    )
    $command = "$Qemu $($arguments -join ' ')"
    [IO.File]::WriteAllText((Join-Path $dir "qemu.${Ram}m.command.txt"), $command)
    $process = Start-Process -FilePath $Qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        if (-not $process.WaitForExit(12000)) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
    }
    finally { if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force } }
    Assert-True (Test-Path -LiteralPath $serial) "$Kind QEMU ${Ram}M serial is missing"
    $actual = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($serial))
    Assert-True ($actual -ceq $Expected) "$Kind QEMU ${Ram}M serial mismatch:`n$actual"
    $traceText = Get-Content -LiteralPath $trace -Raw
    Assert-True (-not $traceText.Contains("Triple fault")) "$Kind QEMU ${Ram}M triple faulted"
    $faults = ([regex]::Matches($traceText, 'v=0e', [Text.RegularExpressions.RegexOptions]::IgnoreCase)).Count
    $telemetry = Read-Telemetry $debug
    if ($Kind -eq "normal") {
        Assert-True ($faults -eq 0) "normal scheduler raised a page fault"
        foreach ($key in @("COUNTER_A", "COUNTER_B", "B_AFTER_A", "REUSE_SLOT", "TICKS", "SWITCHES", "EXITS", "RECLAIMS", "EOI", "LIVE", "FRAME", "REUSED_FRAME")) {
            Assert-True ($telemetry.ContainsKey($key)) "normal telemetry lacks $key"
        }
        Assert-True ($telemetry.COUNTER_A -gt 0 -and $telemetry.COUNTER_B -gt 0) "busy-loop tasks did not both execute"
        Assert-True ($telemetry.B_AFTER_A -gt 0) "task B did not advance after task A exited"
        Assert-True ($telemetry.TICKS -ge 16 -and $telemetry.SWITCHES -gt 2) "real timer preemption was not observed"
        Assert-True ($telemetry.EOI -eq $telemetry.TICKS) "IRQ0 did not have exactly one EOI per tick"
        Assert-True ($telemetry.EXITS -eq 3 -and $telemetry.RECLAIMS -eq 3 -and $telemetry.LIVE -eq 1) "task lifecycle did not converge to idle only"
        Assert-True ($telemetry.REUSE_SLOT -eq 1) "lowest freed scheduler slot was not reused"
        Assert-True ($telemetry.FRAME -eq $telemetry.REUSED_FRAME) "released physical frame was not reused"
    }
    else {
        Assert-True ($faults -eq 1) "guard proof did not raise exactly one real page fault"
        foreach ($key in @("CR2", "ERROR", "GUARD_PTE")) { Assert-True ($telemetry.ContainsKey($key)) "guard telemetry lacks $key" }
        Assert-True ([uint64]$telemetry.CR2 -eq 3240103932) "guard CR2 is wrong"
        Assert-True (($telemetry.ERROR -band 7) -eq 2) "guard page-fault P/W/U bits are wrong"
        Assert-True ($telemetry.GUARD_PTE -eq 0) "guard PTE became present"
    }
    Start-Sleep -Milliseconds 100
    Assert-True ($null -eq (Get-Process -Id $process.Id -ErrorAction SilentlyContinue)) "QEMU PID $($process.Id) remains alive"
    return @{ Command = $command; Serial = $actual; Telemetry = $telemetry; Faults = $faults }
}

Reset-Evidence $evidenceRoot
Reset-Evidence $repeatRoot
foreach ($root in @($evidenceRoot, $repeatRoot)) {
    New-Item -ItemType Directory -Path (Join-Path $root "normal") | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $root "guard") | Out-Null
}
$cargo = (Get-Command cargo -ErrorAction Stop).Source
$objdump = (Get-Command objdump -ErrorAction Stop).Source
$qemu = (Get-Command qemu-system-i386 -ErrorAction Stop).Source
Invoke-Checked $cargo @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build dbytec" | Out-Null
$compiler = Join-Path $repoRoot "target\debug\dbytec.exe"

$allModules = @($normalModules + $guardModules | Select-Object -Unique)
$pc32 = 0
$abs32 = 0
foreach ($buildSet in @(@{ Kind = "normal"; Modules = $normalModules }, @{ Kind = "guard"; Modules = $guardModules })) {
    $kind = $buildSet.Kind
    foreach ($module in $buildSet.Modules) {
        $first = Join-Path (Join-Path $evidenceRoot $kind) "$module.o"
        $second = Join-Path (Join-Path $repeatRoot $kind) "$module.o"
        $source = Join-Path $sourceRoot "$module.dby"
        Invoke-Checked $compiler @("build", $source, "--target", "i686-dbyte-none", "--emit", "obj", "-o", $first) "build $kind/$module object" | Out-Null
        Invoke-Checked $compiler @("build", $source, "--target", "i686-dbyte-none", "--emit", "obj", "-o", $second) "repeat $kind/$module object" | Out-Null
        Assert-SameFile $first $second "$kind/$module object"
        $inspection = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", $first) "inspect $kind/$module object"
        [IO.File]::WriteAllText("$first.objdump.txt", $inspection)
        Assert-True ($inspection.Contains("file format elf32-i386")) "$kind/$module is not ELF32/i386"
        if ($module -in @("scheduler", "tasks", "kernel", "guard_kernel")) {
            Assert-True ($inspection.Contains(".dbyte.tasks")) "$kind/$module lacks checked task metadata"
        }
        $pc32 += ([regex]::Matches($inspection, 'R_386_PC32')).Count
        $abs32 += ([regex]::Matches($inspection, 'R_386_32')).Count
    }
}

function Build-Image([string]$Kind, [string[]]$Modules, [string]$RootName) {
    $firstDir = Join-Path $evidenceRoot $Kind
    $secondDir = Join-Path $repeatRoot $Kind
    $objectsA = @($Modules | ForEach-Object { Join-Path $firstDir "$_.o" })
    $objectsB = @($Modules | ForEach-Object { Join-Path $secondDir "$_.o" })
    $elf = Join-Path $firstDir "$Kind.elf"
    $elf2 = Join-Path $secondDir "$Kind.elf"
    Invoke-Checked $compiler (@("link") + $objectsA + @("-o", $elf)) "link $Kind ELF" | Out-Null
    Invoke-Checked $compiler (@("link") + $objectsB + @("-o", $elf2)) "repeat-link $Kind ELF" | Out-Null
    Assert-SameFile $elf $elf2 "$Kind linked ELF"
    $integrated = Join-Path $firstDir "$Kind.integrated.elf"
    Invoke-Checked $compiler @("build", (Join-Path $sourceRoot $RootName), "--target", "i686-dbyte-none", "--emit", "elf", "-o", $integrated) "integrated $Kind ELF" | Out-Null
    Assert-SameFile $elf $integrated "$Kind object-link and integrated ELF"
    $bin = Join-Path $firstDir "$Kind.bin"
    $bin2 = Join-Path $secondDir "$Kind.bin"
    Invoke-Checked $compiler @("build", (Join-Path $sourceRoot $RootName), "--target", "i686-dbyte-none", "--emit", "bin", "-o", $bin) "build $Kind BIN" | Out-Null
    Invoke-Checked $compiler @("build", (Join-Path $sourceRoot $RootName), "--target", "i686-dbyte-none", "--emit", "bin", "-o", $bin2) "repeat $Kind BIN" | Out-Null
    Assert-SameFile $bin $bin2 "$Kind BIN"
    $inspection = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", $elf) "inspect $Kind ELF"
    [IO.File]::WriteAllText((Join-Path $firstDir "final.objdump.txt"), $inspection)
    Assert-True (-not $inspection.Contains(".dbyte.tasks")) "$Kind final ELF retained task metadata"
    Assert-True (-not $inspection.Contains(".dbyte.interrupts") -and -not $inspection.Contains(".dbyte.entry")) "$Kind final ELF retained compile-time metadata"
    Assert-True (-not $inspection.Contains(".interp") -and -not $inspection.Contains(".dynamic")) "$Kind final ELF has a runtime dependency"
    return @{ Elf = $elf; Bin = $bin; Inspection = $inspection }
}

$normal = Build-Image "normal" $normalModules "kernel.dby"
$guard = Build-Image "guard" $guardModules "guard_kernel.dby"
foreach ($needle in @("<__dbyte_task_trampoline>", "call   eax", "<irq0>", "pusha", "mov    esp,eax", "popa", "iret", "<task_exit>", "<reap_exited>", "invlpg", "sti", "hlt", "cli")) {
    Assert-True ($normal.Inspection.Contains($needle)) "normal disassembly lacks $needle"
}
Assert-True ($normal.Inspection.Contains("ff d0") -and $normal.Inspection.Contains("89 c4")) "trampoline/resume exact opcode bytes are absent"
$irqBody = [regex]::Match($normal.Inspection, '(?s)<scheduler\.o::__dbyte_interrupt_irq0_body>:(.*?)(?=\n[0-9a-f]+ <)').Groups[1].Value
Assert-True ($irqBody.Length -gt 0) "IRQ0 body disassembly was not isolated"
foreach ($forbiddenCall in @("<alloc>", "<allocate>", "<map_kernel_page>", "<unmap_kernel_page>", "<free>")) {
    Assert-True (-not $irqBody.Contains($forbiddenCall)) "IRQ fast path calls $forbiddenCall"
}
Assert-True (([regex]::Matches($irqBody, '<eoi_master>')).Count -eq 1) "IRQ fast path does not contain exactly one EOI call path"
Assert-True ($guard.Inspection.Contains("mov    eax,cr2") -and $guard.Inspection.Contains("<guard_fault.o::__dbyte_interrupt_page_fault_body>")) "guard #PF body does not read CR2"
$guardBody = [regex]::Match($guard.Inspection, '(?s)<guard_fault\.o::__dbyte_interrupt_page_fault_body>:(.*?)(?=\n[0-9a-f]+ <)').Groups[1].Value
Assert-True ($guardBody.Length -gt 0) "guard #PF body disassembly was not isolated"
Assert-True (([regex]::Matches($guardBody, 'mov\s+eax,cr2')).Count -eq 1) "guard #PF body does not read CR2 exactly once"
foreach ($forbiddenGuardPath in @("<eoi_master>", "<task_exit>", "<reap_exited>", "<map_kernel_page>", "<unmap_kernel_page>")) {
    Assert-True (-not $guardBody.Contains($forbiddenGuardPath)) "guard #PF body enters $forbiddenGuardPath"
}
Assert-True (-not $guardBody.Contains("89 c4")) "guard #PF body contains a context-resume sequence"

$taskSource = Get-Content -LiteralPath (Join-Path $sourceRoot "tasks.dby") -Raw
Assert-True (-not [regex]::IsMatch($taskSource, '\byield\s*\(')) "Task A/B source contains yield"
$irDump = Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "kernel.dby"), "--target", "i686-dbyte-none", "--emit", "obj", "-o", (Join-Path $evidenceRoot "normal\kernel.ir.o"), "--dump-ir") "dump scheduler IR"
[IO.File]::WriteAllText((Join-Path $evidenceRoot "normal\scheduler.ir.txt"), $irDump)
Assert-True (-not [regex]::IsMatch($irDump, '\byield\b')) "scheduler proof IR contains yield"
$sourceText = ($allModules | ForEach-Object { Get-Content -LiteralPath (Join-Path $sourceRoot "$_.dby") -Raw }) -join "`n"
foreach ($token in @("global_asm", "nasm", "gcc", "clang", "rustc", "llvm", "std::process", "int 14")) {
    Assert-True (-not $sourceText.Contains($token)) "scheduler source contains forbidden token '$token'"
}

$normal32 = Invoke-Qemu "normal" 32 $normal.Elf $qemu $normalSerial
$normal64 = Invoke-Qemu "normal" 64 $normal.Elf $qemu $normalSerial
$guard32 = Invoke-Qemu "guard" 32 $guard.Elf $qemu $guardSerial
$guard64 = Invoke-Qemu "guard" 64 $guard.Elf $qemu $guardSerial

$hashes = @()
foreach ($kind in @("normal", "guard")) {
    foreach ($file in Get-ChildItem -LiteralPath (Join-Path $evidenceRoot $kind) -Filter "*.o") {
        $hashes += "$kind/$($file.Name) $((Get-FileHash $file.FullName -Algorithm SHA256).Hash)"
    }
}
$normalElfHash = (Get-FileHash $normal.Elf -Algorithm SHA256).Hash
$normalBinHash = (Get-FileHash $normal.Bin -Algorithm SHA256).Hash
$guardElfHash = (Get-FileHash $guard.Elf -Algorithm SHA256).Hash
$guardBinHash = (Get-FileHash $guard.Bin -Algorithm SHA256).Hash
$hashes += "normal/ELF $normalElfHash", "normal/BIN $normalBinHash", "guard/ELF $guardElfHash", "guard/BIN $guardBinHash"
[IO.File]::WriteAllLines((Join-Path $evidenceRoot "sha256.txt"), $hashes)

Write-Host "DBYTE NATIVE KERNEL SCHEDULER: PASS"
Write-Host "objects=$($normalModules.Count + $guardModules.Count) R_386_PC32=$pc32 R_386_32=$abs32"
Write-Host "ticks=$($normal32.Telemetry.TICKS) switches=$($normal32.Telemetry.SWITCHES) exits=$($normal32.Telemetry.EXITS) reclaims=$($normal32.Telemetry.RECLAIMS)"
Write-Host "NORMAL_ELF_SHA256=$normalElfHash"
Write-Host "NORMAL_BIN_SHA256=$normalBinHash"
Write-Host "GUARD_ELF_SHA256=$guardElfHash"
Write-Host "GUARD_BIN_SHA256=$guardBinHash"
Write-Host "QEMU_NORMAL_32=$($normal32.Command)"
Write-Host "QEMU_NORMAL_64=$($normal64.Command)"
Write-Host "QEMU_GUARD_32=$($guard32.Command)"
Write-Host "QEMU_GUARD_64=$($guard64.Command)"
