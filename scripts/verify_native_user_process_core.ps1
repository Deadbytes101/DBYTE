param([string]$MilestoneVersion = "11.11.0")

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$tmpRoot = (Resolve-Path (Join-Path $repoRoot "tmp")).Path
$evidenceRoot = Join-Path $tmpRoot "native_v$MilestoneVersion"
$repeatRoot = Join-Path $tmpRoot "native_v$MilestoneVersion.repeat"
$sourceRoot = Join-Path $repoRoot "examples\native_user_process_core"
$programRoot = Join-Path $sourceRoot "programs"
$initialQemuIds = @(Get-Process -Name "qemu-system-i386" -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$modules = @(
    "arch_x86", "ata", "block_cache", "boot", "byte_ring", "dbfs", "dbfs_bitmap",
    "dbfs_dir", "dbfs_inode", "dbfs_layout", "file_handles", "filesystem_runtime",
    "frames", "gdt", "heap", "idt_normal", "keyboard", "memory_ops",
    "paging", "path", "pic", "pit", "process_context", "processes", "proof",
    "scheduler", "serial", "sha256", "tasks", "user_bundle", "kernel"
)
$programs = @("hello", "hostile", "isoa", "isob", "pagefault", "survive")
$bundleNames = @{
    hello = "HELLO.ELF"; hostile = "HOSTILE.ELF"; isoa = "ISOA.ELF";
    isob = "ISOB.ELF"; pagefault = "PAGEFAULT.ELF"; survive = "SURVIVE.ELF"
}
$expectedSerial = "DBYTEOS USER MODE`r`nTSS ONLINE`r`nUSER ADDRESS SPACE ONLINE`r`nELF LOADER ONLINE`r`nRING3 ONLINE`r`nSYSCALL ABI ONLINE`r`nHELLO FROM DBYTE USER`r`nCONFIG VALUE 42`r`nUSER FS ONLINE`r`nUSER ISOLATION OK`r`nUSER #GP TERMINATED`r`nUSER #PF TERMINATED`r`nPROCESS EXIT OK`r`nPROCESS RECLAIM OK`r`nKERNEL SURVIVED`r`nUSERMODE CHECK OK`r`n"

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
    $allowedA = [IO.Path]::GetFullPath($evidenceRoot).TrimEnd('\') + '\'
    $allowedB = [IO.Path]::GetFullPath($repeatRoot).TrimEnd('\') + '\'
    Assert-True ($full.StartsWith($allowedA, [StringComparison]::OrdinalIgnoreCase) -or $full.StartsWith($allowedB, [StringComparison]::OrdinalIgnoreCase) -or $full -eq $evidenceRoot -or $full -eq $repeatRoot) "evidence path escapes canonical roots: $full"
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

function Read-U16([byte[]]$Bytes, [int]$Offset) { return [BitConverter]::ToUInt16($Bytes, $Offset) }
function Read-U32([byte[]]$Bytes, [int]$Offset) { return [BitConverter]::ToUInt32($Bytes, $Offset) }

function Assert-UserElf([string]$Path) {
    [byte[]]$bytes = [IO.File]::ReadAllBytes($Path)
    Assert-True ($bytes.Length -le 4096 -and $bytes.Length -ge 116) "$Path exceeds compact ELF bound"
    Assert-True ($bytes[0] -eq 0x7f -and [Text.Encoding]::ASCII.GetString($bytes, 1, 3) -ceq "ELF") "$Path lacks ELF magic"
    Assert-True ($bytes[4] -eq 1 -and $bytes[5] -eq 1 -and (Read-U16 $bytes 16) -eq 2 -and (Read-U16 $bytes 18) -eq 3) "$Path is not ELF32/i386 ET_EXEC"
    Assert-True ((Read-U32 $bytes 28) -eq 52 -and (Read-U32 $bytes 32) -eq 0 -and (Read-U16 $bytes 44) -eq 2 -and (Read-U16 $bytes 48) -eq 0) "$Path has non-canonical ELF tables"
    $entry = Read-U32 $bytes 24
    foreach ($index in 0..1) {
        $header = 52 + 32 * $index
        $offset = Read-U32 $bytes ($header + 4)
        $vaddr = Read-U32 $bytes ($header + 8)
        $filesz = Read-U32 $bytes ($header + 16)
        $memsz = Read-U32 $bytes ($header + 20)
        $flags = Read-U32 $bytes ($header + 24)
        Assert-True ((Read-U32 $bytes $header) -eq 1 -and (Read-U32 $bytes ($header + 12)) -eq $vaddr -and (Read-U32 $bytes ($header + 28)) -eq 1) "$Path has an invalid PT_LOAD"
        Assert-True ($filesz -le $memsz -and ([uint64]$offset + $filesz) -le $bytes.Length -and ([uint64]$vaddr + $memsz) -le 0x00800000) "$Path has an out-of-range segment"
        Assert-True ($flags -eq $(if ($index -eq 0) { 5 } else { 6 })) "$Path has wrong segment permissions"
        if ($index -eq 0) { Assert-True ($entry -ge $vaddr -and $entry -lt ($vaddr + $memsz)) "$Path entry is outside RX" }
    }
}

function Patch-Hostile([string]$InputPath, [string]$OutputPath) {
    [byte[]]$bytes = [IO.File]::ReadAllBytes($InputPath)
    $offset = -1
    for ($index = 0; $index -lt $bytes.Length - 1; $index++) {
        if ($bytes[$index] -eq 0xcd -and $bytes[$index + 1] -eq 0x80) { $offset = $index; break }
    }
    Assert-True ($offset -ge 0) "hostile fixture source ELF lacks INT 80"
    $bytes[$offset] = 0xee
    $bytes[$offset + 1] = 0x90
    [IO.File]::WriteAllBytes($OutputPath, $bytes)
    return $offset
}

function New-BlankDisk([string]$Path) {
    Assert-ProofPath $Path
    Assert-True (-not (Test-Path -LiteralPath $Path)) "refusing to overwrite proof disk $Path"
    $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write)
    try { $stream.SetLength(8MB) } finally { $stream.Dispose() }
}

function Read-SharedBytes([string]$Path) {
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
    try {
        [byte[]]$bytes = New-Object byte[] $stream.Length
        [void]$stream.Read($bytes, 0, $bytes.Length)
        return ,$bytes
    } finally { $stream.Dispose() }
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

function Invoke-UserBoot([int]$Ram, [string]$Kernel, [string]$Bundle, [string]$Disk, [string]$Qemu) {
    $runRoot = Split-Path -Parent $Disk
    $serial = Join-Path $runRoot "serial.log"
    $debug = Join-Path $runRoot "debugcon.log"
    $trace = Join-Path $runRoot "qemu.trace.log"
    $stderr = Join-Path $runRoot "qemu.stderr.log"
    $arguments = @("-kernel", $Kernel, "-initrd", $Bundle, "-drive", "file=$Disk,format=raw,if=ide,index=0,media=disk,cache=directsync", "-m", "${Ram}M", "-display", "none", "-serial", "file:$serial", "-debugcon", "file:$debug", "-global", "isa-debugcon.iobase=0xe9", "-d", "int,cpu_reset,guest_errors", "-D", $trace, "-no-reboot", "-no-shutdown")
    [IO.File]::WriteAllText((Join-Path $runRoot "qemu-command.txt"), "$Qemu $($arguments -join ' ')")
    $process = Start-Process -FilePath $Qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden -RedirectStandardError $stderr
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(22)
        $matched = $false
        while ([DateTime]::UtcNow -lt $deadline -and -not $process.HasExited) {
            Start-Sleep -Milliseconds 100
            if (Test-Path -LiteralPath $serial) {
                $current = [Text.Encoding]::ASCII.GetString((Read-SharedBytes $serial))
                if ($current -ceq $expectedSerial) { $matched = $true; break }
                Assert-True ($expectedSerial.StartsWith($current, [StringComparison]::Ordinal)) "${Ram}M serial diverged"
            }
        }
        Assert-True $matched "${Ram}M did not reach user-mode acceptance"
    } finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
    }
    $traceText = Get-Content -LiteralPath $trace -Raw
    Assert-True (([regex]::Matches($traceText, 'v=80')).Count -eq 19) "${Ram}M syscall count is not 19"
    Assert-True (([regex]::Matches($traceText, 'v=0d')).Count -eq 1) "${Ram}M #GP count is not one"
    Assert-True (([regex]::Matches($traceText, 'v=0e')).Count -eq 1) "${Ram}M #PF count is not one"
    Assert-True ($traceText -match 'v=0d e=0000.*cpl=3.*IP=001b') "${Ram}M #GP is not a CPL3 error-zero fault"
    Assert-True ($traceText -match 'v=0e e=0007.*cpl=3.*IP=001b.*CR2=c1000000') "${Ram}M #PF contract mismatch"
    Assert-True ($traceText.Contains('TR =0028') -and $traceText.Contains('SP=0023:')) "${Ram}M lacks TSS/Ring3 trace"
    Assert-True (-not $traceText.Contains('Triple fault')) "${Ram}M triple faulted"
    $telemetry = Read-Telemetry $debug
    foreach ($pair in @{ SYSCALLS=19; WORKER_REQ=9; PROCESS_FAULTS=2; PROCESS_REAP=6; FD_CLEANUP=2; SLOT0_GEN=4 }.GetEnumerator()) {
        Assert-True ($telemetry[$pair.Key] -eq $pair.Value) "${Ram}M telemetry $($pair.Key) mismatch"
    }
    Assert-True ($telemetry.ISO_FRAME_A -ne 0 -and $telemetry.ISO_FRAME_A -ne $telemetry.ISO_FRAME_B) "${Ram}M user text frames alias"
    Assert-True ($telemetry.SCRATCH_MAP -eq $telemetry.SCRATCH_UNMAP -and $telemetry.SCRATCH_MAP -gt 0) "${Ram}M scratch mappings leaked"
    Assert-True ($telemetry.IRQ0_TICKS -eq $telemetry.IRQ0_EOI) "${Ram}M IRQ0 EOI mismatch"
    Assert-True ($telemetry.ATA_IRQ -eq $telemetry.ATA_SLAVE_EOI -and $telemetry.ATA_IRQ -eq $telemetry.ATA_MASTER_EOI) "${Ram}M IRQ14 EOI mismatch"
    Assert-True (([regex]::Matches($traceText, 'v=2e')).Count -eq $telemetry.ATA_IRQ) "${Ram}M IRQ14 trace count mismatch"
    return $telemetry
}

function Inspect-InstalledPrograms([string]$Disk, [hashtable]$Expected, [string]$Report) {
    [byte[]]$bytes = [IO.File]::ReadAllBytes($Disk)
    Assert-True ($bytes.Length -eq 8MB) "DBFS disk size mismatch"
    for ($index = 0; $index -lt 64 * 512; $index++) { Assert-True ($bytes[$index] -eq 0) "reserved disk prefix changed" }
    $super = 64 * 512
    Assert-True ([Text.Encoding]::ASCII.GetString($bytes, $super, 5) -ceq 'DBFS1') "DBFS1 magic mismatch"
    Assert-True ((Read-U32 $bytes ($super + 8)) -eq 1 -and (Read-U32 $bytes ($super + 44)) -eq 77) "DBFS1 layout mismatch"
    $inodeBase = 69 * 512
    function Get-Inode([uint32]$Number) {
        $offset = $inodeBase + 64 * $Number
        $blocks = @()
        foreach ($i in 0..7) { $blocks += Read-U32 $bytes ($offset + 16 + 4 * $i) }
        return @{ Allocated = Read-U32 $bytes $offset; Type = Read-U32 $bytes ($offset + 4); Size = Read-U32 $bytes ($offset + 8); Count = Read-U32 $bytes ($offset + 12); Blocks = $blocks }
    }
    function Find-Child([uint32]$Directory, [string]$Name) {
        $inode = Get-Inode $Directory
        foreach ($block in $inode.Blocks[0..($inode.Count - 1)]) {
            $sector = (77 + $block) * 512
            foreach ($entry in 0..7) {
                $offset = $sector + 64 * $entry
                $number = Read-U32 $bytes $offset
                if ($number -ne 0) {
                    $length = Read-U32 $bytes ($offset + 8)
                    if ([Text.Encoding]::ASCII.GetString($bytes, $offset + 12, $length) -ceq $Name) { return $number }
                }
            }
        }
        throw "missing DBFS child $Name"
    }
    function Read-File([uint32]$Number) {
        $inode = Get-Inode $Number
        [byte[]]$result = New-Object byte[] $inode.Size
        $written = 0
        foreach ($block in $inode.Blocks[0..($inode.Count - 1)]) {
            $amount = [Math]::Min(512, $result.Length - $written)
            [Array]::Copy($bytes, (77 + $block) * 512, $result, $written, $amount)
            $written += $amount
        }
        return ,$result
    }
    $system = Find-Child 1 'SYSTEM'
    $bin = Find-Child $system 'BIN'
    $lines = @("SYSTEM=$system", "BIN=$bin")
    foreach ($name in ($Expected.Keys | Sort-Object)) {
        $inode = Find-Child $bin $name
        [byte[]]$actual = Read-File $inode
        [byte[]]$wanted = [IO.File]::ReadAllBytes($Expected[$name])
        Assert-True ([Linq.Enumerable]::SequenceEqual($actual, $wanted)) "installed $name differs from compiler payload"
        $lines += "$name inode=$inode sha256=$((Get-FileHash $Expected[$name] -Algorithm SHA256).Hash)"
    }
    $config = Find-Child $system 'CONFIG.TXT'
    [byte[]]$configBytes = Read-File $config
    Assert-True ([Text.Encoding]::ASCII.GetString($configBytes) -ceq "VALUE=42`n") "installed config mismatch"
    [IO.File]::WriteAllLines($Report, $lines)
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
    Assert-True (-not $inspection.Contains('.dbyte.profile')) "$module unexpectedly has user profile metadata"
    $pc32 += ([regex]::Matches($inspection, 'R_386_PC32')).Count
    $abs32 += ([regex]::Matches($inspection, 'R_386_32')).Count
}

$userElfs = @{}
$repeatUserElfs = @{}
$userSyscallCount = 0
foreach ($program in $programs) {
    $source = Join-Path $programRoot "$program.dby"
    $object = Join-Path $evidenceRoot "$program.user.o"
    $repeatObject = Join-Path $repeatRoot "$program.user.o"
    $elf = Join-Path $evidenceRoot "$program.elf"
    $repeatElf = Join-Path $repeatRoot "$program.elf"
    Invoke-Checked $compiler @("build", $source, "--target", "i686-dbyte-user", "--emit", "obj", "-o", $object) "build $program user object" | Out-Null
    Invoke-Checked $compiler @("build", $source, "--target", "i686-dbyte-user", "--emit", "obj", "-o", $repeatObject) "repeat $program user object" | Out-Null
    Invoke-Checked $compiler @("build", $source, "--target", "i686-dbyte-user", "--emit", "elf", "-o", $elf) "build $program user ELF" | Out-Null
    Invoke-Checked $compiler @("build", $source, "--target", "i686-dbyte-user", "--emit", "elf", "-o", $repeatElf) "repeat $program user ELF" | Out-Null
    Assert-SameFile $object $repeatObject "$program user object"
    Assert-SameFile $elf $repeatElf "$program user ELF"
    Assert-UserElf $elf
    $inspection = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", $object) "inspect $program user object"
    [IO.File]::WriteAllText("$object.objdump.txt", $inspection)
    Assert-True ($inspection.Contains('.dbyte.profile') -and $inspection.Contains('__dbyte_profile:user')) "$program lacks user metadata"
    $userSyscallCount += ([regex]::Matches($inspection, 'int\s+0x80')).Count
    $userElfs[$bundleNames[$program]] = $elf
    $repeatUserElfs[$bundleNames[$program]] = $repeatElf
}
Assert-True ($userSyscallCount -gt 0) "user object disassembly lacks INT 80"

$hostileOffset = Patch-Hostile $userElfs['HOSTILE.ELF'] (Join-Path $evidenceRoot 'hostile-fixture.elf')
$repeatHostileOffset = Patch-Hostile $repeatUserElfs['HOSTILE.ELF'] (Join-Path $repeatRoot 'hostile-fixture.elf')
Assert-True ($hostileOffset -eq $repeatHostileOffset) "hostile patch offset is not deterministic"
$userElfs['HOSTILE.ELF'] = Join-Path $evidenceRoot 'hostile-fixture.elf'
$repeatUserElfs['HOSTILE.ELF'] = Join-Path $repeatRoot 'hostile-fixture.elf'
Assert-SameFile $userElfs['HOSTILE.ELF'] $repeatUserElfs['HOSTILE.ELF'] "hostile fixture"

$bundle = Join-Path $evidenceRoot 'programs.dbup'
$repeatBundle = Join-Path $repeatRoot 'programs.dbup'
$bundleArgs = @('pack-user') + @($userElfs.Keys | Sort-Object | ForEach-Object { "$_=$($userElfs[$_])" }) + @('-o', $bundle)
$repeatBundleArgs = @('pack-user') + @($repeatUserElfs.Keys | Sort-Object | ForEach-Object { "$_=$($repeatUserElfs[$_])" }) + @('-o', $repeatBundle)
Invoke-Checked $compiler $bundleArgs "pack DBUP1" | Out-Null
Invoke-Checked $compiler $repeatBundleArgs "repeat DBUP1" | Out-Null
Assert-SameFile $bundle $repeatBundle "DBUP1 bundle"

$objectsA = @($modules | ForEach-Object { Join-Path $evidenceRoot "$_.o" })
$objectsB = @($modules | ForEach-Object { Join-Path $repeatRoot "$_.o" })
$kernelElf = Join-Path $evidenceRoot 'kernel.elf'
$repeatKernelElf = Join-Path $repeatRoot 'kernel.elf'
Invoke-Checked $compiler (@('link') + $objectsA + @('-o', $kernelElf)) "link kernel ELF" | Out-Null
Invoke-Checked $compiler (@('link') + $objectsB + @('-o', $repeatKernelElf)) "repeat-link kernel ELF" | Out-Null
Assert-SameFile $kernelElf $repeatKernelElf "kernel ELF"
$integrated = Join-Path $evidenceRoot 'kernel.integrated.elf'
Invoke-Checked $compiler @('build', (Join-Path $sourceRoot 'kernel.dby'), '--target', 'i686-dbyte-none', '--emit', 'elf', '-o', $integrated) "integrated kernel ELF" | Out-Null
Assert-SameFile $kernelElf $integrated "object-link and integrated kernel ELF"
$kernelBin = Join-Path $evidenceRoot 'kernel.bin'
$repeatKernelBin = Join-Path $repeatRoot 'kernel.bin'
Invoke-Checked $compiler @('build', (Join-Path $sourceRoot 'kernel.dby'), '--target', 'i686-dbyte-none', '--emit', 'bin', '-o', $kernelBin) "build kernel BIN" | Out-Null
Invoke-Checked $compiler @('build', (Join-Path $sourceRoot 'kernel.dby'), '--target', 'i686-dbyte-none', '--emit', 'bin', '-o', $repeatKernelBin) "repeat kernel BIN" | Out-Null
Assert-SameFile $kernelBin $repeatKernelBin "kernel BIN"

$finalInspection = Invoke-Checked $objdump @('-f', '-h', '-t', '-r', '-d', '-Mintel', $kernelElf) "inspect kernel ELF"
[IO.File]::WriteAllText((Join-Path $evidenceRoot 'kernel.objdump.txt'), $finalInspection)
foreach ($needle in @('<syscall>', '<syscall_worker>', '<general_protection>', '<page_fault>', '<activate_process_task>', '<scratch_map>', 'ltr', 'mov    cr3', 'pusha', 'popa', 'iret')) {
    Assert-True ($finalInspection.Contains($needle)) "kernel disassembly lacks $needle"
}
Assert-True (-not $finalInspection.Contains('.interp') -and -not $finalInspection.Contains('.dynamic')) "kernel has dynamic runtime state"
Assert-True (-not $finalInspection.Contains('.dbyte.profile') -and -not $finalInspection.Contains('.dbyte.interrupts')) "kernel retained non-loadable metadata"

$userSource = ($programs | ForEach-Object { Get-Content -LiteralPath (Join-Path $programRoot "$_.dby") -Raw }) -join "`n"
foreach ($forbidden in @('native.', 'interrupt(', 'interrupt_address', 'task_address', '0xc1000000 as usize', '"42"')) { Assert-True (-not $userSource.Contains($forbidden)) "user source contains forbidden token $forbidden" }
$processText = Get-Content -LiteralPath (Join-Path $sourceRoot 'processes.dby') -Raw
$handler = [regex]::Match($processText, 'pub interrupt\(no_error_code, cpl3_transition\) fn syscall[\s\S]*?pub interrupt\(error_code, cpl3_transition\) fn general_protection').Value
foreach ($forbidden in @('fs.fs_', 'ata.', 'heap.', 'serial.', 'pic.eoi')) { Assert-True (-not $handler.Contains($forbidden)) "INT80 fast path contains $forbidden" }
Assert-True ($handler.Contains('enqueue_request') -and $handler.Contains('block_current_syscall')) "INT80 does not enqueue and block"
Assert-True ($processText.Contains('paging.scratch_map') -and -not $processText.Contains('native.cpu_write_cr3(CR3[slot])')) "user copy bypasses scratch mapping"

$diskHashes = @()
$telemetryRuns = @()
foreach ($ram in @(32, 64)) {
    $runRoot = Join-Path $evidenceRoot "${ram}m"
    New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
    $disk = Join-Path $runRoot 'disk.raw'
    New-BlankDisk $disk
    $telemetryRuns += Invoke-UserBoot $ram $kernelElf $bundle $disk $qemu
    Inspect-InstalledPrograms $disk $userElfs (Join-Path $runRoot 'host-inspection.txt')
    $diskHashes += (Get-FileHash $disk -Algorithm SHA256).Hash
}
Assert-True ($diskHashes[0] -eq $diskHashes[1]) "32/64 MiB DBFS disks differ"

$hashLines = @()
foreach ($file in Get-ChildItem -LiteralPath $evidenceRoot -File | Where-Object { $_.Extension -in @('.o', '.elf', '.dbup', '.bin') }) {
    $hashLines += "$($file.Name) $((Get-FileHash $file.FullName -Algorithm SHA256).Hash)"
}
$elfHash = (Get-FileHash $kernelElf -Algorithm SHA256).Hash
$binHash = (Get-FileHash $kernelBin -Algorithm SHA256).Hash
$bundleHash = (Get-FileHash $bundle -Algorithm SHA256).Hash
$hashLines += "DISK $($diskHashes[0])"
[IO.File]::WriteAllLines((Join-Path $evidenceRoot 'sha256.txt'), $hashLines)
[IO.File]::WriteAllLines((Join-Path $evidenceRoot 'relocations.txt'), @("R_386_PC32=$pc32", "R_386_32=$abs32"))
[IO.File]::WriteAllLines((Join-Path $evidenceRoot 'verification-report.txt'), @(
    'DBYTE NATIVE USER PROCESS: PASS', "version=$MilestoneVersion", "kernel_objects=$($modules.Count)", "user_programs=$($programs.Count)",
    "R_386_PC32=$pc32", "R_386_32=$abs32", "syscalls=19", "worker_requests=9", "faults=2", "reclaims=6", "fd_cleanups=2",
    "kernel_elf_sha256=$elfHash", "kernel_bin_sha256=$binHash", "dbup_sha256=$bundleHash", "disk_sha256=$($diskHashes[0])", "hostile_patch_offset=$hostileOffset"
))

$remaining = @(Get-Process -Name "qemu-system-i386" -ErrorAction SilentlyContinue | Where-Object { $initialQemuIds -notcontains $_.Id })
Assert-True ($remaining.Count -eq 0) "user-process verifier left QEMU processes: $($remaining.Id -join ', ')"
Write-Host "DBYTE NATIVE USER PROCESS: PASS"
Write-Host "Kernel objects: $($modules.Count); user programs: $($programs.Count); R_386_PC32=$pc32; R_386_32=$abs32"
Write-Host "Kernel ELF SHA-256: $elfHash"
Write-Host "Kernel BIN SHA-256: $binHash"
Write-Host "DBUP1 SHA-256: $bundleHash"
Write-Host "Disk SHA-256: $($diskHashes[0])"
