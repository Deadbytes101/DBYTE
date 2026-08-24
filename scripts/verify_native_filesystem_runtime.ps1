param([string]$MilestoneVersion = "11.10.0")

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$tmpRoot = (Resolve-Path (Join-Path $repoRoot "tmp")).Path
$evidenceRoot = Join-Path $tmpRoot "native_v$MilestoneVersion"
$repeatRoot = Join-Path $tmpRoot "native_v$MilestoneVersion.repeat"
$sourceRoot = Join-Path $repoRoot "examples\native_filesystem_runtime"
$initialQemuIds = @(Get-Process -Name "qemu-system-i386" -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$modules = @(
    "arch_x86", "serial", "gdt", "boot", "frames", "paging", "memory_ops", "heap",
    "pic", "proof", "page_fault", "byte_ring", "file_handles", "scheduler", "keyboard", "ata",
    "dbfs_layout", "dbfs_bitmap", "dbfs_inode", "dbfs_dir", "block_cache", "path", "dbfs",
    "filesystem_runtime", "tasks", "pit",
    "idt_normal", "kernel"
)
$boot1Serial = "DBYTEOS NATIVE FS RUNTIME`r`nNATIVE OPS ONLINE`r`nPATH RESOLVER ONLINE`r`nFILE HANDLES ONLINE`r`nDIRECTORIES ONLINE`r`nBLOCK CACHE ONLINE`r`nKERNEL CONFIG ONLINE`r`nKERNEL LOG ONLINE`r`nBOOT STATE 1 OK`r`nFILESYSTEM RUNTIME CHECK OK`r`n"
$boot2Serial = "DBYTEOS NATIVE FS RUNTIME`r`nDBFS MOUNT ONLINE`r`nCONFIG READ OK`r`nHANDLE SEEK OK`r`nCACHE HIT OK`r`nLOG APPEND OK`r`nBOOT STATE 2 OK`r`nFILESYSTEM RUNTIME CHECK OK`r`n"
$boot3Serial = "DBYTEOS NATIVE FS RUNTIME`r`nDBFS MOUNT ONLINE`r`nCONFIG PERSIST OK`r`nDIRECTORY TREE OK`r`nHANDLE GENERATION OK`r`nBOOT LOG HISTORY OK`r`nBOOT STATE 3 OK`r`nFILESYSTEM RUNTIME CHECK OK`r`n"

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
    $prefix = [IO.Path]::GetFullPath($tmpRoot).TrimEnd('\') + '\native_v11.10.0'
    Assert-True ($full.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) "proof disk/evidence path escapes tmp/native_v11.10.0: $full"
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

function Inspect-Dbfs([string]$Disk, [int]$ExpectedBoot, [string]$ReportPath) {
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
    function Read-Inode([int]$Number) {
        $offset = $inodeBase + (64 * $Number)
        $blocks = @()
        for ($index = 0; $index -lt 8; $index++) { $blocks += Read-U32 $bytes ($offset + 16 + 4 * $index) }
        return @{ Number = $Number; Allocated = Read-U32 $bytes $offset; Kind = Read-U32 $bytes ($offset + 4); Size = Read-U32 $bytes ($offset + 8); Count = Read-U32 $bytes ($offset + 12); Blocks = $blocks }
    }
    function Read-Directory([hashtable]$Inode) {
        Assert-True ($Inode.Kind -eq 2 -and $Inode.Size -eq 4096 -and $Inode.Count -eq 8) "directory inode $($Inode.Number) geometry mismatch"
        $entries = @{}
        for ($sector = 0; $sector -lt 8; $sector++) {
            $base = (77 + $Inode.Blocks[$sector]) * 512
            for ($entry = 0; $entry -lt 8; $entry++) {
                $offset = $base + 64 * $entry
                $number = Read-U32 $bytes $offset
                if ($number -ne 0) {
                    $kind = Read-U32 $bytes ($offset + 4)
                    $length = Read-U32 $bytes ($offset + 8)
                    Assert-True ($length -ge 1 -and $length -le 31) "directory entry name length is invalid"
                    $name = [Text.Encoding]::ASCII.GetString($bytes, $offset + 12, $length)
                    Assert-True (-not $entries.ContainsKey($name)) "duplicate directory name $name"
                    $entries[$name] = @{ Inode = $number; Kind = $kind }
                }
            }
        }
        return $entries
    }
    $inodes = @{}
    for ($number = 0; $number -lt 64; $number++) {
        $inode = Read-Inode $number
        if ($inode.Allocated -eq 1) { $inodes[$number] = $inode }
        elseif ($inode.Allocated -ne 0) { throw "inode $number has noncanonical allocation state" }
    }
    Assert-True (($inodes.Keys | Sort-Object) -join ',' -ceq '1,2,3,4,5,6') "allocated inode set is not exactly 1..6"
    $root = $inodes[1]; $system = $inodes[2]; $log = $inodes[3]
    for ($index = 0; $index -lt 8; $index++) {
        Assert-True ($root.Blocks[$index] -eq $index) "root block mismatch"
        Assert-True ($system.Blocks[$index] -eq 8 + $index) "SYSTEM block mismatch"
        Assert-True ($log.Blocks[$index] -eq 16 + $index) "LOG block mismatch"
    }
    $rootEntries = Read-Directory $root
    $systemEntries = Read-Directory $system
    $logEntries = Read-Directory $log
    Assert-True ($rootEntries.Count -eq 2 -and $rootEntries.SYSTEM.Inode -eq 2 -and $rootEntries.SYSTEM.Kind -eq 2 -and $rootEntries.LOG.Inode -eq 3 -and $rootEntries.LOG.Kind -eq 2) "root hierarchy mismatch"
    Assert-True ($systemEntries.Count -eq 2 -and $systemEntries.'CONFIG.TXT'.Inode -eq 4 -and $systemEntries.'BOOT.CNT'.Inode -eq 5) "SYSTEM hierarchy mismatch"
    Assert-True ($logEntries.Count -eq 1 -and $logEntries.'BOOT.LOG'.Inode -eq 6) "LOG hierarchy mismatch"
    Assert-True ($inodes[4].Kind -eq 1 -and $inodes[4].Size -eq 48 -and $inodes[4].Blocks[0] -eq 24) "CONFIG inode mismatch"
    Assert-True ($inodes[5].Kind -eq 1 -and $inodes[5].Size -eq 4 -and $inodes[5].Blocks[0] -eq 25) "BOOT.CNT inode mismatch"
    Assert-True ($inodes[6].Kind -eq 1 -and $inodes[6].Size -eq (13 * $ExpectedBoot) -and $inodes[6].Blocks[0] -eq 26) "BOOT.LOG inode mismatch"
    [byte[]]$expectedConfig = [Text.Encoding]::ASCII.GetBytes("DBYTEOS=1`nSCHEDULER=ROUND_ROBIN`nIO=IRQ`nVALUE=$((40 + 2))`n")
    [byte[]]$actualConfig = New-Object byte[] 48
    [Array]::Copy($bytes, (77 + 24) * 512, $actualConfig, 0, 48)
    Assert-True ([Linq.Enumerable]::SequenceEqual($actualConfig, $expectedConfig)) "CONFIG.TXT bytes mismatch"
    Assert-True ((Read-U32 $bytes ((77 + 25) * 512)) -eq $ExpectedBoot) "BOOT.CNT value mismatch"
    $expectedLog = (1..$ExpectedBoot | ForEach-Object { "BOOT {0:D4} OK`n" -f $_ }) -join ''
    $actualLog = [Text.Encoding]::ASCII.GetString($bytes, (77 + 26) * 512, 13 * $ExpectedBoot)
    Assert-True ($actualLog -ceq $expectedLog) "BOOT.LOG history mismatch"
    $bitmap = 65 * 512
    for ($block = 0; $block -lt 16307; $block++) {
        $set = ($bytes[$bitmap + [Math]::Floor($block / 8)] -band (1 -shl ($block -band 7))) -ne 0
        Assert-True ($set -eq ($block -le 26)) "bitmap ownership mismatch at data block $block"
    }
    $configHash = ([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash($actualConfig))).Replace('-', '').ToLowerInvariant()
    $logHash = ([BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::ASCII.GetBytes($actualLog)))).Replace('-', '').ToLowerInvariant()
    $report = @(
        "magic=DBFS1", "version=1", "root=1", "system=2", "log=3", "config=4", "counter=5", "boot_log=6",
        "allocated_blocks=0..26", "boot_count=$ExpectedBoot", "config_sha256=$configHash", "boot_log_sha256=$logHash", "reserved_lbas_unchanged=yes"
    )
    [IO.File]::WriteAllLines($ReportPath, $report)
    return "$configHash/$logHash"
}

function Invoke-QemuBoot([int]$Ram, [int]$Boot, [string]$Elf, [string]$Disk, [string]$Qemu, [string]$ExpectedSerial) {
    $runRoot = Join-Path $evidenceRoot "${Ram}m"
    New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
    $serial = Join-Path $runRoot "boot${Boot}.serial.log"
    $trace = Join-Path $runRoot "boot${Boot}.trace.log"
    $debug = Join-Path $runRoot "boot${Boot}.debugcon.log"
    $stderr = Join-Path $runRoot "boot${Boot}.stderr.log"
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
                Remove-Item -LiteralPath $serial, $trace, $debug, $stderr -Force -ErrorAction SilentlyContinue
                Start-Sleep -Seconds 2
            }
            Start-Sleep -Milliseconds 500
            $process = Start-Process -FilePath $Qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden -RedirectStandardError $stderr
            $exitedEarly = $false
            $exitCode = $null
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
                if ($exitedEarly) { $process.WaitForExit(); $exitCode = $process.ExitCode }
            }
            finally {
                if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
            }
            if (-not $matched) {
                $stderrText = if (Test-Path -LiteralPath $stderr) { Get-Content -LiteralPath $stderr -Raw } else { "<missing>" }
                if ($exitedEarly) { throw "QEMU ${Ram}M boot $Boot exited with code $exitCode before acceptance; stderr: $stderrText" }
                foreach ($path in @($serial, $trace, $debug, $stderr)) {
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
    if ($Boot -ge 2) { Assert-True (-not $actual.Contains("DBFS FORMAT ONLINE")) "boot $Boot reformatted the filesystem" }
    $traceText = Get-Content -LiteralPath $trace -Raw
    Assert-True (-not $traceText.Contains("Triple fault")) "QEMU ${Ram}M boot $Boot triple faulted"
    Assert-True (([regex]::Matches($traceText, 'v=0e', 'IgnoreCase')).Count -eq 0) "QEMU ${Ram}M boot $Boot raised #PF"
    Assert-True (([regex]::Matches($traceText, 'v=0d', 'IgnoreCase')).Count -eq 0) "QEMU ${Ram}M boot $Boot raised #GP"
    Assert-True (([regex]::Matches($traceText, 'v=00', 'IgnoreCase')).Count -eq 0) "QEMU ${Ram}M boot $Boot raised #DE"
    $irq14 = ([regex]::Matches($traceText, 'Servicing hardware INT=0x2e')).Count
    $telemetry = Read-Telemetry $debug
    foreach ($key in @("BOOT_PHASE", "CACHE_HITS", "CACHE_MISSES", "CACHE_REPLACE", "CACHE_ATA_READ", "CACHE_ATA_WRITE", "FS_OPENS", "FS_CLOSES", "FS_READ_OPS", "FS_WRITE_OPS", "FS_SEEKS", "HANDLE_CLEANUP", "IRQ14", "IRQ14_ACCEPT", "IRQ14_SPURIOUS", "IRQ14_SLAVE_EOI", "IRQ14_MASTER_EOI", "ATA_POSTS", "READER_BLOCK", "READER_WAKE", "PROGRESS", "PROGRESS_BLOCKED", "PIC_MASTER", "PIC_SLAVE", "CLEANUP_PROVEN", "TICKS", "EOI", "TRAP_EOI", "LIVE", "FRAME", "REUSED_FRAME")) {
        Assert-True ($telemetry.ContainsKey($key)) "QEMU ${Ram}M boot $Boot telemetry lacks $key"
    }
    $operations = $telemetry.CACHE_ATA_READ + $telemetry.CACHE_ATA_WRITE
    Assert-True ($irq14 -eq $operations -and $telemetry.IRQ14 -eq $operations -and $telemetry.IRQ14_ACCEPT -eq $operations) "filesystem I/O and IRQ14 counts differ"
    Assert-True ($telemetry.IRQ14_SPURIOUS -eq 0 -and $telemetry.ATA_POSTS -eq $operations) "IRQ14 completion accounting is wrong"
    Assert-True ($telemetry.IRQ14_SLAVE_EOI -eq $operations -and $telemetry.IRQ14_MASTER_EOI -eq $operations) "IRQ14 EOI accounting is wrong"
    Assert-True ($telemetry.READER_BLOCK -eq $operations -and $telemetry.READER_WAKE -eq $operations) "ATA blocking/wake accounting is wrong"
    Assert-True ($telemetry.PROGRESS -gt 0 -and $telemetry.PROGRESS_BLOCKED -gt 0) "scheduler did not progress during filesystem I/O"
    Assert-True ($telemetry.PIC_MASTER -eq 0xf8 -and $telemetry.PIC_SLAVE -eq 0xbf) "PIC masks changed"
    Assert-True ($telemetry.EOI -eq $telemetry.TICKS -and $telemetry.TRAP_EOI -eq 0) "IRQ0/software-trap EOI ownership is wrong"
    Assert-True ($telemetry.LIVE -eq 1 -and $telemetry.FRAME -eq $telemetry.REUSED_FRAME) "task lifecycle did not reclaim/reuse resources"
    Assert-True ($telemetry.BOOT_PHASE -eq $Boot) "guest boot phase telemetry mismatch"
    Assert-True ($telemetry.CACHE_HITS -gt 0 -and $telemetry.CACHE_MISSES -gt 0 -and $telemetry.CACHE_REPLACE -gt 0) "cache hit/miss/replacement proof failed"
    Assert-True ($telemetry.HANDLE_CLEANUP -eq 1 -and $telemetry.CLEANUP_PROVEN -eq 1) "task-exit handle cleanup failed"
    Start-Sleep -Milliseconds 500
    Assert-True ($null -eq (Get-Process -Id $process.Id -ErrorAction SilentlyContinue)) "QEMU PID $($process.Id) remains alive"
    return @{ Serial = $actual; Telemetry = $telemetry; Irq14 = $irq14 }
}

function Invoke-DivideFault([string]$Name, [string]$Elf, [string]$ExpectedSerial, [string]$Qemu) {
    $serial = Join-Path $evidenceRoot "$Name.serial.log"
    $trace = Join-Path $evidenceRoot "$Name.trace.log"
    $arguments = @("-m", "32M", "-kernel", $Elf, "-display", "none", "-serial", "file:$serial", "-monitor", "none", "-no-reboot", "-no-shutdown", "-d", "int,cpu_reset,guest_errors", "-D", $trace)
    [IO.File]::WriteAllText((Join-Path $evidenceRoot "$Name.command.txt"), "$Qemu $($arguments -join ' ')")
    $process = Start-Process -FilePath $Qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(15)
        $matched = $false
        while ([DateTime]::UtcNow -lt $deadline -and -not $process.HasExited) {
            Start-Sleep -Milliseconds 100
            if (Test-Path -LiteralPath $serial) {
                $current = [Text.Encoding]::ASCII.GetString((Read-SharedBytes $serial))
                if ($current -ceq $ExpectedSerial) { $matched = $true; break }
                Assert-True ($ExpectedSerial.StartsWith($current, [StringComparison]::Ordinal)) "$Name serial diverged"
            }
        }
        Assert-True $matched "$Name did not reach controlled #DE acceptance"
    }
    finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
    }
    $traceText = Get-Content -LiteralPath $trace -Raw
    Assert-True (([regex]::Matches($traceText, 'v=00', 'IgnoreCase')).Count -eq 1) "$Name did not raise exactly one #DE"
    Assert-True (-not $traceText.Contains("Triple fault")) "$Name triple faulted"
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

$divideZeroElf = Join-Path $evidenceRoot "divide-zero.elf"
$divideZeroRepeat = Join-Path $repeatRoot "divide-zero.elf"
$divideOverflowElf = Join-Path $evidenceRoot "divide-overflow.elf"
$divideOverflowRepeat = Join-Path $repeatRoot "divide-overflow.elf"
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "divide_zero_kernel.dby"), "--target", "i686-dbyte-none", "--emit", "elf", "-o", $divideZeroElf) "build divide-zero proof" | Out-Null
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "divide_zero_kernel.dby"), "--target", "i686-dbyte-none", "--emit", "elf", "-o", $divideZeroRepeat) "repeat divide-zero proof" | Out-Null
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "divide_overflow_kernel.dby"), "--target", "i686-dbyte-none", "--emit", "elf", "-o", $divideOverflowElf) "build divide-overflow proof" | Out-Null
Invoke-Checked $compiler @("build", (Join-Path $sourceRoot "divide_overflow_kernel.dby"), "--target", "i686-dbyte-none", "--emit", "elf", "-o", $divideOverflowRepeat) "repeat divide-overflow proof" | Out-Null
Assert-SameFile $divideZeroElf $divideZeroRepeat "divide-zero ELF"
Assert-SameFile $divideOverflowElf $divideOverflowRepeat "divide-overflow ELF"
Invoke-DivideFault "divide-zero" $divideZeroElf "DBYTE NATIVE DIVIDE FAULT`r`nDIVIDE ZERO #DE OK`r`n" $qemu
Invoke-DivideFault "divide-overflow" $divideOverflowElf "DBYTE NATIVE DIVIDE FAULT`r`nDIVIDE OVERFLOW #DE OK`r`n" $qemu

$finalInspection = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", $elf) "inspect final filesystem ELF"
[IO.File]::WriteAllText((Join-Path $evidenceRoot "disasm.txt"), $finalInspection)
Assert-True (-not $finalInspection.Contains(".dbyte.tasks") -and -not $finalInspection.Contains(".dbyte.interrupts") -and -not $finalInspection.Contains(".dbyte.entry")) "final ELF retained metadata"
Assert-True (-not $finalInspection.Contains(".interp") -and -not $finalInspection.Contains(".dynamic")) "final ELF has a runtime dependency"
foreach ($needle in @("<dbfs_format>", "<dbfs_mount>", "<fs_mkdir>", "<fs_open>", "<fs_handle_read>", "<fs_handle_write>", "<fs_seek>", "<cache_read_sector>", "<ata_irq14>", "<semaphore_wait>", "<semaphore_post>", "in     ax,dx", "out    dx,ax", "cdq", "idiv", "div", "pusha", "popa", "iret")) {
    Assert-True ($finalInspection.Contains($needle)) "final disassembly lacks $needle"
}

$fsSources = @("dbfs_layout", "dbfs_bitmap", "dbfs_inode", "dbfs_dir", "path", "file_handles", "block_cache", "dbfs", "filesystem_runtime") | ForEach-Object { Get-Content -LiteralPath (Join-Path $sourceRoot "$_.dby") -Raw }
$fsText = $fsSources -join "`n"
Assert-True ($fsText.Contains("cache.cache_read_sector") -and $fsText.Contains("cache.cache_write_sector")) "DBFS does not route through the cache"
Assert-True ((Get-Content -LiteralPath (Join-Path $sourceRoot "block_cache.dby") -Raw).Contains("lba as usize) % CACHE_ENTRIES")) "cache does not execute native modulo"
Assert-True ($fsText.Contains("(*entry).valid") -and $fsText.Contains("(*handle).generation") -and $fsText.Contains("(*cursor).component_offset")) "runtime does not use typed pointer aggregate fields"
$dbfsText = Get-Content -LiteralPath (Join-Path $sourceRoot "dbfs.dby") -Raw
Assert-True (-not $dbfsText.Contains("ata.ata_read_sector") -and -not $dbfsText.Contains("ata.ata_write_sector")) "DBFS bypasses the cache"
foreach ($forbidden in @("arch.in", "arch.out", "native.io_", "ata_irq14", "while true", "global_asm", "nasm", "gcc", "clang", "rustc", "llvm")) {
    Assert-True (-not $fsText.Contains($forbidden)) "DBFS source contains forbidden token '$forbidden'"
}
$allSource = ($modules | ForEach-Object { Get-Content -LiteralPath (Join-Path $sourceRoot "$_.dby") -Raw }) -join "`n"
Assert-True (-not $allSource.Contains('"42"')) "persistent proof embeds forbidden string literal 42"

$blankHashes = @()
$boot1Hashes = @()
$boot2Hashes = @()
$boot3Hashes = @()
$state1 = @()
$state2 = @()
$state3 = @()
$runs = @()
foreach ($ram in @(32, 64)) {
    $runRoot = Join-Path $evidenceRoot "${ram}m"
    New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
    $disk = Join-Path $runRoot "disk.raw"
    New-BlankDisk $disk
    $blankHashes += (Get-FileHash $disk -Algorithm SHA256).Hash
    $boot1 = Invoke-QemuBoot $ram 1 $elf $disk $qemu $boot1Serial
    $boot1Hashes += (Get-FileHash $disk -Algorithm SHA256).Hash
    $state1 += Inspect-Dbfs $disk 1 (Join-Path $runRoot "boot1.host-inspection.txt")
    $boot2 = Invoke-QemuBoot $ram 2 $elf $disk $qemu $boot2Serial
    $boot2Hashes += (Get-FileHash $disk -Algorithm SHA256).Hash
    $state2 += Inspect-Dbfs $disk 2 (Join-Path $runRoot "boot2.host-inspection.txt")
    $boot3 = Invoke-QemuBoot $ram 3 $elf $disk $qemu $boot3Serial
    $boot3Hashes += (Get-FileHash $disk -Algorithm SHA256).Hash
    $state3 += Inspect-Dbfs $disk 3 (Join-Path $runRoot "boot3.host-inspection.txt")
    Assert-True ($boot1Hashes[-1] -ne $boot2Hashes[-1] -and $boot2Hashes[-1] -ne $boot3Hashes[-1]) "${ram}M persistent boot state did not change"
    Assert-True ($blankHashes[-1] -ne $boot1Hashes[-1]) "${ram}M boot 1 did not change the blank disk"
    $runs += @{ Ram = $ram; Boot1 = $boot1; Boot2 = $boot2; Boot3 = $boot3 }
}
Assert-True ($blankHashes[0] -eq $blankHashes[1]) "blank disks are not deterministic"
Assert-True ($boot1Hashes[0] -eq $boot1Hashes[1]) "32/64 MiB boot 1 DBFS bytes differ"
Assert-True ($boot2Hashes[0] -eq $boot2Hashes[1]) "32/64 MiB boot 2 DBFS bytes differ"
Assert-True ($boot3Hashes[0] -eq $boot3Hashes[1]) "32/64 MiB boot 3 DBFS bytes differ"
Assert-True (($state1 | Select-Object -Unique).Count -eq 1 -and ($state2 | Select-Object -Unique).Count -eq 1 -and ($state3 | Select-Object -Unique).Count -eq 1) "host inspection differs by RAM matrix"

$hashLines = @()
foreach ($file in Get-ChildItem -LiteralPath $evidenceRoot -Filter "*.o") { $hashLines += "$($file.Name) $((Get-FileHash $file.FullName -Algorithm SHA256).Hash)" }
$elfHash = (Get-FileHash $elf -Algorithm SHA256).Hash
$binHash = (Get-FileHash $bin -Algorithm SHA256).Hash
$hashLines += "ELF $elfHash", "BIN $binHash", "BLANK_DISK $($blankHashes[0])", "BOOT1_DISK $($boot1Hashes[0])", "BOOT2_DISK $($boot2Hashes[0])", "BOOT3_DISK $($boot3Hashes[0])", "FINAL_STATE $($state3[0])"
[IO.File]::WriteAllLines((Join-Path $evidenceRoot "sha256.txt"), $hashLines)
[IO.File]::WriteAllLines((Join-Path $evidenceRoot "relocations.txt"), @("R_386_PC32=$pc32", "R_386_32=$abs32"))
[IO.File]::WriteAllLines((Join-Path $evidenceRoot "verification-report.txt"), @(
    "DBYTE NATIVE FILESYSTEM RUNTIME: PASS", "version=$MilestoneVersion", "objects=$($modules.Count)",
    "R_386_PC32=$pc32", "R_386_32=$abs32", "elf_sha256=$elfHash", "bin_sha256=$binHash",
    "blank_disk_sha256=$($blankHashes[0])", "boot1_disk_sha256=$($boot1Hashes[0])", "boot2_disk_sha256=$($boot2Hashes[0])", "boot3_disk_sha256=$($boot3Hashes[0])",
    "final_state=$($state3[0])", "layout=DBFS1 super64 bitmap65+4 inode69+8 data77+16307", "tree=/SYSTEM,/LOG", "boot_count=3"
))

$remaining = @(Get-Process -Name "qemu-system-i386" -ErrorAction SilentlyContinue | Where-Object { $_.Id -notin $initialQemuIds })
Assert-True ($remaining.Count -eq 0) "filesystem verifier left QEMU processes: $($remaining.Id -join ', ')"
Write-Host "DBYTE NATIVE FILESYSTEM RUNTIME: PASS"
Write-Host "Objects: $($modules.Count); R_386_PC32=$pc32; R_386_32=$abs32"
Write-Host "ELF SHA-256: $elfHash"
Write-Host "BIN SHA-256: $binHash"
Write-Host "Disk SHA-256: blank=$($blankHashes[0]) boot1=$($boot1Hashes[0]) boot2=$($boot2Hashes[0]) boot3=$($boot3Hashes[0])"
Write-Host "Final state: $($state3[0])"
Write-Host "DBFS1: root=1 SYSTEM=2 LOG=3 CONFIG=4 COUNTER=5 BOOT.LOG=6 blocks=0..26"
