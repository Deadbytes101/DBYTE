$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'v13_release_identity.ps1')
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$root = Join-Path $repo ('tmp/v13_bitcast/bitcast-' + [Guid]::NewGuid().ToString('N'))
$compiler = Join-Path $repo 'target/debug/dbytec.exe'
$runner = Join-Path $repo 'target/debug/dbyte.exe'
$qemu = 'C:/Program Files/qemu/qemu-system-x86_64.exe'
$expectedHead = Get-V13ExpectedHead
$ownedQemu = [System.Collections.Generic.List[int]]::new()
$script:sequence = 0
New-Item -ItemType Directory -Path $root -Force | Out-Null

function Assert-True($value, [string]$message) {
    if (-not $value) { throw "$message; evidence: $root" }
}
function Hash([string]$path) {
    return (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
}
function Run([string]$exe, [string[]]$arguments, [string]$description) {
    $script:sequence++
    $log = Join-Path $root ('command-{0:D3}.txt' -f $script:sequence)
    $saved = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { $output = & $exe @arguments 2>&1; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    [IO.File]::WriteAllText($log, "COMMAND=$exe $($arguments -join ' ')`nEXIT=$code`n$($output -join "`n")")
    if ($code -ne 0) { throw "$description failed ($code): $log" }
    return ($output -join "`n")
}function LinuxPath([string]$path) {
    return (Run 'wsl.exe' @('-d','Debian','--','wslpath','-a','-u',$path.Replace('\','/')) 'translate WSL path').Trim()
}
function Read-SharedText([string]$path) {
    $stream = [IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
    try {
        $reader = [IO.StreamReader]::new($stream,[Text.Encoding]::ASCII,$false,1024,$true)
        try { return $reader.ReadToEnd() } finally { $reader.Dispose() }
    } finally { $stream.Dispose() }
}
function Run-LinuxBitcast([string]$opt) {
    $dir = Join-Path $root "linux-o$opt"
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    $elf = Join-Path $dir 'bitcast.elf'
    $dump = Run $compiler @('build','examples/native_v13_machine/bitcast_linux.dby','--target','x86_64-dbyte-linux','--opt-level',$opt,'--emit','elf','-o',$elf,'--dump-native-mir') "build Linux bitcast O$opt"
    [IO.File]::WriteAllText((Join-Path $dir 'native-mir.txt'),$dump)
    Assert-True ($dump.Contains('native-mir pointer-bytes=8')) "Linux O$opt lost x86_64 pointer width"
    Assert-True ($dump.Contains('Copy {')) "Linux O$opt contains no raw-copy MIR evidence"
    $linux = LinuxPath $elf
    Run 'wsl.exe' @('-d','Debian','--','chmod','+x',$linux) "chmod Linux bitcast O$opt" | Out-Null
    $saved = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { & wsl.exe -d Debian -- $linux | Out-Null; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    Add-Content -LiteralPath (Join-Path $root 'runtime.txt') -Value "Linux O$opt EXIT=$code"
    Assert-True ($code -eq 0) "Linux O$opt bitcast runtime failed with exit $code"
    $disasm = Run 'objdump.exe' @('-d','-Mintel',$elf) "disassemble Linux bitcast O$opt"
    [IO.File]::WriteAllText((Join-Path $dir 'objdump.txt'),$disasm)
    $conversion = '(?im)\b(?:cvtss2sd|cvtsd2ss|cvtsi2ss|cvtsi2sd|cvtss2si|cvtsd2si|cvttss2si|cvttsd2si)\b'
    Assert-True (-not [regex]::IsMatch($disasm,$conversion)) "Linux O$opt emitted numeric FP conversion"
    Assert-True (-not $disasm.Contains('xmm')) "Linux O$opt unexpectedly uses XMM registers for raw bitcasts"
    return [pscustomobject]@{ target='x86_64-dbyte-linux'; opt=$opt; elf=$elf; elf_sha256=Hash $elf }
}
function Build-None([string]$set, [string]$target, [string]$opt) {
    $dir = Join-Path $root "$target-$set-o$opt"
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    $elf = Join-Path $dir 'kernel.elf'
    $dump = Run $compiler @('build','examples/native_v13_machine/bitcast_kernel.dby','--target',$target,'--opt-level',$opt,'--emit','elf','-o',$elf,'--dump-native-mir') "build $target $set O$opt"
    [IO.File]::WriteAllText((Join-Path $dir 'native-mir.txt'),$dump)
    $pointerBytes = if ($target -ceq 'i686-dbyte-none') { '4' } else { '8' }
    Assert-True ($dump.Contains("native-mir pointer-bytes=$pointerBytes")) "$target $set O$opt pointer width drifted"
    $transport = if ($target -ceq 'i686-dbyte-none') { 'multiboot' } else { 'multiboot2' }
    $linux = LinuxPath $elf
    Run 'wsl.exe' @('-d','Debian','--','grub-file',"--is-x86-$transport",$linux) "grub-file $target $set O$opt" | Out-Null
    return [pscustomobject]@{ set=$set; target=$target; opt=$opt; dir=$dir; elf=$elf; transport=$transport; elf_sha256=Hash $elf }
}
function Make-Iso($build, [string]$date) {
    $isoRoot = Join-Path $build.dir 'iso'
    $grub = Join-Path $isoRoot 'boot/grub'
    New-Item -ItemType Directory -Path $grub -Force | Out-Null
    $copy = Join-Path $isoRoot 'boot/kernel.elf'
    Copy-Item -LiteralPath $build.elf -Destination $copy -Force
    Assert-True ((Hash $copy) -ceq $build.elf_sha256) "$($build.target) $($build.set) O$($build.opt) ISO kernel copy drifted"
    [IO.File]::WriteAllText((Join-Path $grub 'grub.cfg'),"set timeout=0`nset default=0`nmenuentry 'DByte v13 bitcast' {`n $($build.transport) /boot/kernel.elf`n boot`n}`n")
    $iso = Join-Path $build.dir 'bitcast.iso'
    $isoLinux = LinuxPath $iso
    $rootLinux = LinuxPath $isoRoot
    Run 'wsl.exe' @('-d','Debian','--','grub-mkrescue','-o',$isoLinux,("--modification-date=$date"),'--set_all_file_dates',$date,$rootLinux) "build deterministic ISO $($build.target) $($build.set) O$($build.opt)" | Out-Null
    return [pscustomobject]@{ build=$build; path=$iso; sha256=Hash $iso; bytes=(Get-Item $iso).Length }
}
function Run-Qemu($iso) {
    $build = $iso.build
    $dir = Join-Path $root "run-$($build.target)-o$($build.opt)"
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    $serial = Join-Path $dir 'serial.log'
    $trace = Join-Path $dir 'trace.log'
    $arguments = @('-cdrom',$iso.path,'-m','64M','-display','none','-monitor','none','-serial',"file:$serial",'-d','int,cpu_reset,guest_errors','-D',$trace,'-no-reboot','-no-shutdown')
    [IO.File]::WriteAllText((Join-Path $dir 'qemu-command.txt'),"$qemu $($arguments -join ' ')")
    $process = Start-Process -FilePath $qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    $ownedQemu.Add($process.Id)
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(15)
        $text = ''; $complete = $false
        while (-not $process.HasExited -and [DateTime]::UtcNow -lt $deadline) {
            Start-Sleep -Milliseconds 100
            if (Test-Path $serial) {
                $text = Read-SharedText $serial
                if ($text.Contains('BITCAST FAIL') -or $text -ceq "BITCAST OK`r`n") { $complete = $true; break }
            }
        }
        Assert-True $complete "$($build.target) O$($build.opt) QEMU timed out"
    } finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
        $process.Dispose()
    }
    Assert-True ($text -ceq "BITCAST OK`r`n") "$($build.target) O$($build.opt) serial mismatch: '$text'"
    $traceText = [IO.File]::ReadAllText($trace)
    $triple = ([regex]::Matches($traceText,'Triple fault')).Count
    $faults = ([regex]::Matches($traceText,'v=0[0de6]')).Count
    Assert-True ($triple -eq 0) "$($build.target) O$($build.opt) triple fault"
    Assert-True ($faults -eq 0) "$($build.target) O$($build.opt) #UD/#GP/#PF observed"
    return [pscustomobject]@{
        target=$build.target; opt=$build.opt; elf_sha256=$build.elf_sha256
        iso_sha256=$iso.sha256; serial_sha256=Hash $serial; trace_sha256=Hash $trace
        faults=$faults; triple_faults=$triple
    }
}
function Write-InputHashes {
    $paths = @(
        'crates/dbyte_ast/src/lib.rs','crates/dbyte_parser/src/lib.rs',
        'crates/dbyte_hir/src/lib.rs','crates/dbyte_hir/src/executable.rs','crates/dbyte_hir/src/project.rs',
        'crates/dbyte_typeck/src/lib.rs','crates/dbyte_interp/src/concrete.rs','crates/dbyte_interp/src/lib.rs',
        'crates/dbyte_bytecode/src/canonical.rs','crates/dbyte_vm/src/canonical.rs',
        'crates/dbyte_native/src/native_mir_lower.rs','crates/dbyte_native/src/native_select_i686.rs',
        'crates/dbyte_native/src/native_select_x86_64.rs','crates/dbyte_native/src/modern_native.rs',
        'examples/native_v13_machine/bitcast_core.dby','examples/native_v13_machine/bitcast_hosted.dby',
        'examples/native_v13_machine/bitcast_linux.dby','examples/native_v13_machine/bitcast_kernel.dby'
    )
    $hashes = @($paths | ForEach-Object {
        $item = Get-Item -LiteralPath (Join-Path $repo $_)
        [pscustomobject]@{ path=$_; bytes=$item.Length; sha256=Hash $item.FullName }
    })
    $hashes | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $root 'input-hashes.json')
}
Push-Location $repo
try {
    $head = (Run 'git.exe' @('rev-parse','HEAD') 'read HEAD').Trim()
    Assert-True ($head -ceq $expectedHead) 'unexpected HEAD'
    $tag = (Run 'git.exe' @('rev-parse','v12.4.0^{}') 'verify frozen tag').Trim()
    Assert-True ($tag -ceq $V13FrozenBaseHead) 'v12.4.0 peeled tag drifted'
    Run 'cargo.exe' @('build','-q','-p','dbyte_cli','--bins') 'build DByte CLI binaries' | Out-Null
    Run 'cargo.exe' @('test','-q','-p','dbyte_hir','explicit_float_fields_follow_target_native_layout','--','--nocapture') 'HIR explicit-float layout regression' | Out-Null
    Run 'cargo.exe' @('test','-q','-p','dbyte_hir','exact_bitcasts_admit_only_equal_width_integer_and_explicit_float_representations','--','--nocapture') 'HIR exact-bitcast legality regression' | Out-Null
    Run 'cargo.exe' @('test','-q','-p','dbyte_native','exact_bitcast_is_a_same_width_copy_not_a_numeric_cast','--','--nocapture') 'Native MIR exact-bitcast copy regression' | Out-Null
    Run 'cargo.exe' @('test','-q','-p','dbyte_native','explicit_float_c_abi_fails_closed_until_float_register_abi_exists','--','--nocapture') 'Native C float ABI fail-closed regression' | Out-Null
    $hosted = Run $runner @('run','examples/native_v13_machine/bitcast_hosted.dby') 'hosted bitcast corpus'
    Assert-True ($hosted.Trim() -ceq '0') "hosted bitcast corpus expected 0, got '$hosted'"
    $vm = Run $runner @('run','examples/native_v13_machine/bitcast_hosted.dby','--vm') 'VM bitcast corpus'
    Assert-True ($vm.Trim() -ceq '0') "VM bitcast corpus expected 0, got '$vm'"
    $linux = @(Run-LinuxBitcast '0'; Run-LinuxBitcast '1')
    $epoch = [int64](Run 'git.exe' @('show','-s','--format=%ct','HEAD') 'read deterministic ISO epoch').Trim()
    $isoDate = [DateTimeOffset]::FromUnixTimeSeconds($epoch).UtcDateTime.ToString('yyyyMMddHHmmss00',[Globalization.CultureInfo]::InvariantCulture)
    $builds = @()
    $isos = @()
    foreach ($target in @('i686-dbyte-none','x86_64-dbyte-none')) {
        foreach ($opt in @('0','1')) {
            $a = Build-None 'A' $target $opt
            $b = Build-None 'B' $target $opt
            Assert-True ($a.elf_sha256 -ceq $b.elf_sha256) "$target O$opt ELF is not deterministic"
            $ia = Make-Iso $a $isoDate
            $ib = Make-Iso $b $isoDate
            Assert-True ($ia.sha256 -ceq $ib.sha256) "$target O$opt ISO is not deterministic"
            $builds += @($a,$b)
            $isos += @($ia,$ib)
        }
    }
    $receipts = @()
    foreach ($target in @('i686-dbyte-none','x86_64-dbyte-none')) {
        foreach ($opt in @('0','1')) {
            $selected = $isos | Where-Object { $_.build.set -ceq 'A' -and $_.build.target -ceq $target -and $_.build.opt -ceq $opt }
            $receipts += Run-Qemu $selected
        }
    }
    $linux | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $root 'linux-results.json')
    $receipts | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $root 'qemu-results.json')
    Write-InputHashes
    $artifacts = @(Get-ChildItem -LiteralPath $root -Recurse -File | Where-Object { $_.Extension -in @('.elf','.iso') } | ForEach-Object {
        [pscustomobject]@{ path=$_.FullName.Substring($root.Length+1).Replace('\','/'); bytes=$_.Length; sha256=Hash $_.FullName }
    })
    $artifacts | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $root 'artifact-hashes.json')
    $result = [pscustomobject]@{
        status='PASS'; subject='v13-exact-bitcast'
        head=$head; deterministic_iso_date=$isoDate
        hosted='PASS'; vm='PASS'; linux_o0='PASS'; linux_o1='PASS'
        i686_o0='PASS'; i686_o1='PASS'; x86_64_none_o0='PASS'; x86_64_none_o1='PASS'
        qemu_receipts=$receipts.Count; real_biostar='PENDING'; real_storage='DISARMED'; v13_final='NOT_CREATED'
    }
    $result | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $root 'RESULT.json')
    Write-Output "V13 EXACT BITCAST VERIFIER: PASS"
    Write-Output "EVIDENCE ROOT: $root"
} finally {
    Pop-Location
    $deadline = [DateTime]::UtcNow.AddSeconds(3)
    do {
        $orphans = @($ownedQemu | Where-Object { $candidate=Get-CimInstance Win32_Process -Filter ("ProcessId = {0}" -f $_) -ErrorAction SilentlyContinue; $null -ne $candidate -and $candidate.Name -ieq [IO.Path]::GetFileName($qemu) -and $candidate.CommandLine -like ("*" + $root + "*") })
        if ($orphans.Count -eq 0 -or [DateTime]::UtcNow -ge $deadline) { break }
        Start-Sleep -Milliseconds 100
    } while ($true)
    if ($orphans.Count -ne 0) { throw "bitcast verifier left owned QEMU processes: $($orphans -join ', ')" }
}
