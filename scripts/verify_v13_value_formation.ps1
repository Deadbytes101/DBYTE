$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$root = Join-Path $repo ('tmp/v13_g1/value-' + [Guid]::NewGuid().ToString('N'))
$compiler = Join-Path $repo 'target/debug/dbytec.exe'
$runner = Join-Path $repo 'target/debug/dbyte.exe'
$qemu = 'C:/Program Files/qemu/qemu-system-i386.exe'
$ownedQemu = [System.Collections.Generic.List[int]]::new()
New-Item -ItemType Directory -Path $root -Force | Out-Null

function Assert-True($value, [string]$message) { if (-not $value) { throw $message } }
function Run([string]$exe, [string[]]$arguments, [string]$description) {
    $saved = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { $output = & $exe @arguments 2>&1; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    Add-Content -LiteralPath (Join-Path $root 'commands.txt') -Value "$exe $($arguments -join ' ')`nEXIT=$code`n$($output -join "`n")"
    if ($code -ne 0) { throw "$description failed ($code); evidence: $root" }
    return ($output -join "`n")
}
function LinuxPath([string]$path) {
    return (Run 'wsl.exe' @('-d','Debian','--','wslpath','-a','-u',$path.Replace('\','/')) 'translate WSL path').Trim()
}
function Read-SharedText([string]$path) {
    $stream = [IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
    try { $reader = [IO.StreamReader]::new($stream); try { return $reader.ReadToEnd() } finally { $reader.Dispose() } }
    finally { $stream.Dispose() }
}
function Run-I686([string]$elf, [string]$label) {
    $serial = Join-Path $root "$label-serial.log"
    $trace = Join-Path $root "$label-trace.log"
    $arguments = @('-kernel',$elf,'-m','32M','-display','none','-monitor','none','-serial',"file:$serial",'-d','int,cpu_reset,guest_errors','-D',$trace,'-no-reboot','-no-shutdown')
    Add-Content -LiteralPath (Join-Path $root 'commands.txt') -Value "$qemu $($arguments -join ' ')"
    $process = Start-Process -FilePath $qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    $ownedQemu.Add($process.Id)
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(12); $complete = $false
        while (-not $process.HasExited -and [DateTime]::UtcNow -lt $deadline) {
            Start-Sleep -Milliseconds 100
            if (Test-Path $serial) {
                $actual = Read-SharedText $serial
                Assert-True ("I686 WIDTH OK`r`n".StartsWith($actual,[StringComparison]::Ordinal)) "$label serial diverged"
                if ($actual -ceq "I686 WIDTH OK`r`n") { $complete = $true; break }
            }
        }
        Assert-True $complete "$label timed out"
    } finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
        $process.Dispose()
    }
    $faults = [IO.File]::ReadAllText($trace)
    Assert-True (-not $faults.Contains('Triple fault')) "$label triple fault"
    Assert-True (([regex]::Matches($faults,'v=0[0de6]')).Count -eq 0) "$label unexpected CPU fault"
}
Push-Location $repo
try {
    Run 'cargo.exe' @('build','-q','-p','dbyte_cli','--bins') 'build DByte CLI binaries' | Out-Null
    Run 'cargo.exe' @('test','-q','-p','dbyte_native','x64_usize_literal_keeps_target_width_before_optimization','--','--nocapture') 'run x64 usize lowering regression' | Out-Null

    $hostOutput = Run $runner @('run','examples/native_v13_machine/value_hosted.dby') 'hosted value formation'
    Assert-True ($hostOutput.Trim() -ceq '42') "hosted value formation expected 42, got '$hostOutput'"
    $vm = Run $runner @('run','examples/native_v13_machine/value_hosted.dby','--vm') 'VM value formation'
    Assert-True ($vm.Trim() -ceq '42') "VM value formation expected 42, got '$vm'"

    foreach ($opt in @('0','1')) {
        $elf = Join-Path $root "value-x64-o$opt.elf"
        $dump = Join-Path $root "value-x64-o$opt.dump.txt"
        $output = Run $compiler @('build','examples/native_v13_machine/value_native.dby','--target','x86_64-dbyte-linux','--opt-level',$opt,'--emit','elf','-o',$elf,'--dump-ir','--dump-native-mir','--dump-machine-code') "build x64 value O$opt"
        [IO.File]::WriteAllText($dump,$output)
        Assert-True ($output.Contains('native-mir pointer-bytes=8')) "x64 O$opt dump lost pointer width"
        Assert-True ($output.Contains('value: 4294967296')) "x64 O$opt MIR lost 0x100000000"
        if ($opt -ceq '0') { Assert-True ($output.Contains('value: 4294967296, ty: Pointer')) 'x64 O0 did not target-type usize literal before optimization' }
        $linux = LinuxPath $elf
        Run 'wsl.exe' @('-d','Debian','--','chmod','+x',$linux) "chmod x64 O$opt" | Out-Null
        & wsl.exe -d Debian -- $linux | Out-Null
        $nativeExit = $LASTEXITCODE
        Add-Content -LiteralPath (Join-Path $root 'commands.txt') -Value "wsl.exe -d Debian -- $linux`nEXIT=$nativeExit"
        Assert-True ($nativeExit -eq 42) "x64 O$opt runtime exit expected 42, got $nativeExit"
        $disasm = Run 'objdump.exe' @('-d','-Mintel',$elf) "disassemble x64 O$opt"
        [IO.File]::WriteAllText((Join-Path $root "value-x64-o$opt.objdump.txt"),$disasm)
        Assert-True ($disasm.Contains('0x100000000')) "x64 O$opt machine code lacks 0x100000000"
    }

    foreach ($opt in @('0','1')) {
        $elf = Join-Path $root "value-i686-o$opt.elf"
        $dump = Run $compiler @('build','examples/native_v13_machine/value_i686.dby','--target','i686-dbyte-none','--opt-level',$opt,'--emit','elf','-o',$elf,'--dump-native-mir') "build i686 value O$opt"
        [IO.File]::WriteAllText((Join-Path $root "value-i686-o$opt.mir.txt"),$dump)
        Assert-True ($dump.Contains('native-mir pointer-bytes=4')) "i686 O$opt dump lost pointer width"
        Run-I686 $elf "i686-o$opt"
    }

    $inputs = @(
        'examples/native_v13_machine/value_formation.dby',
        'examples/native_v13_machine/value_hosted.dby',
        'examples/native_v13_machine/value_native.dby',
        'examples/native_v13_machine/value_i686.dby',
        'crates/dbyte_native/src/native_mir_lower.rs'
    ) | ForEach-Object { Get-Item -LiteralPath (Join-Path $repo $_) }
    $inputs | ForEach-Object { [pscustomobject]@{ path=$_.FullName; bytes=$_.Length; sha256=(Get-FileHash $_.FullName -Algorithm SHA256).Hash } } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'input-hashes.json')
    Get-ChildItem -LiteralPath $root -File -Filter '*.elf' | ForEach-Object { [pscustomobject]@{ path=$_.FullName; bytes=$_.Length; sha256=(Get-FileHash $_.FullName -Algorithm SHA256).Hash } } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'artifact-hashes.json')
    "V13 VALUE FORMATION PASS: $root"
} finally {
    Pop-Location
    $reapDeadline = [DateTime]::UtcNow.AddSeconds(2)
    do {
        $orphans = @($ownedQemu | Where-Object { $candidate=Get-CimInstance Win32_Process -Filter ("ProcessId = {0}" -f $_) -ErrorAction SilentlyContinue; $null -ne $candidate -and $candidate.Name -ieq [IO.Path]::GetFileName($qemu) -and $candidate.CommandLine -like ("*" + $root + "*") })
        if ($orphans.Count -eq 0 -or [DateTime]::UtcNow -ge $reapDeadline) { break }
        Start-Sleep -Milliseconds 100
    } while ($true)
    if ($orphans.Count -ne 0) { throw "value verifier left owned QEMU processes: $($orphans -join ', ')" }
}
