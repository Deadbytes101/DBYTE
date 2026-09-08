$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'v13_release_identity.ps1')
$expectedHead = Get-V13ExpectedHead
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$root=Join-Path $repo ('tmp/v13_g2/g2c-integration-'+[Guid]::NewGuid().ToString('N'))
$compiler=Join-Path $repo 'target/debug/dbytec.exe'
$qemu='C:/Program Files/qemu/qemu-system-x86_64.exe'
$owned=[System.Collections.Generic.List[int]]::new()
New-Item -ItemType Directory -Path $root -Force|Out-Null
function Assert-True($v,[string]$m){if(-not$v){throw $m}}
function Hash([string]$p){(Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash}
function Run([string]$exe,[string[]]$a,[string]$d){
 $save=$ErrorActionPreference;$ErrorActionPreference='Continue'
 try{Push-Location $repo;try{$o=& $exe @a 2>&1;$c=$LASTEXITCODE}finally{Pop-Location}}finally{$ErrorActionPreference=$save}
 Add-Content (Join-Path $root 'commands.txt') "$exe $($a -join ' ')`nEXIT=$c`n$($o -join "`n")"
 if($c-ne0){throw "$d failed ($c): $root"};return($o-join"`n")
}
function LinuxPath([string]$p){(Run 'wsl.exe' @('-d','Debian','--','wslpath','-a','-u',$p.Replace('\','/')) 'wslpath').Trim()}
function Shared([string]$p){$s=[IO.File]::Open($p,'Open','Read','ReadWrite');try{$r=[IO.StreamReader]::new($s);try{$r.ReadToEnd()}finally{$r.Dispose()}}finally{$s.Dispose()}}
function RunProof([string]$opt){
 $dir=Join-Path $root "o$opt";$grub=Join-Path $dir 'iso/boot/grub';New-Item -ItemType Directory -Path $grub -Force|Out-Null
 $elf=Join-Path $dir 'iso/boot/kernel.elf'
 Run $compiler @('build','examples/native_v13_machine/kernel_g2c.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','elf','-o',$elf) "build G2C O$opt"|Out-Null
 Run 'wsl.exe' @('-d','Debian','--','grub-file','--is-x86-multiboot2',(LinuxPath $elf)) "grub-file G2C O$opt"|Out-Null
 [IO.File]::WriteAllText((Join-Path $grub 'grub.cfg'),"set timeout=0`nset default=0`nmenuentry 'DByte v13 G2C memory' {`n multiboot2 /boot/kernel.elf`n boot`n}`n")
 $iso=Join-Path $dir 'g2c.iso';Run 'wsl.exe' @('-d','Debian','--','grub-mkrescue','-o',(LinuxPath $iso),(LinuxPath (Join-Path $dir 'iso'))) "ISO G2C O$opt"|Out-Null
 $runs=@()
 foreach($ram in @(32,64)){
  $runDir=Join-Path $dir "run-${ram}m";New-Item -ItemType Directory -Path $runDir -Force|Out-Null
  $serial=Join-Path $runDir 'serial.log';$trace=Join-Path $runDir 'trace.log'
  $a=@('-cdrom',$iso,'-m',"${ram}M",'-display','none','-monitor','none','-serial',"file:$serial",'-d','int,cpu_reset,guest_errors','-D',$trace,'-no-reboot','-no-shutdown')
  $p=Start-Process -FilePath $qemu -ArgumentList $a -PassThru -WindowStyle Hidden;$owned.Add($p.Id)
  try{$deadline=[DateTime]::UtcNow.AddSeconds(20);$text='';do{Start-Sleep -Milliseconds 100;if(Test-Path $serial){$text=Shared $serial;if($text.Contains('G2C COMPLETE')-or$text.Contains('G2C ERROR')){break}}}while(-not$p.HasExited-and[DateTime]::UtcNow-lt$deadline);Assert-True($text.Contains('G2C COMPLETE')-or$text.Contains('G2C ERROR'))"O$opt ${ram}M timed out before G2C terminal receipt"}
  finally{if(-not$p.HasExited){Stop-Process -Id $p.Id -Force;$p.WaitForExit()};$p.Dispose()}
  Assert-True(-not$text.Contains('G1 ERROR'))"O$opt ${ram}M G1 error"
  Assert-True(-not$text.Contains('G2 ERROR'))"O$opt ${ram}M G2 error"
  Assert-True(-not$text.Contains('G2B ERROR'))"O$opt ${ram}M G2B error"
  Assert-True(-not$text.Contains('G2B FATAL'))"O$opt ${ram}M fatal interrupt"
  Assert-True(-not$text.Contains('G2B PAGEFAULT'))"O$opt ${ram}M unexpected page fault"
  Assert-True(-not$text.Contains('G2C ERROR'))"O$opt ${ram}M G2C error"
  $frames=[regex]::Match($text,'FRAMES OK frame=([0-9A-F]{16}) alloc=([0-9A-F]{8}) owned=([0-9A-F]{8})')
  $paging=[regex]::Match($text,'PAGING OK cr3=([0-9A-F]{16}) pml4=([0-9A-F]{16})')
  $memory=[regex]::Match($text,'MEMORY OK virt=([0-9A-F]{16}) phys=([0-9A-F]{16}) value=([0-9A-F]{8})')
  $protect=[regex]::Match($text,'PROTECT OK flags=([0-9A-F]{16}) wp=([0-9A-F]{8}) nxe=([0-9A-F]{8})')
  $reclaim=[regex]::Match($text,'RECLAIM OK maps=([0-9A-F]{8}) unmaps=([0-9A-F]{8}) releases=([0-9A-F]{8}) owned=([0-9A-F]{8})')
  foreach($m in @($frames,$paging,$memory,$protect,$reclaim)){Assert-True $m.Success "O$opt ${ram}M missing G2C receipt field"}
  $frame=[Convert]::ToUInt64($frames.Groups[1].Value,16);$alloc=[Convert]::ToUInt32($frames.Groups[2].Value,16);$ownedCount=[Convert]::ToUInt32($frames.Groups[3].Value,16)
  Assert-True($frame-ge[uint64]0x00100000)"O$opt ${ram}M frame below 1 MiB"
  Assert-True(($frame-band[uint64]0xFFF)-eq0)"O$opt ${ram}M frame not page aligned"
  Assert-True($alloc-eq1-and$ownedCount-eq1)"O$opt ${ram}M allocation accounting mismatch"
  $cr3=[Convert]::ToUInt64($paging.Groups[1].Value,16);$pml4=[Convert]::ToUInt64($paging.Groups[2].Value,16)
  Assert-True($cr3-ne0-and$pml4-ne0)"O$opt ${ram}M paging root is zero"
  $pageMask=[uint64]::MaxValue-[uint64]0xFFF
  Assert-True(($cr3-band$pageMask)-eq$pml4)"O$opt ${ram}M CR3/PML4 mismatch"
  $virt=[Convert]::ToUInt64($memory.Groups[1].Value,16);$phys=[Convert]::ToUInt64($memory.Groups[2].Value,16);$value=[Convert]::ToUInt32($memory.Groups[3].Value,16)
  Assert-True($virt-eq[uint64]0x0000000100000000)"O$opt ${ram}M arena virtual mismatch"
  Assert-True($phys-eq$frame)"O$opt ${ram}M mapped physical differs from allocated frame"
  Assert-True($value-eq[uint32]0x0D13C0DE)"O$opt ${ram}M mapped memory roundtrip mismatch"
  $flags=[Convert]::ToUInt64($protect.Groups[1].Value,16);$wp=[Convert]::ToUInt32($protect.Groups[2].Value,16);$nxe=[Convert]::ToUInt32($protect.Groups[3].Value,16)
  Assert-True(($flags-band[uint64]3)-eq[uint64]3)"O$opt ${ram}M PTE present/write flags missing"
  $nxMask=[Convert]::ToUInt64('8000000000000000',16)
  Assert-True(($flags-band$nxMask)-ne0)"O$opt ${ram}M PTE NX flag missing"
  Assert-True($wp-eq1-and$nxe-eq1)"O$opt ${ram}M CR0.WP/EFER.NXE protection not active"
  $maps=[Convert]::ToUInt32($reclaim.Groups[1].Value,16);$unmaps=[Convert]::ToUInt32($reclaim.Groups[2].Value,16);$releases=[Convert]::ToUInt32($reclaim.Groups[3].Value,16);$ownedAfter=[Convert]::ToUInt32($reclaim.Groups[4].Value,16)
  Assert-True($maps-eq1-and$unmaps-eq1-and$releases-eq1-and$ownedAfter-eq0)"O$opt ${ram}M reclaim accounting mismatch"
  $lines=@($text-split"`r?`n"|Where-Object{$_-ne''});Assert-True($lines[-1]-ceq'G2C COMPLETE')"O$opt ${ram}M terminal receipt mismatch"
  $traceText=[IO.File]::ReadAllText($trace);Assert-True(-not$traceText.Contains('Triple fault'))"O$opt ${ram}M triple fault";Assert-True(([regex]::Matches($traceText,'v=0[0de6]')).Count-eq0)"O$opt ${ram}M #UD/#GP/#PF observed"
  $runs+=[pscustomobject]@{opt=$opt;ram=$ram;frame=('0x'+$frames.Groups[1].Value);cr3=('0x'+$paging.Groups[1].Value);virt=('0x'+$memory.Groups[1].Value);phys=('0x'+$memory.Groups[2].Value);serial_sha256=Hash $serial;trace_sha256=Hash $trace}
 }
 return [pscustomobject]@{opt=$opt;elf_sha256=Hash $elf;iso_sha256=Hash $iso;iso_bytes=(Get-Item $iso).Length;runs=$runs}
}
Push-Location $repo
try{
 $head=(Run 'git.exe' @('rev-parse','HEAD') 'read HEAD').Trim();Assert-True($head-ceq$expectedHead)'unexpected HEAD'
 Assert-True(((Run 'git.exe' @('rev-parse','v12.4.0^{}') 'verify frozen tag').Trim()-ceq$V13FrozenBaseHead))'v12.4.0 peeled tag drifted'
 Run 'cargo.exe' @('build','-q','-p','dbyte_cli','--bin','dbytec') 'build dbytec'|Out-Null
 $proofs=@(RunProof '0';RunProof '1')
 $proofs|ConvertTo-Json -Depth 6|Set-Content (Join-Path $root 'RESULT.json')
 Write-Output "V13 G2C INTEGRATION PASS: $root"
}finally{
 Pop-Location
 $deadline=[DateTime]::UtcNow.AddSeconds(3);do{$orphans=@($owned|Where-Object{$candidate=Get-CimInstance Win32_Process -Filter ("ProcessId = {0}" -f $_) -ErrorAction SilentlyContinue; $null -ne $candidate -and $candidate.Name -ieq [IO.Path]::GetFileName($qemu) -and $candidate.CommandLine -like ("*" + $root + "*")});if($orphans.Count-eq0-or[DateTime]::UtcNow-ge$deadline){break};Start-Sleep -Milliseconds 100}while($true)
 if($orphans.Count-ne0){throw "G2C integration verifier left owned QEMU processes: $($orphans -join ', ')"}
}
