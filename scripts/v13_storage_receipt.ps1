function Get-V13SourceBinding([string]$Repository){
 $paths=@(& git.exe -C $Repository ls-files --cached --others --exclude-standard)
 if($LASTEXITCODE-ne0){throw 'source enumeration failed'}
 $entries=@($paths|Where-Object{$_-match'^(crates/|runtime/|examples/native_v13_machine/|scripts/|Cargo\.|rust-toolchain|\.cargo/)'}|Sort-Object -Unique|ForEach-Object{
  $path=Join-Path $Repository $_
  if(-not(Test-Path -LiteralPath $path -PathType Leaf)){throw "missing source: $_"}
  [ordered]@{path=$_;sha256=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash}
 })
 return ConvertTo-Json -InputObject $entries -Compress -Depth 4
}
function New-V13StorageBinding([string]$Repository,[string]$Root,[string]$Before){
 $after=Get-V13SourceBinding $Repository
 if($after-cne$Before){throw 'source changed during storage master'}
 $artifacts=@(Get-ChildItem -LiteralPath $Root -Recurse -File|Where-Object{$_.Name-ne'RESULT.json'}|Sort-Object FullName|ForEach-Object{
  [ordered]@{path=$_.FullName.Substring($Root.Length+1).Replace('\','/');sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash}
 })
 return [ordered]@{schema='v13-storage-binding-v1';sources=$after;compiler_sha256=(Get-FileHash -LiteralPath (Join-Path $Repository 'target/debug/dbytec.exe')).Hash;artifacts=$artifacts}
}
function Assert-V13StorageReceipt([string]$Repository,[string]$Receipt){
 $r=Get-Content -LiteralPath $Receipt -Raw|ConvertFrom-Json
 if($r.status-cne'PASS'-or$r.generation-cne'v13-g4-ide-pio-readonly-qemu'-or$r.g3_predecessor-cne'PASS'-or$r.qemu_receipts-ne4){throw 'storage master not terminal PASS'}
 $b=$r.binding
 if($null-eq$b-or$b.schema-cne'v13-storage-binding-v1'-or@($b.artifacts).Count-lt4){throw 'storage receipt missing binding'}
 $head=(& git.exe -C $Repository rev-parse HEAD).Trim()
 if($LASTEXITCODE-ne0-or$r.head-cne$head){throw 'storage HEAD mismatch'}
 if($b.sources-cne(Get-V13SourceBinding $Repository)){throw 'storage source binding mismatch'}
 if($b.compiler_sha256-cne(Get-FileHash -LiteralPath (Join-Path $Repository 'target/debug/dbytec.exe')).Hash){throw 'storage compiler mismatch'}
 $root=Split-Path (Resolve-Path -LiteralPath $Receipt).Path
 $seen=@{}
 foreach($entry in $b.artifacts){
  $relative=[string]$entry.path
  if([IO.Path]::IsPathRooted($relative)-or$relative-match'(^|[/\\])\.\.([/\\]|$)'-or$seen.ContainsKey($relative)){throw 'invalid artifact path'}
  $seen[$relative]=$true
  $path=[IO.Path]::GetFullPath((Join-Path $root $relative))
  if(-not$path.StartsWith($root+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'artifact escapes receipt root'}
  if($entry.sha256-cne(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash){throw "storage artifact mismatch: $relative"}
 }
 if(-not$seen.ContainsKey('qemu-receipts.json')){throw 'QEMU receipts not bound'}
 return $r
}
