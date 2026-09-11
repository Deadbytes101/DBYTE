$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$root = Join-Path $repo ('tmp\portable_runtime_parity\parity-' + [Guid]::NewGuid().ToString('N'))
$sourceRel = 'examples/native_v13_machine/portable_runtime_parity.dby'
$source = Join-Path $repo $sourceRel
$compiler = Join-Path $repo 'target\debug\dbytec.exe'
$runner = Join-Path $repo 'target\debug\dbyte.exe'
$llvmReadObj = 'C:\Program Files\LLVM\bin\llvm-readobj.exe'
$script:sequence = 0
New-Item -ItemType Directory -Path $root -Force | Out-Null

function Assert-True($value, [string]$message) {
    if (-not $value) { throw "$message; evidence: $root" }
}
function Hash([string]$path) {
    return (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Save-Command([string]$name, [string]$exe, [string[]]$arguments, [int]$code, [string]$text) {
    $script:sequence++
    $log = Join-Path $root ('command-{0:D3}-{1}.txt' -f $script:sequence, $name)
    [IO.File]::WriteAllText($log, "COMMAND=$exe $($arguments -join ' ')`nEXIT=$code`n$text", [Text.UTF8Encoding]::new($false))
    return $log
}
function Run([string]$exe, [string[]]$arguments, [string]$name) {
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { $output = & $exe @arguments 2>&1; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    $text = $output -join "`n"
    $log = Save-Command $name $exe $arguments $code $text
    if ($code -ne 0) { throw "$name failed ($code): $log" }
    return $text
}
function Run-ExpectedExit([string]$exe, [string[]]$arguments, [int]$expected, [string]$name) {
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { $output = & $exe @arguments 2>&1; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    $text = $output -join "`n"
    $log = Save-Command $name $exe $arguments $code $text
    Assert-True ($code -eq $expected) "$name exit $code != $expected; log=$log"
    return [pscustomobject]@{ code=$code; text=$text; log=$log }
}
function Linux-Path([string]$path) {
    return (Run 'wsl.exe' @('-d','Debian','--','wslpath','-a','-u',$path.Replace('\','/')) 'wslpath').Trim()
}
function Build-Native([string]$platform, [string]$target, [string]$emit, [string]$opt) {
    $dir = Join-Path $root "$platform-o$opt"
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    $extension = if ($emit -ceq 'exe') { 'exe' } else { 'elf' }
    $a = Join-Path $dir "a.$extension"
    $b = Join-Path $dir "b.$extension"
    $dump = Run $compiler @('build',$sourceRel,'--target',$target,'--emit',$emit,'--opt-level',$opt,'-o',$a,'--dump-native-mir') "$platform-o$opt-a"
    Run $compiler @('build',$sourceRel,'--target',$target,'--emit',$emit,'--opt-level',$opt,'-o',$b) "$platform-o$opt-b" | Out-Null
    $aHash = Hash $a
    $bHash = Hash $b
    Assert-True ($aHash -ceq $bHash) "$platform O$opt artifact is not deterministic"
    Assert-True ($dump.Contains('native-mir pointer-bytes=8')) "$platform O$opt lost x86_64 native MIR"
    [IO.File]::WriteAllText((Join-Path $dir 'native-mir.txt'), $dump, [Text.UTF8Encoding]::new($false))
    return [pscustomobject]@{ platform=$platform; opt=$opt; path=$a; sha256=$aHash; duplicate_sha256=$bHash }
}
function Expect-PortableError([string]$name, [string]$text, [string]$code) {
    $dir = Join-Path $root 'negative'
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    $path = Join-Path $dir "$name.dby"
    [IO.File]::WriteAllText($path, $text, [Text.UTF8Encoding]::new($false))
    $result = Run-ExpectedExit $compiler @('check-portable',$path) 1 "negative-$name"
    Assert-True ($result.text.Contains($code)) "$name did not report $code"
    return [pscustomobject]@{ name=$name; diagnostic=$code; exit_code=$result.code; source_sha256=Hash $path }
}
Push-Location $repo
try {
    Assert-True (Test-Path -LiteralPath $source -PathType Leaf) 'portable parity source is missing'
    Assert-True (Test-Path -LiteralPath $llvmReadObj -PathType Leaf) 'llvm-readobj is unavailable'
    Run 'cargo.exe' @('build','-q','-p','dbyte_cli','--bins') 'build-cli' | Out-Null
    $head = (Run 'git.exe' @('rev-parse','HEAD') 'git-head').Trim()
    $sourceHash = Hash $source
    $portable = Run $compiler @('check-portable',$sourceRel) 'check-portable'
    Assert-True ($portable.Contains('DBYTE STRICT PORTABLE: PASS')) 'strict portable positive gate did not pass'
    Assert-True ($portable.Contains('profiles=10')) 'strict portable profile matrix is incomplete'

    $hosted = Run $runner @('run',$sourceRel) 'hosted'
    Assert-True ([String]::IsNullOrWhiteSpace($hosted)) 'hosted parity source produced output'
    $vmProof = Run 'cargo.exe' @('test','-q','-p','dbyte_vm','strict_portable_same_source_executes_at_o0_and_o1','--','--nocapture') 'vm-o0-o1'
    Assert-True ($vmProof.Contains('PORTABLE_VM_O0=PASS')) 'VM O0 parity marker missing'
    Assert-True ($vmProof.Contains('PORTABLE_VM_O1=PASS')) 'VM O1 parity marker missing'

    $native = @()
    foreach ($opt in @('0','1')) {
        $native += Build-Native 'linux' 'x86_64-dbyte-linux' 'elf' $opt
        $native += Build-Native 'windows' 'x86_64-dbyte-windows' 'exe' $opt
    }
    $cells = @(
        [pscustomobject]@{ cell='hosted'; status='PASS'; expected_exit=0; source_sha256=$sourceHash },
        [pscustomobject]@{ cell='vm-o0'; status='PASS'; expected_exit=0; source_sha256=$sourceHash },
        [pscustomobject]@{ cell='vm-o1'; status='PASS'; expected_exit=0; source_sha256=$sourceHash }
    )
    foreach ($build in $native) {
        if ($build.platform -ceq 'linux') {
            $linuxPath = Linux-Path $build.path
            $run = Run-ExpectedExit 'wsl.exe' @('-d','Debian','--',$linuxPath) 42 "run-linux-o$($build.opt)"
            $inspection = Run 'wsl.exe' @('-d','Debian','--','readelf','-h','-l','-d',$linuxPath) "readelf-linux-o$($build.opt)"
            Assert-True ($inspection.Contains('Advanced Micro Devices X86-64')) "Linux O$($build.opt) is not EM_X86_64"
            Assert-True (-not $inspection.Contains('INTERP')) "Linux O$($build.opt) has PT_INTERP"
            Assert-True ($inspection.Contains('There is no dynamic section')) "Linux O$($build.opt) has dynamic runtime state"
        } else {
            $run = Run-ExpectedExit $build.path @() 42 "run-windows-o$($build.opt)"
            $inspection = Run $llvmReadObj @('--file-headers','--coff-imports',$build.path) "inspect-windows-o$($build.opt)"
            Assert-True ($inspection.Contains('IMAGE_FILE_MACHINE_AMD64')) "Windows O$($build.opt) is not AMD64"
            $dlls = @([regex]::Matches($inspection,'Name: ([^\r\n]+\.dll)') | ForEach-Object { $_.Groups[1].Value })
            Assert-True (@($dlls | Where-Object { $_ -cne 'KERNEL32.dll' }).Count -eq 0) "Windows O$($build.opt) imports a non-KERNEL32 DLL"
        }
        $cells += [pscustomobject]@{ cell="$($build.platform)-o$($build.opt)"; status='PASS'; expected_exit=42; source_sha256=$sourceHash; artifact_sha256=$build.sha256 }
    }
    $negative = @()
    $negative += Expect-PortableError 'usize' "pub fn width(value: usize) -> usize:`n    return value`n" 'E_PORTABLE_TARGET_WIDTH'
    $negative += Expect-PortableError 'c-abi' "extern `"C`" fn c_add(left: i32, right: i32) -> i32`n" 'E_PORTABLE_C_ABI'
    $negative += Expect-PortableError 'repr-c' "@repr(C)`nstruct Pair:`n    left: u32`n    right: u32`n" 'E_PORTABLE_C_ABI'
    $negative += Expect-PortableError 'index' "pub fn first(values: array[u32, 2]) -> u32:`n    return values[0]`n" 'E_PORTABLE_INDEX_SEMANTICS'
    $negative += Expect-PortableError 'legacy-float' "pub fn bad() -> float:`n    return 1.5`n" 'E_PORTABLE_LEGACY_FLOAT'
    $negative += Expect-PortableError 'target-capability' "pub fn bad(port: u16, value: u8) -> unit:`n    native.out8(port, value)`n" 'E_PORTABLE_TARGET_CAPABILITY'
    $negative += Expect-PortableError 'pointer' "pub fn bad(value: ptr[u32]) -> u32:`n    return *value`n" 'E_PORTABLE_ADDRESS_REPRESENTATION'
    $negative += Expect-PortableError 'layout-observation' "struct Pair:`n    left: u32`n    right: u32`npub fn bad() -> u32:`n    return size_of(Pair) as u32`n" 'E_PORTABLE_TARGET_OBSERVATION'

    Assert-True ($cells.Count -eq 7) "expected seven runtime cells, observed $($cells.Count)"
    Assert-True ($negative.Count -eq 8) "expected eight negative contract probes, observed $($negative.Count)"
    $statusText = (Run 'git.exe' @('status','--porcelain=v1','--untracked-files=all') 'git-status')
    $receipt = [ordered]@{
        schema = 1
        contract = 'DByte strict portable same-source runtime parity'
        status = 'PASS'
        generated_utc = [DateTime]::UtcNow.ToString('o')
        head = $head
        source = $sourceRel
        source_sha256 = $sourceHash
        portable_report = $portable.Trim()
        runtime_cells = $cells
        negative_probes = $negative
        candidate_dirty = -not [String]::IsNullOrWhiteSpace($statusText)
        real_biostar = 'PENDING'
        real_storage = 'DISARMED'
    }
    $receiptPath = Join-Path $root 'RESULT.json'
    [IO.File]::WriteAllText($receiptPath, ($receipt | ConvertTo-Json -Depth 8) + "`n", [Text.UTF8Encoding]::new($false))
    Write-Output 'DBYTE STRICT PORTABLE RUNTIME PARITY: PASS'
    Write-Output "Cells: $($cells.Count)/7; negatives: $($negative.Count)/8"
    Write-Output "Source SHA-256: $sourceHash"
    Write-Output "Evidence root: $root"
}
finally {
    Pop-Location
}
