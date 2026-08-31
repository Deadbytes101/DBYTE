param([switch]$Quick)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$proofRoot = Join-Path $repoRoot "tmp\vm_v12.3.0\collections"
$compiler = Join-Path $repoRoot "target\debug\dbytec.exe"
$qemu = "C:\Program Files\qemu\qemu-system-x86_64.exe"
$qemuI686 = "C:\Program Files\qemu\qemu-system-i386.exe"
$expected = "DBYTE COLLECTIONS V12.3`r`nHASHMAP ONLINE`r`nHASHSET ONLINE`r`nDEQUE ONLINE`r`nQUEUE ONLINE`r`nBUFFER ONLINE`r`n6502 ARRAY BUFFER MATCH`r`nCOLLECTIONS CHECK OK`r`n"
$expectedCaptured = $expected.Replace("`r`n", "`n").TrimEnd("`n")
$initialQemu = @(Get-Process qemu-system-x86_64 -ErrorAction SilentlyContinue | ForEach-Object Id)
$initialQemuI686 = @(Get-Process qemu-system-i386 -ErrorAction SilentlyContinue | ForEach-Object Id)

& (Join-Path $PSScriptRoot "verify_vm_architecture.ps1") -SkipTests
if ($LASTEXITCODE -ne 0) { throw "VM_COLLECTIONS_ARCHITECTURE_GATE_FAILED" }

function Assert-True($value, [string]$message) { if (-not $value) { throw $message } }
function Invoke-Checked([string]$file, [string[]]$arguments, [string]$description) {
    $saved = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    try { $output = & $file @arguments 2>&1; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    if ($code -ne 0) { throw "$description failed ($code):`n$($output -join "`n")" }
    return ($output -join "`n")
}
function Assert-Same([string]$left, [string]$right, [string]$name) {
    $a = (Get-FileHash -LiteralPath $left -Algorithm SHA256).Hash.ToLowerInvariant()
    $b = (Get-FileHash -LiteralPath $right -Algorithm SHA256).Hash.ToLowerInvariant()
    Assert-True ($a -ceq $b) "$name is not deterministic"
    return $a
}
function Read-SharedText([string]$path) {
    $stream = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
    try {
        $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::ASCII, $false, 1024, $true)
        try { return $reader.ReadToEnd() } finally { $reader.Dispose() }
    } finally { $stream.Dispose() }
}
function Wsl-Path([string]$path) {
    $portable = $path.Replace('\', '/')
    return (Invoke-Checked "wsl.exe" @("-d", "Debian", "--", "wslpath", "-a", "-u", $portable) "translate WSL path").Trim()
}
function Build-Target([string]$root, [string]$target, [string]$source, [string[]]$emits) {
    New-Item -ItemType Directory -Path $root -Force | Out-Null
    foreach ($emit in $emits) {
        $extension = switch ($emit) { "staticlib" { "a" } "exe" { "exe" } "obj" { "o" } default { $emit } }
        $arguments = @("build", $source, "--target", $target, "--emit", $emit, "-o", (Join-Path $root "collections.$extension"))
        if ($emit -eq "obj") { $arguments += @("--dump-ir", "--dump-machine-code") }
        Invoke-Checked $compiler $arguments "build $target $emit" | Set-Content (Join-Path $root "$emit-build.txt")
    }
}
function Run-None([int]$ram, [string]$elf) {
    $runRoot = Join-Path $proofRoot "none-${ram}m"
    $isoRoot = Join-Path $runRoot "iso"
    New-Item -ItemType Directory -Path (Join-Path $isoRoot "boot\grub") -Force | Out-Null
    Copy-Item -LiteralPath $elf -Destination (Join-Path $isoRoot "boot\kernel.elf") -Force
    [IO.File]::WriteAllText((Join-Path $isoRoot "boot\grub\grub.cfg"), "set timeout=0`nset default=0`nmenuentry 'DByte v12.3 collections' {`n multiboot2 /boot/kernel.elf`n boot`n}`n")
    $iso = Join-Path $runRoot "collections.iso"
    Invoke-Checked "wsl.exe" @("-d", "Debian", "--", "grub-mkrescue", "-o", (Wsl-Path $iso), (Wsl-Path $isoRoot)) "create collections GRUB ISO" | Set-Content (Join-Path $runRoot "grub.txt")
    $serial = Join-Path $runRoot "serial.log"; $trace = Join-Path $runRoot "qemu.log"
    $arguments = @("-cdrom", $iso, "-m", "${ram}M", "-display", "none", "-monitor", "none", "-serial", "file:$serial", "-d", "int,cpu_reset,guest_errors", "-D", $trace, "-no-reboot", "-no-shutdown")
    [IO.File]::WriteAllText((Join-Path $runRoot "qemu-command.txt"), "$qemu $($arguments -join ' ')")
    $process = Start-Process -FilePath $qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(30); $complete = $false
        do {
            Start-Sleep -Milliseconds 100
            if (Test-Path $serial) {
                $actual = Read-SharedText $serial
                Assert-True ($expected.StartsWith($actual, [StringComparison]::Ordinal)) "${ram}M collections serial diverged"
                if ($actual -ceq $expected) { $complete = $true; break }
            }
        } while (-not $process.HasExited -and [DateTime]::UtcNow -lt $deadline)
        Assert-True $complete "${ram}M collections proof timed out"
    } finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
    }
    $traceText = [IO.File]::ReadAllText($trace)
    Assert-True (-not $traceText.Contains("Triple fault")) "${ram}M collections triple fault"
    Assert-True (([regex]::Matches($traceText, 'v=0[0de6]')).Count -eq 0) "${ram}M collections CPU fault"
}
function Run-I686([int]$ram, [string]$elf) {
    $runRoot = Join-Path $proofRoot "i686-${ram}m"
    New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
    $serial = Join-Path $runRoot "serial.log"; $trace = Join-Path $runRoot "qemu.log"; $debug = Join-Path $runRoot "debugcon.log"
    $arguments = @("-kernel", $elf, "-m", "${ram}M", "-display", "none", "-monitor", "none", "-serial", "file:$serial", "-debugcon", "file:$debug", "-global", "isa-debugcon.iobase=0xe9", "-d", "int,cpu_reset,guest_errors", "-D", $trace, "-no-reboot", "-no-shutdown")
    [IO.File]::WriteAllText((Join-Path $runRoot "qemu-command.txt"), "$qemuI686 $($arguments -join ' ')")
    $process = Start-Process -FilePath $qemuI686 -ArgumentList $arguments -PassThru -WindowStyle Hidden
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(30); $complete = $false
        do {
            Start-Sleep -Milliseconds 100
            if (Test-Path $serial) {
                $actual = Read-SharedText $serial
                Assert-True ($expected.StartsWith($actual, [StringComparison]::Ordinal) -or $actual.StartsWith($expected, [StringComparison]::Ordinal)) "i686 ${ram}M collections serial diverged"
                if ($actual.StartsWith($expected, [StringComparison]::Ordinal)) { $complete = $true; break }
            }
        } while (-not $process.HasExited -and [DateTime]::UtcNow -lt $deadline)
        Assert-True $complete "i686 ${ram}M collections proof timed out"
    } finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
    }
    $traceText = [IO.File]::ReadAllText($trace)
    Assert-True (-not $traceText.Contains("Triple fault")) "i686 ${ram}M collections triple fault"
    Assert-True (([regex]::Matches($traceText, 'v=0[0de6]')).Count -eq 0) "i686 ${ram}M collections CPU fault"
}

Push-Location $repoRoot
try {
    if (Test-Path $proofRoot) { Remove-Item -LiteralPath $proofRoot -Recurse -Force }
    New-Item -ItemType Directory -Path $proofRoot -Force | Out-Null
    Invoke-Checked "cargo.exe" @("build", "-q", "-p", "dbyte_cli", "--bin", "dbytec") "build dbytec" | Out-Null

    $targets = @(
        @{ Name="i686"; Triple="i686-dbyte-none"; Source="examples\native_v12_3_collections\none.dby"; Emits=@("obj","staticlib","elf","bin") },
        @{ Name="none"; Triple="x86_64-dbyte-none"; Source="examples\native_v12_3_collections\none.dby"; Emits=@("obj","staticlib","elf","bin") },
        @{ Name="linux"; Triple="x86_64-dbyte-linux"; Source="examples\native_v12_3_collections\linux.dby"; Emits=@("obj","staticlib","elf") },
        @{ Name="windows"; Triple="x86_64-dbyte-windows"; Source="examples\native_v12_3_collections\windows.dby"; Emits=@("obj","staticlib","exe") }
    )
    $receipts = @()
    foreach ($target in $targets) {
        $first = Join-Path $proofRoot "$($target.Name)-first"; $second = Join-Path $proofRoot "$($target.Name)-second"
        Build-Target $first $target.Triple $target.Source $target.Emits
        Build-Target $second $target.Triple $target.Source $target.Emits
        foreach ($emit in $target.Emits) {
            $extension = switch ($emit) { "staticlib" { "a" } "exe" { "exe" } "obj" { "o" } default { $emit } }
            $hash = Assert-Same (Join-Path $first "collections.$extension") (Join-Path $second "collections.$extension") "$($target.Name) $emit"
            $receipts += "$hash  $($target.Name)/collections.$extension"
        }
        $dump = Invoke-Checked "objdump.exe" @("-f", "-h", "-t", "-r", "-d", "-Mintel", (Join-Path $first "collections.o")) "inspect $($target.Name) object"
        if ($target.Name -eq "i686") {
            Assert-True (-not $dump.Contains(".dbyte.target")) "i686 object changed the frozen metadata contract"
        } else {
            Assert-True ($dump.Contains(".dbyte.target")) "$($target.Name) object lacks target metadata"
        }
        $hasRelocations = if ($target.Name -eq "i686") {
            $dump.Contains("R_386_")
        } elseif ($target.Name -eq "windows") {
            $dump.Contains("IMAGE_REL_AMD64") -or $dump.Contains("DISP32") -or $dump.Contains("ADDR64")
        } else {
            $dump.Contains("R_X86_64_")
        }
        Assert-True $hasRelocations "$($target.Name) object lacks real relocations"
        Assert-True ($dump.Contains("call")) "$($target.Name) object lacks callback/vtable call instructions"
        [IO.File]::WriteAllText((Join-Path $first "objdump.txt"), $dump)
    }

    $linuxElf = Join-Path $proofRoot "linux-first\collections.elf"
    $linuxOutput = Invoke-Checked "wsl.exe" @("-d", "Debian", "--", (Wsl-Path $linuxElf)) "execute Linux collections"
    Assert-True ($linuxOutput -ceq $expectedCaptured) "Linux collections output diverged"
    $readelf = Invoke-Checked "wsl.exe" @("-d", "Debian", "--", "readelf", "-h", "-l", "-r", (Wsl-Path $linuxElf)) "inspect Linux collections ELF"
    Assert-True (-not $readelf.Contains("INTERP")) "Linux collections ELF has PT_INTERP"
    Assert-True ($readelf.Contains("There are no relocations")) "Linux collections ELF retains relocations"
    [IO.File]::WriteAllText((Join-Path $proofRoot "linux-first\readelf.txt"), $readelf)

    $windowsExe = Join-Path $proofRoot "windows-first\collections.exe"
    $windowsOutput = Invoke-Checked $windowsExe @() "execute Windows collections"
    Assert-True ($windowsOutput -ceq $expectedCaptured) "Windows collections output diverged"
    $pe = Invoke-Checked "C:\Program Files\LLVM\bin\llvm-readobj.exe" @("--file-headers", "--sections", "--coff-imports", $windowsExe) "inspect Windows collections PE"
    Assert-True ($pe.Contains("KERNEL32.dll")) "Windows collections PE lacks canonical Kernel32 import"
    Assert-True (-not $pe.Contains("msvcrt") -and -not $pe.Contains("ucrt")) "Windows collections PE imports a CRT"
    [IO.File]::WriteAllText((Join-Path $proofRoot "windows-first\llvm-readobj.txt"), $pe)

    $noneElf = Join-Path $proofRoot "none-first\collections.elf"
    Invoke-Checked "wsl.exe" @("-d", "Debian", "--", "grub-file", "--is-x86-multiboot2", (Wsl-Path $noneElf)) "validate collections Multiboot2" | Out-Null
    Run-None 32 $noneElf
    if (-not $Quick) { Run-None 64 $noneElf }
    $i686Elf = Join-Path $proofRoot "i686-first\collections.elf"
    Run-I686 32 $i686Elf
    if (-not $Quick) { Run-I686 64 $i686Elf }

    $sourceText = @(
        [IO.File]::ReadAllText((Join-Path $repoRoot "runtime\v12.3.0\hash_map.dby")),
        [IO.File]::ReadAllText((Join-Path $repoRoot "runtime\v12.3.0\hash_set.dby")),
        [IO.File]::ReadAllText((Join-Path $repoRoot "runtime\v12.3.0\deque.dby")),
        [IO.File]::ReadAllText((Join-Path $repoRoot "runtime\v12.3.0\queue.dby")),
        [IO.File]::ReadAllText((Join-Path $repoRoot "runtime\v12.3.0\buffer.dby"))
    ) -join "`n"
    Assert-True ($sourceText.Contains("fn(borrow K) -> u64")) "HashMap lacks typed hash callback"
    Assert-True ($sourceText.Contains("drop fn destroy")) "collections lack resolved Drop glue"
    $runtimeEscapes = @(Get-ChildItem (Join-Path $repoRoot "runtime\v12.3.0") -Filter *.dby | Select-String -Pattern "__dbyte_runtime")
    Assert-True ($runtimeEscapes.Count -eq 2) "collections expose unexpected raw runtime capabilities"
    Assert-True (($runtimeEscapes | Where-Object {
        $_.Path -notlike "*buffer.dby" -or $_.Line -notmatch "(mut_)?slice_from_raw"
    }).Count -eq 0) "raw runtime capability escaped the Buffer view boundary"
    [IO.File]::WriteAllLines((Join-Path $proofRoot "SHA256SUMS"), $receipts)
    "DBYTE VM COLLECTIONS VERIFIER: PASS"
    $receipts
} finally {
    Pop-Location
    $after = @(Get-Process qemu-system-x86_64 -ErrorAction SilentlyContinue | ForEach-Object Id)
    $orphans = @($after | Where-Object { $_ -notin $initialQemu })
    if ($orphans.Count -ne 0) { throw "collections verifier left QEMU processes: $($orphans -join ', ')" }
    $afterI686 = @(Get-Process qemu-system-i386 -ErrorAction SilentlyContinue | ForEach-Object Id)
    $orphansI686 = @($afterI686 | Where-Object { $_ -notin $initialQemuI686 })
    if ($orphansI686.Count -ne 0) { throw "collections verifier left i686 QEMU processes: $($orphansI686 -join ', ')" }
}
