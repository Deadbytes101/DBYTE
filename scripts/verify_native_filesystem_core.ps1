param([string]$MilestoneVersion = "11.9.0")

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$tmpRoot = (Resolve-Path (Join-Path $repoRoot "tmp")).Path
$evidenceRoot = Join-Path $tmpRoot "native_v$MilestoneVersion"
$repeatRoot = Join-Path $tmpRoot "native_v$MilestoneVersion.repeat"
$sourceRoot = Join-Path $repoRoot "examples\native_filesystem_core"
$initialQemuIds = @(Get-Process -Name "qemu-system-i386" -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$modules = @(
    "arch_x86", "serial", "gdt", "boot", "frames", "paging", "memory_ops", "heap",
    "pic", "proof", "page_fault", "byte_ring", "scheduler", "keyboard", "ata",
    "dbfs_layout", "dbfs_bitmap", "dbfs_inode", "dbfs_dir", "dbfs", "tasks", "pit",
    "idt_normal", "kernel"
)
$boot1Serial = "DBYTEOS NATIVE FILESYSTEM`r`nDBFS FORMAT ONLINE`r`nDBFS MOUNT ONLINE`r`nROOT DIRECTORY ONLINE`r`nFILE CREATE OK`r`nFILE WRITE OK`r`nFILE READBACK OK`r`nBLOCK REUSE OK`r`nPERSISTENCE ARMED`r`nFILESYSTEM CHECK OK`r`n"
$boot2Serial = "DBYTEOS NATIVE FILESYSTEM`r`nDBFS MOUNT ONLINE`r`nPERSISTENT FILE FOUND`r`nFILE CONTENT OK`r`nDIRECTORY STATE OK`r`nPERSISTENCE CHECK OK`r`nFILESYSTEM CHECK OK`r`n"

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

function Assert-ProofPath([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    $prefix = [IO.Path]::GetFullPath($tmpRoot).TrimEnd('\') + '\native_v11.9.0'
    Assert-True ($full.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) "proof disk/evidence path escapes tmp/native_v11.9.0: $full"
}

function Reset-Evidence([string]$Path) {
    Assert-ProofPath $Path
    if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Recurse -Force }
    New-Item -ItemType Directory -Path $Path -Force | Out-Null
}

function Assert-SameFile([string]$Left, [string]$Right, [string]$Role) {
    [byte[]]$a = [IO.File]::ReadAllBytes($Left)
    [byte[]]$b = [IO.File]::ReadAllBytes($Right)
    Assert-True ([Linq.Enumerable]::SequenceEqual($a, $b)) "$Role is not deterministic"
}

function Read-SharedBytes([string]$Path) {
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
    try {
        [byte[]]$bytes = New-Object byte[] $stream.Length
        [void]$stream.Read($bytes, 0, $bytes.Length)
        return ,$bytes
    }
    finally { $stream.Dispose() }
}

function Read-U32([byte[]]$Bytes, [int]$Offset) {
    return [BitConverter]::ToUInt32($Bytes, $Offset)
}

function Get-Fnv1a([byte[]]$Bytes, [int]$Offset, [int]$Count) {
    [uint32]$hash = 2166136261
    for ($index = 0; $index -lt $Count; $index++) {
        [uint64]$product = [uint64]($hash -bxor $Bytes[$Offset + $index]) * [uint64]16777619
        $hash = [uint32]($product -band [uint64]4294967295)
    }
    return $hash
}

function Get-ExpectedPayload {
    [byte[]]$payload = New-Object byte[] 128
    [byte[]]$prefix = [Text.Encoding]::ASCII.GetBytes("DBYTEFS1`n")
    [Array]::Copy($prefix, 0, $payload, 0, $prefix.Length)
    $value = 40 + 2
    $payload[9] = [byte](48 + [Math]::Floor($value / 10))
    $payload[10] = [byte](48 + ($value % 10))
    $payload[11] = 10
    for ($index = 12; $index -lt 128; $index++) {
        $payload[$index] = [byte](($index * 31 + 0x5d) -band 0xff)
    }
    return $payload
}

function New-BlankDisk([string]$Path) {
    Assert-ProofPath $Path
    Assert-True (-not (Test-Path -LiteralPath $Path)) "refusing to overwrite existing proof disk: $Path"
    [IO.File]::WriteAllBytes($Path, (New-Object byte[] (8MB)))
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

function Inspect-Dbfs([string]$Disk, [string]$ReportPath) {
    Assert-ProofPath $Disk
    [byte[]]$bytes = [IO.File]::ReadAllBytes($Disk)
    Assert-True ($bytes.Length -eq 8MB) "DBFS proof disk is not exactly 8 MiB"
    for ($index = 0; $index -lt (64 * 512); $index++) {
        Assert-True ($bytes[$index] -eq 0) "reserved LBA 0..63 changed at byte $index"
    }
    $super = 64 * 512
    [byte[]]$magic = [Text.Encoding]::ASCII.GetBytes("DBFS1")
    for ($index = 0; $index -lt $magic.Length; $index++) {
        Assert-True ($bytes[$super + $index] -eq $magic[$index]) "DBFS1 magic mismatch"
    }
    foreach ($index in 5..7) { Assert-True ($bytes[$super + $index] -eq 0) "DBFS1 magic padding is nonzero" }
    Assert-True ((Read-U32 $bytes ($super + 8)) -eq 1) "DBFS1 version mismatch"
    Assert-True ((Read-U32 $bytes ($super + 12)) -eq 512) "DBFS1 sector size mismatch"
    Assert-True ((Read-U32 $bytes ($super + 16)) -eq 64) "DBFS1 base LBA mismatch"
    Assert-True ((Read-U32 $bytes ($super + 20)) -eq 16320) "DBFS1 filesystem sector count mismatch"
    Assert-True ((Read-U32 $bytes ($super + 24)) -eq 65) "DBFS1 bitmap start mismatch"
    Assert-True ((Read-U32 $bytes ($super + 28)) -eq 4) "DBFS1 bitmap sector count mismatch"
    Assert-True ((Read-U32 $bytes ($super + 32)) -eq 69) "DBFS1 inode start mismatch"
    Assert-True ((Read-U32 $bytes ($super + 36)) -eq 64) "DBFS1 inode count mismatch"
    Assert-True ((Read-U32 $bytes ($super + 40)) -eq 8) "DBFS1 inode sectors mismatch"
    Assert-True ((Read-U32 $bytes ($super + 44)) -eq 77) "DBFS1 data start mismatch"
    Assert-True ((Read-U32 $bytes ($super + 48)) -eq 16307) "DBFS1 data blocks mismatch"
    Assert-True ((Read-U32 $bytes ($super + 52)) -eq 1) "DBFS1 root inode mismatch"
    Assert-True ((Read-U32 $bytes ($super + 56)) -eq 64) "DBFS1 directory entry size mismatch"
    Assert-True ((Read-U32 $bytes ($super + 60)) -eq (Get-Fnv1a $bytes $super 60)) "DBFS1 checksum mismatch"
    for ($index = 64; $index -lt 512; $index++) { Assert-True ($bytes[$super + $index] -eq 0) "DBFS1 superblock tail is nonzero" }

    $inodeBase = 69 * 512
    $root = $inodeBase + 64
    Assert-True ((Read-U32 $bytes $root) -eq 1 -and (Read-U32 $bytes ($root + 4)) -eq 2) "root inode type/allocation mismatch"
    Assert-True ((Read-U32 $bytes ($root + 8)) -eq 4096 -and (Read-U32 $bytes ($root + 12)) -eq 8) "root inode geometry mismatch"
    for ($index = 0; $index -lt 8; $index++) { Assert-True ((Read-U32 $bytes ($root + 16 + 4 * $index)) -eq $index) "root direct block $index mismatch" }
    $file = $inodeBase + (2 * 64)
    Assert-True ((Read-U32 $bytes $file) -eq 1 -and (Read-U32 $bytes ($file + 4)) -eq 1) "persistent inode type/allocation mismatch"
    Assert-True ((Read-U32 $bytes ($file + 8)) -eq 128 -and (Read-U32 $bytes ($file + 12)) -eq 1) "persistent inode size/block count mismatch"
    Assert-True ((Read-U32 $bytes ($file + 16)) -eq 8) "persistent inode does not own data block 8"
    for ($index = 1; $index -lt 8; $index++) { Assert-True ((Read-U32 $bytes ($file + 16 + 4 * $index)) -eq [uint32]::MaxValue) "persistent unused direct block is not sentinel" }
    for ($inode = 3; $inode -lt 64; $inode++) {
        $offset = $inodeBase + ($inode * 64)
        for ($byte = 0; $byte -lt 64; $byte++) { Assert-True ($bytes[$offset + $byte] -eq 0) "inode $inode leaked state" }
    }

    $directory = 77 * 512
    Assert-True ((Read-U32 $bytes $directory) -eq 2) "root directory does not reference inode 2"
    Assert-True ((Read-U32 $bytes ($directory + 4)) -eq 1 -and (Read-U32 $bytes ($directory + 8)) -eq 11) "persistent directory entry fields mismatch"
    $name = [Text.Encoding]::ASCII.GetString($bytes, $directory + 12, 11)
    Assert-True ($name -ceq "PERSIST.TXT") "persistent filename mismatch"
    for ($entry = 1; $entry -lt 64; $entry++) {
        $offset = $directory + ($entry * 64)
        for ($byte = 0; $byte -lt 64; $byte++) { Assert-True ($bytes[$offset + $byte] -eq 0) "directory entry $entry leaked state" }
    }

    $bitmap = 65 * 512
    for ($block = 0; $block -lt 16307; $block++) {
        $set = ($bytes[$bitmap + [Math]::Floor($block / 8)] -band (1 -shl ($block -band 7))) -ne 0
        Assert-True ($set -eq ($block -le 8)) "bitmap ownership mismatch at data block $block"
    }
    [byte[]]$expected = Get-ExpectedPayload
    [byte[]]$actual = New-Object byte[] 128
    [Array]::Copy($bytes, 85 * 512, $actual, 0, 128)
    Assert-True ([Linq.Enumerable]::SequenceEqual($actual, $expected)) "persistent payload bytes mismatch"
    $payloadHash = ([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($actual))).Replace('-', '').ToLowerInvariant()
    Assert-True ($payloadHash -ceq "bec0c8787425f40f28a46208c995fcd0b3b800ac23820b50b309f3135d745c70") "persistent payload SHA-256 mismatch"
    $report = @(
        "magic=DBFS1", "version=1", "base_lba=64", "bitmap=65+4", "inode_table=69+8",
        "data_start=77", "data_blocks=16307", "root_inode=1", "directory_capacity=64",
        "persistent_inode=2", "persistent_block=8", "persistent_lba=85", "persistent_size=128",
        "persistent_sha256=$payloadHash", "allocated_blocks=0..8", "reserved_lbas_unchanged=yes"
    )
    [IO.File]::WriteAllLines($ReportPath, $report)
    return $payloadHash
}

function Invoke-QemuBoot([int]$Ram, [int]$Boot, [string]$Elf, [string]$Disk, [string]$Qemu, [string]$ExpectedSerial) {
    $runRoot = Join-Path $evidenceRoot "${Ram}m"
    New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
    $serial = Join-Path $runRoot "boot${Boot}.serial.log"
    $trace = Join-Path $runRoot "boot${Boot}.trace.log"
    $debug = Join-Path $runRoot "boot${Boot}.debugcon.log"
    $arguments = @(
        "-m", "${Ram}M", "-kernel", $Elf, "-display", "none", "-serial", "file:$serial",
        "-monitor", "none", "-no-reboot", "-no-shutdown", "-d", "int,cpu_reset,guest_errors", "-D", $trace,
        "-chardev", "file,id=dbyte_debugcon,path=$debug", "-device", "isa-debugcon,iobase=0xe9,chardev=dbyte_debugcon",
        "-drive", "file=$Disk,format=raw,if=ide,index=0,media=disk,cache=directsync,throttling.iops-total=25"
    )
    $commandPath = Join-Path $runRoot "boot${Boot}.command.txt"
    [IO.File]::WriteAllText($commandPath, "$Qemu $($arguments -join ' ')")
    $diskBefore = Join-Path $runRoot "boot${Boot}.disk.before"
    Copy-Item -LiteralPath $Disk -Destination $diskBefore -Force
    $matched = $false
    try {
        for ($attempt = 1; $attempt -le 3 -and -not $matched; $attempt++) {
            if ($attempt -gt 1) {
                Copy-Item -LiteralPath $diskBefore -Destination $Disk -Force
                Remove-Item -LiteralPath $serial, $trace, $debug -Force -ErrorAction SilentlyContinue
                Start-Sleep -Seconds 2
            }
            Start-Sleep -Milliseconds 500
            $process = Start-Process -FilePath $Qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
            $exitedEarly = $false
            try {
                $deadline = [DateTime]::UtcNow.AddSeconds(60)
                while ([DateTime]::UtcNow -lt $deadline -and -not $process.HasExited) {
                    Start-Sleep -Milliseconds 100
                    if (Test-Path -LiteralPath $serial) {
                        $current = [Text.Encoding]::ASCII.GetString((Read-SharedBytes $serial))
                        if ($current -ceq $ExpectedSerial) { $matched = $true; break }
                        Assert-True ($ExpectedSerial.StartsWith($current, [StringComparison]::Ordinal)) "QEMU ${Ram}M boot $Boot serial diverged:`n$current"
                    }
                }
                $exitedEarly = -not $matched -and $process.HasExited
            }
            finally {
                if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
            }
            if (-not $matched) {
                Assert-True (-not $exitedEarly) "QEMU ${Ram}M boot $Boot exited before exact serial acceptance"
                foreach ($path in @($serial, $trace, $debug)) {
                    if (Test-Path -LiteralPath $path) { Copy-Item -LiteralPath $path -Destination "$path.attempt$attempt" -Force }
                }
            }
        }
        Assert-True $matched "QEMU ${Ram}M boot $Boot did not reach exact serial acceptance"
    }
    finally {
        Remove-Item -LiteralPath $diskBefore -Force -ErrorAction SilentlyContinue
    }
    $actual = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($serial))
    Assert-True ($actual -ceq $ExpectedSerial) "QEMU ${Ram}M boot $Boot serial mismatch"
    if ($Boot -eq 2) { Assert-True (-not $actual.Contains("DBFS FORMAT ONLINE")) "boot 2 reformatted the filesystem" }
    $traceText = Get-Content -LiteralPath $trace -Raw
    Assert-True (-not $traceText.Contains("Triple fault")) "QEMU ${Ram}M boot $Boot triple faulted"
    Assert-True (([regex]::Matches($traceText, 'v=0e', 'IgnoreCase')).Count -eq 0) "QEMU ${Ram}M boot $Boot raised #PF"
    Assert-True (([regex]::Matches($traceText, 'v=0d', 'IgnoreCase')).Count -eq 0) "QEMU ${Ram}M boot $Boot raised #GP"
    $irq14 = ([regex]::Matches($traceText, 'Servicing hardware INT=0x2e')).Count
    $telemetry = Read-Telemetry $debug
    foreach ($key in @("BOOT_PHASE", "FS_READS", "FS_WRITES", "FS_RELEASED", "FS_REUSED", "DATA_START", "DATA_BLOCKS", "IRQ14", "IRQ14_ACCEPT", "IRQ14_SPURIOUS", "IRQ14_SLAVE_EOI", "IRQ14_MASTER_EOI", "ATA_POSTS", "READER_BLOCK", "READER_WAKE", "PROGRESS", "PROGRESS_BLOCKED", "PIC_MASTER", "PIC_SLAVE", "TICKS", "EOI", "TRAP_EOI", "LIVE", "REUSE_SLOT", "FRAME", "REUSED_FRAME")) {
        Assert-True ($telemetry.ContainsKey($key)) "QEMU ${Ram}M boot $Boot telemetry lacks $key"
    }
    $operations = $telemetry.FS_READS + $telemetry.FS_WRITES
    Assert-True ($irq14 -eq $operations -and $telemetry.IRQ14 -eq $operations -and $telemetry.IRQ14_ACCEPT -eq $operations) "filesystem I/O and IRQ14 counts differ"
    Assert-True ($telemetry.IRQ14_SPURIOUS -eq 0 -and $telemetry.ATA_POSTS -eq $operations) "IRQ14 completion accounting is wrong"
    Assert-True ($telemetry.IRQ14_SLAVE_EOI -eq $operations -and $telemetry.IRQ14_MASTER_EOI -eq $operations) "IRQ14 EOI accounting is wrong"
    Assert-True ($telemetry.READER_BLOCK -eq $operations -and $telemetry.READER_WAKE -eq $operations) "ATA blocking/wake accounting is wrong"
    Assert-True ($telemetry.PROGRESS -gt 0 -and $telemetry.PROGRESS_BLOCKED -gt 0) "scheduler did not progress during filesystem I/O"
    Assert-True ($telemetry.PIC_MASTER -eq 0xf8 -and $telemetry.PIC_SLAVE -eq 0xbf) "PIC masks changed"
    Assert-True ($telemetry.EOI -eq $telemetry.TICKS -and $telemetry.TRAP_EOI -eq 0) "IRQ0/software-trap EOI ownership is wrong"
    Assert-True ($telemetry.LIVE -eq 1 -and $telemetry.REUSE_SLOT -eq 1 -and $telemetry.FRAME -eq $telemetry.REUSED_FRAME) "task lifecycle did not reclaim/reuse resources"
    Assert-True ($telemetry.DATA_START -eq 77 -and $telemetry.DATA_BLOCKS -eq 16307) "guest DBFS layout telemetry is wrong"
    if ($Boot -eq 1) {
        Assert-True ($telemetry.BOOT_PHASE -eq 1 -and $telemetry.FS_RELEASED -eq 9 -and $telemetry.FS_REUSED -eq 9) "boot 1 block reuse proof failed"
    } else {
        Assert-True ($telemetry.BOOT_PHASE -eq 2 -and $telemetry.FS_WRITES -eq 0) "boot 2 mutated DBFS"
    }
    Start-Sleep -Milliseconds 500
    Assert-True ($null -eq (Get-Process -Id $process.Id -ErrorAction SilentlyContinue)) "QEMU PID $($process.Id) remains alive"
    return @{ Serial = $actual; Telemetry = $telemetry; Irq14 = $irq14 }
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
$elf = Join-Path $evidenceRoot "filesystem.elf"
$repeatElf = Join-Path $repeatRoot "filesystem.elf"
Invoke-Checked $compiler (@("link") + $objectsA + @("-o", $elf)) "link filesystem ELF" | Out-Null
Invoke-Checked $compiler (@("link") + $objectsB + @("-o", $repeatElf)) "repeat-link filesystem ELF" | Out-Null
Assert-SameFile $elf $repeatElf "linked filesystem ELF"
$integrated = Join-Path $evidenceRoot "filesystem.integrated.elf"
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "kernel.dby"), "--target", "i686-dbyte-none", "--emit", "elf", "-o", $integrated) "integrated filesystem ELF" | Out-Null
Assert-SameFile $elf $integrated "object-link and integrated filesystem ELF"
$bin = Join-Path $evidenceRoot "filesystem.bin"
$repeatBin = Join-Path $repeatRoot "filesystem.bin"
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "kernel.dby"), "--target", "i686-dbyte-none", "--emit", "bin", "-o", $bin) "build filesystem BIN" | Out-Null
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "kernel.dby"), "--target", "i686-dbyte-none", "--emit", "bin", "-o", $repeatBin) "repeat filesystem BIN" | Out-Null
Assert-SameFile $bin $repeatBin "filesystem BIN"

$finalInspection = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", $elf) "inspect final filesystem ELF"
[IO.File]::WriteAllText((Join-Path $evidenceRoot "disasm.txt"), $finalInspection)
Assert-True (-not $finalInspection.Contains(".dbyte.tasks") -and -not $finalInspection.Contains(".dbyte.interrupts") -and -not $finalInspection.Contains(".dbyte.entry")) "final ELF retained metadata"
Assert-True (-not $finalInspection.Contains(".interp") -and -not $finalInspection.Contains(".dynamic")) "final ELF has a runtime dependency"
foreach ($needle in @("<dbfs_format>", "<dbfs_mount>", "<dbfs_create>", "<dbfs_write>", "<dbfs_read>", "<dbfs_delete>", "<ata_irq14>", "<semaphore_wait>", "<semaphore_post>", "in     ax,dx", "out    dx,ax", "pusha", "popa", "iret")) {
    Assert-True ($finalInspection.Contains($needle)) "final disassembly lacks $needle"
}

$fsSources = @("dbfs_layout", "dbfs_bitmap", "dbfs_inode", "dbfs_dir", "dbfs") | ForEach-Object { Get-Content -LiteralPath (Join-Path $sourceRoot "$_.dby") -Raw }
$fsText = $fsSources -join "`n"
Assert-True ($fsText.Contains("ata.ata_read_sector") -and $fsText.Contains("ata.ata_write_sector")) "DBFS does not use the DByte block API"
foreach ($forbidden in @("arch.in", "arch.out", "native.io_", "ata_irq14", "while true", "global_asm", "nasm", "gcc", "clang", "rustc", "llvm")) {
    Assert-True (-not $fsText.Contains($forbidden)) "DBFS source contains forbidden token '$forbidden'"
}
$allSource = ($modules | ForEach-Object { Get-Content -LiteralPath (Join-Path $sourceRoot "$_.dby") -Raw }) -join "`n"
Assert-True (-not $allSource.Contains('"42"')) "persistent proof embeds forbidden string literal 42"

$blankHashes = @()
$boot1Hashes = @()
$boot2Hashes = @()
$payloadHashes = @()
$runs = @()
foreach ($ram in @(32, 64)) {
    $runRoot = Join-Path $evidenceRoot "${ram}m"
    New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
    $disk = Join-Path $runRoot "disk.raw"
    New-BlankDisk $disk
    $blankHashes += (Get-FileHash $disk -Algorithm SHA256).Hash
    $boot1 = Invoke-QemuBoot $ram 1 $elf $disk $qemu $boot1Serial
    $boot1Hashes += (Get-FileHash $disk -Algorithm SHA256).Hash
    $payloadHashes += Inspect-Dbfs $disk (Join-Path $runRoot "boot1.host-inspection.txt")
    $boot2 = Invoke-QemuBoot $ram 2 $elf $disk $qemu $boot2Serial
    $boot2Hashes += (Get-FileHash $disk -Algorithm SHA256).Hash
    $payloadHashes += Inspect-Dbfs $disk (Join-Path $runRoot "boot2.host-inspection.txt")
    Assert-True ($boot1Hashes[-1] -eq $boot2Hashes[-1]) "${ram}M boot 2 changed the persistent disk"
    Assert-True ($blankHashes[-1] -ne $boot1Hashes[-1]) "${ram}M boot 1 did not change the blank disk"
    $runs += @{ Ram = $ram; Boot1 = $boot1; Boot2 = $boot2 }
}
Assert-True ($blankHashes[0] -eq $blankHashes[1]) "blank disks are not deterministic"
Assert-True ($boot1Hashes[0] -eq $boot1Hashes[1]) "32/64 MiB boot 1 DBFS bytes differ"
Assert-True ($boot2Hashes[0] -eq $boot2Hashes[1]) "32/64 MiB boot 2 DBFS bytes differ"
Assert-True (($payloadHashes | Select-Object -Unique).Count -eq 1) "persistent payload hashes differ"

$hashLines = @()
foreach ($file in Get-ChildItem -LiteralPath $evidenceRoot -Filter "*.o") { $hashLines += "$($file.Name) $((Get-FileHash $file.FullName -Algorithm SHA256).Hash)" }
$elfHash = (Get-FileHash $elf -Algorithm SHA256).Hash
$binHash = (Get-FileHash $bin -Algorithm SHA256).Hash
$hashLines += "ELF $elfHash", "BIN $binHash", "BLANK_DISK $($blankHashes[0])", "BOOT1_DISK $($boot1Hashes[0])", "BOOT2_DISK $($boot2Hashes[0])", "PERSISTENT_PAYLOAD $($payloadHashes[0])"
[IO.File]::WriteAllLines((Join-Path $evidenceRoot "sha256.txt"), $hashLines)
[IO.File]::WriteAllLines((Join-Path $evidenceRoot "relocations.txt"), @("R_386_PC32=$pc32", "R_386_32=$abs32"))
[IO.File]::WriteAllLines((Join-Path $evidenceRoot "verification-report.txt"), @(
    "DBYTE NATIVE PERSISTENT FILESYSTEM: PASS", "version=$MilestoneVersion", "objects=$($modules.Count)",
    "R_386_PC32=$pc32", "R_386_32=$abs32", "elf_sha256=$elfHash", "bin_sha256=$binHash",
    "blank_disk_sha256=$($blankHashes[0])", "boot1_disk_sha256=$($boot1Hashes[0])", "boot2_disk_sha256=$($boot2Hashes[0])",
    "persistent_sha256=$($payloadHashes[0])", "layout=super64 bitmap65+4 inode69+8 data77+16307", "released_block=9", "reused_block=9"
))

$remaining = @(Get-Process -Name "qemu-system-i386" -ErrorAction SilentlyContinue | Where-Object { $_.Id -notin $initialQemuIds })
Assert-True ($remaining.Count -eq 0) "filesystem verifier left QEMU processes: $($remaining.Id -join ', ')"
Write-Host "DBYTE NATIVE PERSISTENT FILESYSTEM: PASS"
Write-Host "Objects: $($modules.Count); R_386_PC32=$pc32; R_386_32=$abs32"
Write-Host "ELF SHA-256: $elfHash"
Write-Host "BIN SHA-256: $binHash"
Write-Host "Disk SHA-256: blank=$($blankHashes[0]) boot1=$($boot1Hashes[0]) boot2=$($boot2Hashes[0])"
Write-Host "Persistent payload SHA-256: $($payloadHashes[0])"
Write-Host "DBFS1: super=64 bitmap=65+4 inode=69+8 data=77+16307 root=1 file=2 block=8"
