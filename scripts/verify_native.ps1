param(
    [string]$MilestoneVersion = "11.1.0",
    [string]$WorkspaceVersion = ""
)

if ([string]::IsNullOrWhiteSpace($WorkspaceVersion)) { $WorkspaceVersion = $MilestoneVersion }

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$tmpRoot = Join-Path $repoRoot "tmp"
$proofRoot = Join-Path $tmpRoot "native_v$MilestoneVersion"
$repeatRoot = Join-Path $tmpRoot "native_v$MilestoneVersion.repeat"
$sourceRoot = Join-Path $repoRoot "examples\native_kernel_multimodule"
$finalElf = Join-Path $tmpRoot "dbyte_native_system_v$MilestoneVersion.elf"
$repeatElf = Join-Path $tmpRoot "dbyte_native_system_v$MilestoneVersion.second.elf"
$directElf = Join-Path $proofRoot "direct-linked.elf"
$finalBin = Join-Path $tmpRoot "dbyte_native_system_v$MilestoneVersion.bin"
$repeatBin = Join-Path $tmpRoot "dbyte_native_system_v$MilestoneVersion.second.bin"
$serialPath = Join-Path $tmpRoot "dbyte_native_system_v$MilestoneVersion.serial.log"
$directSerialPath = Join-Path $tmpRoot "dbyte_native_system_v$MilestoneVersion.direct.serial.log"
$legacyElf = Join-Path $tmpRoot "dbyte_native_kernel_v11.0.0.compat.elf"
$legacyRepeatElf = Join-Path $tmpRoot "dbyte_native_kernel_v11.0.0.compat.second.elf"
$legacySerialPath = Join-Path $tmpRoot "dbyte_native_kernel_v11.0.0.compat.serial.log"
$dumpPath = Join-Path $proofRoot "systems-ir-machine.txt"
$disasmPath = Join-Path $tmpRoot "dbyte_native_system_v$MilestoneVersion.disasm.txt"
$relocsPath = Join-Path $tmpRoot "dbyte_native_system_v$MilestoneVersion.relocs.txt"
$symbolsPath = Join-Path $tmpRoot "dbyte_native_system_v$MilestoneVersion.symbols.txt"
$modules = @("arch_x86", "serial", "numbers", "memory", "kernel")

function Assert-True($Condition, [string]$Message) {
    if (-not $Condition) {
        throw $Message
    }
}

function Assert-Contains([string]$Text, [string]$Needle, [string]$Description) {
    Assert-True ($Text.Contains($Needle)) "missing ${Description}: $Needle"
}

function Read-U16([byte[]]$Bytes, [int]$Offset) {
    Assert-True ($Offset -ge 0 -and $Offset + 2 -le $Bytes.Length) "ELF u16 read is out of range at $Offset"
    return [System.BitConverter]::ToUInt16($Bytes, $Offset)
}

function Read-U32([byte[]]$Bytes, [int]$Offset) {
    Assert-True ($Offset -ge 0 -and $Offset + 4 -le $Bytes.Length) "ELF u32 read is out of range at $Offset"
    return [System.BitConverter]::ToUInt32($Bytes, $Offset)
}

function Invoke-Checked([string]$Executable, [string[]]$Arguments, [string]$Description) {
    $savedPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $Executable @Arguments 2>&1
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $savedPreference
    }
    if ($exitCode -ne 0) {
        throw "$Description failed with exit code ${exitCode}:`n$($output -join "`n")"
    }
    return $output
}

function Assert-SameFile([string]$Left, [string]$Right, [string]$Description) {
    [byte[]]$leftBytes = [System.IO.File]::ReadAllBytes($Left)
    [byte[]]$rightBytes = [System.IO.File]::ReadAllBytes($Right)
    Assert-True ([System.Linq.Enumerable]::SequenceEqual($leftBytes, $rightBytes)) "$Description is not byte-for-byte deterministic"
}

function Get-Sha256([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}

function Assert-ElfIdentity([string]$Path, [uint16]$ExpectedType) {
    [byte[]]$bytes = [System.IO.File]::ReadAllBytes($Path)
    Assert-True ($bytes.Length -ge 52) "$Path is shorter than an ELF32 header"
    Assert-True ($bytes[0] -eq 0x7f -and $bytes[1] -eq 0x45 -and $bytes[2] -eq 0x4c -and $bytes[3] -eq 0x46) "$Path has invalid ELF magic"
    Assert-True ($bytes[4] -eq 1 -and $bytes[5] -eq 1) "$Path is not little-endian ELFCLASS32"
    Assert-True ((Read-U16 $bytes 16) -eq $ExpectedType) "$Path has the wrong ELF type"
    Assert-True ((Read-U16 $bytes 18) -eq 3) "$Path is not EM_386"
    return $bytes
}

function Get-ElfSections([byte[]]$Bytes) {
    $sectionOffset = Read-U32 $Bytes 32
    $sectionEntrySize = Read-U16 $Bytes 46
    $sectionCount = Read-U16 $Bytes 48
    $sectionNameIndex = Read-U16 $Bytes 50
    Assert-True ($sectionEntrySize -eq 40 -and $sectionCount -gt 0) "invalid ELF32 section table"
    Assert-True ($sectionNameIndex -lt $sectionCount) "invalid ELF section-name table index"
    Assert-True ([uint64]$sectionOffset + [uint64]$sectionEntrySize * $sectionCount -le $Bytes.Length) "ELF section table is out of bounds"
    $nameHeader = [int]$sectionOffset + $sectionNameIndex * $sectionEntrySize
    $namesOffset = Read-U32 $Bytes ($nameHeader + 16)
    $namesSize = Read-U32 $Bytes ($nameHeader + 20)
    Assert-True ([uint64]$namesOffset + $namesSize -le $Bytes.Length) "ELF section-name table is out of bounds"

    $sections = @{}
    for ($index = 1; $index -lt $sectionCount; $index++) {
        $at = [int]$sectionOffset + $index * $sectionEntrySize
        $nameOffset = Read-U32 $Bytes $at
        Assert-True ($nameOffset -lt $namesSize) "ELF section name offset is out of bounds"
        $nameStart = [int]$namesOffset + $nameOffset
        $nameEnd = $nameStart
        $nameLimit = [int]$namesOffset + $namesSize
        while ($nameEnd -lt $nameLimit -and $Bytes[$nameEnd] -ne 0) { $nameEnd++ }
        $name = [System.Text.Encoding]::ASCII.GetString($Bytes, $nameStart, $nameEnd - $nameStart)
        $type = Read-U32 $Bytes ($at + 4)
        $offset = Read-U32 $Bytes ($at + 16)
        $size = Read-U32 $Bytes ($at + 20)
        $link = Read-U32 $Bytes ($at + 24)
        $entrySize = Read-U32 $Bytes ($at + 36)
        if ($type -ne 8) {
            Assert-True ([uint64]$offset + $size -le $Bytes.Length) "ELF section $name is out of bounds"
        }
        Assert-True (-not $sections.ContainsKey($name)) "duplicate ELF section $name"
        $sections[$name] = [pscustomobject]@{
            Index = $index
            Type = $type
            Offset = [uint64]$offset
            Size = [uint64]$size
            Link = $link
            EntrySize = $entrySize
        }
    }
    return $sections
}

function Assert-ObjectContract([string]$Path) {
    [byte[]]$bytes = Assert-ElfIdentity $Path 1
    $sections = Get-ElfSections $bytes
    foreach ($name in @(".text", ".rodata", ".data", ".bss", ".symtab", ".strtab", ".shstrtab")) {
        Assert-True ($sections.ContainsKey($name)) "$Path is missing required section $name"
    }
    Assert-True ($sections[".bss"].Type -eq 8) "$Path .bss is not SHT_NOBITS"
    Assert-True ($sections[".symtab"].Type -eq 2 -and $sections[".symtab"].EntrySize -eq 16) "$Path has an invalid ELF32 symbol table"
    Assert-True ($sections[".symtab"].Link -eq $sections[".strtab"].Index) "$Path symbol table does not link to .strtab"
    foreach ($name in @(".rel.text", ".rel.data", ".rel.rodata")) {
        if ($sections.ContainsKey($name)) {
            Assert-True ($sections[$name].Type -eq 9 -and $sections[$name].EntrySize -eq 8) "$Path has invalid REL entries in $name"
        }
    }
    return [pscustomobject]@{ Bytes = $bytes; Sections = $sections }
}

function Assert-ExecutableContract([string]$Path) {
    [byte[]]$bytes = Assert-ElfIdentity $Path 2
    $programOffset = Read-U32 $bytes 28
    $programEntrySize = Read-U16 $bytes 42
    $programCount = Read-U16 $bytes 44
    Assert-True ($programEntrySize -eq 32 -and $programCount -ge 1) "$Path has an invalid ELF32 program table"
    Assert-True ([uint64]$programOffset + [uint64]$programEntrySize * $programCount -le $bytes.Length) "$Path program table is out of bounds"
    $loadCount = 0
    for ($index = 0; $index -lt $programCount; $index++) {
        $at = [int]$programOffset + $index * $programEntrySize
        $type = Read-U32 $bytes $at
        Assert-True ($type -ne 2 -and $type -ne 3) "$Path contains PT_DYNAMIC or PT_INTERP"
        if ($type -eq 1) { $loadCount++ }
    }
    Assert-True ($loadCount -ge 1) "$Path has no PT_LOAD segment"
    $sections = Get-ElfSections $bytes
    foreach ($name in @(".multiboot", ".text", ".rodata", ".data", ".bss", ".symtab", ".strtab", ".shstrtab")) {
        Assert-True ($sections.ContainsKey($name)) "$Path is missing required section $name"
    }
    Assert-True (-not $sections.ContainsKey(".dynamic")) "$Path contains a dynamic section"
    $multibootOffset = [int]$sections[".multiboot"].Offset
    Assert-True ($multibootOffset -le 8192 - 12) "$Path Multiboot header lies outside the first 8 KiB"
    $magic = Read-U32 $bytes $multibootOffset
    $flags = Read-U32 $bytes ($multibootOffset + 4)
    $checksum = Read-U32 $bytes ($multibootOffset + 8)
    $sum = ([uint64]$magic + [uint64]$flags + [uint64]$checksum) -band [uint64]4294967295
    Assert-True ($magic -eq 0x1badb002 -and $sum -eq 0) "$Path has an invalid Multiboot v1 header"
    return [pscustomobject]@{ Bytes = $bytes; Sections = $sections }
}

function Build-ObjectSet([string]$Destination, [string]$Compiler) {
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    foreach ($module in $modules) {
        $source = Join-Path $sourceRoot "$module.dby"
        $object = Join-Path $Destination "$module.o"
        Invoke-Checked $Compiler @("build", $source, "--target", "i686-dbyte-none", "--emit", "obj", "-o", $object) "build $module.o" | Out-Null
        Assert-ObjectContract $object | Out-Null
    }
}

function Invoke-QemuSerialProof(
    [string]$Qemu,
    [string]$Elf,
    [string]$Serial,
    [byte[]]$Expected
) {
    if (Test-Path -LiteralPath $Serial) { Remove-Item -LiteralPath $Serial -Force }
    $arguments = "-kernel `"$Elf`" -serial `"file:$Serial`" -display none -monitor none -no-reboot -no-shutdown"
    $process = $null
    try {
        $process = Start-Process -FilePath $Qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
        $timer = [System.Diagnostics.Stopwatch]::StartNew()
        while ($timer.Elapsed.TotalSeconds -lt 10) {
            if (Test-Path -LiteralPath $Serial) {
                try {
                    [byte[]]$observed = [System.IO.File]::ReadAllBytes($Serial)
                    if ([System.Linq.Enumerable]::SequenceEqual($observed, $Expected)) { break }
                }
                catch { }
            }
            if ($process.HasExited) { break }
            Start-Sleep -Milliseconds 100
        }
    }
    finally {
        if ($null -ne $process -and -not $process.HasExited) {
            Stop-Process -Id $process.Id -Force
            $process.WaitForExit()
        }
    }
    Assert-True (Test-Path -LiteralPath $Serial) "QEMU did not create serial proof for $Elf"
    [byte[]]$serialBytes = [System.IO.File]::ReadAllBytes($Serial)
    Assert-True ([System.Linq.Enumerable]::SequenceEqual($serialBytes, $Expected)) "QEMU serial proof does not match for $Elf"
    return $arguments
}

Set-Location $repoRoot
$baselineIsAncestor = & git merge-base --is-ancestor "fa85c484c05ceebb74ce70298d1b9f6e0fb32509" HEAD
Assert-True ($LASTEXITCODE -eq 0) "native release history does not descend from the required v11.0.0 commit"
Assert-True ((git rev-parse "v11.0.0^{}").Trim() -eq "fa85c484c05ceebb74ce70298d1b9f6e0fb32509") "v11.0.0 tag does not resolve to the required baseline"

$manifest = Get-Content -LiteralPath (Join-Path $repoRoot "Cargo.toml") -Raw
$versionMatch = [regex]::Match($manifest, '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"')
Assert-True ($versionMatch.Success -and $versionMatch.Groups[1].Value -eq $WorkspaceVersion) "workspace version does not match expected compatibility version $WorkspaceVersion"

foreach ($directory in @($proofRoot, $repeatRoot)) {
    if (Test-Path -LiteralPath $directory) { Remove-Item -LiteralPath $directory -Recurse -Force }
}
New-Item -ItemType Directory -Path $proofRoot -Force | Out-Null
foreach ($path in @($finalElf, $repeatElf, $finalBin, $repeatBin, $serialPath, $directSerialPath, $legacyElf, $legacyRepeatElf, $legacySerialPath, $disasmPath, $relocsPath, $symbolsPath)) {
    if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
}

$cargo = (Get-Command cargo -ErrorAction Stop).Source
Invoke-Checked $cargo @("build", "-p", "dbyte_cli", "--bins") "build DByte CLIs" | Out-Null
$dbytec = Join-Path $repoRoot "target\debug\dbytec.exe"
$dbyte = Join-Path $repoRoot "target\debug\dbyte.exe"
Assert-True (Test-Path -LiteralPath $dbytec) "dbytec executable was not produced"
Assert-True (Test-Path -LiteralPath $dbyte) "dbyte executable was not produced"

$version = Invoke-Checked $dbyte @("--version") "dbyte version"
Assert-True (($version -join "`n").Trim() -eq "DByte $WorkspaceVersion") "dbyte version does not match $WorkspaceVersion"

Write-Host "Building five DByte ET_REL modules twice..."
Build-ObjectSet $proofRoot $dbytec
Build-ObjectSet $repeatRoot $dbytec
$objectHashes = [ordered]@{}
foreach ($module in $modules) {
    $first = Join-Path $proofRoot "$module.o"
    $second = Join-Path $repeatRoot "$module.o"
    Assert-SameFile $first $second "$module.o"
    $objectHashes[$module] = Get-Sha256 $first
}

$orderedObjects = @($modules | ForEach-Object { Join-Path $proofRoot "$_.o" })
$reverseObjects = @($modules[($modules.Count - 1)..0] | ForEach-Object { Join-Path $repeatRoot "$_.o" })
Invoke-Checked $dbytec (@("link") + $orderedObjects + @("-o", $directElf)) "direct DByte link" | Out-Null
$reverseElf = Join-Path $repeatRoot "reverse-linked.elf"
Invoke-Checked $dbytec (@("link") + $reverseObjects + @("-o", $reverseElf)) "reverse-order DByte link" | Out-Null
Assert-SameFile $directElf $reverseElf "DByte linker output across input order"

$kernelSource = Join-Path $sourceRoot "kernel.dby"
$dump = Invoke-Checked $dbytec @("build", $kernelSource, "--target", "i686-dbyte-none", "--emit", "elf", "-o", $finalElf, "--dump-ir", "--dump-machine-code") "integrated native ELF build"
$dumpText = $dump -join "`n"
$dumpText | Set-Content -LiteralPath $dumpPath -Encoding utf8
Invoke-Checked $dbytec @("build", $kernelSource, "--target", "i686-dbyte-none", "--emit", "elf", "-o", $repeatElf) "repeat integrated native ELF build" | Out-Null
Invoke-Checked $dbytec @("build", $kernelSource, "--target", "i686-dbyte-none", "--emit", "bin", "-o", $finalBin) "integrated native BIN build" | Out-Null
Invoke-Checked $dbytec @("build", $kernelSource, "--target", "i686-dbyte-none", "--emit", "bin", "-o", $repeatBin) "repeat integrated native BIN build" | Out-Null
Assert-SameFile $finalElf $repeatElf "integrated ELF"
Assert-SameFile $finalBin $repeatBin "integrated flat BIN"
Assert-True ((Get-Item -LiteralPath $finalBin).Length -gt 0) "integrated native BIN is empty"
$executable = Assert-ExecutableContract $finalElf

$legacySource = Join-Path $repoRoot "examples\native_kernel\kernel.dby"
Invoke-Checked $dbytec @("build", $legacySource, "--target", "i686-dbyte-none", "--emit", "elf", "-o", $legacyElf) "legacy v11.0.0 native build" | Out-Null
Invoke-Checked $dbytec @("build", $legacySource, "--target", "i686-dbyte-none", "--emit", "elf", "-o", $legacyRepeatElf) "repeat legacy v11.0.0 native build" | Out-Null
Assert-SameFile $legacyElf $legacyRepeatElf "legacy v11.0.0 native ELF"
Assert-ExecutableContract $legacyElf | Out-Null

foreach ($needle in @(
    "native-mir pointer-bytes=4",
    "function out8 instance=",
    "function kernel_main instance=",
    "function compute instance=",
    "function add instance=",
    "function write instance=",
    "AggregateZero",
    "AddressOffset",
    "ty: I16",
    "op: BitOr",
    "op: ShiftLeft",
    "GlobalAddress",
    'callee: Direct { symbol: "add" }',
    "function kernel_main opt=O1",
    "function compute opt=O1"
)) {
    Assert-Contains $dumpText $needle "Native MIR or machine-selection evidence"
}

$allSource = ($modules | ForEach-Object { Get-Content -LiteralPath (Join-Path $sourceRoot "$_.dby") -Raw }) -join "`n"
Assert-True (-not [regex]::IsMatch($allSource, '"42"')) "native proof embeds a pre-baked string literal 42"
foreach ($needle in @("numbers.add(record.value, increment)", "memory.compute(40 as u32)", "return left + right", "ptr[u8]", "array[u8, 4]", "packed struct", "size_of(SystemRecord)", "align_of(SystemRecord)")) {
    Assert-Contains $allSource $needle "multi-module systems execution source"
}

$kernelObject = Assert-ObjectContract (Join-Path $proofRoot "kernel.o")
$rodata = $kernelObject.Sections[".rodata"]
if ($rodata.Size -ge 2) {
    [byte[]]$rodataBytes = $kernelObject.Bytes[[int]$rodata.Offset..([int]($rodata.Offset + $rodata.Size - 1))]
    Assert-True (-not [System.Text.Encoding]::ASCII.GetString($rodataBytes).Contains("42")) "kernel object embeds ASCII 42 in .rodata"
}

$objdump = (Get-Command objdump -ErrorAction Stop).Source
$relocationEvidence = New-Object System.Collections.Generic.List[string]
$symbolEvidence = New-Object System.Collections.Generic.List[string]
foreach ($module in $modules) {
    $object = Join-Path $proofRoot "$module.o"
    $relocationEvidence.AddRange([string[]](Invoke-Checked $objdump @("-r", $object) "objdump relocations for $module.o"))
    $symbolEvidence.AddRange([string[]](Invoke-Checked $objdump @("-t", $object) "objdump symbols for $module.o"))
}
$relocationText = $relocationEvidence -join "`n"
$symbolText = $symbolEvidence -join "`n"
$relocationText | Set-Content -LiteralPath $relocsPath -Encoding utf8
$symbolText | Set-Content -LiteralPath $symbolsPath -Encoding utf8
Assert-Contains $relocationText "R_386_PC32" "cross-object function relocation"
Assert-Contains $relocationText "R_386_32" "address/data relocation"
Assert-Contains $relocationText "add" "memory-to-numbers relocation"
Assert-Contains $symbolText "*UND*" "undefined imported object symbols"
Assert-Contains $symbolText "kernel_main" "kernel entry symbol"

$inspection = Invoke-Checked $objdump @("-f", "-h", "-p", "-t", "-d", "-Mintel", $finalElf) "independent final ELF inspection"
$inspectionText = $inspection -join "`n"
$inspectionText | Set-Content -LiteralPath $disasmPath -Encoding utf8
foreach ($needle in @("file format elf32-i386", "<_start>", "<kernel_main>", "<compute>", "<add>", "<out8>", "<putc>")) {
    Assert-Contains $inspectionText $needle "final ELF disassembly"
}
Assert-True ([regex]::IsMatch($inspectionText, '(?im)\bcall\b[^\r\n]*<compute>')) "kernel_main does not call cross-object compute"
Assert-True ([regex]::IsMatch($inspectionText, '(?im)\bcall\b[^\r\n]*<add>')) "memory code does not call cross-object add"
Assert-True ([regex]::IsMatch($inspectionText, '(?im)\bout\s+dx\s*,\s*al\b')) "final machine code has no direct port output"
Assert-True ([regex]::IsMatch($inspectionText, '(?im)\bin\s+al\s*,\s*dx\b')) "final machine code has no direct port input"
Assert-True (-not $inspectionText.Contains("DLL Name") -and -not $inspectionText.Contains("INTERP")) "final ELF has a hosted loader dependency"

$nativeFiles = Get-ChildItem -LiteralPath (Join-Path $repoRoot "crates\dbyte_native\src") -Filter "*.rs" -File
$nativeText = ($nativeFiles | ForEach-Object {
    $text = Get-Content -LiteralPath $_.FullName -Raw
    $testMarker = $text.IndexOf('#[cfg(test)]', [StringComparison]::Ordinal)
    if ($testMarker -ge 0) { $text.Substring(0, $testMarker) } else { $text }
}) -join "`n"
$nativeManifest = Get-Content -LiteralPath (Join-Path $repoRoot "crates\dbyte_native\Cargo.toml") -Raw
foreach ($forbidden in @("std::process::Command", "process::Command", "Command::new", "llvm", "cranelift", "gcc", "clang", "nasm", "yasm", "fasm")) {
    Assert-True (-not $nativeText.Contains($forbidden)) "native backend contains forbidden generator/process token: $forbidden"
}
Assert-True ($nativeManifest.Contains("dbyte_ast") -and $nativeManifest.Contains("dbyte_lexer") -and $nativeManifest.Contains("dbyte_parser")) "native backend does not reuse the shared frontend"
Assert-True (-not $nativeManifest.Contains("dbyte_interp") -and -not $nativeManifest.Contains("dbyte_vm") -and -not $nativeManifest.Contains("dbyte_compiler")) "native backend depends on a hosted execution path"

$qemu = (Get-Command qemu-system-i386 -ErrorAction Stop).Source
[byte[]]$expectedSerial = [System.Text.Encoding]::ASCII.GetBytes("DBYTE NATIVE SYSTEM`r`nMULTI MODULE ONLINE`r`nLINKER ONLINE`r`nSYSTEM TYPES ONLINE`r`n42`r`n")
$qemuArguments = Invoke-QemuSerialProof $qemu $finalElf $serialPath $expectedSerial
$directQemuArguments = Invoke-QemuSerialProof $qemu $directElf $directSerialPath $expectedSerial
[byte[]]$expectedLegacySerial = [System.Text.Encoding]::ASCII.GetBytes("DBYTE NATIVE KERNEL`r`nMACHINE CODE ONLINE`r`n42`r`n")
$legacyQemuArguments = Invoke-QemuSerialProof $qemu $legacyElf $legacySerialPath $expectedLegacySerial

Write-Host "native systems compiler tests and execution proof passed"
foreach ($module in $modules) {
    Write-Host "$module.o SHA256: $($objectHashes[$module])"
}
Write-Host "ELF SHA256: $(Get-Sha256 $finalElf)"
Write-Host "direct-link ELF SHA256: $(Get-Sha256 $directElf)"
Write-Host "BIN SHA256: $(Get-Sha256 $finalBin)"
Write-Host "serial SHA256: $(Get-Sha256 $serialPath)"
Write-Host "direct-link serial SHA256: $(Get-Sha256 $directSerialPath)"
Write-Host "QEMU command: `"$qemu`" $qemuArguments"
Write-Host "direct-link QEMU command: `"$qemu`" $directQemuArguments"
Write-Host "serial proof: DBYTE NATIVE SYSTEM | MULTI MODULE ONLINE | LINKER ONLINE | SYSTEM TYPES ONLINE | 42"
Write-Host "legacy QEMU command: `"$qemu`" $legacyQemuArguments"
Write-Host "legacy serial proof: DBYTE NATIVE KERNEL | MACHINE CODE ONLINE | 42"
Write-Host "native verify passed"
