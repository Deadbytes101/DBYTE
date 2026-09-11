$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'verify_git_boundary.ps1')
$root = Join-Path $repo ('tmp\native_explicit_float\float-' + [Guid]::NewGuid().ToString('N'))
$compiler = Join-Path $repo 'target\debug\dbytec.exe'
$sourceRel = 'examples/native_v13_machine/explicit_float_native.dby'
$abiDbyteRel = 'examples/native_v13_machine/explicit_float_c_abi.dby'
$abiCRel = 'examples/native_v13_machine/explicit_float_c_abi.c'
$clang = 'C:\Program Files\LLVM\bin\clang.exe'
$llvmReadObj = 'C:\Program Files\LLVM\bin\llvm-readobj.exe'
$script:sequence = 0
New-Item -ItemType Directory -Path $root -Force | Out-Null

function Assert-True($value, [string]$message) {
    if (-not $value) { throw "$message; evidence: $root" }
}
function Hash([string]$path) {
    return (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Save-Command([string]$name,[string]$exe,[string[]]$arguments,[int]$code,[string]$text) {
    $script:sequence++
    $log = Join-Path $root ('command-{0:D3}-{1}.txt' -f $script:sequence,$name)
    [IO.File]::WriteAllText($log,"COMMAND=$exe $($arguments -join ' ')`nEXIT=$code`n$text",[Text.UTF8Encoding]::new($false))
    return $log
}function Run([string]$exe,[string[]]$arguments,[string]$name) {
    $saved = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { $output = & $exe @arguments 2>&1; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    $text = $output -join "`n"; $log = Save-Command $name $exe $arguments $code $text
    if ($code -ne 0) { throw "$name failed ($code): $log" }
    return $text
}
function Run-Exit([string]$exe,[string[]]$arguments,[int]$expected,[string]$name) {
    $saved = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { $output = & $exe @arguments 2>&1; $code = $LASTEXITCODE }
    finally { $ErrorActionPreference = $saved }
    $text = $output -join "`n"; $log = Save-Command $name $exe $arguments $code $text
    Assert-True ($code -eq $expected) "$name exit $code != $expected; log=$log"
    return [pscustomobject]@{ code=$code; log=$log; text=$text }
}
function Linux-Path([string]$path) {
    return (Run 'wsl.exe' @('-d','Debian','--','wslpath','-a','-u',$path.Replace('\','/')) 'wslpath').Trim()
}
function Build-Own([string]$platform,[string]$target,[string]$emit,[string]$opt) {
    $dir = Join-Path $root "$platform-o$opt"; New-Item -ItemType Directory -Path $dir -Force | Out-Null
    $ext = if ($emit -ceq 'exe') { 'exe' } else { 'elf' }
    $a = Join-Path $dir "a.$ext"; $b = Join-Path $dir "b.$ext"
    $dump = Run $compiler @('build',$sourceRel,'--target',$target,'--emit',$emit,'--opt-level',$opt,'-o',$a,'--dump-native-mir','--dump-machine-code') "$platform-o$opt-a"
    Run $compiler @('build',$sourceRel,'--target',$target,'--emit',$emit,'--opt-level',$opt,'-o',$b) "$platform-o$opt-b" | Out-Null
    $ha=Hash $a; $hb=Hash $b; Assert-True ($ha -ceq $hb) "$platform O$opt artifact is not deterministic"
    Assert-True ($dump.Contains('F32')) "$platform O$opt native MIR lost F32"
    Assert-True ($dump.Contains('F64')) "$platform O$opt native MIR lost F64"
    [IO.File]::WriteAllText((Join-Path $dir 'native-dump.txt'),$dump,[Text.UTF8Encoding]::new($false))
    $disasm = Run 'objdump.exe' @('-d','-Mintel',$a) "$platform-o$opt-disasm"
    Assert-True ([regex]::IsMatch($disasm,'(?i)\b(addss|addsd|divss|divsd|ucomiss|ucomisd|movd|movq)\b')) "$platform O$opt has no scalar SSE2 evidence"
    [IO.File]::WriteAllText((Join-Path $dir 'objdump.txt'),$disasm,[Text.UTF8Encoding]::new($false))
    if ($platform -ceq 'linux') {
        $run = Run-Exit 'wsl.exe' @('-d','Debian','--',(Linux-Path $a)) 42 "run-linux-o$opt"
    } else {
        $run = Run-Exit $a @() 42 "run-windows-o$opt"
    }
    return [pscustomobject]@{ cell="$platform-o$opt"; sha256=$ha; duplicate_sha256=$hb; exit_code=$run.code }
}
function Build-CAbi([string]$platform,[string]$target,[string]$opt,[string]$cObject) {
    $dir=Join-Path $root "cabi-$platform-o$opt"; New-Item -ItemType Directory -Path $dir -Force | Out-Null
    $dobj=Join-Path $dir 'dbyte.obj'; $exe=Join-Path $dir $(if($platform -ceq 'windows'){'interop.exe'}else{'interop.elf'})
    Run $compiler @('build',$abiDbyteRel,'--target',$target,'--emit','obj','--opt-level',$opt,'-o',$dobj,'--dump-native-mir') "cabi-$platform-o$opt-build" | Out-Null
    Run $compiler @('link','--target',$target,$dobj,$cObject,'-o',$exe) "cabi-$platform-o$opt-link" | Out-Null
    if($platform -ceq 'linux'){$run=Run-Exit 'wsl.exe' @('-d','Debian','--',(Linux-Path $exe)) 42 "cabi-linux-o$opt-run"}
    else{$run=Run-Exit $exe @() 42 "cabi-windows-o$opt-run"}
    return [pscustomobject]@{ cell="cabi-$platform-o$opt"; exit_code=$run.code; artifact_sha256=Hash $exe }
}
Push-Location $repo
try {
    Assert-True (Test-Path $clang) 'pinned LLVM clang is unavailable'
    Assert-True (Test-Path $llvmReadObj) 'llvm-readobj is unavailable'
    foreach($rel in @($sourceRel,$abiDbyteRel,$abiCRel)){Assert-True (Test-Path (Join-Path $repo $rel)) "missing fixture $rel"}
    Run 'cargo.exe' @('build','-q','-p','dbyte_cli','--bin','dbytec') 'build-dbytec' | Out-Null
    $head=(Run 'git.exe' @('rev-parse','HEAD') 'git-head').Trim()
    Run 'cargo.exe' @('test','-q','-p','dbyte_hir','explicit_float_arithmetic_is_canonical_and_rejects_integer_only_operators') 'hir-float' | Out-Null
    Run 'cargo.exe' @('test','-q','-p','dbyte_interp','hosted_explicit_float_arithmetic_preserves_ieee_results') 'hosted-float' | Out-Null
    Run 'cargo.exe' @('test','-q','-p','dbyte_vm','explicit_float_arithmetic_matches_ieee_bits_at_o0_and_o1') 'vm-float' | Out-Null
    Run 'cargo.exe' @('test','-q','-p','dbyte_native','explicit_float_c_abi_preserves_float_machine_types') 'native-mir-float' | Out-Null
    Run 'cargo.exe' @('test','-q','-p','dbyte_native','scalar_sse2_encodings_are_exact_and_transactional') 'sse2-encoder' | Out-Null
    Run 'cargo.exe' @('test','-q','-p','dbyte_native','win64_synthesizes_fltused_for_freestanding_float_objects') 'win64-fltused' | Out-Null

    $native=@()
    foreach($opt in @('0','1')){
        $native += Build-Own 'linux' 'x86_64-dbyte-linux' 'elf' $opt
        $native += Build-Own 'windows' 'x86_64-dbyte-windows' 'exe' $opt
    }
    $cLinux=Join-Path $root 'c-linux-o2.o'; $cWindows=Join-Path $root 'c-windows-o2.obj'
    Run $clang @('--target=x86_64-none-elf','-O2','-fno-omit-frame-pointer','-ffreestanding','-fno-pic','-fno-pie','-fno-stack-protector','-fno-builtin','-c',$abiCRel,'-o',$cLinux) 'clang-linux-o2' | Out-Null
    Run $clang @('--target=x86_64-pc-windows-msvc','-O2','-fno-omit-frame-pointer','-ffreestanding','-fno-stack-protector','-fno-builtin','-c',$abiCRel,'-o',$cWindows) 'clang-windows-o2' | Out-Null
    $linuxInspect=Run $llvmReadObj @('--sections','--symbols','--relocations',$cLinux) 'inspect-linux-c-o2'
    Assert-True ($linuxInspect.Contains('.rodata.cst4')) 'Linux O2 fixture did not exercise .rodata.cst4 importer'
    Assert-True ($linuxInspect.Contains('.rodata.cst8')) 'Linux O2 fixture did not exercise .rodata.cst8 importer'
    Assert-True ($linuxInspect.Contains('.LCPI')) 'Linux O2 fixture lost local constant symbols'
    $windowsInspect=Run $llvmReadObj @('--sections','--symbols','--relocations',$cWindows) 'inspect-windows-c-o2'
    Assert-True ($windowsInspect.Contains('_fltused')) 'Win64 O2 fixture did not request _fltused'
    Assert-True ($windowsInspect.Contains('__real@')) 'Win64 O2 fixture did not exercise floating COMDAT constants'
    $rdataCount=([regex]::Matches($windowsInspect,'(?m)^\s*Name: \.rdata(?:\s|\()')).Count
    Assert-True ($rdataCount -ge 2) "Win64 O2 fixture did not contain duplicate .rdata COMDATs ($rdataCount)"
    [IO.File]::WriteAllText((Join-Path $root 'linux-c-o2-inspection.txt'),$linuxInspect,[Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $root 'windows-c-o2-inspection.txt'),$windowsInspect,[Text.UTF8Encoding]::new($false))

    $cabi=@()
    foreach($opt in @('0','1')){
        $cabi += Build-CAbi 'linux' 'x86_64-dbyte-linux' $opt $cLinux
        $cabi += Build-CAbi 'windows' 'x86_64-dbyte-windows' $opt $cWindows
    }
    Assert-True ($native.Count -eq 4) "expected four own-native cells, got $($native.Count)"
    Assert-True ($cabi.Count -eq 4) "expected four C ABI cells, got $($cabi.Count)"
    $inputs=@($sourceRel,$abiDbyteRel,$abiCRel,'crates/dbyte_native/src/linker64.rs','crates/dbyte_native/src/external_elf64.rs','crates/dbyte_native/src/external_coff64.rs') | ForEach-Object {
        $p=Join-Path $repo $_; [pscustomobject]@{path=$_;sha256=Hash $p;bytes=(Get-Item $p).Length}
    }
    $status=Run 'git.exe' @('status','--porcelain=v1','--untracked-files=all') 'git-status'
    $result=[ordered]@{
        schema=1; status='PASS'; subject='native-explicit-float-sse2-c-abi'
        generated_utc=[DateTime]::UtcNow.ToString('o'); head=$head
        native_cells=$native; c_abi_cells=$cabi; input_hashes=$inputs
        clang_linux_o2_sha256=Hash $cLinux; clang_windows_o2_sha256=Hash $cWindows
        candidate_dirty=-not [String]::IsNullOrWhiteSpace($status)
        real_biostar='PENDING'; real_storage='DISARMED'
    }
    $resultPath=Join-Path $root 'RESULT.json'
    [IO.File]::WriteAllText($resultPath,($result|ConvertTo-Json -Depth 8)+"`n",[Text.UTF8Encoding]::new($false))
    Write-Output 'DBYTE NATIVE EXPLICIT FLOAT VERIFIER: PASS'
    Write-Output "Native cells: $($native.Count)/4; C ABI cells: $($cabi.Count)/4"
    Write-Output "Evidence root: $root"
}
catch {
    $failure=[ordered]@{schema=1;status='FAIL';subject='native-explicit-float-sse2-c-abi';generated_utc=[DateTime]::UtcNow.ToString('o');error=$_.Exception.Message;real_biostar='PENDING';real_storage='DISARMED'}
    [IO.File]::WriteAllText((Join-Path $root 'RESULT.json'),($failure|ConvertTo-Json -Depth 5)+"`n",[Text.UTF8Encoding]::new($false))
    throw
}
finally { Pop-Location }
$global:LASTEXITCODE=0