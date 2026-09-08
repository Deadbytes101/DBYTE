$ErrorActionPreference='Stop'
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$root=Join-Path $repo ('tmp/v13_g1/bootinfo-'+[Guid]::NewGuid().ToString('N'))
$compiler=Join-Path $repo 'target/debug/dbytec.exe'
$qemu='C:/Program Files/qemu/qemu-system-x86_64.exe'
$expected="BOOTINFO NORMALIZE OK`r`n"
$owned=[System.Collections.Generic.List[int]]::new()
New-Item -ItemType Directory -Path $root -Force | Out-Null
function Assert-True($v,[string]$m){if(-not $v){throw $m}}
function Run([string]$exe,[string[]]$a,[string]$d){
 $save=$ErrorActionPreference;$ErrorActionPreference='Continue'
 try{$o=& $exe @a 2>&1;$c=$LASTEXITCODE}finally{$ErrorActionPreference=$save}
 Add-Content (Join-Path $root 'commands.txt') "$exe $($a -join ' ')`nEXIT=$c`n$($o -join "`n")"
 if($c -ne 0){throw "$d failed ($c); evidence: $root"};return($o -join "`n")
}
function LinuxPath([string]$p){(Run wsl.exe @('-d','Debian','--','wslpath','-a','-u',$p.Replace('\','/')) 'wslpath').Trim()}
function Shared([string]$p){$s=[IO.File]::Open($p,'Open','Read','ReadWrite');try{$r=[IO.StreamReader]::new($s);try{$r.ReadToEnd()}finally{$r.Dispose()}}finally{$s.Dispose()}}
function RunProof([string]$opt){
 $dir=Join-Path $root "o$opt";$grub=Join-Path $dir 'iso/boot/grub';New-Item -ItemType Directory -Path $grub -Force|Out-Null
 $elf=Join-Path $dir 'iso/boot/kernel.elf'
 Run $compiler @('build','examples/native_v13_machine/boot_info_regression.dby','--target','x86_64-dbyte-none','--opt-level',$opt,'--emit','elf','-o',$elf) "build BootInfo O$opt"|Out-Null
 [IO.File]::WriteAllText((Join-Path $grub 'grub.cfg'),"set timeout=0`nset default=0`nmenuentry 'DByte BootInfo regression' {`n multiboot2 /boot/kernel.elf`n boot`n}`n")
 Run wsl.exe @('-d','Debian','--','grub-file','--is-x86-multiboot2',(LinuxPath $elf)) "grub-file O$opt"|Out-Null
 $iso=Join-Path $dir 'bootinfo.iso';Run wsl.exe @('-d','Debian','--','grub-mkrescue','-o',(LinuxPath $iso),(LinuxPath (Join-Path $dir 'iso'))) "ISO O$opt"|Out-Null
 $serial=Join-Path $dir 'serial.log';$trace=Join-Path $dir 'trace.log'
 $a=@('-cdrom',$iso,'-m','32M','-display','none','-monitor','none','-serial',"file:$serial",'-d','int,cpu_reset,guest_errors','-D',$trace,'-no-reboot','-no-shutdown')
 $p=Start-Process -FilePath $qemu -ArgumentList $a -PassThru -WindowStyle Hidden;$owned.Add($p.Id)
 try{$deadline=[DateTime]::UtcNow.AddSeconds(12);$done=$false;while(-not $p.HasExited-and[DateTime]::UtcNow-lt$deadline){Start-Sleep -Milliseconds 100;if(Test-Path $serial){$x=Shared $serial;if($x -ceq $expected){$done=$true;break};Assert-True($expected.StartsWith($x,[StringComparison]::Ordinal))"O$opt serial diverged: '$x'"}};Assert-True $done "O$opt timed out"}
 finally{if(-not$p.HasExited){Stop-Process -Id $p.Id -Force;$p.WaitForExit()};$p.Dispose()}
 $f=[IO.File]::ReadAllText($trace);Assert-True(-not$f.Contains('Triple fault'))"O$opt triple fault";Assert-True(([regex]::Matches($f,'v=0[0de6]')).Count-eq 0)"O$opt CPU fault"
 [pscustomobject]@{opt=$opt;elf_sha256=(Get-FileHash $elf -Algorithm SHA256).Hash;iso_sha256=(Get-FileHash $iso -Algorithm SHA256).Hash}
}
Push-Location $repo
try{
 Run cargo.exe @('build','-q','-p','dbyte_cli','--bin','dbytec') 'build dbytec'|Out-Null
 $proofs=@(RunProof '0';RunProof '1')
 $proofs|ConvertTo-Json|Set-Content (Join-Path $root 'hashes.json')
 "V13 BOOTINFO REGRESSION PASS: $root"
}finally{
 Pop-Location
 $deadline=[DateTime]::UtcNow.AddSeconds(2)
 do{$orphans=@($owned|Where-Object{Get-Process -Id $_ -ErrorAction SilentlyContinue});if($orphans.Count-eq 0-or[DateTime]::UtcNow-ge$deadline){break};Start-Sleep -Milliseconds 100}while($true)
 if($orphans.Count-ne 0){throw "BootInfo verifier left QEMU: $($orphans -join ', ')"}
}
