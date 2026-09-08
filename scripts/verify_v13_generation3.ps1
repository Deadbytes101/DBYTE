$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'v13_release_identity.ps1')
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$root=Join-Path $repo ('tmp/v13_g3/generation3-'+[Guid]::NewGuid().ToString('N'))
$compiler=Join-Path $repo 'target/debug/dbytec.exe'
$qemu='C:/Program Files/qemu/qemu-system-x86_64.exe'
$expectedHead = Get-V13ExpectedHead
$g2ReservedEnd=[uint64]0x02100000
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
function Run-ExpectFailure([string]$exe,[string[]]$a,[string]$needle,[string]$d,[string]$cwd=$repo){
 $script:sequence++;$log=Join-Path $root ('command-{0:D3}.txt'-f $script:sequence)
 $save=$ErrorActionPreference;$ErrorActionPreference='Continue'
 try{Push-Location $cwd;try{$o=& $exe @a 2>&1;$c=$LASTEXITCODE}finally{Pop-Location}}finally{$ErrorActionPreference=$save}
 $text=$o-join"`n";[IO.File]::WriteAllText($log,"CWD=$cwd`nCOMMAND=$exe $($a -join ' ')`nEXIT=$c`n$text")
 Assert-True($c-ne0)"$d unexpectedly succeeded";Assert-True($text.Contains($needle))"$d failed without '$needle': $log";return $text
}
function LinuxPath([string]$p){(Run 'wsl.exe' @('-d','Debian','--','wslpath','-a','-u',$p.Replace('\','/')) 'translate WSL path').Trim()}
function Shared([string]$p){
 $s=[IO.File]::Open($p,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
 try{$r=[IO.StreamReader]::new($s,[Text.Encoding]::ASCII,$false,1024,$true);try{return $r.ReadToEnd()}finally{$r.Dispose()}}finally{$s.Dispose()}
}
function HexU32([string]$s){[Convert]::ToUInt32($s,16)}
function Validate-Receipt([string]$text,[int]$ram,[string]$opt,[bool]$bridge){
 Assert-True(-not$text.Contains('G1 ERROR'))"O$opt ${ram}M reported G1 ERROR"
 Assert-True(-not$text.Contains('G2 ERROR'))"O$opt ${ram}M reported G2 ERROR"
 Assert-True(-not$text.Contains('G2B ERROR'))"O$opt ${ram}M reported G2B ERROR"
 Assert-True(-not$text.Contains('G2C ERROR'))"O$opt ${ram}M reported G2C ERROR"
 Assert-True(-not$text.Contains('G3 ERROR'))"O$opt ${ram}M reported G3 ERROR"
 Assert-True(-not$text.Contains('G2B FATAL'))"O$opt ${ram}M reported G2B fatal interrupt"
 Assert-True(-not$text.Contains('G2B PAGEFAULT'))"O$opt ${ram}M reported unexpected page fault"
 $lines=@($text -split "`r?`n"|Where-Object{$_ -ne ''})
 Assert-True($lines.Count-ge30)"O$opt ${ram}M receipt too short"
 Assert-True($lines[0]-ceq'DBYTE V13 G1')"O$opt ${ram}M bad G1 banner"
 $boot=[regex]::Match($text,'BOOTINFO OK regions=([0-9A-F]{8}) flags=([0-9A-F]{8})')
 $acpi=[regex]::Match($text,'ACPI OK root=([0-9A-F]{16}) kind=([0-9A-F]{8})')
 $apic=[regex]::Match($text,'APIC OK lapic=([0-9A-F]{8}) ioapic=([0-9A-F]{8}) iso=([0-9A-F]{8})')
 $timer=[regex]::Match($text,'TIMER OK pm=([0-9A-F]{8})');$pci=[regex]::Match($text,'PCI OK devices=([0-9A-F]{8})')
 $cpu=[regex]::Match($text,'CPU OK max=([0-9A-F]{8}) ext=([0-9A-F]{8})')
 $features=[regex]::Match($text,'CPU FEATURES ecx=([0-9A-F]{8}) edx=([0-9A-F]{8}) ext_edx=([0-9A-F]{8})')
 $control=[regex]::Match($text,'CPU CONTROL cr0=([0-9A-F]{8}) cr3=([0-9A-F]{16}) cr4=([0-9A-F]{8})')
 $efer=[regex]::Match($text,'CPU EFER ([0-9A-F]{16})')
 $tss=[regex]::Match($text,'TSS OK base=([0-9A-F]{16}) ist1=([0-9A-F]{16})');$idt=[regex]::Match($text,'IDT OK base=([0-9A-F]{16})')
 $int3=[regex]::Match($text,'INT3 OK count=([0-9A-F]{8})');$istFrame=[regex]::Match($text,'IST1 FRAME ([0-9A-F]{16})')
 $frames=[regex]::Match($text,'FRAMES OK frame=([0-9A-F]{16}) alloc=([0-9A-F]{8}) owned=([0-9A-F]{8})')
 $paging=[regex]::Match($text,'PAGING OK cr3=([0-9A-F]{16}) pml4=([0-9A-F]{16})')
 $memory=[regex]::Match($text,'MEMORY OK virt=([0-9A-F]{16}) phys=([0-9A-F]{16}) value=([0-9A-F]{8})')
 $protect=[regex]::Match($text,'PROTECT OK flags=([0-9A-F]{16}) wp=([0-9A-F]{8}) nxe=([0-9A-F]{8})')
 $reclaim=[regex]::Match($text,'RECLAIM OK maps=([0-9A-F]{8}) unmaps=([0-9A-F]{8}) releases=([0-9A-F]{8}) owned=([0-9A-F]{8})')
 $topo=[regex]::Match($text,'PCI TOPO buses=([0-9A-F]{8}) bridges=([0-9A-F]{8}) reads=([0-9A-F]{8})')
 $lapicRun=[regex]::Match($text,'LAPIC OK base=([0-9A-F]{16}) id=([0-9A-F]{8}) ver=([0-9A-F]{8})')
 $ioRun=[regex]::Match($text,'IOAPIC OK ver=([0-9A-F]{8}) redirs=([0-9A-F]{8}) irq0_gsi=([0-9A-F]{8}) flags=([0-9A-F]{4})')
 $mmio=[regex]::Match($text,'MMIO OK lapic=([0-9A-F]{16}) ioapic=([0-9A-F]{16})')
 $cal=[regex]::Match($text,'TIMER CAL pm_hz=([0-9A-F]{8}) lapic_per_ms=([0-9A-F]{8}) ticks=([0-9A-F]{8}) spurious=([0-9A-F]{8})')
 foreach($m in @($boot,$acpi,$apic,$timer,$pci,$cpu,$features,$control,$efer,$tss,$idt,$int3,$istFrame,$frames,$paging,$memory,$protect,$reclaim,$topo,$lapicRun,$ioRun,$mmio,$cal)){Assert-True $m.Success "O$opt ${ram}M G3 receipt field missing"}
 Assert-True((HexU32 $boot.Groups[1].Value)-gt0)"O$opt ${ram}M zero memory regions"
 Assert-True(((HexU32 $boot.Groups[2].Value)-band3)-eq3)"O$opt ${ram}M missing memory or ACPI flag"
 Assert-True([Convert]::ToUInt64($acpi.Groups[1].Value,16)-ne0)"O$opt ${ram}M zero ACPI root"
 Assert-True((HexU32 $apic.Groups[1].Value)-gt0-and(HexU32 $apic.Groups[2].Value)-gt0)"O$opt ${ram}M APIC inventory empty"
 Assert-True((HexU32 $timer.Groups[1].Value)-gt0)"O$opt ${ram}M PM timer port zero"
 $details=[regex]::Matches($text,'(?m)^PCI [0-9A-F]{2}:[0-9A-F]{2}\.[0-9A-F]{2} [0-9A-F]{4}:[0-9A-F]{4} class=[0-9A-F]{2}:[0-9A-F]{2}:[0-9A-F]{2}\r?$')
 $deviceCount=HexU32 $pci.Groups[1].Value;Assert-True($deviceCount-gt0)"O$opt ${ram}M PCI empty";Assert-True($details.Count-eq$deviceCount)"O$opt ${ram}M PCI detail/count mismatch"
 $busCount=HexU32 $topo.Groups[1].Value;$bridgeCount=HexU32 $topo.Groups[2].Value;$readCount=HexU32 $topo.Groups[3].Value
 Assert-True($busCount-ge1)"O$opt ${ram}M topology has no root bus";Assert-True($readCount-gt0-and$readCount-le512)"O$opt ${ram}M PCI traversal is not bounded"
 if($bridge){Assert-True($busCount-ge2-and$bridgeCount-ge1)"O$opt ${ram}M bridge topology was not traversed";Assert-True($text-match'(?m)^PCI (?!00:)[0-9A-F]{2}:')"O$opt ${ram}M no downstream PCI device observed"}else{Assert-True($busCount-eq1-and$bridgeCount-eq0)"O$opt ${ram}M unexpected baseline PCI topology"}
 $featureEdx=HexU32 $features.Groups[2].Value;$extEdx=HexU32 $features.Groups[3].Value
 Assert-True(($featureEdx-band[uint32]512)-eq[uint32]512)"O$opt ${ram}M APIC CPU feature absent";Assert-True(($extEdx-band[uint32]536870912)-eq[uint32]536870912)"O$opt ${ram}M long mode feature absent"
 $cr0=HexU32 $control.Groups[1].Value;$cr3=[Convert]::ToUInt64($control.Groups[2].Value,16);$cr4=HexU32 $control.Groups[3].Value;$eferValue=[Convert]::ToUInt64($efer.Groups[1].Value,16)
 Assert-True(($cr0-band[uint32]2147483649)-eq[uint32]2147483649)"O$opt ${ram}M CR0 invariant failed";Assert-True($cr3-ne0-and($cr3-band[uint64]0xFFF)-eq0)"O$opt ${ram}M CR3 invariant failed"
 Assert-True(($cr4-band[uint32]0x20)-eq[uint32]0x20)"O$opt ${ram}M CR4 PAE invariant failed";Assert-True(($eferValue-band[uint64]0x500)-eq[uint64]0x500)"O$opt ${ram}M EFER invariant failed"
 $istTop=[Convert]::ToUInt64($tss.Groups[2].Value,16);$frame=[Convert]::ToUInt64($istFrame.Groups[1].Value,16);Assert-True((HexU32 $int3.Groups[1].Value)-eq1)"O$opt ${ram}M INT3 count mismatch";Assert-True($frame-ge($istTop-[uint64]0x4000)-and$frame-lt$istTop)"O$opt ${ram}M INT3 did not use IST1"
 Assert-True((HexU32 $frames.Groups[2].Value)-eq1-and(HexU32 $frames.Groups[3].Value)-eq1)"O$opt ${ram}M frame ownership receipt mismatch"
 Assert-True([Convert]::ToUInt64($paging.Groups[1].Value,16)-eq[Convert]::ToUInt64($paging.Groups[2].Value,16))"O$opt ${ram}M CR3/PML4 mismatch"
 Assert-True($memory.Groups[3].Value-ceq'0D13C0DE')"O$opt ${ram}M mapped memory value mismatch"
 Assert-True((HexU32 $protect.Groups[2].Value)-eq1-and(HexU32 $protect.Groups[3].Value)-eq1)"O$opt ${ram}M WP/NXE receipt mismatch"
 Assert-True((HexU32 $reclaim.Groups[1].Value)-eq1-and(HexU32 $reclaim.Groups[2].Value)-eq1-and(HexU32 $reclaim.Groups[3].Value)-eq1-and(HexU32 $reclaim.Groups[4].Value)-eq0)"O$opt ${ram}M reclaim receipt mismatch"
 $lapicBase=[Convert]::ToUInt64($lapicRun.Groups[1].Value,16);$lapicVer=HexU32 $lapicRun.Groups[3].Value;$ioRedirs=HexU32 $ioRun.Groups[2].Value;$irq0=HexU32 $ioRun.Groups[3].Value
 Assert-True($lapicBase-ne0-and($lapicBase-band[uint64]0xFFF)-eq0)"O$opt ${ram}M LAPIC base invalid";Assert-True(($lapicVer-band[uint32]0xFF)-ne0)"O$opt ${ram}M LAPIC version invalid"
 Assert-True($ioRedirs-gt0-and$irq0-lt$ioRedirs)"O$opt ${ram}M IOAPIC routing receipt invalid"
 $lapicFlags=[Convert]::ToUInt64($mmio.Groups[1].Value,16);$ioFlags=[Convert]::ToUInt64($mmio.Groups[2].Value,16)
 Assert-True(($lapicFlags-band[uint64]0x99)-eq[uint64]0x99)"O$opt ${ram}M LAPIC mapping lacks present/PWT/PCD/huge";Assert-True(($ioFlags-band[uint64]0x99)-eq[uint64]0x99)"O$opt ${ram}M IOAPIC mapping lacks present/PWT/PCD/huge"
 Assert-True((HexU32 $cal.Groups[1].Value)-eq3579545)"O$opt ${ram}M PM timer frequency receipt mismatch";Assert-True((HexU32 $cal.Groups[2].Value)-gt0)"O$opt ${ram}M LAPIC calibration is zero";Assert-True((HexU32 $cal.Groups[3].Value)-ge3)"O$opt ${ram}M LAPIC timer interrupts below acceptance";Assert-True((HexU32 $cal.Groups[4].Value)-eq0)"O$opt ${ram}M spurious interrupts observed"
 Assert-True($lines[-1]-ceq'G3 COMPLETE')"O$opt ${ram}M completion is not terminal G3 receipt"
 $positions=@($text.IndexOf('BOOTINFO OK'),$text.IndexOf('ACPI OK'),$text.IndexOf('PCI OK'),$text.IndexOf('G1 COMPLETE'),$text.IndexOf('G2 CPU COMPLETE'),$text.IndexOf('G2B COMPLETE'),$text.IndexOf('G2C COMPLETE'),$text.IndexOf('PCI TOPO'),$text.IndexOf('DBYTE V13 G3 APIC'),$text.IndexOf('LAPIC OK'),$text.IndexOf('IOAPIC OK'),$text.IndexOf('MMIO OK'),$text.IndexOf('TIMER CAL'),$text.IndexOf('G3 COMPLETE'))
 for($i=1;$i-lt$positions.Count;$i++){Assert-True($positions[$i]-gt$positions[$i-1])"O$opt ${ram}M G3 receipt order invalid"}
 return [pscustomobject]@{opt=$opt;ram=$ram;bridge_topology=$bridge;regions=HexU32 $boot.Groups[1].Value;pci_devices=$deviceCount;pci_buses=$busCount;pci_bridges=$bridgeCount;pci_reads=$readCount;lapic_base=('0x'+$lapicRun.Groups[1].Value);ioapic_redirs=$ioRedirs;irq0_gsi=$irq0;lapic_ticks_per_ms=HexU32 $cal.Groups[2].Value;timer_test_ticks=HexU32 $cal.Groups[3].Value;spurious=HexU32 $cal.Groups[4].Value}
}
function Inspect-Elf([string]$elf,[string]$label){
 $w=LinuxPath $elf;Run 'wsl.exe' @('-d','Debian','--','grub-file','--is-x86-multiboot2',$w) "grub-file $label"|Out-Null
 $readelf=Run 'wsl.exe' @('-d','Debian','--','readelf','-lW',$w) "readelf $label";[IO.File]::WriteAllText((Join-Path (Split-Path $elf) 'readelf.txt'),$readelf)
 Assert-True(-not$readelf.Contains('INTERP'))"$label has ELF interpreter"
 $max=[uint64]0
 foreach($line in($readelf-split"`n")){if($line.TrimStart().StartsWith('LOAD ')){ $f=@($line.Trim()-split'\s+');$v=[Convert]::ToUInt64($f[2].Substring(2),16);$m=[Convert]::ToUInt64($f[5].Substring(2),16);$e=$v+$m;if($e-gt$max){$max=$e}}}
 Assert-True($max-le$g2ReservedEnd)"$label loaded end 0x$($max.ToString('X')) exceeds G2 reservation"
 return $max
}
function Audit-Opcodes([string]$disasm,[string]$label){
 foreach($needle in @('cpuid','rdmsr','wrmsr','mov    rax,cr0','mov    cr0,rax','mov    rax,cr3','mov    cr3,rax','mov    rax,cr4','mov    cr4,rax','lgdt','ltr','lidt','int3','iretq')){Assert-True($disasm.Contains($needle))"$label missing opcode '$needle'"}
 Assert-True(([regex]::Matches($disasm,'\biretq\b')).Count-ge7)"$label has fewer than seven IRETQ interrupt exits"
 $lines=@($disasm-split"`r?`n");$cpuid=@()
 for($i=0;$i-lt$lines.Count;$i++){if($lines[$i]-match'\tcpuid\s*$'){$cpuid+=$i}}
 Assert-True($cpuid.Count-ge4)"$label has too few CPUID sites"
 foreach($i in $cpuid){$lo=[Math]::Max(0,$i-8);$hi=[Math]::Min($lines.Count-1,$i+8);$window=($lines[$lo..$hi]-join"`n");Assert-True($window-match'push\s+rbx')"$label CPUID lacks RBX save";Assert-True($window-match'pop\s+rbx')"$label CPUID lacks RBX restore"}
 Assert-True($disasm-match'(?s)cpuid.{0,180}mov\s+r11,rbx.{0,180}pop\s+rbx')"$label lacks EBX-result capture before RBX restore"
}
function Audit-Interrupt-Metadata([string]$obj,[string]$label){
 $symbols=Run 'objdump.exe' @('-t','-r',$obj) "interrupt metadata $label"
 [IO.File]::WriteAllText((Join-Path (Split-Path $obj) 'interrupt-metadata.txt'),$symbols)
 foreach($needle in @(
  '__dbyte_interrupt:def:no_error_code:breakpoint',
  '__dbyte_interrupt:def:no_error_code:lapic_timer',
  '__dbyte_interrupt:def:no_error_code:spurious',
  '__dbyte_interrupt:def:no_error_code:invalid_opcode',
  '__dbyte_interrupt:def:error_code:double_fault',
  '__dbyte_interrupt:def:error_code:general_protection',
  '__dbyte_interrupt:def:error_code:page_fault',
  '__dbyte_interrupt:use:003:no_error_code:breakpoint',
  '__dbyte_interrupt:use:006:no_error_code:invalid_opcode',
  '__dbyte_interrupt:use:008:error_code:double_fault',
  '__dbyte_interrupt:use:013:error_code:general_protection',
  '__dbyte_interrupt:use:014:error_code:page_fault',
  '__dbyte_interrupt:use:064:no_error_code:lapic_timer',
  '__dbyte_interrupt:use:255:no_error_code:spurious',
  '__dbyte_interrupt_breakpoint_body',
  '__dbyte_interrupt_lapic_timer_body',
  '__dbyte_interrupt_spurious_body',
  '__dbyte_interrupt_invalid_opcode_body',
  '__dbyte_interrupt_double_fault_body',
  '__dbyte_interrupt_general_protection_body',
  '__dbyte_interrupt_page_fault_body'
 )){Assert-True($symbols.Contains($needle))"$label missing interrupt metadata '$needle'"}
 Assert-True(([regex]::Matches($symbols,'R_X86_64_PC32\s+__dbyte_interrupt_')).Count-ge7)"$label lacks seven stub-to-body PC32 relocations"
}
function Build-Set([string]$set,[string]$opt){
 $dir=Join-Path $root "$set-o$opt";New-Item -ItemType Directory -Path $dir -Force|Out-Null
 $obj=Join-Path $dir 'kernel.o';$elf=Join-Path $dir 'kernel.elf'
 $dump=Run $compiler @('build','examples/native_v13_machine/kernel_g3.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','obj','-o',$obj,'--dump-native-mir','--dump-machine-code') "build G3 $set O$opt object"
 [IO.File]::WriteAllText((Join-Path $dir 'object-dump.txt'),$dump)
 $disasm=Run 'objdump.exe' @('-d','-Mintel',$obj) "objdump G3 $set O$opt object";[IO.File]::WriteAllText((Join-Path $dir 'objdump.txt'),$disasm);Audit-Opcodes $disasm "$set O$opt";Audit-Interrupt-Metadata $obj "$set O$opt"
 Run $compiler @('build','examples/native_v13_machine/kernel_g3.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','elf','-o',$elf) "build G3 $set O$opt ELF"|Out-Null
 $end=Inspect-Elf $elf "$set O$opt"
 return [pscustomobject]@{set=$set;opt=$opt;dir=$dir;obj=$obj;elf=$elf;obj_sha256=Hash $obj;elf_sha256=Hash $elf;loaded_end=('0x'+$end.ToString('X'))}
}
function Make-Iso($build,[string]$date){
 $isoRoot=Join-Path $build.dir 'iso';$grub=Join-Path $isoRoot 'boot/grub';New-Item -ItemType Directory -Path $grub -Force|Out-Null
 $copy=Join-Path $isoRoot 'boot/kernel.elf';Copy-Item -LiteralPath $build.elf -Destination $copy -Force
 Assert-True((Hash $copy)-ceq$build.elf_sha256)"O$($build.opt) ISO kernel copy hash drift"
 [IO.File]::WriteAllText((Join-Path $grub 'grub.cfg'),"set timeout=0`nset default=0`nmenuentry 'DByte v13 G3 APIC' {`n multiboot2 /boot/kernel.elf`n boot`n}`n")
 $iso=Join-Path $build.dir 'dbyte-v13-g3.iso';Run 'wsl.exe' @('-d','Debian','--','grub-mkrescue','-o',(LinuxPath $iso),("--modification-date=$date"),'--set_all_file_dates',$date,(LinuxPath $isoRoot)) "build deterministic G3 ISO $($build.set) O$($build.opt)"|Out-Null
 return [pscustomobject]@{build=$build;path=$iso;sha256=Hash $iso;bytes=(Get-Item $iso).Length}
}
function Run-Qemu($iso,[int]$ram,[bool]$bridge){
 $opt=$iso.build.opt;$suffix=if($bridge){'-bridge'}else{''};$dir=Join-Path $root "run-o$opt-${ram}m$suffix";New-Item -ItemType Directory -Path $dir -Force|Out-Null
 $serial=Join-Path $dir 'serial.log';$trace=Join-Path $dir 'trace.log';$entryResets=$null;$text=''
 $a=@('-cdrom',$iso.path,'-m',"${ram}M",'-display','none','-monitor','none','-serial',"file:$serial",'-d','int,cpu_reset,guest_errors','-D',$trace,'-no-reboot','-no-shutdown')
 if($bridge){$a+=@('-device','pci-bridge,id=br0,chassis_nr=1','-device','e1000,bus=br0,addr=1')}
 [IO.File]::WriteAllText((Join-Path $dir 'qemu-command.txt'),"$qemu $($a -join ' ')")
 $p=Start-Process -FilePath $qemu -ArgumentList $a -PassThru -WindowStyle Hidden;$owned.Add($p.Id)
 $terminal=$false
 try{$deadline=[DateTime]::UtcNow.AddSeconds(35);do{Start-Sleep -Milliseconds 100;if(Test-Path $serial){$text=Shared $serial;if($text.Length-gt0-and$null-eq$entryResets-and(Test-Path $trace)){$entryResets=([regex]::Matches((Shared $trace),'CPU Reset')).Count};$terminal=$text.Contains('G3 COMPLETE')-or$text.Contains('G1 ERROR')-or$text.Contains('G2 ERROR')-or$text.Contains('G2B ERROR')-or$text.Contains('G2C ERROR')-or$text.Contains('G3 ERROR')-or$text.Contains('G2B FATAL')-or$text.Contains('G2B PAGEFAULT');if($terminal){break}}}while(-not$p.HasExited-and[DateTime]::UtcNow-lt$deadline);Assert-True($terminal)"O$opt ${ram}M bridge=$bridge QEMU timed out before terminal G3 receipt"}
 finally{if(-not$p.HasExited){Stop-Process -Id $p.Id -Force;$p.WaitForExit()};$p.Dispose()}
 $traceText=[IO.File]::ReadAllText($trace);$triple=([regex]::Matches($traceText,'Triple fault')).Count;$faults=([regex]::Matches($traceText,'v=0[0de6]')).Count;$finalResets=([regex]::Matches($traceText,'CPU Reset')).Count
 Assert-True($triple-eq0)"O$opt ${ram}M bridge=$bridge triple fault";Assert-True($faults-eq0)"O$opt ${ram}M bridge=$bridge #UD/#GP/#PF observed";Assert-True($null-ne$entryResets)"O$opt ${ram}M bridge=$bridge no entry reset sample";Assert-True($finalResets-eq$entryResets)"O$opt ${ram}M bridge=$bridge CPU reset after kernel entry"
 $receipt=Validate-Receipt $text $ram $opt $bridge;$receipt|Add-Member iso_sha256 $iso.sha256;$receipt|Add-Member serial_sha256 (Hash $serial);$receipt|Add-Member trace_sha256 (Hash $trace);$receipt|Add-Member entry_resets $entryResets;$receipt|Add-Member final_resets $finalResets;$receipt|Add-Member faults $faults;$receipt|Add-Member triple_faults $triple
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
 Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_generation2_interrupts.ps1')) 'fresh G2B predecessor gate'|Out-Null
 Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_generation2_memory.ps1')) 'fresh G2C predecessor gate'|Out-Null
 Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_value_formation.ps1')) 'value formation gate'|Out-Null
 Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_range_regression.ps1'),'-OptLevel','0') 'range O0 gate'|Out-Null
 Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_range_regression.ps1'),'-OptLevel','1') 'range O1 gate'|Out-Null
 Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_boot_info_regression.ps1')) 'BootInfo gate'|Out-Null
 Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_acpi_regression.ps1')) 'ACPI gate'|Out-Null
 Run 'cargo.exe' @('build','-q','-p','dbyte_cli','--bin','dbytec') 'fresh dbytec'|Out-Null
 foreach($opt in @('0','1')){
  Run $compiler @('build','examples/native_v13_machine/cpu_feature_regression.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','obj','-o',(Join-Path $root "cpu-feature-o$opt.o")) "CPU feature regression O$opt"|Out-Null
  Run $compiler @('build','examples/native_v13_machine/cpu_control_regression.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','obj','-o',(Join-Path $root "cpu-control-o$opt.o")) "CPU control regression O$opt"|Out-Null
  Run $compiler @('build','examples/native_v13_machine/pci_topology_regression.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','obj','-o',(Join-Path $root "pci-topology-o$opt.o")) "PCI topology regression O$opt"|Out-Null
 }
 Run-ExpectFailure $compiler @('build','examples/native_v13_machine/cpu_feature_regression.dby','--target','i686-dbyte-none','--opt-level','0','--emit','obj','-o',(Join-Path $root 'cpu-feature-i686-negative.o')) 'E_PROFILE_CAPABILITY' 'i686 DBYTE64 CPU-feature capability gate'|Out-Null
 Run 'cargo.exe' @('test','-q','-p','dbyte_native') 'full dbyte_native tests'|Out-Null
 Run 'cargo.exe' @('fmt','--all','--','--check') 'cargo fmt check'|Out-Null
 Run 'cargo.exe' @('clippy','-q','-p','dbyte_hir','-p','dbyte_native','--','-D','warnings') 'native clippy'|Out-Null
 $epoch=[int64](Run 'git.exe' @('show','-s','--format=%ct','HEAD') 'read packaging epoch').Trim();$isoDate=[DateTimeOffset]::FromUnixTimeSeconds($epoch).UtcDateTime.ToString('yyyyMMddHHmmss00',[Globalization.CultureInfo]::InvariantCulture)
 $builds=@()
 foreach($opt in @('0','1')){$a=Build-Set 'A' $opt;$b=Build-Set 'B' $opt;Assert-True($a.obj_sha256-ceq$b.obj_sha256)"O$opt object is not deterministic";Assert-True($a.elf_sha256-ceq$b.elf_sha256)"O$opt ELF is not deterministic";$builds+=@($a,$b)}
 $isos=@()
 foreach($opt in @('0','1')){$a=$builds|Where-Object{$_.set-eq'A'-and$_.opt-eq$opt};$b=$builds|Where-Object{$_.set-eq'B'-and$_.opt-eq$opt};$ia=Make-Iso $a $isoDate;$ib=Make-Iso $b $isoDate;Assert-True($ia.sha256-ceq$ib.sha256)"O$opt ISO is not deterministic";$isos+=@($ia,$ib)}
 $receipts=@()
 foreach($opt in @('0','1')){$iso=$isos|Where-Object{$_.build.set-eq'A'-and$_.build.opt-eq$opt};foreach($ram in @(32,64)){$receipts+=Run-Qemu $iso $ram $false};foreach($ram in @(32,64)){$receipts+=Run-Qemu $iso $ram $true}}
 $receipts|ConvertTo-Json -Depth 5|Set-Content (Join-Path $root 'qemu-receipts.json')
 $boundary=Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_v13_historical_boundary.ps1')) 'historical frozen boundary';[IO.File]::WriteAllText((Join-Path $root 'historical-boundary.txt'),$boundary)
 $suffix='v13g3-'+[Guid]::NewGuid().ToString('N').Substring(0,8)
 $none=Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_modern_native_none.ps1'),'-MilestoneVersion',$suffix) 'historical modern-none behavior';[IO.File]::WriteAllText((Join-Path $root 'historical-modern-none.txt'),$none)
 $wide=Run 'powershell.exe' @('-NoProfile','-ExecutionPolicy','Bypass','-File',(Join-Path $repo 'scripts/verify_i686_wide_integer.ps1'),'-MilestoneVersion',$suffix) 'historical i686-wide behavior';[IO.File]::WriteAllText((Join-Path $root 'historical-i686-wide.txt'),$wide)
 $selectedBuild=$builds|Where-Object{$_.set-eq'A'-and$_.opt-eq'1'};$selectedIso=$isos|Where-Object{$_.build.set-eq'A'-and$_.build.opt-eq'1'}
 $identity=Write-Identity $selectedIso $selectedBuild
 $physical=Join-Path $root 'physical-candidate';New-Item -ItemType Directory -Path $physical -Force|Out-Null
 Copy-Item -LiteralPath $selectedBuild.elf -Destination (Join-Path $physical 'dbyte-v13-g3.elf') -Force;Copy-Item -LiteralPath $selectedIso.path -Destination (Join-Path $physical 'dbyte-v13-g3.iso') -Force
 Assert-True((Hash (Join-Path $physical 'dbyte-v13-g3.elf'))-ceq$selectedBuild.elf_sha256)'physical-candidate ELF hash drift';Assert-True((Hash (Join-Path $physical 'dbyte-v13-g3.iso'))-ceq$selectedIso.sha256)'physical-candidate ISO hash drift'
 $result=[pscustomobject]@{status='PASS';generation='v13-g3-apic-ioapic-timer-pci-topology-qemu';packaging_date=$isoDate;candidate_identity=$identity;selected_object_sha256=$selectedBuild.obj_sha256;selected_elf_sha256=$selectedBuild.elf_sha256;selected_iso_sha256=$selectedIso.sha256;selected_iso_bytes=$selectedIso.bytes;qemu_receipts=$receipts.Count;physical_candidate='CREATED_NOT_BOOTED';real_biostar='PENDING';real_storage='DISARMED';g2a='PASS';g2b='PASS';g2c='PASS';full_g2='PASS';g3_virtual='PASS';g3_physical='PENDING';v13_final='NOT_CREATED'}
 $result|ConvertTo-Json -Depth 6|Set-Content (Join-Path $root 'RESULT.json')
 Write-Output "V13 GENERATION 3 APIC/IOAPIC/TIMER/PCI VERIFIER: PASS"
 Write-Output "EVIDENCE ROOT: $root"
 Write-Output "SELECTED ISO SHA256: $($result.selected_iso_sha256)"
}finally{
 Pop-Location
 $deadline=[DateTime]::UtcNow.AddSeconds(3);do{$orphans=@($owned|Where-Object{$candidate=Get-CimInstance Win32_Process -Filter ("ProcessId = {0}" -f $_) -ErrorAction SilentlyContinue; $null -ne $candidate -and $candidate.Name -ieq [IO.Path]::GetFileName($qemu) -and $candidate.CommandLine -like ("*" + $root + "*")});if($orphans.Count-eq0-or[DateTime]::UtcNow-ge$deadline){break};Start-Sleep -Milliseconds 100}while($true)
 if($orphans.Count-ne0){throw "G3 verifier left owned QEMU processes: $($orphans -join ', ')"}
}
