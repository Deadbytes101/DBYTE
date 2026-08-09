param(
    [string]$MilestoneVersion = "11.0.0"
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$tmpRoot = Join-Path $repoRoot "tmp"
$sourcePath = Join-Path $repoRoot "examples\native_kernel\kernel.dby"
$elfA = Join-Path $tmpRoot "dbyte_native_kernel_v$MilestoneVersion.elf"
$elfB = Join-Path $tmpRoot "dbyte_native_kernel_v$MilestoneVersion.second.elf"
$elfCli = Join-Path $tmpRoot "dbyte_native_kernel_v$MilestoneVersion.cli.elf"
$binPath = Join-Path $tmpRoot "dbyte_native_kernel_v$MilestoneVersion.bin"
$dumpPath = Join-Path $tmpRoot "dbyte_native_kernel_v$MilestoneVersion.dumps.txt"
$disasmPath = Join-Path $tmpRoot "dbyte_native_kernel_v$MilestoneVersion.disasm.txt"
$serialPath = Join-Path $tmpRoot "dbyte_native_kernel_v$MilestoneVersion.serial.log"
$negativeOutput = Join-Path $tmpRoot "dbyte_native_negative.elf"

function Assert-True($condition, $message) {
    if (-not $condition) {
        throw $message
    }
}

function Read-U16([byte[]]$bytes, [int]$offset) {
    Assert-True ($offset -ge 0 -and $offset + 2 -le $bytes.Length) "ELF u16 read is out of range at $offset"
    return [System.BitConverter]::ToUInt16($bytes, $offset)
}

function Read-U32([byte[]]$bytes, [int]$offset) {
    Assert-True ($offset -ge 0 -and $offset + 4 -le $bytes.Length) "ELF u32 read is out of range at $offset"
    return [System.BitConverter]::ToUInt32($bytes, $offset)
}

function Read-AsciiZ([byte[]]$bytes, [int]$offset, [int]$limit) {
    Assert-True ($offset -ge 0 -and $offset -lt $limit -and $limit -le $bytes.Length) "ELF string offset is out of range"
    $chars = New-Object System.Collections.Generic.List[char]
    for ($i = $offset; $i -lt $limit -and $bytes[$i] -ne 0; $i++) {
        $chars.Add([char]$bytes[$i])
    }
    return -join $chars
}

function Invoke-Cargo([string[]]$Arguments) {
    & $script:cargo @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "cargo command failed with exit code ${LASTEXITCODE}: cargo $($Arguments -join ' ')"
    }
}

Set-Location $repoRoot
New-Item -ItemType Directory -Path $tmpRoot -Force | Out-Null
foreach ($path in @($elfA, $elfB, $elfCli, $binPath, $dumpPath, $disasmPath, $serialPath, $negativeOutput)) {
    Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
}

$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
Assert-True ($null -ne $cargoCommand) "native verifier requires cargo"
$script:cargo = $cargoCommand.Source

$rootManifestText = Get-Content -LiteralPath (Join-Path $repoRoot "Cargo.toml") -Raw
$workspaceVersionMatch = [regex]::Match(
    $rootManifestText,
    '(?ms)^\[workspace\.package\]\s+.*?^version\s*=\s*"([^"]+)"'
)
Assert-True $workspaceVersionMatch.Success "workspace package version is missing from Cargo.toml"
$workspaceVersion = $workspaceVersionMatch.Groups[1].Value
Assert-True ($MilestoneVersion -eq $workspaceVersion) "native verifier version $MilestoneVersion does not match workspace version $workspaceVersion"

$cliVersion = & $script:cargo run -q -p dbyte_cli --bin dbyte -- --version
if ($LASTEXITCODE -ne 0) { throw "dbyte --version failed" }
Assert-True (($cliVersion -join "`n").Trim() -eq "DByte $MilestoneVersion") "dbyte CLI version does not match native milestone $MilestoneVersion"

Write-Host "Running native compiler unit and integration tests..."
Invoke-Cargo @("test", "-p", "dbyte_native")

$nativeRust = Get-ChildItem -LiteralPath (Join-Path $repoRoot "crates\dbyte_native\src") -Filter "*.rs" |
    ForEach-Object { Get-Content -LiteralPath $_.FullName -Raw }
$nativeRustText = $nativeRust -join "`n"
$nativeManifestText = Get-Content -LiteralPath (Join-Path $repoRoot "crates\dbyte_native\Cargo.toml") -Raw
foreach ($forbidden in @(
    "std::process::Command",
    "process::Command",
    "Command::new",
    "llvm",
    "cranelift",
    "rustc",
    "gcc",
    "clang",
    "nasm",
    "yasm",
    "fasm"
)) {
    Assert-True (-not $nativeRustText.Contains($forbidden)) "native backend contains forbidden generator/process token: $forbidden"
}
Assert-True ($nativeRustText.Contains("IrProgram")) "native backend must contain an explicit IR program"
Assert-True ($nativeRustText.Contains("EM_386") -or $nativeRustText.Contains("ELF_MACHINE_386")) "native backend must own the ELF32 i386 machine declaration"
Assert-True ($nativeManifestText.Contains("dbyte_ast") -and $nativeManifestText.Contains("dbyte_lexer") -and $nativeManifestText.Contains("dbyte_parser")) "native compiler must reuse the existing DByte frontend crates"
Assert-True (-not $nativeManifestText.Contains("dbyte_interp") -and -not $nativeManifestText.Contains("dbyte_vm") -and -not $nativeManifestText.Contains("dbyte_compiler")) "native compiler must not depend on an interpreter, VM, or bytecode compiler"
Assert-True (-not $nativeRustText.Contains("DBYTE NATIVE KERNEL") -and -not $nativeRustText.Contains("MACHINE CODE ONLINE")) "native backend must not embed the kernel proof markers"

$kernelSourceText = Get-Content -LiteralPath $sourcePath -Raw
Assert-True (-not [regex]::IsMatch($kernelSourceText, '"42"')) "native proof must not contain a pre-baked string literal 42"
Assert-True ($kernelSourceText.Contains("20 * 2") -and $kernelSourceText.Contains("add(base, 2)")) "native proof must compute 42 through arithmetic and a function call"
Assert-True ($kernelSourceText.Contains("if value == 42:") -and $kernelSourceText.Contains("serial_write_decimal_0_99(value)")) "native proof must branch on and print the computed value"

Write-Host "Building the native kernel twice with dbytec, then checking the integrated dbyte entrypoint..."
$dumpOutput = & $script:cargo run -q -p dbyte_cli --bin dbytec -- build $sourcePath `
    --target i686-dbyte-none --emit elf -o $elfA --dump-ir --dump-machine-code
if ($LASTEXITCODE -ne 0) { throw "dbytec native build with inspection dumps failed" }
$dumpText = $dumpOutput -join "`n"
$dumpText | Set-Content -LiteralPath $dumpPath -Encoding utf8
foreach ($requiredDump in @(
    "target i686-dbyte-none",
    "fn kernel_main(",
    "mul.i32",
    "call add 2 -> i32",
    "jump_if_false",
    "intrinsic native.io_out8",
    "function add ",
    "fixup call-rel32",
    "patched absolute immediate",
    "patched-machine-code"
)) {
    Assert-True ($dumpText.Contains($requiredDump)) "native inspection dump is missing evidence: $requiredDump"
}

Invoke-Cargo @(
    "run", "-q", "-p", "dbyte_cli", "--bin", "dbytec", "--", "build", $sourcePath,
    "--target", "i686-dbyte-none", "--emit", "elf", "-o", $elfB
)
Invoke-Cargo @(
    "run", "-q", "-p", "dbyte_cli", "--bin", "dbyte", "--", "build", $sourcePath,
    "--target", "i686-dbyte-none", "--emit", "elf", "-o", $elfCli
)
Invoke-Cargo @(
    "run", "-q", "-p", "dbyte_cli", "--bin", "dbytec", "--", "build", $sourcePath,
    "--target", "i686-dbyte-none", "--emit", "bin", "-o", $binPath
)

$hashA = (Get-FileHash -Algorithm SHA256 -LiteralPath $elfA).Hash
$hashB = (Get-FileHash -Algorithm SHA256 -LiteralPath $elfB).Hash
$hashCli = (Get-FileHash -Algorithm SHA256 -LiteralPath $elfCli).Hash
Assert-True ($hashA -eq $hashB) "native ELF is not deterministic: $hashA != $hashB"
Assert-True ($hashA -eq $hashCli) "dbyte and dbytec native entrypoints emitted different ELF bytes: $hashA != $hashCli"
Assert-True ((Get-Item -LiteralPath $binPath).Length -gt 12) "native flat binary is unexpectedly empty"

Write-Host "Proving native rejection preserves an existing output byte for byte..."
[byte[]]$sentinelBytes = 0x44, 0x42, 0x59, 0x54, 0x45, 0x2d, 0x53, 0x45, 0x4e, 0x54, 0x49, 0x4e, 0x45, 0x4c
[System.IO.File]::WriteAllBytes($negativeOutput, $sentinelBytes)
$sentinelHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $negativeOutput).Hash
$oldPreference = $ErrorActionPreference
$ErrorActionPreference = "Continue"
$negativeText = & $script:cargo run -q -p dbyte_cli --bin dbytec -- build `
    (Join-Path $repoRoot "examples\hello.dby") --target i686-dbyte-none --emit elf -o $negativeOutput 2>&1
$negativeCode = $LASTEXITCODE
$ErrorActionPreference = $oldPreference
Assert-True ($negativeCode -ne 0) "hosted-only DByte source unexpectedly compiled in freestanding mode"
Assert-True (Test-Path -LiteralPath $negativeOutput) "failed native compilation removed the previous output"
$preservedHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $negativeOutput).Hash
Assert-True ($preservedHash -eq $sentinelHash) "failed native compilation modified the previous output"
Assert-True (($negativeText -join "`n").Contains("Unsupported")) "native rejection did not report an unsupported-feature category"

Write-Host "Parsing ELF32 structure independently..."
[byte[]]$elf = [System.IO.File]::ReadAllBytes($elfA)
Assert-True ($elf.Length -ge 84) "native ELF is shorter than its ELF/program headers"
Assert-True ($elf[0] -eq 0x7f -and $elf[1] -eq 0x45 -and $elf[2] -eq 0x4c -and $elf[3] -eq 0x46) "invalid ELF magic"
Assert-True ($elf[4] -eq 1) "ELF class is not ELF32"
Assert-True ($elf[5] -eq 1) "ELF data encoding is not little endian"
Assert-True ((Read-U16 $elf 16) -eq 2) "ELF type is not ET_EXEC"
Assert-True ((Read-U16 $elf 18) -eq 3) "ELF machine is not Intel 80386"
Assert-True ((Read-U16 $elf 40) -eq 52) "unexpected ELF32 header size"

$entry = Read-U32 $elf 24
$programOffset = Read-U32 $elf 28
$sectionOffset = Read-U32 $elf 32
$programEntrySize = Read-U16 $elf 42
$programCount = Read-U16 $elf 44
$sectionEntrySize = Read-U16 $elf 46
$sectionCount = Read-U16 $elf 48
$sectionNameIndex = Read-U16 $elf 50
Assert-True ($programEntrySize -eq 32 -and $programCount -eq 1) "native ELF must contain exactly one ELF32 program header"
Assert-True ($sectionEntrySize -eq 40 -and $sectionCount -ge 7) "invalid ELF32 section-header table"
Assert-True ([uint64]$programOffset + [uint64]$programEntrySize * $programCount -le $elf.Length) "program-header table is out of range"
Assert-True ([uint64]$sectionOffset + [uint64]$sectionEntrySize * $sectionCount -le $elf.Length) "section-header table is out of range"

$entryInExecutableLoad = $false
$loadSegment = $null
for ($i = 0; $i -lt $programCount; $i++) {
    $at = [int]$programOffset + $i * $programEntrySize
    $type = Read-U32 $elf $at
    $offset = Read-U32 $elf ($at + 4)
    $virtualAddress = Read-U32 $elf ($at + 8)
    $fileSize = Read-U32 $elf ($at + 16)
    $memorySize = Read-U32 $elf ($at + 20)
    $flags = Read-U32 $elf ($at + 24)
    Assert-True ($type -ne 2 -and $type -ne 3) "native ELF contains PT_DYNAMIC or PT_INTERP"
    if ($type -eq 1) {
        Assert-True ($null -eq $loadSegment) "native ELF contains more than one PT_LOAD segment"
        Assert-True ([uint64]$offset + $fileSize -le $elf.Length) "PT_LOAD file range is out of bounds"
        Assert-True ($memorySize -ge $fileSize) "PT_LOAD memory size is smaller than file size"
        Assert-True (($flags -band 1) -ne 0 -and ($flags -band 2) -ne 0 -and ($flags -band 4) -ne 0) "native PT_LOAD does not have the expected RWX bootstrap flags"
        $loadSegment = [pscustomobject]@{
            Offset = [uint64]$offset
            Address = [uint64]$virtualAddress
            FileSize = [uint64]$fileSize
            MemorySize = [uint64]$memorySize
        }
        if (($flags -band 1) -ne 0 -and $entry -ge $virtualAddress -and [uint64]$entry -lt [uint64]$virtualAddress + $fileSize) {
            $entryInExecutableLoad = $true
        }
    }
}
Assert-True ($null -ne $loadSegment) "native ELF does not contain PT_LOAD"
Assert-True $entryInExecutableLoad "ELF entry point does not lie in executable loaded code"

Assert-True ($sectionNameIndex -lt $sectionCount) "invalid section-name table index"
$sectionNameHeader = [int]$sectionOffset + $sectionNameIndex * $sectionEntrySize
$sectionNamesOffset = Read-U32 $elf ($sectionNameHeader + 16)
$sectionNamesSize = Read-U32 $elf ($sectionNameHeader + 20)
Assert-True ([uint64]$sectionNamesOffset + $sectionNamesSize -le $elf.Length) "section-name table is out of range"

$requiredSections = @(".multiboot", ".text", ".rodata", ".data", ".bss", ".symtab", ".strtab", ".shstrtab")
$foundSections = @{}
$fileRanges = New-Object System.Collections.Generic.List[object]
for ($i = 1; $i -lt $sectionCount; $i++) {
    $at = [int]$sectionOffset + $i * $sectionEntrySize
    $nameOffset = Read-U32 $elf $at
    $type = Read-U32 $elf ($at + 4)
    $flags = Read-U32 $elf ($at + 8)
    $address = Read-U32 $elf ($at + 12)
    $offset = Read-U32 $elf ($at + 16)
    $size = Read-U32 $elf ($at + 20)
    $link = Read-U32 $elf ($at + 24)
    $entrySize = Read-U32 $elf ($at + 36)
    Assert-True ($nameOffset -lt $sectionNamesSize) "section name offset is out of range"
    $name = Read-AsciiZ $elf ([int]$sectionNamesOffset + $nameOffset) ([int]$sectionNamesOffset + $sectionNamesSize)
    Assert-True (-not $foundSections.ContainsKey($name)) "duplicate ELF section name: $name"
    $foundSections[$name] = @{
        Index = $i
        Address = [uint64]$address
        Offset = [uint64]$offset
        Size = [uint64]$size
        Type = $type
        Flags = $flags
        Link = $link
        EntrySize = $entrySize
    }
    if ($type -ne 8 -and $size -gt 0) {
        Assert-True ([uint64]$offset + $size -le $elf.Length) "section $name is out of file bounds"
        $fileRanges.Add([pscustomobject]@{ Name = $name; Start = [uint64]$offset; End = [uint64]$offset + $size })
    }
}
foreach ($name in $requiredSections) {
    Assert-True ($foundSections.ContainsKey($name)) "required ELF section is missing: $name"
}
$expectedSectionTypes = @{
    ".multiboot" = 1
    ".text" = 1
    ".rodata" = 1
    ".data" = 1
    ".bss" = 8
    ".symtab" = 2
    ".strtab" = 3
    ".shstrtab" = 3
}
foreach ($name in $expectedSectionTypes.Keys) {
    Assert-True ($foundSections[$name].Type -eq $expectedSectionTypes[$name]) "ELF section $name has the wrong type"
}
Assert-True (($foundSections[".text"].Flags -band 0x6) -eq 0x6) ".text is not allocated executable code"
Assert-True (($foundSections[".rodata"].Flags -band 0x2) -eq 0x2 -and ($foundSections[".rodata"].Flags -band 0x1) -eq 0) ".rodata flags are invalid"
Assert-True (($foundSections[".data"].Flags -band 0x3) -eq 0x3) ".data is not allocated writable data"
Assert-True (($foundSections[".bss"].Flags -band 0x3) -eq 0x3 -and $foundSections[".bss"].Size -gt 0) ".bss is not a nonempty allocated writable region"
Assert-True ($foundSections[".symtab"].EntrySize -eq 16 -and $foundSections[".symtab"].Size % 16 -eq 0) ".symtab does not contain bounded ELF32 symbols"
Assert-True ($foundSections[".symtab"].Link -eq $foundSections[".strtab"].Index) ".symtab does not link to .strtab"
$orderedRanges = $fileRanges | Sort-Object Start, End
for ($i = 1; $i -lt $orderedRanges.Count; $i++) {
    Assert-True ($orderedRanges[$i].Start -ge $orderedRanges[$i - 1].End) "ELF sections overlap: $($orderedRanges[$i - 1].Name) and $($orderedRanges[$i].Name)"
}
$headerEnd = [Math]::Max(52, [uint64]$programOffset + [uint64]$programEntrySize * $programCount)
foreach ($range in $orderedRanges) {
    Assert-True ($range.Start -ge $headerEnd) "ELF section $($range.Name) overlaps the ELF/program headers"
    Assert-True ($range.End -le $sectionOffset -or $range.Start -ge [uint64]$sectionOffset + [uint64]$sectionEntrySize * $sectionCount) "ELF section $($range.Name) overlaps the section-header table"
}
$textSection = $foundSections[".text"]
Assert-True ($entry -ge $textSection.Address -and [uint64]$entry -lt [uint64]$textSection.Address + $textSection.Size) "ELF entry point is outside .text"
$bssSection = $foundSections[".bss"]
Assert-True ($bssSection.Address -ge $loadSegment.Address -and $bssSection.Address + $bssSection.Size -le $loadSegment.Address + $loadSegment.MemorySize) ".bss is outside PT_LOAD memory bounds"
Assert-True (-not $foundSections.ContainsKey(".dynamic")) "native ELF contains a dynamic section"

$multibootFound = $false
$multibootOffset = -1
$scanLimit = [Math]::Min(8192 - 12, $elf.Length - 12)
for ($offset = 0; $offset -le $scanLimit; $offset += 4) {
    if ((Read-U32 $elf $offset) -eq 0x1badb002) {
        $flags = Read-U32 $elf ($offset + 4)
        $checksum = Read-U32 $elf ($offset + 8)
        $sum = ([uint64]0x1badb002 + [uint64]$flags + [uint64]$checksum) -band [uint64]4294967295
        if ($sum -eq 0) {
            $multibootFound = $true
            $multibootOffset = $offset
            break
        }
    }
}
Assert-True $multibootFound "valid Multiboot v1 header was not found in the first 8 KiB"
Assert-True ($multibootOffset -eq $foundSections[".multiboot"].Offset) "Multiboot header is not at the start of .multiboot"

$objdump = Get-Command objdump -ErrorAction SilentlyContinue
Assert-True ($null -ne $objdump) "native verifier requires GNU objdump for independent inspection"
$inspection = & $objdump.Source -f -h -p -d -Mintel $elfA 2>&1
if ($LASTEXITCODE -ne 0) {
    throw "objdump rejected the DByte-generated ELF"
}
$inspection | Set-Content -LiteralPath $disasmPath -Encoding utf8
$inspectionText = $inspection -join "`n"
Assert-True ($inspectionText.Contains("file format elf32-i386")) "objdump did not recognize ELF32 i386"
Assert-True ($inspectionText.Contains("<_start>")) "objdump did not find the synthetic entry symbol"
Assert-True ($inspectionText.Contains("<kernel_main>")) "objdump did not find generated DByte kernel_main"
Assert-True ([regex]::IsMatch($inspectionText, '(?im)\bimul\b')) "disassembly does not show DByte multiplication"
Assert-True ([regex]::IsMatch($inspectionText, '(?im)\bcall\b[^\r\n]*<add>')) "disassembly does not show a generated call to DByte add"
Assert-True ([regex]::IsMatch($inspectionText, '(?im)\bcmp\b')) "disassembly does not show generated DByte comparisons"
Assert-True ([regex]::IsMatch($inspectionText, '(?im)\bj(?:e|ne|l|le|g|ge)\b')) "disassembly does not show generated DByte conditional control flow"
Assert-True ([regex]::IsMatch($inspectionText, '(?im)\bout\s+dx\s*,\s*al\b')) "disassembly does not show direct DByte serial output"
Assert-True ([regex]::IsMatch($inspectionText, '(?im)\bin\s+al\s*,\s*dx\b')) "disassembly does not show direct DByte serial input"
Assert-True (-not $inspectionText.Contains("DLL Name")) "native ELF unexpectedly imports a DLL"
Assert-True (-not $inspectionText.Contains("INTERP")) "native ELF unexpectedly contains an interpreter"

$qemu = Get-Command qemu-system-i386 -ErrorAction SilentlyContinue
Assert-True ($null -ne $qemu) "native verifier requires qemu-system-i386"
$qemuArgumentLine = "-kernel `"$elfA`" -serial `"file:$serialPath`" -display none -monitor none -no-reboot -no-shutdown"
$qemuProcess = $null
try {
    $qemuProcess = Start-Process -FilePath $qemu.Source -ArgumentList $qemuArgumentLine -PassThru -WindowStyle Hidden
    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $serialText = ""
    while ($timer.Elapsed.TotalSeconds -lt 10) {
        if (Test-Path -LiteralPath $serialPath) {
            try {
                $serialText = [System.IO.File]::ReadAllText($serialPath)
            }
            catch {
                $serialText = ""
            }
            $normalized = (([string]$serialText -replace "`r`n", "`n") -replace "`r", "`n").Trim()
            if ($normalized.Contains("DBYTE NATIVE KERNEL") -and $normalized.Contains("MACHINE CODE ONLINE") -and $normalized.Contains("42")) {
                break
            }
        }
        if ($qemuProcess.HasExited) {
            break
        }
        Start-Sleep -Milliseconds 100
    }
}
finally {
    if ($null -ne $qemuProcess -and -not $qemuProcess.HasExited) {
        Stop-Process -Id $qemuProcess.Id -Force
        $qemuProcess.WaitForExit()
    }
}

Assert-True (Test-Path -LiteralPath $serialPath) "QEMU did not create the native serial log"
[byte[]]$serialBytes = [System.IO.File]::ReadAllBytes($serialPath)
[byte[]]$expectedSerialBytes = [System.Text.Encoding]::ASCII.GetBytes("DBYTE NATIVE KERNEL`r`nMACHINE CODE ONLINE`r`n42`r`n")
$serialHex = ($serialBytes | ForEach-Object { $_.ToString("X2") }) -join "-"
$expectedSerialHex = ($expectedSerialBytes | ForEach-Object { $_.ToString("X2") }) -join "-"
Assert-True ($serialHex -eq $expectedSerialHex) "native serial bytes mismatch: $serialHex"
$serialText = [System.Text.Encoding]::ASCII.GetString($serialBytes)
$serialNormalized = ($serialText -replace "`r`n", "`n").TrimEnd("`n")

Write-Host "native compiler tests passed"
Write-Host "deterministic SHA256 A: $hashA"
Write-Host "deterministic SHA256 B: $hashB"
Write-Host "integrated CLI SHA256: $hashCli"
Write-Host "generated ELF SHA256: $hashA"
Write-Host "QEMU command: `"$($qemu.Source)`" $qemuArgumentLine"
Write-Host "serial bytes: $serialHex"
Write-Host "serial proof: $($serialNormalized -replace "`n", ' | ')"
Write-Host "native verify passed"
