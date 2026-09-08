$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'v13_release_identity.ps1')
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'v13_storage_receipt.ps1')
$sourceBefore=Get-V13SourceBinding $repo
$freeBefore=(Get-PSDrive -Name ([IO.Path]::GetPathRoot($repo).Substring(0,1))).Free
$root=Join-Path $repo ('tmp/v13_g4/storage-'+[Guid]::NewGuid().ToString('N'))
$compiler=Join-Path $repo 'target/debug/dbytec.exe'
$qemu='C:/Program Files/qemu/qemu-system-x86_64.exe'
$expectedHead = Get-V13ExpectedHead
$owned=[System.Collections.Generic.List[int]]::new()
$script:sequence=0
New-Item -ItemType Directory -Path $root -Force|Out-Null
function Assert-True($v,[string]$m){if(-not $v){throw $m}}
function Hash([string]$p){(Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash}
function Run([string]$exe,[string[]]$a,[string]$d,[string]$cwd=$repo){
 $script:sequence++;$log=Join-Path $root ('command-{0:D3}.txt'-f $script:sequence)
 $save=$ErrorActionPreference;$ErrorActionPreference='Continue'
 try{Push-Location $cwd;try{$o=& $exe @a 2>&1;$c=$LASTEXITCODE}finally{Pop-Location}}finally{$ErrorActionPreference=$save}
 $text=$o-join"`n"
 [IO.File]::WriteAllText($log,"CWD=$cwd`nCOMMAND=$exe $($a -join ' ')`nEXIT=$c`n$text")
 if($c-ne0){throw "$d failed ($c): $log"}
 return $text
}
function LinuxPath([string]$p){
 return (Run 'wsl.exe' @('-d','Debian','--','wslpath','-a','-u',$p.Replace('\','/')) 'translate WSL path').Trim()
}function Shared([string]$p){
 $s=[IO.File]::Open($p,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
 try{$r=[IO.StreamReader]::new($s,[Text.Encoding]::ASCII,$false,1024,$true);try{return $r.ReadToEnd()}finally{$r.Dispose()}}finally{$s.Dispose()}
}
function New-TestDisk(){
 $disk=Join-Path $root 'g4-readonly.raw'
 [byte[]]$bytes=New-Object byte[] (8MB)
 $base=8*512
 [byte[]]$header=[Text.Encoding]::ASCII.GetBytes('DBYTE_G4_READONLY_V1')
 [Array]::Copy($header,0,$bytes,$base,$header.Length)
 for($i=$header.Length;$i-lt512;$i++){$bytes[$base+$i]=[byte](($i*37+0x2B)-band0xff)}
 [IO.File]::WriteAllBytes($disk,$bytes)
 [uint32]$h=[Convert]::ToUInt32('C0DEC0DE',16)
 for($i=0;$i-lt512;$i++){
  [uint64]$left=([uint64]$h*32)%4294967296
  [uint64]$right=[math]::Floor(([double]$h)/134217728)
  $h=[uint32](($left+$right)-bxor[uint64]$bytes[$base+$i])
 }
 return [pscustomobject]@{path=$disk;sha256=Hash $disk;expected_hash=('{0:X8}'-f$h);sectors='00004000'}
}
function Inspect-Io16([string]$obj,[string]$label){
 $d=Run 'objdump.exe' @('-d','-Mintel',$obj) "objdump $label"
 Assert-True($d.Contains('out    dx,ax'))"$label missing OUT DX,AX"
 Assert-True($d.Contains('in     ax,dx'))"$label missing IN AX,DX"
 Assert-True($d.Contains('movzx  rax,ax'))"$label missing 16-bit result zero extension"
}
function Build-Set([string]$set,[string]$opt){
 $dir=Join-Path $root "$set-o$opt";New-Item -ItemType Directory -Path $dir -Force|Out-Null
 $io=Join-Path $dir 'io16.o'
 Run $compiler @('build','examples/native_v13_machine/io16_regression.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','obj','-o',$io) "IO16 O$opt"|Out-Null
 Inspect-Io16 $io "IO16 O$opt"
 $obj=Join-Path $dir 'kernel.o';$elf=Join-Path $dir 'kernel.elf'
 Run $compiler @('build','examples/native_v13_machine/kernel_g4.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','obj','-o',$obj) "build G4 $set O$opt object"|Out-Null
 $disasm=Run 'objdump.exe' @('-d','-Mintel',$obj) "objdump G4 $set O$opt"
 Assert-True($disasm.Contains('in     ax,dx'))"G4 $set O$opt lacks ATA word input"
 Run $compiler @('build','examples/native_v13_machine/kernel_g4.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','elf','-o',$elf) "build G4 $set O$opt ELF"|Out-Null
 $w=LinuxPath $elf
 Run 'wsl.exe' @('-d','Debian','--','grub-file','--is-x86-multiboot2',$w) "grub-file G4 $set O$opt"|Out-Null
 return [pscustomobject]@{set=$set;opt=$opt;dir=$dir;obj=$obj;elf=$elf;obj_sha256=Hash $obj;elf_sha256=Hash $elf}
}
function Make-Iso($build,[string]$date){
 $isoRoot=Join-Path $build.dir 'iso';$grub=Join-Path $isoRoot 'boot/grub'
 New-Item -ItemType Directory -Path $grub -Force|Out-Null
 Copy-Item -LiteralPath $build.elf -Destination (Join-Path $isoRoot 'boot/kernel.elf') -Force
 [IO.File]::WriteAllText((Join-Path $grub 'grub.cfg'),"set timeout=0`nset default=0`nmenuentry 'DByte v13 G4 Storage' {`n multiboot2 /boot/kernel.elf`n boot`n}`n")
 $iso=Join-Path $build.dir 'dbyte-v13-g4.iso'
 Run 'wsl.exe' @('-d','Debian','--','grub-mkrescue','-o',(LinuxPath $iso),("--modification-date=$date"),'--set_all_file_dates',$date,(LinuxPath $isoRoot)) "build deterministic G4 ISO $($build.set) O$($build.opt)"|Out-Null
 return [pscustomobject]@{build=$build;path=$iso;sha256=Hash $iso;bytes=(Get-Item $iso).Length}
}
function Validate-Receipt([string]$text,$disk,[int]$ram,[string]$opt){
 foreach($bad in @('G1 ERROR','G2 ERROR','G2B ERROR','G2C ERROR','G3 ERROR','G4 ERROR','G2B FATAL','G2B PAGEFAULT')){Assert-True(-not$text.Contains($bad))"O$opt ${ram}M reported $bad"}
 Assert-True($text.Contains('G3 COMPLETE'))"O$opt ${ram}M missing G3 predecessor completion"
 Assert-True($text.Contains('DBYTE V13 G4 STORAGE'))"O$opt ${ram}M missing G4 banner"
 $ide=[regex]::Match($text,'IDE OK bdf=([0-9A-F]{2}):([0-9A-F]{2})\.([0-9A-F]{2}) id=([0-9A-F]{4}):([0-9A-F]{4}) prog=([0-9A-F]{2})')
 $identify=[regex]::Match($text,'IDENTIFY OK sectors=([0-9A-F]{8})')
 $read=[regex]::Match($text,'READ OK lba=([0-9A-F]{8}) hash=([0-9A-F]{8})')
 Assert-True($ide.Success-and$identify.Success-and$read.Success)"O$opt ${ram}M incomplete G4 receipt"
 Assert-True($ide.Groups[1].Value-ceq'00'-and$ide.Groups[2].Value-ceq'01'-and$ide.Groups[3].Value-ceq'01')"O$opt ${ram}M unexpected IDE BDF"
 Assert-True($ide.Groups[4].Value-ceq'8086'-and$ide.Groups[5].Value-ceq'7010')"O$opt ${ram}M unexpected IDE identity"
 Assert-True($identify.Groups[1].Value-ceq$disk.sectors)"O$opt ${ram}M IDENTIFY sector count mismatch"
 Assert-True($read.Groups[1].Value-ceq'00000008')"O$opt ${ram}M wrong LBA read"
 Assert-True($read.Groups[2].Value-ceq$disk.expected_hash)"O$opt ${ram}M sector checksum mismatch"
 $lines=@($text-split"`r?`n"|Where-Object{$_-ne''})
 Assert-True($lines[-1]-ceq'G4 READONLY COMPLETE')"O$opt ${ram}M completion is not terminal G4 receipt"
}
function Run-Qemu($iso,$disk,[int]$ram){
 $opt=$iso.build.opt;$dir=Join-Path $root "run-o$opt-${ram}m";New-Item -ItemType Directory -Path $dir -Force|Out-Null
 $serial=Join-Path $dir 'serial.log';$trace=Join-Path $dir 'trace.log';$text='';$entryResets=$null
 $before=Hash $disk.path
 $a=@('-snapshot','-cdrom',$iso.path,'-boot','order=d','-m',"${ram}M",'-display','none','-monitor','none','-serial',"file:$serial",'-d','int,cpu_reset,guest_errors','-D',$trace,'-no-reboot','-no-shutdown','-drive',"file=$($disk.path),format=raw,if=ide,index=0,media=disk")
 [IO.File]::WriteAllText((Join-Path $dir 'qemu-command.txt'),"$qemu $($a-join' ')")
 $p=Start-Process -FilePath $qemu -ArgumentList $a -PassThru -WindowStyle Hidden;$owned.Add($p.Id)
 try{$deadline=[DateTime]::UtcNow.AddSeconds(35);do{Start-Sleep -Milliseconds 100;if(Test-Path $serial){$text=Shared $serial;if($text.Length-gt0-and$null-eq$entryResets-and(Test-Path $trace)){$entryResets=([regex]::Matches((Shared $trace),'CPU Reset')).Count};if($text.Contains('G4 READONLY COMPLETE')-or$text.Contains('G4 ERROR')){break}}}while(-not$p.HasExited-and[DateTime]::UtcNow-lt$deadline);Assert-True($text.Contains('G4 READONLY COMPLETE')-or$text.Contains('G4 ERROR'))"O$opt ${ram}M QEMU timeout"}
 finally{if(-not$p.HasExited){Stop-Process -Id $p.Id -Force;$p.WaitForExit()};$p.Dispose()}
 $after=Hash $disk.path;Assert-True($before-ceq$after)"O$opt ${ram}M base disk mutated"
 $traceText=[IO.File]::ReadAllText($trace);$triple=([regex]::Matches($traceText,'Triple fault')).Count;$faults=([regex]::Matches($traceText,'v=0[0de6]')).Count;$finalResets=([regex]::Matches($traceText,'CPU Reset')).Count
 Assert-True($triple-eq0)"O$opt ${ram}M triple fault";Assert-True($faults-eq0)"O$opt ${ram}M #UD/#GP/#PF observed"
 Assert-True($null-ne$entryResets-and$finalResets-eq$entryResets)"O$opt ${ram}M CPU reset after entry"
 Validate-Receipt $text $disk $ram $opt
 $r=[pscustomobject]@{opt=$opt;ram=$ram;disk_sha256=$after;sector_hash=$disk.expected_hash;serial_sha256=Hash $serial;trace_sha256=Hash $trace;entry_resets=$entryResets;final_resets=$finalResets;faults=$faults;triple_faults=$triple}
 $r|ConvertTo-Json|Set-Content (Join-Path $dir 'receipt.json');return $r
}
Push-Location $repo
try{
 $head=(Run 'git.exe' @('rev-parse','HEAD') 'verify HEAD').Trim();Assert-True($head-ceq$expectedHead)'unexpected HEAD'
 Assert-True(((Run 'git.exe' @('rev-parse','v12.4.0^{}') 'verify frozen tag').Trim()-ceq$V13FrozenBaseHead))'v12.4.0 tag drifted'
 $storage=[IO.File]::ReadAllText((Join-Path $repo 'examples/native_v13_machine/storage_ide.dby'))
 Assert-True(-not($storage-match'(?i)COMMAND_WRITE|CACHE_FLUSH|io_out16\s*\('))'G4 read-only driver contains media-write primitive'
 $g3=Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_generation3.ps1')) 'fresh G3 predecessor'
 [IO.File]::WriteAllText((Join-Path $root 'g3-predecessor.txt'),$g3)
 Run 'cargo.exe' @('build','-q','-p','dbyte_cli','--bin','dbytec') 'fresh dbytec'|Out-Null
 $disk=New-TestDisk;Assert-True($disk.expected_hash-ceq'559A49B3')'host checksum oracle drifted'
 $epoch=[int64](Run 'git.exe' @('show','-s','--format=%ct','HEAD') 'packaging epoch').Trim()
 $date=[DateTimeOffset]::FromUnixTimeSeconds($epoch).UtcDateTime.ToString('yyyyMMddHHmmss00',[Globalization.CultureInfo]::InvariantCulture)
 $builds=@()
 foreach($opt in @('0','1')){$a=Build-Set 'A' $opt;$b=Build-Set 'B' $opt;Assert-True($a.obj_sha256-ceq$b.obj_sha256)"O$opt object nondeterministic";Assert-True($a.elf_sha256-ceq$b.elf_sha256)"O$opt ELF nondeterministic";$builds+=@($a,$b)}
 $isos=@()
 foreach($opt in @('0','1')){$a=$builds|Where-Object{$_.set-eq'A'-and$_.opt-eq$opt};$b=$builds|Where-Object{$_.set-eq'B'-and$_.opt-eq$opt};$ia=Make-Iso $a $date;$ib=Make-Iso $b $date;Assert-True($ia.sha256-ceq$ib.sha256)"O$opt ISO nondeterministic";$isos+=@($ia,$ib)} $receipts=@()
 foreach($opt in @('0','1')){$iso=$isos|Where-Object{$_.build.set-eq'A'-and$_.build.opt-eq$opt};foreach($ram in @(32,64)){$receipts+=Run-Qemu $iso $disk $ram}}
 $receipts|ConvertTo-Json -Depth 5|Set-Content (Join-Path $root 'qemu-receipts.json')
 $selectedBuild=$builds|Where-Object{$_.set-eq'A'-and$_.opt-eq'1'};$selectedIso=$isos|Where-Object{$_.build.set-eq'A'-and$_.build.opt-eq'1'}
 $result=[pscustomobject]@{status='PASS';generation='v13-g4-ide-pio-readonly-qemu';head=$head;disk_sha256=$disk.sha256;expected_sector_hash=$disk.expected_hash;selected_object_sha256=$selectedBuild.obj_sha256;selected_elf_sha256=$selectedBuild.elf_sha256;selected_iso_sha256=$selectedIso.sha256;selected_iso_bytes=$selectedIso.bytes;qemu_receipts=$receipts.Count;g3_predecessor='PASS';storage_media_writes='DISARMED';real_biostar='PENDING';real_storage='DISARMED';dbfs1='PENDING';vfs='PENDING';g5='PENDING';v13_final='NOT_CREATED'}
 $result|Add-Member -NotePropertyName binding -NotePropertyValue (New-V13StorageBinding $repo $root $sourceBefore)
 $result|Add-Member -NotePropertyName free_bytes_before -NotePropertyValue $freeBefore
 $result|ConvertTo-Json -Depth 8|Set-Content (Join-Path $root 'RESULT.json')
 Write-Output 'V13 GENERATION 4 IDE PIO READONLY VERIFIER: PASS'
 Write-Output "EVIDENCE ROOT: $root"
 Write-Output "SELECTED ISO SHA256: $($result.selected_iso_sha256)"
 Write-Output "TEST DISK SHA256: $($result.disk_sha256)"
}finally{
 Pop-Location
 $deadline=[DateTime]::UtcNow.AddSeconds(3);do{$orphans=@($owned|Where-Object{$candidate=Get-CimInstance Win32_Process -Filter ("ProcessId = {0}" -f $_) -ErrorAction SilentlyContinue; $null -ne $candidate -and $candidate.Name -ieq [IO.Path]::GetFileName($qemu) -and $candidate.CommandLine -like ("*" + $root + "*")});if($orphans.Count-eq0-or[DateTime]::UtcNow-ge$deadline){break};Start-Sleep -Milliseconds 100}while($true)
 if($orphans.Count-ne0){throw "G4 verifier left owned QEMU processes: $($orphans-join', ')"}
}
