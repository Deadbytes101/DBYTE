param(
    [Parameter(Mandatory = $true)][string]$ZipPath,
    [Parameter(Mandatory = $true)][string]$ReceiptPath,
    [Parameter(Mandatory = $true)][string]$SourceRoot,
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$ExpectedCommit = "",
    [string]$ProvenanceRepositoryRoot = ""
)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "windows_package_common.ps1")

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if ([string]::IsNullOrWhiteSpace($ProvenanceRepositoryRoot)) { $ProvenanceRepositoryRoot = $repoRoot }
$manifest = Get-CanonicalPackageManifest $SourceRoot $Version $ProvenanceRepositoryRoot $ExpectedCommit
if (-not (Test-Path -LiteralPath $ReceiptPath -PathType Leaf)) { throw "PACKAGE_RECEIPT_REQUIRED" }
$receipt = Get-Content -LiteralPath $ReceiptPath -Raw | ConvertFrom-Json
if ($receipt.schema -cne "dbyte-windows-package-receipt-v1") { throw "PACKAGE_RECEIPT_SCHEMA_INVALID" }
if ($receipt.version -cne $Version) { throw "PACKAGE_RECEIPT_VERSION_MISMATCH" }
if ($ExpectedCommit -and $receipt.source_commit -cne $manifest.Commit) { throw "PACKAGE_RECEIPT_COMMIT_MISMATCH" }

$receiptEntries = @($receipt.files | ForEach-Object {
    [pscustomobject]@{ Path = [string]$_.path; Size = [int64]$_.size; Sha256 = [string]$_.sha256 }
})
$receiptMap = [System.Collections.Generic.Dictionary[string, object]]::new([StringComparer]::Ordinal)
foreach ($item in $receiptEntries) {
    if (-not $receiptMap.TryAdd($item.Path, $item)) { throw "PACKAGE_RECEIPT_DUPLICATE: $($item.Path)" }
}
foreach ($sourceEntry in $manifest.Entries) {
    if (-not $receiptMap.ContainsKey($sourceEntry.Path)) { throw "PACKAGE_RECEIPT_SOURCE_MISSING: $($sourceEntry.Path)" }
    if ($receiptMap[$sourceEntry.Path].Sha256 -cne $sourceEntry.Sha256 -or $receiptMap[$sourceEntry.Path].Size -ne $sourceEntry.Size) {
        throw "PACKAGE_RECEIPT_SOURCE_DRIFT: $($sourceEntry.Path)"
    }
}
foreach ($binary in @("dbyte.exe", "dbytec.exe")) {
    if (-not $receiptMap.ContainsKey($binary)) { throw "PACKAGE_RECEIPT_BINARY_MISSING: $binary" }
}
if ($receiptEntries.Count -ne ($manifest.Entries.Count + 2)) { throw "PACKAGE_RECEIPT_ENTRY_COUNT_INVALID" }

$evidence = Test-CanonicalPackageZip $ZipPath $receiptEntries $Version -RequireFixedMetadata
if ($receipt.zip_sha256 -cne $evidence.Sha256 -or [int64]$receipt.zip_size -ne $evidence.Size) {
    throw "PACKAGE_RECEIPT_ZIP_MISMATCH"
}
if ([int]$receipt.entry_count -ne $evidence.EntryCount -or [int]$receipt.runtime_count -ne $evidence.RuntimeCount) {
    throw "PACKAGE_RECEIPT_COUNT_MISMATCH"
}

$extractRoot = Join-Path ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($ReceiptPath))) ("verify-bin-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $extractRoot | Out-Null
try {
    $zip = [IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $ZipPath).Path)
    try {
        foreach ($binary in @("dbyte.exe", "dbytec.exe")) {
            $entry = $zip.GetEntry($binary)
            if ($null -eq $entry) { throw "PACKAGE_BINARY_MISSING: $binary" }
            $destination = Join-Path $extractRoot $binary
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $destination, $false)
            if ($binary -ceq "dbyte.exe") {
                $versionOutput = @(& $destination --version 2>&1)
                if ($LASTEXITCODE -ne 0 -or ($versionOutput -join "`n") -notmatch [regex]::Escape($Version)) {
                    throw "PACKAGE_BINARY_VERSION_MISMATCH: $binary"
                }
            }
            else {
                $probe = @(& $destination 2>&1)
                if ($LASTEXITCODE -ne 2 -or -not ($probe -join "`n").Contains("dbytec build")) {
                    throw "PACKAGE_BINARY_CAPABILITY_MISMATCH: $binary"
                }
            }
        }
    }
    finally { $zip.Dispose() }
}
finally {
    if (Test-Path -LiteralPath $extractRoot) { Remove-Item -LiteralPath $extractRoot -Recurse -Force }
}

"DBYTE WINDOWS PACKAGE VERIFIER: PASS"
$evidence
$global:LASTEXITCODE = 0
