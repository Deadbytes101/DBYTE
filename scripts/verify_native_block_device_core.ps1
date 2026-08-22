param([string]$MilestoneVersion = "11.8.0")

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$initialQemuIds = @(Get-Process -Name "qemu-system-i386" -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$tmpRoot = (Resolve-Path (Join-Path $repoRoot "tmp")).Path
$evidenceRoot = Join-Path $tmpRoot "native_v$MilestoneVersion"
$repeatRoot = Join-Path $tmpRoot "native_v$MilestoneVersion.repeat"
$sourceRoot = Join-Path $repoRoot "examples\native_block_device_core"
$modules = @("arch_x86", "serial", "gdt", "boot", "frames", "paging", "memory_ops", "heap", "pic", "proof", "page_fault", "byte_ring", "scheduler", "keyboard", "ata", "tasks", "pit", "idt_normal", "kernel")
$expectedSerial = "DBYTEOS NATIVE BLOCK DEVICE`r`nATA IDENTIFY ONLINE`r`nATA IRQ14 ONLINE`r`nBLOCK READ ONLINE`r`nBLOCK WRITE ONLINE`r`nIRQ COMPLETION OK`r`nSECTOR READ OK`r`nSECTOR WRITE OK`r`nNO BUSY WAIT OK`r`nBLOCK DEVICE CHECK OK`r`n"

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

function New-TestDisk([string]$Path) {
    $directory = Split-Path -Parent $Path
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    [byte[]]$bytes = New-Object byte[] (8MB)
    [byte[]]$header = [Text.Encoding]::ASCII.GetBytes("DBYTE_BLOCK_TEST_V1")
    [Array]::Copy($header, 0, $bytes, 8 * 512, $header.Length)
    for ($index = 19; $index -lt 512; $index++) {
        $bytes[(8 * 512) + $index] = [byte](($index * 37 + 0x2B) -band 0xFF)
    }
    for ($index = 0; $index -lt 512; $index++) {
        $bytes[(9 * 512) + $index] = [byte](($index * 13 + 0x5A) -band 0xFF)
    }
    [IO.File]::WriteAllBytes($Path, $bytes)
}

function Assert-DiskSeed([string]$Path) {
    [byte[]]$bytes = [IO.File]::ReadAllBytes($Path)
    Assert-True ($bytes.Length -eq 8MB) "test disk is not exactly 8 MiB"
    [byte[]]$header = [Text.Encoding]::ASCII.GetBytes("DBYTE_BLOCK_TEST_V1")
    for ($index = 0; $index -lt $header.Length; $index++) {
        Assert-True ($bytes[(8 * 512) + $index] -eq $header[$index]) "LBA8 header differs at byte $index"
    }
    for ($index = 19; $index -lt 512; $index++) {
        Assert-True ($bytes[(8 * 512) + $index] -eq (($index * 37 + 0x2B) -band 0xFF)) "LBA8 pattern differs at byte $index"
    }
    for ($index = 0; $index -lt 512; $index++) {
        Assert-True ($bytes[(9 * 512) + $index] -eq (($index * 13 + 0x5A) -band 0xFF)) "LBA9 seed differs at byte $index"
    }
}

function Assert-DiskWritten([string]$Path) {
    [byte[]]$bytes = [IO.File]::ReadAllBytes($Path)
    for ($index = 0; $index -lt 512; $index++) {
        Assert-True ($bytes[(9 * 512) + $index] -eq (($index * 29 + 0xA7) -band 0xFF)) "LBA9 writeback differs at byte $index"
    }
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
    $runRoot = Join-Path $evidenceRoot "${Ram}m"
    New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
    $disk = Join-Path $runRoot "test-disk.raw"
    New-TestDisk $disk
    Assert-DiskSeed $disk
    $seedHash = (Get-FileHash $disk -Algorithm SHA256).Hash
    $serial = Join-Path $runRoot "serial.log"
    $trace = Join-Path $runRoot "trace.log"
    $debug = Join-Path $runRoot "debugcon.log"
    $arguments = @(
        "-m", "${Ram}M", "-kernel", $Elf, "-display", "none",
        "-serial", "file:$serial", "-monitor", "none", "-no-reboot", "-no-shutdown",
        "-d", "int,cpu_reset,guest_errors", "-D", $trace,
        "-chardev", "file,id=dbyte_debugcon,path=$debug",
        "-device", "isa-debugcon,iobase=0xe9,chardev=dbyte_debugcon",
        "-drive", "file=$disk,format=raw,if=ide,index=0,media=disk,cache=directsync,throttling.iops-total=25"
    )
    $command = "$Qemu $($arguments -join ' ')"
    [IO.File]::WriteAllText((Join-Path $runRoot "command.txt"), $command)
    $process = Start-Process -FilePath $Qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        if (-not $process.WaitForExit(8000)) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
    }
    finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
    }
    Assert-True (Test-Path -LiteralPath $serial) "QEMU ${Ram}M serial is missing"
    $actual = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($serial))
    Assert-True ($actual -ceq $expectedSerial) "QEMU ${Ram}M serial mismatch:`n$actual"
    $traceText = Get-Content -LiteralPath $trace -Raw
    Assert-True (-not $traceText.Contains("Triple fault")) "QEMU ${Ram}M triple faulted"
    Assert-True (([regex]::Matches($traceText, 'v=0e', [Text.RegularExpressions.RegexOptions]::IgnoreCase)).Count -eq 0) "QEMU ${Ram}M raised a page fault"
    Assert-True (([regex]::Matches($traceText, 'v=0d', [Text.RegularExpressions.RegexOptions]::IgnoreCase)).Count -eq 0) "QEMU ${Ram}M raised a general-protection fault"
    $irq14Deliveries = ([regex]::Matches($traceText, 'Servicing hardware INT=0x2e')).Count
    Assert-True ($irq14Deliveries -eq 4) "QEMU ${Ram}M delivered IRQ14 $irq14Deliveries times instead of four"
    $telemetry = Read-Telemetry $debug
    foreach ($key in @(
        "SECTORS", "ATA_LAST", "IRQ14", "IRQ14_ACCEPT", "IRQ14_SPURIOUS",
        "IRQ14_SLAVE_EOI", "IRQ14_MASTER_EOI", "ATA_POSTS", "READ_WORDS", "WRITE_WORDS",
        "READER_BLOCK", "READER_WAKE", "EARLY_COMPLETE", "PROOF_BLOCK", "PROOF_WAKE",
        "PROGRESS", "PROGRESS_BLOCKED", "PIC_MASTER", "PIC_SLAVE", "RAW_ERROR", "REUSE_SLOT",
        "TICKS", "SWITCHES", "EXITS", "RECLAIMS", "EOI", "LIVE", "TRAPS", "TRAP_EOI",
        "SEM_WAITS", "SEM_POSTS", "SEM_POST_OK", "SEM_WAKES", "SEM_OVERFLOW", "FRAME", "REUSED_FRAME"
    )) { Assert-True ($telemetry.ContainsKey($key)) "QEMU ${Ram}M telemetry lacks $key" }
    Assert-True ($telemetry.SECTORS -eq 16384 -and $telemetry.ATA_LAST -eq 0 -and $telemetry.RAW_ERROR -eq 0) "ATA IDENTIFY or final status is wrong"
    Assert-True ($telemetry.IRQ14 -eq 4 -and $telemetry.IRQ14_ACCEPT -eq 4 -and $telemetry.IRQ14_SPURIOUS -eq 0) "IRQ14 request accounting is wrong"
    Assert-True ($telemetry.IRQ14_SLAVE_EOI -eq 4 -and $telemetry.IRQ14_MASTER_EOI -eq 4) "IRQ14 EOI accounting is wrong"
    Assert-True ($telemetry.ATA_POSTS -eq 4 -and $telemetry.READER_BLOCK -eq 4 -and $telemetry.READER_WAKE -eq 4) "ATA completion did not cover four requests"
    Assert-True ($telemetry.READ_WORDS -eq 768 -and $telemetry.WRITE_WORDS -eq 256) "PIO word counts are wrong"
    Assert-True ($telemetry.PROOF_WAKE -gt $telemetry.PROOF_BLOCK) "proof read did not remain blocked across an IRQ0 tick"
    Assert-True ($telemetry.PROGRESS -gt 0 -and $telemetry.PROGRESS_BLOCKED -gt 0) "runnable task did not progress during disk blocking"
    Assert-True ($telemetry.PIC_MASTER -eq 0xf8 -and $telemetry.PIC_SLAVE -eq 0xbf) "PIC masks are not IRQ0+IRQ1+cascade/IRQ14 only"
    Assert-True ($telemetry.EOI -eq $telemetry.TICKS) "IRQ0 did not emit exactly one EOI per tick"
    Assert-True ($telemetry.TRAPS -eq 4 -and $telemetry.TRAP_EOI -eq 0) "block trap or EOI ownership is wrong"
    Assert-True ($telemetry.SEM_WAITS -eq 4 -and $telemetry.SEM_POSTS -eq 4 -and $telemetry.SEM_POST_OK -eq 4) "completion semaphore accounting is wrong"
    Assert-True (($telemetry.SEM_WAKES + $telemetry.EARLY_COMPLETE) -eq 4 -and $telemetry.EARLY_COMPLETE -eq 1) "IRQ wake plus lost-wake token accounting is wrong"
    Assert-True ($telemetry.EXITS -eq 3 -and $telemetry.RECLAIMS -eq 3 -and $telemetry.LIVE -eq 1) "block-device lifecycle did not converge to idle only"
    Assert-True ($telemetry.REUSE_SLOT -eq 1 -and $telemetry.FRAME -eq $telemetry.REUSED_FRAME) "slot/frame ownership was not reused exactly"
    Assert-DiskWritten $disk
    $finalHash = (Get-FileHash $disk -Algorithm SHA256).Hash
    Assert-True ($seedHash -ne $finalHash) "scratch sector did not change the disposable disk"
    Start-Sleep -Milliseconds 100
    Assert-True ($null -eq (Get-Process -Id $process.Id -ErrorAction SilentlyContinue)) "QEMU PID $($process.Id) remains alive"
    return @{ Command = $command; Serial = $actual; Telemetry = $telemetry; SeedHash = $seedHash; FinalHash = $finalHash }
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
    if ($module -in @("scheduler", "keyboard", "ata")) { Assert-True ($inspection.Contains(".dbyte.interrupts")) "$module lacks interrupt metadata" }
    $pc32 += ([regex]::Matches($inspection, 'R_386_PC32')).Count
    $abs32 += ([regex]::Matches($inspection, 'R_386_32')).Count
}

$objectsA = @($modules | ForEach-Object { Join-Path $evidenceRoot "$_.o" })
$objectsB = @($modules | ForEach-Object { Join-Path $repeatRoot "$_.o" })
$elf = Join-Path $evidenceRoot "block_device.elf"
$repeatElf = Join-Path $repeatRoot "block_device.elf"
Invoke-Checked $compiler (@("link") + $objectsA + @("-o", $elf)) "link block-device ELF" | Out-Null
Invoke-Checked $compiler (@("link") + $objectsB + @("-o", $repeatElf)) "repeat-link block-device ELF" | Out-Null
Assert-SameFile $elf $repeatElf "linked ELF"
$integrated = Join-Path $evidenceRoot "block_device.integrated.elf"
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "kernel.dby"), "--target", "i686-dbyte-none", "--emit", "elf", "-o", $integrated) "integrated block-device ELF" | Out-Null
Assert-SameFile $elf $integrated "object-link and integrated ELF"
$bin = Join-Path $evidenceRoot "block_device.bin"
$repeatBin = Join-Path $repeatRoot "block_device.bin"
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "kernel.dby"), "--target", "i686-dbyte-none", "--emit", "bin", "-o", $bin) "build block-device BIN" | Out-Null
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "kernel.dby"), "--target", "i686-dbyte-none", "--emit", "bin", "-o", $repeatBin) "repeat block-device BIN" | Out-Null
Assert-SameFile $bin $repeatBin "block-device BIN"

$finalInspection = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", $elf) "inspect final ELF"
[IO.File]::WriteAllText((Join-Path $evidenceRoot "final.objdump.txt"), $finalInspection)
Assert-True (-not $finalInspection.Contains(".dbyte.tasks") -and -not $finalInspection.Contains(".dbyte.interrupts") -and -not $finalInspection.Contains(".dbyte.entry")) "final ELF retained compile-time metadata"
Assert-True (-not $finalInspection.Contains(".interp") -and -not $finalInspection.Contains(".dynamic")) "final ELF has a runtime dependency"
foreach ($needle in @("<ata_irq14>", "<ata.o::__dbyte_interrupt_ata_irq14_body>", "<ata_read_sector>", "<ata_write_sector>", "<semaphore_wait>", "<semaphore_post>", "in     ax,dx", "out    dx,ax", "pusha", "popa", "iret")) {
    Assert-True ($finalInspection.Contains($needle)) "final disassembly lacks $needle"
}
$irqBody = [regex]::Match($finalInspection, '(?s)<ata\.o::__dbyte_interrupt_ata_irq14_body>:(.*?)(?=\n[0-9a-f]+ <)').Groups[1].Value
Assert-True ($irqBody.Length -gt 0) "IRQ14 body was not isolated"
Assert-True (([regex]::Matches($irqBody, '<eoi_slave_then_master>')).Count -eq 1) "IRQ14 lacks one common slave-before-master EOI path"
foreach ($forbidden in @("<alloc>", "<allocate>", "<map_kernel_page>", "<unmap_kernel_page>", "<free>", "<interrupt_resume>", "cd 81", "<write>")) {
    Assert-True (-not $irqBody.Contains($forbidden)) "IRQ14 fast path calls $forbidden"
}
$picSource = Get-Content -LiteralPath (Join-Path $sourceRoot "pic.dby") -Raw
$eoiBody = [regex]::Match($picSource, '(?ms)^pub fn eoi_slave_then_master\(.*?(?=^pub fn |^fn |\z)').Value
Assert-True ($eoiBody.IndexOf('arch.out8(SLAVE_COMMAND') -lt $eoiBody.IndexOf('arch.out8(MASTER_COMMAND')) "slave EOI does not precede master EOI"
$ataSource = Get-Content -LiteralPath (Join-Path $sourceRoot "ata.dby") -Raw
$readBody = [regex]::Match($ataSource, '(?ms)^pub fn ata_read_sector\(.*?(?=^pub fn |^pub interrupt|^fn |\z)').Value
$writeBody = [regex]::Match($ataSource, '(?ms)^pub fn ata_write_sector\(.*?(?=^pub fn |^pub interrupt|^fn |\z)').Value
Assert-True ($readBody.Contains('await_completion') -and $writeBody.Contains('await_completion')) "ATA APIs do not use completion blocking"
Assert-True (([regex]::Matches($ataSource, 'while attempt < ATA_POLL_LIMIT')).Count -eq 2) "ATA status polls are not the two fixed-bound loops"
Assert-True ($ataSource.Contains('while word < SECTOR_WORDS')) "ATA PIO transfers lack exact word bounds"
$taskSource = Get-Content -LiteralPath (Join-Path $sourceRoot "tasks.dby") -Raw
$diskBody = [regex]::Match($taskSource, '(?ms)^pub fn disk_worker\(.*?(?=^pub fn |^fn |\z)').Value
Assert-True (-not [regex]::IsMatch($diskBody, 'STATUS_PORT|arch\.in|task_sleep|cpu_hlt|\byield\s*\(')) "disk worker polls, halts, sleeps, or yields"
$sourceText = ($modules | ForEach-Object { Get-Content -LiteralPath (Join-Path $sourceRoot "$_.dby") -Raw }) -join "`n"
foreach ($token in @("global_asm", "nasm", "gcc", "clang", "rustc", "llvm", "std::process", "int 14", "AHCI", "LBA48")) {
    Assert-True (-not $sourceText.Contains($token)) "block-device source contains forbidden token '$token'"
}

$run32 = Invoke-Qemu 32 $elf $qemu
$run64 = Invoke-Qemu 64 $elf $qemu
Assert-True ($run32.SeedHash -eq $run64.SeedHash) "32/64 MiB disk seeds are not deterministic"
Assert-True ($run32.FinalHash -eq $run64.FinalHash) "32/64 MiB guest writes differ"
$hashes = @()
foreach ($file in Get-ChildItem -LiteralPath $evidenceRoot -Filter "*.o") {
    $hashes += "$($file.Name) $((Get-FileHash $file.FullName -Algorithm SHA256).Hash)"
}
$elfHash = (Get-FileHash $elf -Algorithm SHA256).Hash
$binHash = (Get-FileHash $bin -Algorithm SHA256).Hash
$hashes += "ELF $elfHash", "BIN $binHash", "DISK_SEED $($run32.SeedHash)", "DISK_FINAL $($run32.FinalHash)"
[IO.File]::WriteAllLines((Join-Path $evidenceRoot "sha256.txt"), $hashes)
$orphanQemu = @(Get-Process -Name "qemu-system-i386" -ErrorAction SilentlyContinue | Where-Object { $initialQemuIds -notcontains $_.Id })
Assert-True ($orphanQemu.Count -eq 0) "block-device verifier left QEMU processes running: $($orphanQemu.Id -join ', ')"

Write-Host "DBYTE NATIVE KERNEL BLOCK DEVICE: PASS"
Write-Host "objects=$($modules.Count) R_386_PC32=$pc32 R_386_32=$abs32"
Write-Host "ticks=$($run32.Telemetry.TICKS) irq14=$($run32.Telemetry.IRQ14) slave_eoi=$($run32.Telemetry.IRQ14_SLAVE_EOI) master_eoi=$($run32.Telemetry.IRQ14_MASTER_EOI) read_words=$($run32.Telemetry.READ_WORDS) write_words=$($run32.Telemetry.WRITE_WORDS)"
Write-Host "ELF_SHA256=$elfHash"
Write-Host "BIN_SHA256=$binHash"
Write-Host "DISK_SEED_SHA256=$($run32.SeedHash)"
Write-Host "DISK_FINAL_SHA256=$($run32.FinalHash)"
Write-Host "QEMU_32=$($run32.Command)"
Write-Host "QEMU_64=$($run64.Command)"
