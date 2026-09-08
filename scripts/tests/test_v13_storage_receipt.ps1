$ErrorActionPreference='Stop'
$helper=Join-Path $PSScriptRoot '../v13_storage_receipt.ps1'
if(-not(Test-Path $helper)){throw 'RED: storage receipt validator missing'}
. $helper
$repo=(Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$root=Join-Path $repo ('tmp/receipt-tests-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root|Out-Null
foreach($case in @('{}','{"status":"PASS"}','{"status":"FAIL","receipt_schema":"v13-storage-binding-v1"}')){
 $path=Join-Path $root 'invalid.json';[IO.File]::WriteAllText($path,$case)
 $rejected=$false
 try{Assert-V13StorageReceipt $repo $path|Out-Null}catch{$rejected=$true}
 if(-not$rejected){throw 'incomplete receipt accepted'}
}
'STORAGE RECEIPT NEGATIVE TESTS PASS'
