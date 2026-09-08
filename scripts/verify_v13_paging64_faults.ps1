$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'v13_release_identity.ps1')
$expectedHead = Get-V13ExpectedHead
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$root=Join-Path $repo ('tmp/v13_g2/paging64-faults-'+[Guid]::NewGuid().ToString('N'))
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
function RunCase([string]$name,[string]$source,[string]$arm,[string]$error,[string]$opt){
 $dir=Join-Path $root "$name-o$opt";$grub=Join-Path $dir 'iso/boot/grub';New-Item -ItemType Directory -Path $grub -Force|Out-Null
 $elf=Join-Path $dir 'iso/boot/kernel.elf'
 Run $compiler @('build',$source,'--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','elf','-o',$elf) "build $name O$opt"|Out-Null
 Run 'wsl.exe' @('-d','Debian','--','grub-file','--is-x86-multiboot2',(LinuxPath $elf)) "grub-file $name O$opt"|Out-Null
 [IO.File]::WriteAllText((Join-Path $grub 'grub.cfg'),"set timeout=0`nset default=0`nmenuentry 'DByte paging64 fault' {`n multiboot2 /boot/kernel.elf`n boot`n}`n")
 $iso=Join-Path $dir "$name.iso";Run 'wsl.exe' @('-d','Debian','--','grub-mkrescue','-o',(LinuxPath $iso),(LinuxPath (Join-Path $dir 'iso'))) "ISO $name O$opt"|Out-Null
 $serial=Join-Path $dir 'serial.log';$trace=Join-Path $dir 'trace.log'
 $a=@('-cdrom',$iso,'-m','64M','-display','none','-monitor','none','-serial',"file:$serial",'-d','int,cpu_reset,guest_errors','-D',$trace,'-no-reboot','-no-shutdown')
 $p=Start-Process -FilePath $qemu -ArgumentList $a -PassThru -WindowStyle Hidden;$owned.Add($p.Id)
 try{
  $deadline=[DateTime]::UtcNow.AddSeconds(12);$text=''
  while(-not$p.HasExited-and[DateTime]::UtcNow-lt$deadline){Start-Sleep -Milliseconds 100;if(Test-Path $serial){$text=Shared $serial;if($text.Contains('G2B PAGEFAULT')){break};if($text.Contains('MISSED')-or$text.Contains('FAIL')){throw "$name O$opt diverged: '$text'"}}}
  Assert-True($text.Contains($arm)) "$name O$opt never armed"
  Assert-True($text.Contains('G2B PAGEFAULT cr2=0000000100000000 error='+$error)) "$name O$opt wrong page-fault receipt: '$text'"
  Assert-True(-not$text.Contains('MISSED')) "$name O$opt fault was not enforced"
 }finally{if(-not$p.HasExited){Stop-Process -Id $p.Id -Force;$p.WaitForExit()};$p.Dispose()}
 $traceText=[IO.File]::ReadAllText($trace)
 Assert-True(-not$traceText.Contains('Triple fault')) "$name O$opt triple fault"
 Assert-True(([regex]::Matches($traceText,'v=06')).Count-eq0) "$name O$opt #UD"
 Assert-True(([regex]::Matches($traceText,'v=0d')).Count-eq0) "$name O$opt #GP"
 Assert-True(([regex]::Matches($traceText,'v=0e')).Count-eq1) "$name O$opt expected exactly one #PF"
 return [pscustomobject]@{case=$name;opt=$opt;elf_sha256=Hash $elf;iso_sha256=Hash $iso;serial_sha256=Hash $serial;trace_sha256=Hash $trace}
}
Push-Location $repo
try{
 $head=(Run 'git.exe' @('rev-parse','HEAD') 'read HEAD').Trim();Assert-True($head-ceq$expectedHead) 'unexpected HEAD'
 Assert-True(((Run 'git.exe' @('rev-parse','v12.4.0^{}') 'verify frozen tag').Trim()-ceq$V13FrozenBaseHead)) 'v12.4.0 peeled tag drifted'
 Run 'cargo.exe' @('build','-q','-p','dbyte_cli','--bin','dbytec') 'build dbytec'|Out-Null
 $proofs=@()
 foreach($opt in @('0','1')){
  $proofs+=RunCase 'guard' 'examples/native_v13_machine/paging64_guard_fault.dby' 'PAGING64 GUARD ARM' '0000000000000000' $opt
  $proofs+=RunCase 'readonly' 'examples/native_v13_machine/paging64_readonly_fault.dby' 'PAGING64 RO ARM' '0000000000000003' $opt
 }
 $proofs|ConvertTo-Json -Depth 4|Set-Content (Join-Path $root 'hashes.json')
 Write-Output "V13 PAGING64 EXPECTED FAULTS PASS: $root"
}finally{
 Pop-Location
 $deadline=[DateTime]::UtcNow.AddSeconds(3);do{$orphans=@($owned|Where-Object{$candidate=Get-CimInstance Win32_Process -Filter ("ProcessId = {0}" -f $_) -ErrorAction SilentlyContinue; $null -ne $candidate -and $candidate.Name -ieq [IO.Path]::GetFileName($qemu) -and $candidate.CommandLine -like ("*" + $root + "*")});if($orphans.Count-eq0-or[DateTime]::UtcNow-ge$deadline){break};Start-Sleep -Milliseconds 100}while($true)
 if($orphans.Count-ne0){throw "paging64 fault verifier left owned QEMU processes: $($orphans -join ', ')"}
}
