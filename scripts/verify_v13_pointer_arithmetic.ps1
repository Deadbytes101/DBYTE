$ErrorActionPreference='Stop'
$repo=(Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$root=Join-Path $repo ('tmp/v13_pointer/'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root -Force|Out-Null
function Run([string]$exe,[string[]]$a){
 $saved=$ErrorActionPreference;$ErrorActionPreference='Continue'
 try{$output=& $exe @a 2>&1;$code=$LASTEXITCODE}finally{$ErrorActionPreference=$saved}
 Add-Content (Join-Path $root 'commands.txt') "$exe $($a -join ' ')`nEXIT=$code`n$($output -join "`n")"
 if($code-ne0){throw "command failed ($code): $root"};return($output -join "`n")
}
function LinuxPath([string]$p){(Run wsl.exe @('-d','Debian','--','wslpath','-a','-u',$p.Replace('\','/'))).Trim()}
Push-Location $repo
try{
 Run cargo @('build','-q','-p','dbyte_cli','--bin','dbytec')|Out-Null
 foreach($target in @('i686-dbyte-none','x86_64-dbyte-none')){
  foreach($opt in @('0','1')){
   $dir=Join-Path $root "$target-o$opt";$grub=Join-Path $dir 'iso/boot/grub'
   New-Item -ItemType Directory -Path $grub -Force|Out-Null
   $elf=Join-Path $dir 'iso/boot/kernel.elf'
   Run ./target/debug/dbytec.exe @('build','examples/native_v13_machine/pointer_arithmetic.dby','--target',$target,'--opt-level',$opt,'--emit','elf','-o',$elf,'--dump-native-mir')|Set-Content (Join-Path $dir 'mir.txt')
   $transport=if($target-eq'i686-dbyte-none'){'multiboot'}else{'multiboot2'}
   Run wsl.exe @('-d','Debian','--','grub-file',"--is-x86-$transport",(LinuxPath $elf))|Out-Null
   [IO.File]::WriteAllText((Join-Path $grub 'grub.cfg'),"set timeout=0`nmenuentry 'Pointer proof' {`n $transport /boot/kernel.elf`n boot`n}`n")
   $iso=Join-Path $dir 'proof.iso'
   Run wsl.exe @('-d','Debian','--','grub-mkrescue','-o',(LinuxPath $iso),(LinuxPath (Join-Path $dir 'iso')))|Out-Null
   $serial=Join-Path $dir 'serial.log';$trace=Join-Path $dir 'trace.log'
   $qemu='C:/Program Files/qemu/qemu-system-x86_64.exe'
   $a=@('-cdrom',$iso,'-m','64M','-display','none','-monitor','none','-serial',"file:$serial",'-d','int,cpu_reset,guest_errors','-D',$trace,'-no-reboot','-no-shutdown')
   Add-Content (Join-Path $root 'commands.txt') "$qemu $($a -join ' ')"
   $owned=Start-Process -FilePath $qemu -ArgumentList $a -WindowStyle Hidden -PassThru
   try{Start-Sleep -Seconds 4}finally{if(-not $owned.HasExited){Stop-Process -Id $owned.Id;$owned.WaitForExit()};$owned.Dispose()}
   if([IO.File]::ReadAllText($serial)-cne"POINTER ARITHMETIC OK`r`n"){throw "serial mismatch: $dir"}
   if([IO.File]::ReadAllText($trace)-match'Triple fault|v=0[06de]'){throw "CPU fault: $dir"}
   Get-FileHash -LiteralPath $elf,$iso|Select-Object Path,Hash|ConvertTo-Json|Set-Content (Join-Path $dir 'hashes.json')
  }
 }
 "POINTER NATIVE O0/O1 PASS: $root"
}finally{Pop-Location}
