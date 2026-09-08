param([string]$StorageReceipt='')
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'v13_release_identity.ps1')
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
. (Join-Path $PSScriptRoot 'v13_storage_receipt.ps1')
$root=Join-Path $repo ('tmp/v13_g4/filesystem-'+[Guid]::NewGuid().ToString('N'))
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
function Put-U32([byte[]]$b,[int]$o,[uint32]$v){
 $b[$o]=[byte]($v-band255);$b[$o+1]=[byte](($v-shr8)-band255);$b[$o+2]=[byte](($v-shr16)-band255);$b[$o+3]=[byte](($v-shr24)-band255)
}
function Fnv1a([byte[]]$b,[int]$o,[int]$n){
 [uint32]$h=2166136261
 for($i=0;$i-lt$n;$i++){[uint64]$p=[uint64]($h-bxor$b[$o+$i])*[uint64]16777619;$h=[uint32]($p-band[uint64]4294967295)}
 return $h
}
function Check-ReadOnlySource(){
 $storage=[IO.File]::ReadAllText((Join-Path $repo 'examples/native_v13_machine/storage_ide.dby'))
 Assert-True(-not($storage-match'(?i)COMMAND_WRITE|WRITE_SECTOR|CACHE_FLUSH|FLUSH_CACHE'))'ATA media-write symbol present'
 Assert-True(-not$storage.Contains('native.io_out16('))'ATA data-port OUT16 present'
 $sector=[IO.File]::ReadAllText((Join-Path $repo 'examples/native_v13_machine/sector_service.dby'))
 Assert-True(-not($sector-match'(?i)write|flush|format|native\.io_'))'sector service escaped read-only boundary'
 $dbfs=[IO.File]::ReadAllText((Join-Path $repo 'examples/native_v13_machine/dbfs1_ro.dby'))
 Assert-True($dbfs.Contains('import "sector_service.dby" as sector'))'DBFS1 does not depend on sector service'
 Assert-True(-not($dbfs-match'(?i)storage_ide|native\.io_|cache_write|ata_write|format\('))'DBFS1 bypasses read-only sector boundary'
 $vfs=[IO.File]::ReadAllText((Join-Path $repo 'examples/native_v13_machine/vfs_ro.dby'))
 Assert-True($vfs.Contains('import "dbfs1_ro.dby" as dbfs'))'VFS does not depend on DBFS1'
 Assert-True(-not($vfs-match'(?i)sector_service|storage_ide|native\.io_|pci\.'))'VFS leaks hardware boundary'
}
function New-DbfsFixture([string]$name){
 $path=Join-Path $root $name;[byte[]]$disk=New-Object byte[] (8MB)
 [byte[]]$payload=[Text.Encoding]::ASCII.GetBytes("DBYTE V13 DBFS1 READONLY`n")
 $super=64*512
 [byte[]]$magic=[Text.Encoding]::ASCII.GetBytes('DBFS1');[Array]::Copy($magic,0,$disk,$super,$magic.Length)
 Put-U32 $disk ($super+8) 1;Put-U32 $disk ($super+12) 512;Put-U32 $disk ($super+16) 64
 Put-U32 $disk ($super+20) 16320;Put-U32 $disk ($super+24) 65;Put-U32 $disk ($super+28) 4
 Put-U32 $disk ($super+32) 69;Put-U32 $disk ($super+36) 64;Put-U32 $disk ($super+40) 8
 Put-U32 $disk ($super+44) 77;Put-U32 $disk ($super+48) 16307;Put-U32 $disk ($super+52) 1;Put-U32 $disk ($super+56) 64
 Put-U32 $disk ($super+60) (Fnv1a $disk $super 60)
 $bitmap=65*512;$disk[$bitmap]=255;$disk[$bitmap+1]=1
 $inodeBase=69*512;$rootInode=$inodeBase+64
 Put-U32 $disk $rootInode 1;Put-U32 $disk ($rootInode+4) 2;Put-U32 $disk ($rootInode+8) 4096;Put-U32 $disk ($rootInode+12) 8
 for($i=0;$i-lt8;$i++){Put-U32 $disk ($rootInode+16+4*$i) ([uint32]$i)}
 $fileInode=$inodeBase+128
 Put-U32 $disk $fileInode 1;Put-U32 $disk ($fileInode+4) 1;Put-U32 $disk ($fileInode+8) ([uint32]$payload.Length);Put-U32 $disk ($fileInode+12) 1;Put-U32 $disk ($fileInode+16) 8
 for($i=1;$i-lt8;$i++){Put-U32 $disk ($fileInode+16+4*$i) ([uint32]::MaxValue)}
 $dir=77*512;Put-U32 $disk $dir 2;Put-U32 $disk ($dir+4) 1;Put-U32 $disk ($dir+8) 9
 [byte[]]$fileName=[Text.Encoding]::ASCII.GetBytes('HELLO.TXT');[Array]::Copy($fileName,0,$disk,$dir+12,$fileName.Length)
 $data=85*512;[Array]::Copy($payload,0,$disk,$data,$payload.Length)
 [IO.File]::WriteAllBytes($path,$disk)
 return [pscustomobject]@{path=$path;sha256=Hash $path;payload=$payload;payload_hash=('{0:X8}'-f(Fnv1a $payload 0 $payload.Length));payload_bytes=$payload.Length}
}
function Clone-Corrupt([string]$source,[string]$name,[string]$kind){
 $path=Join-Path $root $name;[byte[]]$b=[IO.File]::ReadAllBytes($source)
 if($kind-eq'super'){$b[(64*512)+60]=$b[(64*512)+60]-bxor1}
 elseif($kind-eq'bitmap'){$b[65*512]=$b[65*512]-band0xfe}
 else{throw "unknown corruption $kind"}
 [IO.File]::WriteAllBytes($path,$b);return [pscustomobject]@{path=$path;sha256=Hash $path;kind=$kind}
}

function Build-Filesystem([string]$set,[string]$opt,[string]$date){
 $dir=Join-Path $root "$set-o$opt";New-Item -ItemType Directory -Path $dir|Out-Null
 $obj=Join-Path $dir 'kernel.o';$elf=Join-Path $dir 'kernel.elf'
 foreach($format in @('obj','elf')){
  $output=if($format-eq'obj'){$obj}else{$elf}
  Run $compiler @('build','examples/native_v13_machine/kernel_g4_fs.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit',$format,'-o',$output) "filesystem $set O$opt $format"|Out-Null
 }
 Run 'objdump.exe' @('-d','-Mintel',$elf) 'disassembly'|Set-Content (Join-Path $dir 'disassembly.txt')
 $headers=Run 'objdump.exe' @('-p',$elf) 'ELF headers'
 Assert-True(-not($headers-match'INTERP|DYNAMIC'))'dynamic runtime present'
 Run 'wsl.exe' @('-d','Debian','--','grub-file','--is-x86-multiboot2',(LinuxPath $elf)) 'Multiboot2 validation'|Out-Null
 $tree=Join-Path $dir 'iso';$grub=Join-Path $tree 'boot/grub';New-Item -ItemType Directory -Path $grub -Force|Out-Null
 Copy-Item -LiteralPath $elf -Destination (Join-Path $tree 'boot/kernel.elf')
 [IO.File]::WriteAllText((Join-Path $grub 'grub.cfg'),"set timeout=0`nmenuentry 'DByte filesystem read-only' {`n multiboot2 /boot/kernel.elf`n boot`n}`n")
 $iso=Join-Path $dir 'filesystem.iso'
 Run 'wsl.exe' @('-d','Debian','--','grub-mkrescue','-o',(LinuxPath $iso),"--modification-date=$date",'--set_all_file_dates',$date,(LinuxPath $tree)) 'deterministic ISO'|Out-Null
 return [pscustomobject]@{set=$set;opt=$opt;obj=$obj;elf=$elf;iso=$iso;object_hash=Hash $obj;elf_hash=Hash $elf;iso_hash=Hash $iso}
}
function Boot-Filesystem($build,$fixture,[int]$ram,[string]$kind){
 $dir=Join-Path $root "run-o$($build.opt)-${ram}m-$kind";New-Item -ItemType Directory -Path $dir|Out-Null
 # Use a disposable ordinary raw file without snapshot mode: before/after hashing
 # must detect guest writes, not merely verify that a backing image was protected.
 $disk=Join-Path $dir 'test.raw';Copy-Item -LiteralPath $fixture.path -Destination $disk
 $before=Hash $disk;$serial=Join-Path $dir 'serial.log';$trace=Join-Path $dir 'trace.log'
 Assert-True((Hash $build.iso)-ceq$build.iso_hash)'boot artifact changed'
 $a=@('-cdrom',$build.iso,'-boot','order=d','-m',"${ram}M",'-display','none','-monitor','none','-serial',"file:$serial",'-d','int,cpu_reset,guest_errors','-D',$trace,'-no-reboot','-no-shutdown','-drive',"file=$disk,format=raw,if=ide,index=0,media=disk,cache=directsync")
 [IO.File]::WriteAllText((Join-Path $dir 'qemu-command.txt'),"$qemu $($a-join' ')")
 $p=Start-Process -FilePath $qemu -ArgumentList $a -WindowStyle Hidden -PassThru;$owned.Add($p.Id)
 $text='';$entryResets=$null
 try{
  $deadline=[DateTime]::UtcNow.AddSeconds(45)
  do{
   Start-Sleep -Milliseconds 100
   if(Test-Path $serial){
    $text=Shared $serial
    if($text.Length-gt0-and$null-eq$entryResets-and(Test-Path $trace)){$entryResets=([regex]::Matches((Shared $trace),'CPU Reset')).Count}
    if($text-match'G4 FILESYSTEM READONLY COMPLETE|G4FS ERROR'){Start-Sleep -Milliseconds 200;$text=Shared $serial;break}
   }
  }while(-not$p.HasExited-and[DateTime]::UtcNow-lt$deadline)
 }finally{if(-not$p.HasExited){Stop-Process -Id $p.Id;$p.WaitForExit()};$owned.Remove($p.Id)|Out-Null;$p.Dispose()}
 $after=Hash $disk;Assert-True($before-ceq$after)'filesystem guest mutated disk'
 $t=[IO.File]::ReadAllText($trace);$faults=([regex]::Matches($t,'v=0[06de]|Triple fault')).Count;$resets=([regex]::Matches($t,'CPU Reset')).Count
 Assert-True($faults-eq0)'unexpected CPU fault'
 Assert-True($null-ne$entryResets-and$resets-eq$entryResets)'missing entry interval or reset after entry'
 Assert-True($text.Contains("G3 COMPLETE`r`n"))'G3 predecessor did not complete'
 $start=$text.IndexOf('DBYTE V13 G4 FILESYSTEM');Assert-True($start-ge0)'filesystem entry missing'
 if($kind-eq'positive'){
  $expected="DBYTE V13 G4 FILESYSTEM`r`nDBFS1 MOUNT OK`r`nVFS LOOKUP OK inode=00000002 size=$('{0:X8}'-f$fixture.payload_bytes)`r`nVFS READ OK bytes=$('{0:X8}'-f$fixture.payload_bytes) hash=$($fixture.payload_hash)`r`nG4 FILESYSTEM READONLY COMPLETE`r`n"
 }else{$expected="DBYTE V13 G4 FILESYSTEM`r`nG4FS ERROR MOUNT status=00000004`r`n"}
 Assert-True($text.Substring($start)-ceq$expected)"filesystem exact receipt mismatch: $dir"
 return [pscustomobject]@{opt=$build.opt;ram=$ram;kind=$kind;disk_before=$before;disk_after=$after;serial_hash=Hash $serial;trace_hash=Hash $trace;faults=$faults;entry_resets=$entryResets;final_resets=$resets;iso_hash=$build.iso_hash}
}
Push-Location $repo
try{
 Assert-True(((Run git.exe @('rev-parse','HEAD') 'HEAD').Trim())-ceq$expectedHead)'HEAD drift'
 Assert-True(((Run git.exe @('rev-parse','v12.4.0^{}') 'frozen tag').Trim())-ceq$V13FrozenBaseHead)'tag drift'
 Check-ReadOnlySource
 Run cargo.exe @('build','-q','-p','dbyte_cli','--bin','dbytec') 'fresh compiler'|Out-Null
 $fixture=New-DbfsFixture 'valid.raw';$super=Clone-Corrupt $fixture.path 'bad-super.raw' 'super';$bitmap=Clone-Corrupt $fixture.path 'bad-bitmap.raw' 'bitmap'
 Assert-True($fixture.payload_bytes-eq25)'payload length self-test'
 $epoch=[int64](Run git.exe @('show','-s','--format=%ct','HEAD') 'epoch').Trim()
 $date=[DateTimeOffset]::FromUnixTimeSeconds($epoch).UtcDateTime.ToString('yyyyMMddHHmmss00',[Globalization.CultureInfo]::InvariantCulture)
 $builds=@();$receipts=@()
 foreach($opt in @('0','1')){
  $a=Build-Filesystem 'A' $opt $date;$b=Build-Filesystem 'B' $opt $date
  Assert-True($a.object_hash-ceq$b.object_hash-and$a.elf_hash-ceq$b.elf_hash-and$a.iso_hash-ceq$b.iso_hash)"O$opt nondeterministic artifacts"
  $builds+=@($a,$b)
  foreach($ram in @(32,64)){$receipts+=Boot-Filesystem $a $fixture $ram 'positive'}
  $receipts+=Boot-Filesystem $a $super 64 'super';$receipts+=Boot-Filesystem $a $bitmap 64 'bitmap'
 }
 $receipts|ConvertTo-Json -Depth 5|Set-Content (Join-Path $root 'qemu-receipts.json')
 $builds|ConvertTo-Json -Depth 5|Set-Content (Join-Path $root 'builds.json')
 if($StorageReceipt){
  Assert-V13StorageReceipt $repo $StorageReceipt|Out-Null
  [pscustomobject]@{receipt=(Resolve-Path -LiteralPath $StorageReceipt).Path;sha256=Hash $StorageReceipt}|ConvertTo-Json|Set-Content (Join-Path $root 'storage-master.txt')
 }else{
  Run powershell.exe @('-NoProfile','-File',(Join-Path $repo 'scripts/verify_v13_generation4_storage.ps1')) 'fresh storage master'|Set-Content (Join-Path $root 'storage-master.txt')
 }
 Run git.exe @('diff','--check') 'diff check'|Out-Null
 Run git.exe @('diff','--binary') 'tracked diff'|Set-Content (Join-Path $root 'tracked.diff')
 $inputs=@(Run git.exe @('ls-files','--cached','--others','--exclude-standard') 'source manifest')-split"`n"
 $manifest=@($inputs|Where-Object{$_-match'^(crates/|examples/native_v13_machine/|runtime/|scripts/|Cargo\.)'-and(Test-Path -LiteralPath $_ -PathType Leaf)}|Sort-Object -Unique|ForEach-Object{[pscustomobject]@{path=$_;sha256=Hash $_}})
 $manifest|ConvertTo-Json -Depth 4|Set-Content (Join-Path $root 'input-manifest.json')
 [pscustomobject]@{status='PASS';generation='v13-g4-filesystem-readonly';head=$expectedHead;compiler_sha256=Hash $compiler;manifest_sha256=Hash (Join-Path $root 'input-manifest.json');fixture_sha256=$fixture.sha256;payload_hash=$fixture.payload_hash;qemu_receipts=$receipts.Count;storage_master='PASS';real_biostar='PENDING';real_storage='DISARMED';v13_final='NOT_CREATED'}|ConvertTo-Json|Set-Content (Join-Path $root 'RESULT.json')
 "V13 G4 FILESYSTEM READONLY: PASS`nEVIDENCE ROOT: $root"
}finally{Pop-Location;foreach($pidOwned in $owned){$process=Get-Process -Id $pidOwned -ErrorAction SilentlyContinue;if($process){Stop-Process -InputObject $process}}}
