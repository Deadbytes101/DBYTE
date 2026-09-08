$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'v13_release_identity.ps1')
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$root=Join-Path $repo ('tmp/v13_g1/generation1-'+[Guid]::NewGuid().ToString('N'))
$compiler=Join-Path $repo 'target/debug/dbytec.exe'
$qemu='C:/Program Files/qemu/qemu-system-x86_64.exe'
$expectedHead = Get-V13ExpectedHead
$g1ReservedEnd=[uint64]0x02100000
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
function HexU32([string]$s){[Convert]::ToUInt32($s,16)}
function Validate-Receipt([string]$text,[int]$ram,[string]$opt){
 Assert-True(-not$text.Contains('G1 ERROR'))"O$opt ${ram}M reported G1 ERROR"
 $lines=@($text -split "`r?`n"|Where-Object{$_ -ne ''})
 Assert-True($lines.Count-ge7)"O$opt ${ram}M receipt too short"
 Assert-True($lines[0]-ceq'DBYTE V13 G1')"O$opt ${ram}M bad banner"
 $boot=[regex]::Match($text,'BOOTINFO OK regions=([0-9A-F]{8}) flags=([0-9A-F]{8})')
 $acpi=[regex]::Match($text,'ACPI OK root=([0-9A-F]{16}) kind=([0-9A-F]{8})')
 $apic=[regex]::Match($text,'APIC OK lapic=([0-9A-F]{8}) ioapic=([0-9A-F]{8}) iso=([0-9A-F]{8})')
 $timer=[regex]::Match($text,'TIMER OK pm=([0-9A-F]{8})');$pci=[regex]::Match($text,'PCI OK devices=([0-9A-F]{8})')
 foreach($m in @($boot,$acpi,$apic,$timer,$pci)){Assert-True $m.Success "O$opt ${ram}M receipt field missing"}
 Assert-True((HexU32 $boot.Groups[1].Value)-gt0)"O$opt ${ram}M zero memory regions"
 Assert-True(((HexU32 $boot.Groups[2].Value)-band3)-eq3)"O$opt ${ram}M missing memory or ACPI flag"
 Assert-True([Convert]::ToUInt64($acpi.Groups[1].Value,16)-ne0)"O$opt ${ram}M zero ACPI root"
 Assert-True((HexU32 $apic.Groups[1].Value)-gt0-and(HexU32 $apic.Groups[2].Value)-gt0)"O$opt ${ram}M APIC inventory empty"
 Assert-True((HexU32 $timer.Groups[1].Value)-gt0)"O$opt ${ram}M timer port zero"
 $details=[regex]::Matches($text,'(?m)^PCI [0-9A-F]{2}:[0-9A-F]{2}\.[0-9A-F]{2} [0-9A-F]{4}:[0-9A-F]{4} class=[0-9A-F]{2}:[0-9A-F]{2}:[0-9A-F]{2}\r?$')
 $deviceCount=HexU32 $pci.Groups[1].Value
 Assert-True($deviceCount-gt0)"O$opt ${ram}M PCI empty"
 Assert-True($details.Count-eq$deviceCount)"O$opt ${ram}M PCI detail/count mismatch"
 Assert-True($lines[-1]-ceq'G1 COMPLETE')"O$opt ${ram}M completion is not terminal receipt"
 $positions=@($text.IndexOf('BOOTINFO OK'),$text.IndexOf('ACPI OK'),$text.IndexOf('APIC OK'),$text.IndexOf('TIMER OK'),$text.IndexOf('PCI OK'),$text.IndexOf('G1 COMPLETE'))
 for($i=1;$i-lt$positions.Count;$i++){Assert-True($positions[$i]-gt$positions[$i-1])"O$opt ${ram}M receipt order invalid"}
 return [pscustomobject]@{opt=$opt;ram=$ram;regions=HexU32 $boot.Groups[1].Value;flags=('0x'+$boot.Groups[2].Value);acpi_root=('0x'+$acpi.Groups[1].Value);acpi_kind=HexU32 $acpi.Groups[2].Value;lapic=HexU32 $apic.Groups[1].Value;ioapic=HexU32 $apic.Groups[2].Value;iso=HexU32 $apic.Groups[3].Value;timer=('0x'+$timer.Groups[1].Value);pci_devices=$deviceCount}
}
function Inspect-Elf([string]$elf,[string]$label){
 $w=LinuxPath $elf;Run 'wsl.exe' @('-d','Debian','--','grub-file','--is-x86-multiboot2',$w) "grub-file $label"|Out-Null
 $readelf=Run 'wsl.exe' @('-d','Debian','--','readelf','-lW',$w) "readelf $label";[IO.File]::WriteAllText((Join-Path (Split-Path $elf) 'readelf.txt'),$readelf)
 Assert-True(-not$readelf.Contains('INTERP'))"$label has ELF interpreter"
 $max=[uint64]0
 foreach($line in($readelf-split"`n")){if($line.TrimStart().StartsWith('LOAD ')){ $f=@($line.Trim()-split'\s+');$v=[Convert]::ToUInt64($f[2].Substring(2),16);$m=[Convert]::ToUInt64($f[5].Substring(2),16);$e=$v+$m;if($e-gt$max){$max=$e}}}
 Assert-True($max-le$g1ReservedEnd)"$label loaded end 0x$($max.ToString('X')) exceeds G1 reservation"
 return $max
}
function Build-Set([string]$set,[string]$opt){
 $dir=Join-Path $root "$set-o$opt";New-Item -ItemType Directory -Path $dir -Force|Out-Null
 $obj=Join-Path $dir 'kernel.o';$elf=Join-Path $dir 'kernel.elf'
 $dump=Run $compiler @('build','examples/native_v13_machine/kernel.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','obj','-o',$obj,'--dump-native-mir','--dump-machine-code') "build $set O$opt object"
 [IO.File]::WriteAllText((Join-Path $dir 'object-dump.txt'),$dump)
 Run $compiler @('build','examples/native_v13_machine/kernel.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','elf','-o',$elf) "build $set O$opt ELF"|Out-Null
 $end=Inspect-Elf $elf "$set O$opt"
 $disasm=Run 'objdump.exe' @('-d','-Mintel',$elf) "objdump $set O$opt";[IO.File]::WriteAllText((Join-Path $dir 'objdump.txt'),$disasm)
 return [pscustomobject]@{set=$set;opt=$opt;dir=$dir;obj=$obj;elf=$elf;obj_sha256=Hash $obj;elf_sha256=Hash $elf;loaded_end=('0x'+$end.ToString('X'))}
}
function Make-Iso($build,[string]$date){
 $isoRoot=Join-Path $build.dir 'iso';$grub=Join-Path $isoRoot 'boot/grub';New-Item -ItemType Directory -Path $grub -Force|Out-Null
 $copy=Join-Path $isoRoot 'boot/kernel.elf';Copy-Item -LiteralPath $build.elf -Destination $copy -Force
 Assert-True((Hash $copy)-ceq$build.elf_sha256)"O$($build.opt) ISO kernel copy hash drift"
 [IO.File]::WriteAllText((Join-Path $grub 'grub.cfg'),"set timeout=0`nset default=0`nmenuentry 'DByte v13 G1' {`n multiboot2 /boot/kernel.elf`n boot`n}`n")
 $iso=Join-Path $build.dir 'dbyte-v13-g1.iso';Run 'wsl.exe' @('-d','Debian','--','grub-mkrescue','-o',(LinuxPath $iso),("--modification-date=$date"),'--set_all_file_dates',$date,(LinuxPath $isoRoot)) "build deterministic ISO $($build.set) O$($build.opt)"|Out-Null
 return [pscustomobject]@{build=$build;path=$iso;sha256=Hash $iso;bytes=(Get-Item $iso).Length}
}
function Run-Qemu($iso,[int]$ram){
 $opt=$iso.build.opt;$dir=Join-Path $root "run-o$opt-${ram}m";New-Item -ItemType Directory -Path $dir -Force|Out-Null
 $serial=Join-Path $dir 'serial.log';$trace=Join-Path $dir 'trace.log';$entryResets=$null;$text=''
 $a=@('-cdrom',$iso.path,'-m',"${ram}M",'-display','none','-monitor','none','-serial',"file:$serial",'-d','int,cpu_reset,guest_errors','-D',$trace,'-no-reboot','-no-shutdown')
 [IO.File]::WriteAllText((Join-Path $dir 'qemu-command.txt'),"$qemu $($a -join ' ')")
 $p=Start-Process -FilePath $qemu -ArgumentList $a -PassThru -WindowStyle Hidden;$owned.Add($p.Id)
 try{$deadline=[DateTime]::UtcNow.AddSeconds(30);do{Start-Sleep -Milliseconds 100;if(Test-Path $serial){$text=Shared $serial;if($text.Length-gt0-and$null-eq$entryResets-and(Test-Path $trace)){$entryResets=([regex]::Matches((Shared $trace),'CPU Reset')).Count};if($text.Contains('G1 COMPLETE')-or$text.Contains('G1 ERROR')){break}}}while(-not$p.HasExited-and[DateTime]::UtcNow-lt$deadline);Assert-True($text.Contains('G1 COMPLETE')-or$text.Contains('G1 ERROR'))"O$opt ${ram}M QEMU timed out"}
 finally{if(-not$p.HasExited){Stop-Process -Id $p.Id -Force;$p.WaitForExit()};$p.Dispose()}
 $traceText=[IO.File]::ReadAllText($trace);$triple=([regex]::Matches($traceText,'Triple fault')).Count;$faults=([regex]::Matches($traceText,'v=0[0de6]')).Count;$finalResets=([regex]::Matches($traceText,'CPU Reset')).Count
 Assert-True($triple-eq0)"O$opt ${ram}M triple fault";Assert-True($faults-eq0)"O$opt ${ram}M #UD/#GP/#PF observed";Assert-True($null-ne$entryResets)"O$opt ${ram}M no entry reset sample";Assert-True($finalResets-eq$entryResets)"O$opt ${ram}M CPU reset after kernel entry"
 $receipt=Validate-Receipt $text $ram $opt;$receipt|Add-Member iso_sha256 $iso.sha256;$receipt|Add-Member serial_sha256 (Hash $serial);$receipt|Add-Member trace_sha256 (Hash $trace);$receipt|Add-Member entry_resets $entryResets;$receipt|Add-Member final_resets $finalResets;$receipt|Add-Member faults $faults;$receipt|Add-Member triple_faults $triple
 $receipt|ConvertTo-Json|Set-Content (Join-Path $dir 'receipt.json');return $receipt
}
function Write-Identity($selectedIso,$selectedBuild){
 $diff=(Run 'git.exe' @('diff','--binary','--no-ext-diff','HEAD') 'capture tracked diff');$diffPath=Join-Path $root 'tracked.diff';[IO.File]::WriteAllText($diffPath,$diff,[Text.UTF8Encoding]::new($false))
 $files=@(Get-ChildItem (Join-Path $repo 'examples/native_v13_machine') -File -Filter '*.dby';Get-ChildItem (Join-Path $repo 'scripts') -File -Filter 'verify_v13_*.ps1';Get-Item (Join-Path $repo 'runtime/v12.1.0/serial.dby'))|Sort-Object FullName -Unique
 $manifest=@($files|ForEach-Object{[pscustomobject]@{path=$_.FullName.Substring($repo.Length+1).Replace('\','/');bytes=$_.Length;sha256=Hash $_.FullName}})
 $manifestPath=Join-Path $root 'input-manifest.json';$manifest|ConvertTo-Json -Depth 4|Set-Content $manifestPath
 $identity=[pscustomobject]@{head=(Run 'git.exe' @('rev-parse','HEAD') 'read HEAD').Trim();tracked_diff_sha256=Hash $diffPath;input_manifest_sha256=Hash $manifestPath;compiler_sha256=Hash $compiler;kernel_elf_sha256=$selectedBuild.elf_sha256;iso_sha256=$selectedIso.sha256;iso_bytes=$selectedIso.bytes}
 $identity|ConvertTo-Json|Set-Content (Join-Path $root 'candidate-identity.json');return $identity
}
function Check-PciReadOnly(){
 $all=(Get-ChildItem (Join-Path $repo 'examples/native_v13_machine') -Filter '*.dby' -File|ForEach-Object{[IO.File]::ReadAllText($_.FullName)})-join"`n"
 $pci=[IO.File]::ReadAllText((Join-Path $repo 'examples/native_v13_machine/pci.dby'))
 Assert-True(([regex]::Matches($all,'native\.io_out32\s*\(')).Count-eq1)'G1 has more than one 32-bit port write call site'
 Assert-True($pci.Contains('native.io_out32(CONFIG_ADDRESS, address(bus, device, function, offset))'))'PCI selector write is not bound to CF8 alias'
 Assert-True($pci.Contains('native.io_in32(CONFIG_DATA)'))'PCI data read is not bound to CFC alias'
 Assert-True(-not$pci.Contains('native.io_out32(CONFIG_DATA'))'PCI configuration data write detected'
 Assert-True(-not($pci-match'(?i)bar sizing|bus master|bus_master'))'PCI configuration mutation marker detected'
}
Push-Location $repo
try{
 $head=(Run 'git.exe' @('rev-parse','HEAD') 'verify HEAD').Trim();Assert-True($head-ceq$expectedHead)'unexpected HEAD'
 Assert-True(((Run 'git.exe' @('rev-parse','v12.4.0^{}') 'verify frozen tag').Trim()-ceq$V13FrozenBaseHead))'v12.4.0 peeled tag drifted'
 Check-PciReadOnly
 Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_value_formation.ps1')) 'value formation gate'|Out-Null
 Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_range_regression.ps1'),'-OptLevel','0') 'range O0 gate'|Out-Null
 Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_range_regression.ps1'),'-OptLevel','1') 'range O1 gate'|Out-Null
 Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_boot_info_regression.ps1')) 'BootInfo gate'|Out-Null
 Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_acpi_regression.ps1')) 'ACPI gate'|Out-Null
 Run 'cargo.exe' @('build','-q','-p','dbyte_cli','--bin','dbytec') 'fresh dbytec'|Out-Null
 Run 'cargo.exe' @('test','-q','-p','dbyte_native') 'full dbyte_native tests'|Out-Null
 Run 'cargo.exe' @('fmt','--all','--','--check') 'cargo fmt check'|Out-Null
 Run 'cargo.exe' @('clippy','-q','-p','dbyte_hir','-p','dbyte_native','--','-D','warnings') 'native clippy'|Out-Null
 $epoch=[int64](Run 'git.exe' @('show','-s','--format=%ct','HEAD') 'read packaging epoch').Trim();$isoDate=[DateTimeOffset]::FromUnixTimeSeconds($epoch).UtcDateTime.ToString('yyyyMMddHHmmss00',[Globalization.CultureInfo]::InvariantCulture)
 $builds=@()
 foreach($opt in @('0','1')){$a=Build-Set 'A' $opt;$b=Build-Set 'B' $opt;Assert-True($a.obj_sha256-ceq$b.obj_sha256)"O$opt object is not deterministic";Assert-True($a.elf_sha256-ceq$b.elf_sha256)"O$opt ELF is not deterministic";$builds+=@($a,$b)}
 $isos=@()
 foreach($opt in @('0','1')){$a=$builds|Where-Object{$_.set-eq'A'-and$_.opt-eq$opt};$b=$builds|Where-Object{$_.set-eq'B'-and$_.opt-eq$opt};$ia=Make-Iso $a $isoDate;$ib=Make-Iso $b $isoDate;Assert-True($ia.sha256-ceq$ib.sha256)"O$opt ISO is not deterministic";$isos+=@($ia,$ib)}
 $receipts=@()
 foreach($opt in @('0','1')){$iso=$isos|Where-Object{$_.build.set-eq'A'-and$_.build.opt-eq$opt};foreach($ram in @(32,64)){$receipts+=Run-Qemu $iso $ram}}
 $receipts|ConvertTo-Json -Depth 5|Set-Content (Join-Path $root 'qemu-receipts.json')
 $boundary=Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_historical_boundary.ps1')) 'historical frozen boundary';[IO.File]::WriteAllText((Join-Path $root 'historical-boundary.txt'),$boundary)
 $suffix='v13g1-'+[Guid]::NewGuid().ToString('N').Substring(0,8)
 $none=Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_modern_native_none.ps1'),'-MilestoneVersion',$suffix) 'historical modern-none behavior';[IO.File]::WriteAllText((Join-Path $root 'historical-modern-none.txt'),$none)
 $wide=Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_i686_wide_integer.ps1'),'-MilestoneVersion',$suffix) 'historical i686-wide behavior';[IO.File]::WriteAllText((Join-Path $root 'historical-i686-wide.txt'),$wide)
 $selectedBuild=$builds|Where-Object{$_.set-eq'A'-and$_.opt-eq'1'};$selectedIso=$isos|Where-Object{$_.build.set-eq'A'-and$_.build.opt-eq'1'}
 $physical=Join-Path $root 'physical-candidate';New-Item -ItemType Directory -Path $physical -Force|Out-Null
 $physicalElf=Join-Path $physical 'dbyte-v13-g1-biostar.elf';$physicalIso=Join-Path $physical 'dbyte-v13-g1-biostar.iso';Copy-Item $selectedBuild.elf $physicalElf -Force;Copy-Item $selectedIso.path $physicalIso -Force
 Assert-True((Hash $physicalElf)-ceq$selectedBuild.elf_sha256)'physical ELF copy hash drift';Assert-True((Hash $physicalIso)-ceq$selectedIso.sha256)'physical ISO copy hash drift'
 $identity=Write-Identity $selectedIso $selectedBuild
 $result=[pscustomobject]@{status='PASS';generation='v13-g1-bios-read-only';packaging_date=$isoDate;candidate_identity=$identity;physical_elf=$physicalElf;physical_elf_sha256=Hash $physicalElf;physical_iso=$physicalIso;physical_iso_sha256=Hash $physicalIso;physical_iso_bytes=(Get-Item $physicalIso).Length;qemu_receipts=$receipts.Count;real_biostar='PENDING';real_storage='DISARMED';full_os='PENDING';v13_final='NOT_CREATED'}
 $result|ConvertTo-Json -Depth 6|Set-Content (Join-Path $root 'RESULT.json')
 Write-Output "V13 GENERATION 1 VERIFIER: PASS"
 Write-Output "EVIDENCE ROOT: $root"
 Write-Output "PHYSICAL ISO: $physicalIso"
 Write-Output "PHYSICAL ISO SHA256: $($result.physical_iso_sha256)"
}finally{
 Pop-Location
 $deadline=[DateTime]::UtcNow.AddSeconds(3);do{$orphans=@($owned|Where-Object{$candidate=Get-CimInstance Win32_Process -Filter ("ProcessId = {0}" -f $_) -ErrorAction SilentlyContinue; $null -ne $candidate -and $candidate.Name -ieq [IO.Path]::GetFileName($qemu) -and $candidate.CommandLine -like ("*" + $root + "*")});if($orphans.Count-eq0-or[DateTime]::UtcNow-ge$deadline){break};Start-Sleep -Milliseconds 100}while($true)
 if($orphans.Count-ne0){throw "G1 verifier left owned QEMU processes: $($orphans -join ', ')"}
}
