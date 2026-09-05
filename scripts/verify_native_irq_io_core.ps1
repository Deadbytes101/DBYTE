param([string]$MilestoneVersion = "11.7.0")

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$tmpRoot = (Resolve-Path (Join-Path $repoRoot "tmp")).Path
$evidenceRoot = Join-Path $tmpRoot "native_v$MilestoneVersion"
$repeatRoot = Join-Path $tmpRoot "native_v$MilestoneVersion.repeat"
$sourceRoot = Join-Path $repoRoot "examples\native_irq_io_core"
$modules = @("arch_x86", "serial", "gdt", "boot", "frames", "paging", "memory_ops", "heap", "pic", "proof", "page_fault", "byte_ring", "scheduler", "keyboard", "tasks", "pit", "idt_normal", "kernel")
$expectedSerial = "DBYTEOS NATIVE IRQ IO`r`nSEMAPHORE ONLINE`r`nBYTE RING ONLINE`r`nIRQ1 ONLINE`r`nBLOCKING INPUT ONLINE`r`nWAKE FROM IRQ OK`r`nFIFO DATA OK`r`nNO BUSY WAIT OK`r`nIRQ IO CHECK OK`r`n"

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
    Assert-True (([regex]::Matches($traceText, 'v=0d', [Text.RegularExpressions.RegexOptions]::IgnoreCase)).Count -eq 0) "QEMU ${Ram}M raised a general-protection fault"
    $irq1Deliveries = ([regex]::Matches($traceText, 'Servicing hardware INT=0x21')).Count
    Assert-True ($irq1Deliveries -eq 4) "QEMU ${Ram}M delivered IRQ1 $irq1Deliveries times instead of four"
    $telemetry = Read-Telemetry $debug
    foreach ($key in @(
        "INPUT_0", "INPUT_1", "INPUT_2", "INPUT_3", "IRQ1", "IRQ1_EOI", "IRQ1_PUSH",
        "IRQ1_POST", "READER_BLOCK", "READER_WAKE", "READER_POP", "RING_DROP",
        "RING_PROBE_DROP", "B_WAKE_ORDER", "D_WAKE_ORDER", "PROGRESS", "PROGRESS_BLOCKED",
        "READER_START", "FIRST_INJECT", "PIC_MASTER", "PIC_SLAVE", "REUSE_SLOT", "TICKS",
        "SWITCHES", "EXITS", "RECLAIMS", "EOI", "LIVE", "WAIT_SEQ", "WAKE_SEQ", "TRAPS",
        "TRAP_EOI", "SEM_WAITS", "SEM_POSTS", "SEM_POST_OK", "SEM_WAKES", "SEM_OVERFLOW",
        "FRAME", "REUSED_FRAME"
    )) { Assert-True ($telemetry.ContainsKey($key)) "QEMU ${Ram}M telemetry lacks $key" }
    Assert-True ($telemetry.INPUT_0 -eq 0x1e -and $telemetry.INPUT_1 -eq 0x30 -and $telemetry.INPUT_2 -eq 0x2e -and $telemetry.INPUT_3 -eq 0x20) "raw scan bytes are not FIFO exact"
    Assert-True ($telemetry.IRQ1 -eq 4 -and $telemetry.IRQ1_EOI -eq 4 -and $telemetry.IRQ1_PUSH -eq 4 -and $telemetry.IRQ1_POST -eq 4) "IRQ1 handler accounting is incomplete"
    Assert-True ($telemetry.READER_BLOCK -eq 4 -and $telemetry.READER_WAKE -eq 4 -and $telemetry.READER_POP -eq 4) "reader did not block, wake, and pop four times"
    Assert-True ($telemetry.RING_DROP -eq 0 -and $telemetry.RING_PROBE_DROP -eq 1) "ring drop accounting is wrong"
    Assert-True ($telemetry.B_WAKE_ORDER -eq 1 -and $telemetry.D_WAKE_ORDER -eq 2) "semaphore wake order is not FIFO"
    Assert-True ($telemetry.PROGRESS -gt 0 -and $telemetry.PROGRESS_BLOCKED -gt 0) "runnable task did not progress while input was blocked"
    Assert-True ($telemetry.FIRST_INJECT -gt $telemetry.READER_START) "input was injected before the reader blocked"
    Assert-True ($telemetry.PIC_MASTER -eq 0xfc -and $telemetry.PIC_SLAVE -eq 0xff) "PIC masks are not IRQ0+IRQ1 only"
    Assert-True ($telemetry.EOI -eq $telemetry.TICKS) "IRQ0 did not emit exactly one EOI per tick"
    Assert-True ($telemetry.TRAPS -eq 11 -and $telemetry.TRAP_EOI -eq 0) "software trap count or EOI ownership is wrong"
    Assert-True ($telemetry.SEM_WAITS -eq 7 -and $telemetry.SEM_POSTS -eq 10 -and $telemetry.SEM_POST_OK -eq 9 -and $telemetry.SEM_WAKES -eq 6 -and $telemetry.SEM_OVERFLOW -eq 1) "semaphore accounting is wrong"
    Assert-True ($telemetry.EXITS -eq 7 -and $telemetry.RECLAIMS -eq 7 -and $telemetry.LIVE -eq 1) "IRQ I/O lifecycle did not converge to idle only"
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
    if ($module -in @("scheduler", "keyboard")) { Assert-True ($inspection.Contains(".dbyte.interrupts")) "$module lacks interrupt metadata" }
    $pc32 += ([regex]::Matches($inspection, 'R_386_PC32')).Count
    $abs32 += ([regex]::Matches($inspection, 'R_386_32')).Count
}

$objectsA = @($modules | ForEach-Object { Join-Path $evidenceRoot "$_.o" })
$objectsB = @($modules | ForEach-Object { Join-Path $repeatRoot "$_.o" })
$elf = Join-Path $evidenceRoot "irq_io.elf"
$repeatElf = Join-Path $repeatRoot "irq_io.elf"
Invoke-Checked $compiler (@("link") + $objectsA + @("-o", $elf)) "link IRQ I/O ELF" | Out-Null
Invoke-Checked $compiler (@("link") + $objectsB + @("-o", $repeatElf)) "repeat-link IRQ I/O ELF" | Out-Null
Assert-SameFile $elf $repeatElf "linked ELF"
$integrated = Join-Path $evidenceRoot "irq_io.integrated.elf"
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "kernel.dby"), "--target", "i686-dbyte-none", "--emit", "elf", "-o", $integrated) "integrated IRQ I/O ELF" | Out-Null
Assert-SameFile $elf $integrated "object-link and integrated ELF"
$bin = Join-Path $evidenceRoot "irq_io.bin"
$repeatBin = Join-Path $repeatRoot "irq_io.bin"
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "kernel.dby"), "--target", "i686-dbyte-none", "--emit", "bin", "-o", $bin) "build IRQ I/O BIN" | Out-Null
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "kernel.dby"), "--target", "i686-dbyte-none", "--emit", "bin", "-o", $repeatBin) "repeat IRQ I/O BIN" | Out-Null
Assert-SameFile $bin $repeatBin "IRQ I/O BIN"

$finalInspection = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", $elf) "inspect final ELF"
[IO.File]::WriteAllText((Join-Path $evidenceRoot "final.objdump.txt"), $finalInspection)
Assert-True (-not $finalInspection.Contains(".dbyte.tasks") -and -not $finalInspection.Contains(".dbyte.interrupts") -and -not $finalInspection.Contains(".dbyte.entry")) "final ELF retained compile-time metadata"
Assert-True (-not $finalInspection.Contains(".interp") -and -not $finalInspection.Contains(".dynamic")) "final ELF has a runtime dependency"
foreach ($needle in @("<block_trap>", "<scheduler.o::__dbyte_interrupt_block_trap_body>", "int    0x81", "cd 81", "pusha", "mov    esp,eax", "popa", "iret", "<irq0>", "<irq1>", "<keyboard.o::__dbyte_interrupt_irq1_body>", "<semaphore_wait>", "<semaphore_post>", "<byte_ring_push>", "<byte_ring_pop>")) {
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
$irq1Body = [regex]::Match($finalInspection, '(?s)<keyboard\.o::__dbyte_interrupt_irq1_body>:(.*?)(?=\n[0-9a-f]+ <)').Groups[1].Value
Assert-True ($irq1Body.Length -gt 0) "IRQ1 body was not isolated"
Assert-True (([regex]::Matches($irq1Body, '<eoi_master>')).Count -eq 1) "IRQ1 lacks exactly one EOI path"
foreach ($forbidden in @("<alloc>", "<allocate>", "<map_kernel_page>", "<unmap_kernel_page>", "<free>", "<interrupt_resume>", "cd 81", "<write>")) {
    Assert-True (-not $irq1Body.Contains($forbidden)) "IRQ1 fast path calls $forbidden"
}
$signalBody = [regex]::Match($finalInspection, '(?s)<semaphore_post>:(.*?)(?=\n[0-9a-f]+ <)').Groups[1].Value
Assert-True ($signalBody.Length -gt 0) "semaphore_post body was not isolated"
foreach ($forbidden in @("<eoi_master>", "<interrupt_resume>", "<alloc>", "<allocate>", "<map_kernel_page>", "<unmap_kernel_page>", "cd 81")) {
    Assert-True (-not $signalBody.Contains($forbidden)) "semaphore_post contains $forbidden"
}

$taskSource = Get-Content -LiteralPath (Join-Path $sourceRoot "tasks.dby") -Raw
foreach ($function in @("reader", "semaphore_waiter_b", "semaphore_waiter_d")) {
    $body = [regex]::Match($taskSource, "(?ms)^pub fn $function\(.*?(?=^pub fn |^fn |\z)").Value
    Assert-True ($body.Length -gt 0) "proof source lacks $function"
    Assert-True (-not [regex]::IsMatch($body, 'tick_count|cpu_hlt|\byield\s*\(')) "$function polls, halts, or yields"
}
$schedulerSource = Get-Content -LiteralPath (Join-Path $sourceRoot "scheduler.dby") -Raw
Assert-True ($schedulerSource.Contains("if (flags & IF_BIT)")) "semaphore operations do not restore IF conditionally"
foreach ($wrapper in @("event_wait", "task_sleep_ticks", "semaphore_wait")) {
    $body = [regex]::Match($schedulerSource, "(?ms)^pub fn $wrapper\(.*?(?=^pub fn |^pub interrupt|^fn |\z)").Value
    Assert-True ($body.Contains("native.task_block_trap")) "$wrapper does not cross INT 0x81"
    Assert-True (-not [regex]::IsMatch($body, '\bset_state\s*\(|\bstore\s*\(')) "$wrapper mutates task state before INT 0x81"
}
$keyboardSource = Get-Content -LiteralPath (Join-Path $sourceRoot "keyboard.dby") -Raw
$readerBody = [regex]::Match($keyboardSource, '(?ms)^pub fn keyboard_read_scancode\(.*?(?=^pub fn |^pub interrupt|^fn |\z)').Value
Assert-True ($readerBody.Contains('scheduler.semaphore_wait') -and $readerBody.Contains('ring.byte_ring_pop')) "keyboard reader does not block then pop"
Assert-True (-not [regex]::IsMatch($readerBody, 'tick_count|STATUS_PORT|arch\.in8|task_sleep|\byield\s*\(')) "keyboard reader polls instead of blocking"
$sourceText = ($modules | ForEach-Object { Get-Content -LiteralPath (Join-Path $sourceRoot "$_.dby") -Raw }) -join "`n"
foreach ($token in @("global_asm", "nasm", "gcc", "clang", "rustc", "llvm", "std::process", "int 14")) {
    Assert-True (-not $sourceText.Contains($token)) "IRQ I/O source contains forbidden token '$token'"
}
$irDump = Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "scheduler.dby"), "--target", "i686-dbyte-none", "--emit", "obj", "-o", (Join-Path $evidenceRoot "scheduler.ir.object"), "--dump-ir") "dump blocking IR"
[IO.File]::WriteAllText((Join-Path $evidenceRoot "irq_io.ir.txt"), $irDump)
$typedTrapPattern = 'Intrinsic \{ destination: Some\(VirtualRegister\(\d+\)\), intrinsic: TaskBlockTrap, arguments: \[VirtualRegister\(\d+\), VirtualRegister\(\d+\)\], result: Some\(I32\), volatile: true \}'
Assert-True (([regex]::Matches($irDump, $typedTrapPattern)).Count -eq 3) "IRQ I/O Native MIR lacks the three typed two-argument block traps"
Assert-True ($schedulerSource.Contains("native.task_block_trap(3 as u32")) "scheduler source lacks checked semaphore request kind"
Assert-True (-not [regex]::IsMatch($irDump, '\byield\b')) "IRQ I/O proof IR contains yield"

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

Write-Host "DBYTE NATIVE KERNEL IRQ IO: PASS"
Write-Host "objects=$($modules.Count) R_386_PC32=$pc32 R_386_32=$abs32"
Write-Host "ticks=$($run32.Telemetry.TICKS) switches=$($run32.Telemetry.SWITCHES) traps=$($run32.Telemetry.TRAPS) irq1=$($run32.Telemetry.IRQ1) sem_waits=$($run32.Telemetry.SEM_WAITS) sem_posts=$($run32.Telemetry.SEM_POSTS) sem_wakes=$($run32.Telemetry.SEM_WAKES)"
Write-Host "ELF_SHA256=$elfHash"
Write-Host "BIN_SHA256=$binHash"
Write-Host "QEMU_32=$($run32.Command)"
Write-Host "QEMU_64=$($run64.Command)"
