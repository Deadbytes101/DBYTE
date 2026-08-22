param([string]$MilestoneVersion = "11.6.0")

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$tmpRoot = (Resolve-Path (Join-Path $repoRoot "tmp")).Path
$evidenceRoot = Join-Path $tmpRoot "native_v$MilestoneVersion"
$repeatRoot = Join-Path $tmpRoot "native_v$MilestoneVersion.repeat"
$sourceRoot = Join-Path $repoRoot "examples\native_blocking_core"
$modules = @("arch_x86", "serial", "gdt", "boot", "frames", "paging", "memory_ops", "heap", "pic", "proof", "page_fault", "scheduler", "tasks", "pit", "idt_normal", "kernel")
$expectedSerial = "DBYTEOS NATIVE BLOCKING`r`nSLEEP QUEUE ONLINE`r`nBLOCKING ONLINE`r`nSLEEP WAKE OK`r`nEVENT WAIT OK`r`nEVENT WAKE OK`r`nNO BUSY WAIT OK`r`nBLOCK RECLAIM OK`r`nBLOCKING CHECK OK`r`n"

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

function Invoke-Qemu([int]$Ram, [string]$Elf, [string]$Qemu) {
    $serial = Join-Path $evidenceRoot "qemu.${Ram}m.serial.log"
    $trace = Join-Path $evidenceRoot "qemu.${Ram}m.trace.log"
    $debug = Join-Path $evidenceRoot "qemu.${Ram}m.debugcon.log"
    $arguments = @(
        "-m", "${Ram}M", "-kernel", $Elf, "-display", "none",
        "-serial", "file:$serial", "-monitor", "none", "-no-reboot", "-no-shutdown",
        "-d", "int,cpu_reset,guest_errors", "-D", $trace,
        "-chardev", "file,id=dbyte_debugcon,path=$debug",
        "-device", "isa-debugcon,iobase=0xe9,chardev=dbyte_debugcon"
    )
    $command = "$Qemu $($arguments -join ' ')"
    [IO.File]::WriteAllText((Join-Path $evidenceRoot "qemu.${Ram}m.command.txt"), $command)
    $process = Start-Process -FilePath $Qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        if (-not $process.WaitForExit(12000)) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
    }
    finally { if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force } }
    Assert-True (Test-Path -LiteralPath $serial) "QEMU ${Ram}M serial is missing"
    $actual = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($serial))
    Assert-True ($actual -ceq $expectedSerial) "QEMU ${Ram}M serial mismatch:`n$actual"
    $traceText = Get-Content -LiteralPath $trace -Raw
    Assert-True (-not $traceText.Contains("Triple fault")) "QEMU ${Ram}M triple faulted"
    Assert-True (([regex]::Matches($traceText, 'v=0e', [Text.RegularExpressions.RegexOptions]::IgnoreCase)).Count -eq 0) "QEMU ${Ram}M raised a page fault"
    $telemetry = Read-Telemetry $debug
    foreach ($key in @(
        "SLEEP_START", "SLEEP_END", "SLEEP_DELTA", "A_SWITCH_BEFORE", "A_SWITCH_AFTER",
        "B_WAKE_ORDER", "D_WAKE_ORDER", "C_PROGRESS", "C_WHILE_BLOCKED", "A_PROGRESS",
        "B_PROGRESS", "D_PROGRESS", "E_PROGRESS", "F_PROGRESS", "E_WAIT_TICK", "E_WAKE_TICK",
        "F_WAIT_TICK", "F_WAKE_TICK", "TASK_IF_BEFORE", "TASK_IF_AFTER", "EMPTY_IF_BEFORE",
        "EMPTY_IF_AFTER", "BLOCKED_MAPPED", "REUSE_SLOT", "TICKS", "SWITCHES", "EXITS",
        "RECLAIMS", "EOI", "LIVE", "WAIT_SEQ", "WAKE_SEQ", "TRAPS", "TRAP_EOI",
        "IRQ_EMPTY", "IRQ_WAKE", "IRQ_IF_BEFORE", "IRQ_IF_AFTER", "FRAME", "REUSED_FRAME"
    )) { Assert-True ($telemetry.ContainsKey($key)) "QEMU ${Ram}M telemetry lacks $key" }
    Assert-True ($telemetry.SLEEP_DELTA -ge 5 -and [uint64]$telemetry.SLEEP_DELTA -le 2147483647) "sleep deadline was not honored"
    Assert-True ($telemetry.A_SWITCH_AFTER -eq ($telemetry.A_SWITCH_BEFORE + 1)) "sleeping task consumed scheduler turns"
    Assert-True ($telemetry.B_WAKE_ORDER -eq 1 -and $telemetry.D_WAKE_ORDER -eq 2) "event wake order is not FIFO"
    Assert-True ($telemetry.C_PROGRESS -gt 0 -and $telemetry.C_WHILE_BLOCKED -gt 0) "runnable task did not progress while peers blocked"
    Assert-True ($telemetry.A_PROGRESS -eq 3 -and $telemetry.B_PROGRESS -eq 3 -and $telemetry.D_PROGRESS -eq 2) "task-context block lifecycle is incomplete"
    Assert-True ($telemetry.E_PROGRESS -eq 2 -and $telemetry.E_WAKE_TICK -gt $telemetry.E_WAIT_TICK) "task empty pulse became sticky"
    Assert-True ($telemetry.F_PROGRESS -eq 2 -and $telemetry.F_WAIT_TICK -gt 1 -and $telemetry.F_WAKE_TICK -ge 9) "IRQ empty pulse became sticky or wake was lost"
    Assert-True (($telemetry.TASK_IF_BEFORE -band 0x200) -ne 0 -and ($telemetry.TASK_IF_AFTER -band 0x200) -ne 0) "task signal did not preserve IF=1"
    Assert-True (($telemetry.EMPTY_IF_BEFORE -band 0x200) -ne 0 -and ($telemetry.EMPTY_IF_AFTER -band 0x200) -ne 0) "empty task signal did not preserve IF=1"
    Assert-True (($telemetry.IRQ_IF_BEFORE -band 0x200) -eq 0 -and ($telemetry.IRQ_IF_AFTER -band 0x200) -eq 0) "IRQ signal enabled interrupts"
    Assert-True ($telemetry.IRQ_EMPTY -eq 0 -and $telemetry.IRQ_WAKE -eq 0) "IRQ event signal failed"
    Assert-True ($telemetry.BLOCKED_MAPPED -eq 1) "blocked task stacks were not retained"
    Assert-True ($telemetry.EOI -eq $telemetry.TICKS) "IRQ0 did not emit exactly one EOI per tick"
    Assert-True ($telemetry.TRAPS -eq 5 -and $telemetry.TRAP_EOI -eq 0) "software trap count or EOI ownership is wrong"
    Assert-True ($telemetry.WAIT_SEQ -eq 4 -and $telemetry.WAKE_SEQ -eq 5) "block/wake accounting is wrong"
    Assert-True ($telemetry.EXITS -eq 7 -and $telemetry.RECLAIMS -eq 7 -and $telemetry.LIVE -eq 1) "blocking lifecycle did not converge to idle only"
    Assert-True ($telemetry.REUSE_SLOT -eq 1 -and $telemetry.FRAME -eq $telemetry.REUSED_FRAME) "slot/frame ownership was not reused exactly"
    Start-Sleep -Milliseconds 100
    Assert-True ($null -eq (Get-Process -Id $process.Id -ErrorAction SilentlyContinue)) "QEMU PID $($process.Id) remains alive"
    return @{ Command = $command; Serial = $actual; Telemetry = $telemetry }
}

Reset-Evidence $evidenceRoot
Reset-Evidence $repeatRoot
$cargo = (Get-Command cargo -ErrorAction Stop).Source
$objdump = (Get-Command objdump -ErrorAction Stop).Source
$qemu = (Get-Command qemu-system-i386 -ErrorAction Stop).Source
Invoke-Checked $cargo @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build dbytec" | Out-Null
$compiler = Join-Path $repoRoot "target\debug\dbytec.exe"

$pc32 = 0
$abs32 = 0
foreach ($module in $modules) {
    $source = Join-Path $sourceRoot "$module.dby"
    $first = Join-Path $evidenceRoot "$module.o"
    $second = Join-Path $repeatRoot "$module.o"
    Invoke-Checked $compiler @("build", $source, "--target", "i686-dbyte-none", "--emit", "obj", "-o", $first) "build $module object" | Out-Null
    Invoke-Checked $compiler @("build", $source, "--target", "i686-dbyte-none", "--emit", "obj", "-o", $second) "repeat $module object" | Out-Null
    Assert-SameFile $first $second "$module object"
    $inspection = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", $first) "inspect $module object"
    [IO.File]::WriteAllText("$first.objdump.txt", $inspection)
    Assert-True ($inspection.Contains("file format elf32-i386")) "$module is not ELF32/i386"
    if ($module -in @("scheduler", "tasks", "kernel")) { Assert-True ($inspection.Contains(".dbyte.tasks")) "$module lacks task metadata" }
    if ($module -eq "scheduler") { Assert-True ($inspection.Contains(".dbyte.interrupts")) "scheduler lacks interrupt metadata" }
    $pc32 += ([regex]::Matches($inspection, 'R_386_PC32')).Count
    $abs32 += ([regex]::Matches($inspection, 'R_386_32')).Count
}

$objectsA = @($modules | ForEach-Object { Join-Path $evidenceRoot "$_.o" })
$objectsB = @($modules | ForEach-Object { Join-Path $repeatRoot "$_.o" })
$elf = Join-Path $evidenceRoot "blocking.elf"
$repeatElf = Join-Path $repeatRoot "blocking.elf"
Invoke-Checked $compiler (@("link") + $objectsA + @("-o", $elf)) "link blocking ELF" | Out-Null
Invoke-Checked $compiler (@("link") + $objectsB + @("-o", $repeatElf)) "repeat-link blocking ELF" | Out-Null
Assert-SameFile $elf $repeatElf "linked ELF"
$integrated = Join-Path $evidenceRoot "blocking.integrated.elf"
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "kernel.dby"), "--target", "i686-dbyte-none", "--emit", "elf", "-o", $integrated) "integrated blocking ELF" | Out-Null
Assert-SameFile $elf $integrated "object-link and integrated ELF"
$bin = Join-Path $evidenceRoot "blocking.bin"
$repeatBin = Join-Path $repeatRoot "blocking.bin"
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "kernel.dby"), "--target", "i686-dbyte-none", "--emit", "bin", "-o", $bin) "build blocking BIN" | Out-Null
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "kernel.dby"), "--target", "i686-dbyte-none", "--emit", "bin", "-o", $repeatBin) "repeat blocking BIN" | Out-Null
Assert-SameFile $bin $repeatBin "blocking BIN"

$finalInspection = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", $elf) "inspect final ELF"
[IO.File]::WriteAllText((Join-Path $evidenceRoot "final.objdump.txt"), $finalInspection)
Assert-True (-not $finalInspection.Contains(".dbyte.tasks") -and -not $finalInspection.Contains(".dbyte.interrupts") -and -not $finalInspection.Contains(".dbyte.entry")) "final ELF retained compile-time metadata"
Assert-True (-not $finalInspection.Contains(".interp") -and -not $finalInspection.Contains(".dynamic")) "final ELF has a runtime dependency"
foreach ($needle in @("<block_trap>", "<scheduler.o::__dbyte_interrupt_block_trap_body>", "int    0x81", "cd 81", "pusha", "mov    esp,eax", "popa", "iret", "<irq0>", "<event_signal>", "<task_sleep_ticks>", "<event_wait>")) {
    Assert-True ($finalInspection.Contains($needle)) "final disassembly lacks $needle"
}
$trapBody = [regex]::Match($finalInspection, '(?s)<scheduler\.o::__dbyte_interrupt_block_trap_body>:(.*?)(?=\n[0-9a-f]+ <)').Groups[1].Value
Assert-True ($trapBody.Length -gt 0) "block trap body was not isolated"
foreach ($forbidden in @("<eoi_master>", "<alloc>", "<allocate>", "<map_kernel_page>", "<unmap_kernel_page>", "<free>")) {
    Assert-True (-not $trapBody.Contains($forbidden)) "block trap calls $forbidden"
}
$irqBody = [regex]::Match($finalInspection, '(?s)<scheduler\.o::__dbyte_interrupt_irq0_body>:(.*?)(?=\n[0-9a-f]+ <)').Groups[1].Value
Assert-True ($irqBody.Length -gt 0) "IRQ0 body was not isolated"
Assert-True (([regex]::Matches($irqBody, '<eoi_master>')).Count -eq 1) "IRQ0 lacks exactly one EOI path"
foreach ($forbidden in @("<alloc>", "<allocate>", "<map_kernel_page>", "<unmap_kernel_page>", "<free>")) {
    Assert-True (-not $irqBody.Contains($forbidden)) "IRQ0 fast path calls $forbidden"
}
$signalBody = [regex]::Match($finalInspection, '(?s)<event_signal>:(.*?)(?=\n[0-9a-f]+ <)').Groups[1].Value
Assert-True ($signalBody.Length -gt 0) "event_signal body was not isolated"
foreach ($forbidden in @("<eoi_master>", "<interrupt_resume>", "<alloc>", "<allocate>", "<map_kernel_page>", "<unmap_kernel_page>", "cd 81")) {
    Assert-True (-not $signalBody.Contains($forbidden)) "event_signal contains $forbidden"
}

$taskSource = Get-Content -LiteralPath (Join-Path $sourceRoot "tasks.dby") -Raw
foreach ($function in @("task_a", "task_b", "task_d", "task_empty_probe", "irq_waiter")) {
    $body = [regex]::Match($taskSource, "(?ms)^pub fn $function\(.*?(?=^pub fn |^fn |\z)").Value
    Assert-True ($body.Length -gt 0) "proof source lacks $function"
    Assert-True (-not [regex]::IsMatch($body, '\bwhile\b|cpu_hlt|\byield\s*\(')) "$function busy-waits or yields"
}
$schedulerSource = Get-Content -LiteralPath (Join-Path $sourceRoot "scheduler.dby") -Raw
Assert-True ($schedulerSource.Contains("if (flags & IF_BIT)")) "event_signal does not restore IF conditionally"
Assert-True (([regex]::Matches($schedulerSource, 'native\.cpu_sti\(\)')).Count -eq 1) "scheduler source contains an unconditional or duplicate STI"
foreach ($wrapper in @("event_wait", "task_sleep_ticks")) {
    $body = [regex]::Match($schedulerSource, "(?ms)^pub fn $wrapper\(.*?(?=^pub fn |^pub interrupt|^fn |\z)").Value
    Assert-True ($body.Contains("native.task_block_trap")) "$wrapper does not cross INT 0x81"
    Assert-True (-not [regex]::IsMatch($body, '\bset_state\s*\(|\bstore\s*\(')) "$wrapper mutates task state before INT 0x81"
}
$sourceText = ($modules | ForEach-Object { Get-Content -LiteralPath (Join-Path $sourceRoot "$_.dby") -Raw }) -join "`n"
foreach ($token in @("global_asm", "nasm", "gcc", "clang", "rustc", "llvm", "std::process", "int 14")) {
    Assert-True (-not $sourceText.Contains($token)) "blocking source contains forbidden token '$token'"
}
$irDump = Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "scheduler.dby"), "--target", "i686-dbyte-none", "--emit", "obj", "-o", (Join-Path $evidenceRoot "scheduler.ir.object"), "--dump-ir") "dump blocking IR"
[IO.File]::WriteAllText((Join-Path $evidenceRoot "blocking.ir.txt"), $irDump)
Assert-True ($irDump.Contains("intrinsic native.task_block_trap 2 -> i32")) "blocking IR lacks typed trap"
Assert-True (-not [regex]::IsMatch($irDump, '\byield\b')) "blocking proof IR contains yield"

$run32 = Invoke-Qemu 32 $elf $qemu
$run64 = Invoke-Qemu 64 $elf $qemu
$hashes = @()
foreach ($file in Get-ChildItem -LiteralPath $evidenceRoot -Filter "*.o") {
    $hashes += "$($file.Name) $((Get-FileHash $file.FullName -Algorithm SHA256).Hash)"
}
$elfHash = (Get-FileHash $elf -Algorithm SHA256).Hash
$binHash = (Get-FileHash $bin -Algorithm SHA256).Hash
$hashes += "ELF $elfHash", "BIN $binHash"
[IO.File]::WriteAllLines((Join-Path $evidenceRoot "sha256.txt"), $hashes)

Write-Host "DBYTE NATIVE KERNEL BLOCKING: PASS"
Write-Host "objects=$($modules.Count) R_386_PC32=$pc32 R_386_32=$abs32"
Write-Host "ticks=$($run32.Telemetry.TICKS) switches=$($run32.Telemetry.SWITCHES) traps=$($run32.Telemetry.TRAPS) waits=$($run32.Telemetry.WAIT_SEQ) wakes=$($run32.Telemetry.WAKE_SEQ)"
Write-Host "ELF_SHA256=$elfHash"
Write-Host "BIN_SHA256=$binHash"
Write-Host "QEMU_32=$($run32.Command)"
Write-Host "QEMU_64=$($run64.Command)"
