$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'v13_release_identity.ps1')
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$root=Join-Path $repo ('tmp/v13_g4/generation4-'+[Guid]::NewGuid().ToString('N'))
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
 [IO.File]::WriteAllText($log,"CWD=$cwd`nCOMMAND=$exe $($a -join ' ')`nEXIT=$c`n$($o -join "`n")")
 if($c-ne0){throw "$d failed ($c): $log"};return($o -join "`n")
}
function LinuxPath([string]$p){(Run 'wsl.exe' @('-d','Debian','--','wslpath','-a','-u',$p.Replace('\','/')) 'translate WSL path').Trim()}
function Shared([string]$p){
 $s=[IO.File]::Open($p,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
 try{$r=[IO.StreamReader]::new($s,[Text.Encoding]::ASCII,$false,1024,$true);try{return $r.ReadToEnd()}finally{$r.Dispose()}}finally{$s.Dispose()}
}
function Check-StorageReadOnlySource(){
 $storage=[IO.File]::ReadAllText((Join-Path $repo 'examples/native_v13_machine/storage_ide.dby'))
 Assert-True($storage.Contains('const COMMAND_READ: u8 = 0x20 as u8'))'ATA READ command drifted'
 Assert-True($storage.Contains('const COMMAND_IDENTIFY: u8 = 0xec as u8'))'ATA IDENTIFY command drifted'
 Assert-True(-not($storage-match'(?i)COMMAND_WRITE|WRITE_SECTOR|CACHE_FLUSH|FLUSH_CACHE'))'ATA write/flush symbol present in G4 read-only driver'
 Assert-True(-not$storage.Contains('native.io_out16('))'16-bit ATA data-port write detected'
 Assert-True(([regex]::Matches($storage,'native\.io_in16\(DATA_PORT\)')).Count-eq2)'ATA data-port read must have exactly IDENTIFY and READ sites'
 $cmdSites=[regex]::Matches($storage,'native\.io_out8\(COMMAND_PORT,\s*([A-Z_]+)\)')
 Assert-True($cmdSites.Count-eq2)'unexpected ATA command submission count'
 foreach($m in $cmdSites){Assert-True($m.Groups[1].Value-in@('COMMAND_IDENTIFY','COMMAND_READ'))"forbidden ATA command $($m.Groups[1].Value)"}
}
function New-DiskFixture(){
 $path=Join-Path $root 'disk.raw';[byte[]]$disk=New-Object byte[] (8MB);$base=8*512
 [byte[]]$hdr=[Text.Encoding]::ASCII.GetBytes('DBYTE_G4_READONLY_V1');[Array]::Copy($hdr,0,$disk,$base,$hdr.Length)
 for($i=$hdr.Length;$i-lt512;$i++){$disk[$base+$i]=[byte](($i*37+0x2B)-band0xff)}
 [IO.File]::WriteAllBytes($path,$disk);[uint64]$h=[Convert]::ToUInt64('C0DEC0DE',16);[uint64]$mask=4294967295
 for($i=0;$i-lt512;$i++){[uint64]$left=($h-shl5)-band$mask;[uint64]$right=$h-shr27;$h=(($left-bor$right)-bxor[uint64]$disk[$base+$i])-band$mask}
 $sectorHash=('{0:X8}'-f[uint32]$h);Assert-True($sectorHash-ceq'559A49B3')"G4 host oracle self-test drifted: $sectorHash"
 return [pscustomobject]@{path=$path;sha256=Hash $path;bytes=(Get-Item $path).Length;sector_hash=$sectorHash;sectors=[uint32]($disk.Length/512)}
}
function Build-Set([string]$set,[string]$opt){
 $dir=Join-Path $root "$set-o$opt";New-Item -ItemType Directory -Path $dir -Force|Out-Null
 $storage=Join-Path $dir 'storage.o';$obj=Join-Path $dir 'kernel.o';$elf=Join-Path $dir 'kernel.elf'
 $dump=Run $compiler @('build','examples/native_v13_machine/storage_ide.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','obj','-o',$storage,'--dump-machine-code') "build G4 storage $set O$opt"
 [IO.File]::WriteAllText((Join-Path $dir 'storage-machine.txt'),$dump)
 $sd=Run 'objdump.exe' @('-d','-Mintel',$storage) "disassemble G4 storage $set O$opt";[IO.File]::WriteAllText((Join-Path $dir 'storage-objdump.txt'),$sd)
 Assert-True($sd.Contains('in     ax,dx'))"$set O$opt storage object lacks IN16"
 Assert-True($sd.Contains('movzx  rax,ax'))"$set O$opt storage object lacks IN16 zero-extension"
 Assert-True(-not$sd.Contains('out    dx,ax'))"$set O$opt storage object contains OUT16 data write"
 $kb=Run $compiler @('build','examples/native_v13_machine/kernel_g4.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','obj','-o',$obj,'--dump-native-mir','--dump-machine-code') "build G4 $set O$opt object"
 [IO.File]::WriteAllText((Join-Path $dir 'kernel-machine.txt'),$kb)
 Run $compiler @('build','examples/native_v13_machine/kernel_g4.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','elf','-o',$elf) "build G4 $set O$opt ELF"|Out-Null
 $linux=LinuxPath $elf;Run 'wsl.exe' @('-d','Debian','--','grub-file','--is-x86-multiboot2',$linux) "grub-file G4 $set O$opt"|Out-Null
 $readelf=Run 'wsl.exe' @('-d','Debian','--','readelf','-lW',$linux) "readelf G4 $set O$opt";Assert-True(-not$readelf.Contains('INTERP'))"$set O$opt has interpreter"
 return [pscustomobject]@{set=$set;opt=$opt;dir=$dir;storage=$storage;obj=$obj;elf=$elf;storage_sha256=Hash $storage;obj_sha256=Hash $obj;elf_sha256=Hash $elf}
}
function Make-Iso($build,[string]$date){
 $isoRoot=Join-Path $build.dir 'iso';$grub=Join-Path $isoRoot 'boot/grub';New-Item -ItemType Directory -Path $grub -Force|Out-Null
 Copy-Item -LiteralPath $build.elf -Destination (Join-Path $isoRoot 'boot/kernel.elf') -Force
 [IO.File]::WriteAllText((Join-Path $grub 'grub.cfg'),"set timeout=0`nset default=0`nmenuentry 'DByte v13 G4' {`n multiboot2 /boot/kernel.elf`n boot`n}`n")
 $iso=Join-Path $build.dir 'dbyte-v13-g4.iso';Run 'wsl.exe' @('-d','Debian','--','grub-mkrescue','-o',(LinuxPath $iso),("--modification-date=$date"),'--set_all_file_dates',$date,(LinuxPath $isoRoot)) "build G4 ISO $($build.set) O$($build.opt)"|Out-Null
 return [pscustomobject]@{build=$build;path=$iso;sha256=Hash $iso;bytes=(Get-Item $iso).Length}
}
function Validate-G4([string]$text,[int]$ram,[string]$opt,$fixture){
 foreach($bad in @('G1 ERROR','G2 ERROR','G2B ERROR','G2C ERROR','G3 ERROR','G4 ERROR','G2B FATAL','G2B PAGEFAULT')){Assert-True(-not$text.Contains($bad))"O$opt ${ram}M reported $bad"}
 $ide=[regex]::Match($text,'IDE OK bdf=([0-9A-F]{2}):([0-9A-F]{2})\.([0-9A-F]{2}) id=([0-9A-F]{4}):([0-9A-F]{4}) prog=([0-9A-F]{2})')
 $identify=[regex]::Match($text,'IDENTIFY OK sectors=([0-9A-F]{8})')
 $read=[regex]::Match($text,'READ OK lba=([0-9A-F]{8}) hash=([0-9A-F]{8})')
 Assert-True($ide.Success-and$identify.Success-and$read.Success)"O$opt ${ram}M missing G4 receipt fields"
 Assert-True($ide.Groups[1].Value-ceq'00'-and$ide.Groups[2].Value-ceq'01'-and$ide.Groups[3].Value-ceq'01')"O$opt ${ram}M wrong IDE BDF"
 Assert-True($ide.Groups[4].Value-ceq'8086'-and$ide.Groups[5].Value-ceq'7010'-and$ide.Groups[6].Value-ceq'80')"O$opt ${ram}M wrong IDE identity/prog-if"
 Assert-True([Convert]::ToUInt32($identify.Groups[1].Value,16)-eq$fixture.sectors)"O$opt ${ram}M IDENTIFY capacity mismatch"
 Assert-True($read.Groups[1].Value-ceq'00000008')"O$opt ${ram}M wrong read LBA"
 Assert-True($read.Groups[2].Value-ceq$fixture.sector_hash)"O$opt ${ram}M guest/host sector hash mismatch"
 $lines=@($text-split"`r?`n"|Where-Object{$_-ne''});Assert-True($lines[-1]-ceq'G4 READONLY COMPLETE')"O$opt ${ram}M completion is not terminal"
 $order=@($text.IndexOf('G3 COMPLETE'),$text.IndexOf('DBYTE V13 G4 STORAGE'),$text.IndexOf('IDE OK'),$text.IndexOf('IDENTIFY OK'),$text.IndexOf('READ OK'),$text.IndexOf('G4 READONLY COMPLETE'))
 for($i=1;$i-lt$order.Count;$i++){Assert-True($order[$i]-gt$order[$i-1])"O$opt ${ram}M G4 receipt order invalid"}
}
function Run-Qemu($iso,[int]$ram,$fixture){
 $opt=$iso.build.opt;$dir=Join-Path $root "run-o$opt-${ram}m";New-Item -ItemType Directory -Path $dir -Force|Out-Null
 $serial=Join-Path $dir 'serial.log';$trace=Join-Path $dir 'trace.log';$before=Hash $fixture.path;$entryResets=$null;$text=''
 $a=@('-snapshot','-cdrom',$iso.path,'-boot','order=d','-m',"${ram}M",'-display','none','-monitor','none','-serial',"file:$serial",'-d','int,cpu_reset,guest_errors','-D',$trace,'-no-reboot','-no-shutdown','-drive',"file=$($fixture.path),format=raw,if=ide,index=0,media=disk")
 [IO.File]::WriteAllText((Join-Path $dir 'qemu-command.txt'),"$qemu $($a -join ' ')")
 $p=Start-Process -FilePath $qemu -ArgumentList $a -PassThru -WindowStyle Hidden;$owned.Add($p.Id);$terminal=$false
 try{
  $deadline=[DateTime]::UtcNow.AddSeconds(35)
  do{Start-Sleep -Milliseconds 100
   if(Test-Path $serial){$text=Shared $serial
    if($text.Length-gt0-and$null-eq$entryResets-and(Test-Path $trace)){$entryResets=([regex]::Matches((Shared $trace),'CPU Reset')).Count}
    $terminal=$text.Contains('G4 READONLY COMPLETE')-or$text.Contains('G1 ERROR')-or$text.Contains('G2 ERROR')-or$text.Contains('G2B ERROR')-or$text.Contains('G2C ERROR')-or$text.Contains('G3 ERROR')-or$text.Contains('G4 ERROR')-or$text.Contains('G2B FATAL')-or$text.Contains('G2B PAGEFAULT')
    if($terminal){break}
   }
  }while(-not$p.HasExited-and[DateTime]::UtcNow-lt$deadline)
  Assert-True($terminal)"O$opt ${ram}M QEMU timed out before terminal G4 receipt"
 }finally{if(-not$p.HasExited){Stop-Process -Id $p.Id -Force;$p.WaitForExit()};$p.Dispose()}
 $after=Hash $fixture.path
 Assert-True($before-ceq$after)"O$opt ${ram}M original fixture hash changed"
 $traceText=[IO.File]::ReadAllText($trace)
 $triple=([regex]::Matches($traceText,'Triple fault')).Count
 $faults=([regex]::Matches($traceText,'v=0[0de6]')).Count
 $finalResets=([regex]::Matches($traceText,'CPU Reset')).Count
 Assert-True($triple-eq0)"O$opt ${ram}M triple fault"
 Assert-True($faults-eq0)"O$opt ${ram}M #UD/#GP/#PF observed"
 Assert-True($null-ne$entryResets)"O$opt ${ram}M no entry reset sample"
 Assert-True($finalResets-eq$entryResets)"O$opt ${ram}M CPU reset after kernel entry"
 Validate-G4 $text $ram $opt $fixture
 $receipt=[pscustomobject]@{opt=$opt;ram=$ram;iso_sha256=$iso.sha256;fixture_before=$before;fixture_after=$after;fixture_immutable=($before-ceq$after);sector_hash=$fixture.sector_hash;serial_sha256=Hash $serial;trace_sha256=Hash $trace;entry_resets=$entryResets;final_resets=$finalResets;faults=$faults;triple_faults=$triple}
 $receipt|ConvertTo-Json|Set-Content (Join-Path $dir 'receipt.json')
 return $receipt
}
function Write-Identity($selectedIso,$selectedBuild,$fixture){
 $diff=Run 'git.exe' @('diff','--binary','--no-ext-diff','HEAD') 'capture tracked diff';$diffPath=Join-Path $root 'tracked.diff';[IO.File]::WriteAllText($diffPath,$diff,[Text.UTF8Encoding]::new($false))
 $files=@(Get-ChildItem (Join-Path $repo 'examples/native_v13_machine') -File -Filter '*.dby';Get-ChildItem (Join-Path $repo 'scripts') -File -Filter 'verify_v13_*.ps1')|Sort-Object FullName -Unique
 $manifest=@($files|ForEach-Object{[pscustomobject]@{path=$_.FullName.Substring($repo.Length+1).Replace('\','/');bytes=$_.Length;sha256=Hash $_.FullName}})
 $manifestPath=Join-Path $root 'input-manifest.json';$manifest|ConvertTo-Json -Depth 4|Set-Content $manifestPath
 $identity=[pscustomobject]@{head=(Run 'git.exe' @('rev-parse','HEAD') 'read HEAD').Trim();tracked_diff_sha256=Hash $diffPath;input_manifest_sha256=Hash $manifestPath;compiler_sha256=Hash $compiler;storage_object_sha256=$selectedBuild.storage_sha256;kernel_object_sha256=$selectedBuild.obj_sha256;kernel_elf_sha256=$selectedBuild.elf_sha256;iso_sha256=$selectedIso.sha256;iso_bytes=$selectedIso.bytes;fixture_sha256=$fixture.sha256;fixture_sector_hash=$fixture.sector_hash}
 $identity|ConvertTo-Json|Set-Content (Join-Path $root 'candidate-identity.json');return $identity
}
Push-Location $repo
try{
 $head=(Run 'git.exe' @('rev-parse','HEAD') 'verify HEAD').Trim();Assert-True($head-ceq$expectedHead)'unexpected HEAD'
 Assert-True(((Run 'git.exe' @('rev-parse','v12.4.0^{}') 'verify frozen tag').Trim()-ceq$V13FrozenBaseHead))'v12.4.0 peeled tag drifted'
 Check-StorageReadOnlySource
 Run 'cargo.exe' @('build','-q','-p','dbyte_cli','--bin','dbytec') 'fresh dbytec'|Out-Null
 Run 'cargo.exe' @('test','-q','-p','dbyte_native','port_io_byte_word_and_dword_forms_are_exact_and_transactional') 'focused IO16 encoder gate'|Out-Null
 $g3=Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_generation3.ps1')) 'fresh G3 predecessor master'
 [IO.File]::WriteAllText((Join-Path $root 'generation3-predecessor.txt'),$g3)
 Assert-True($g3.Contains('V13 GENERATION 3 APIC/IOAPIC/TIMER/PCI VERIFIER: PASS'))'fresh G3 did not emit PASS'
 $epoch=[int64](Run 'git.exe' @('show','-s','--format=%ct','HEAD') 'read packaging epoch').Trim()
 $isoDate=[DateTimeOffset]::FromUnixTimeSeconds($epoch).UtcDateTime.ToString('yyyyMMddHHmmss00',[Globalization.CultureInfo]::InvariantCulture)
 $fixture=New-DiskFixture
 Assert-True($fixture.bytes-eq8388608)'G4 fixture size mismatch'
 Assert-True($fixture.sectors-eq16384)'G4 fixture sector count mismatch'
 $builds=@()
 foreach($opt in @('0','1')){
  $a=Build-Set 'A' $opt;$b=Build-Set 'B' $opt
  Assert-True($a.storage_sha256-ceq$b.storage_sha256)"O$opt storage object is not deterministic"
  Assert-True($a.obj_sha256-ceq$b.obj_sha256)"O$opt kernel object is not deterministic"
  Assert-True($a.elf_sha256-ceq$b.elf_sha256)"O$opt kernel ELF is not deterministic"
  $builds+=@($a,$b)
 }
 $isos=@()
 foreach($opt in @('0','1')){
  $a=$builds|Where-Object{$_.set-eq'A'-and$_.opt-eq$opt};$b=$builds|Where-Object{$_.set-eq'B'-and$_.opt-eq$opt}
  $ia=Make-Iso $a $isoDate;$ib=Make-Iso $b $isoDate;Assert-True($ia.sha256-ceq$ib.sha256)"O$opt ISO is not deterministic";$isos+=@($ia,$ib)
 }
 $receipts=@()
 foreach($opt in @('0','1')){
  $iso=$isos|Where-Object{$_.build.set-eq'A'-and$_.build.opt-eq$opt}
  foreach($ram in @(32,64)){$receipts+=Run-Qemu $iso $ram $fixture}
 }
 $receipts|ConvertTo-Json -Depth 5|Set-Content (Join-Path $root 'qemu-receipts.json')
 Assert-True(($receipts|Where-Object{-not$_.fixture_immutable}).Count-eq0)'G4 fixture immutability matrix failed'
 $selectedBuild=$builds|Where-Object{$_.set-eq'A'-and$_.opt-eq'1'}
 $selectedIso=$isos|Where-Object{$_.build.set-eq'A'-and$_.build.opt-eq'1'}
 $identity=Write-Identity $selectedIso $selectedBuild $fixture
 $candidate=Join-Path $root 'physical-candidate';New-Item -ItemType Directory -Path $candidate -Force|Out-Null
 Copy-Item -LiteralPath $selectedBuild.elf -Destination (Join-Path $candidate 'dbyte-v13-g4.elf') -Force
 Copy-Item -LiteralPath $selectedIso.path -Destination (Join-Path $candidate 'dbyte-v13-g4.iso') -Force
 Assert-True((Hash (Join-Path $candidate 'dbyte-v13-g4.elf'))-ceq$selectedBuild.elf_sha256)'candidate ELF hash drift'
 Assert-True((Hash (Join-Path $candidate 'dbyte-v13-g4.iso'))-ceq$selectedIso.sha256)'candidate ISO hash drift'
 $result=[pscustomobject]@{status='PASS';generation='v13-g4-readonly-ide-qemu';packaging_date=$isoDate;candidate_identity=$identity;qemu_receipts=$receipts.Count;io16='PASS';g3_fresh='PASS';g4_readonly='PASS';disk_immutability='PASS';g4_determinism='PASS';sector_service='PENDING';dbfs1='PENDING';vfs='PENDING';durability='PENDING';real_biostar='PENDING';real_storage='DISARMED';v13_final='NOT_CREATED'}
 $result|ConvertTo-Json -Depth 6|Set-Content (Join-Path $root 'RESULT.json')
 Write-Output 'V13 GENERATION 4 READONLY IDE VERIFIER: PASS'
 Write-Output "EVIDENCE ROOT: $root"
 Write-Output "SELECTED ISO SHA256: $($selectedIso.sha256)"
 Write-Output "FIXTURE SHA256: $($fixture.sha256) SECTOR8 HASH: $($fixture.sector_hash)"
}finally{
 Pop-Location
 $deadline=[DateTime]::UtcNow.AddSeconds(3)
 do{$orphans=@($owned|Where-Object{$candidate=Get-CimInstance Win32_Process -Filter ("ProcessId = {0}" -f $_) -ErrorAction SilentlyContinue; $null -ne $candidate -and $candidate.Name -ieq [IO.Path]::GetFileName($qemu) -and $candidate.CommandLine -like ("*" + $root + "*")});if($orphans.Count-eq0-or[DateTime]::UtcNow-ge$deadline){break};Start-Sleep -Milliseconds 100}while($true)
 if($orphans.Count-ne0){throw "G4 verifier left owned QEMU processes: $($orphans -join ', ')"}
}
