param(
    [string]$EvidenceRoot = "",
    [switch]$ReplaceRootArtifacts
)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "windows_package_common.ps1")

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$canonicalTmp = [IO.Path]::GetFullPath((Join-Path $repoRoot "tmp"))
if ([string]::IsNullOrWhiteSpace($EvidenceRoot)) {
    $EvidenceRoot = Join-Path $canonicalTmp "windows_package_correction_v12.2.1"
}
$evidence = [IO.Path]::GetFullPath($EvidenceRoot)
if (-not ($evidence.StartsWith($canonicalTmp + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase))) {
    throw "PACKAGE_CORRECTION_EVIDENCE_ROOT_INVALID: $evidence"
}

function Export-Commit([string]$Commit, [string]$Destination, [string]$ArchivePath) {
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    & git.exe -C $repoRoot -c core.autocrlf=false archive --format=zip -o $ArchivePath $Commit
    if ($LASTEXITCODE -ne 0) { throw "PACKAGE_TAG_EXPORT_FAILED: $Commit" }
    Expand-Archive -LiteralPath $ArchivePath -DestinationPath $Destination -Force
    Assert-SourceTreeMatchesCommit $Destination $repoRoot $Commit
}

function Compare-OldAndCorrected([string]$OldZip, [string]$NewZip, [object]$Manifest, [string]$ReceiptPath) {
    $oldEntries = if (Test-Path -LiteralPath $OldZip) { Get-PackageZipEntries $OldZip } else { @() }
    $newEntries = Get-PackageZipEntries $NewZip
    $oldArchiveEntryCount = 0
    if (Test-Path -LiteralPath $OldZip) {
        $oldArchive = [IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $OldZip).Path)
        try { $oldArchiveEntryCount = $oldArchive.Entries.Count } finally { $oldArchive.Dispose() }
    }
    $newArchive = [IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $NewZip).Path)
    try { $newArchiveEntryCount = $newArchive.Entries.Count } finally { $newArchive.Dispose() }
    $oldMap = [System.Collections.Generic.Dictionary[string, object]]::new([StringComparer]::Ordinal)
    foreach ($entry in $oldEntries) { $oldMap.Add($entry.Path, $entry) }
    $newMap = [System.Collections.Generic.Dictionary[string, object]]::new([StringComparer]::Ordinal)
    foreach ($entry in $newEntries) { $newMap.Add($entry.Path, $entry) }
    $added = @($newMap.Keys | Where-Object { -not $oldMap.ContainsKey($_) } | Sort-Object -CaseSensitive)
    $removed = @($oldMap.Keys | Where-Object { -not $newMap.ContainsKey($_) } | Sort-Object -CaseSensitive)
    $unchanged = @($newMap.Keys | Where-Object { $oldMap.ContainsKey($_) -and $oldMap[$_].Sha256 -ceq $newMap[$_].Sha256 } | Sort-Object -CaseSensitive)
    $changed = @($newMap.Keys | Where-Object { $oldMap.ContainsKey($_) -and $oldMap[$_].Sha256 -cne $newMap[$_].Sha256 } | Sort-Object -CaseSensitive)
    $canonicalPaths = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($entry in $Manifest.Entries) { [void]$canonicalPaths.Add($entry.Path) }
    [void]$canonicalPaths.Add("dbyte.exe"); [void]$canonicalPaths.Add("dbytec.exe")
    $unexpected = @($newMap.Keys | Where-Object { -not $canonicalPaths.Contains($_) })
    if ($unexpected.Count -ne 0) { throw "PACKAGE_CORRECTION_UNEXPECTED_DIVERGENCE: $($unexpected -join ', ')" }
    $record = [ordered]@{
        schema = "dbyte-windows-package-correction-v1"
        old_zip = if (Test-Path -LiteralPath $OldZip) { [ordered]@{ sha256 = (Get-FileHash $OldZip -Algorithm SHA256).Hash.ToLowerInvariant(); size = (Get-Item $OldZip).Length; archive_entries = $oldArchiveEntryCount; file_entries = $oldEntries.Count } } else { $null }
        corrected_zip = [ordered]@{ sha256 = (Get-FileHash $NewZip -Algorithm SHA256).Hash.ToLowerInvariant(); size = (Get-Item $NewZip).Length; archive_entries = $newArchiveEntryCount; file_entries = $newEntries.Count }
        added_count = $added.Count
        added = $added
        removed_contamination_count = $removed.Count
        removed_contamination = $removed
        unchanged_valid_count = $unchanged.Count
        changed_valid_count = $changed.Count
        changed_valid = $changed
        unexpected_divergence_count = $unexpected.Count
        runtime_count = @($newMap.Keys | Where-Object { $_.StartsWith("runtime/", [StringComparison]::Ordinal) }).Count
    }
    [IO.File]::WriteAllText($ReceiptPath, ($record | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
    return [pscustomobject]$record
}

New-Item -ItemType Directory -Path $evidence -Force | Out-Null
$originalRoot = Join-Path $evidence "original"
New-Item -ItemType Directory -Path $originalRoot -Force | Out-Null

$results = [System.Collections.Generic.List[object]]::new()
foreach ($version in @("12.1.0", "12.2.0")) {
    $tag = "v$version"
    $tagType = (@(Invoke-PackageGit $repoRoot @("cat-file", "-t", $tag) "PACKAGE_TAG_REQUIRED"))[0].Trim()
    if ($tagType -cne "tag") { throw "PACKAGE_ANNOTATED_TAG_REQUIRED: $tag" }
    $commit = (@(Invoke-PackageGit $repoRoot @("rev-parse", "$tag^{}") "PACKAGE_TAG_REQUIRED"))[0].Trim()
    $versionRoot = Join-Path $evidence $version
    $outputA = Join-Path $versionRoot "build-a"
    $outputB = Join-Path $versionRoot "build-b"
    $sourceA = Join-Path $versionRoot "export-a"
    $sourceB = Join-Path $versionRoot "export-b"
    New-Item -ItemType Directory -Path $versionRoot -Force | Out-Null
    Export-Commit $commit $sourceA (Join-Path $versionRoot "export-a.zip")
    Export-Commit $commit $sourceB (Join-Path $versionRoot "export-b.zip")

    & (Join-Path $PSScriptRoot "package_release.ps1") -Version $version -SourceRoot $sourceA -OutputRoot $outputA -ExpectedCommit $commit -ProvenanceRepositoryRoot $repoRoot -SkipBundle
    if ($LASTEXITCODE -ne 0) { throw "PACKAGE_CORRECTION_BUILD_FAILED: $version build A" }
    & (Join-Path $PSScriptRoot "package_release.ps1") -Version $version -SourceRoot $sourceB -OutputRoot $outputB -ExpectedCommit $commit -ProvenanceRepositoryRoot $repoRoot -SkipBundle
    if ($LASTEXITCODE -ne 0) { throw "PACKAGE_CORRECTION_BUILD_FAILED: $version build B" }
    $name = "dbyte-v$version-windows-x64"
    $zipA = Join-Path $outputA "$name.zip"
    $zipB = Join-Path $outputB "$name.zip"
    $hashA = (Get-FileHash $zipA -Algorithm SHA256).Hash
    $hashB = (Get-FileHash $zipB -Algorithm SHA256).Hash
    if ($hashA -cne $hashB) { throw "PACKAGE_NONDETERMINISTIC: $version $hashA $hashB" }
    & (Join-Path $PSScriptRoot "verify_windows_package.ps1") -ZipPath $zipA -ReceiptPath (Join-Path $outputA "$name.manifest.json") -SourceRoot $sourceA -Version $version -ExpectedCommit $commit -ProvenanceRepositoryRoot $repoRoot | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "PACKAGE_CORRECTION_VERIFY_FAILED: $version" }

    $oldZip = Join-Path $repoRoot "$name.zip"
    $originalZip = Join-Path $originalRoot "$name.zip"
    if ((Test-Path -LiteralPath $oldZip) -and -not (Test-Path -LiteralPath $originalZip)) {
        Copy-Item -LiteralPath $oldZip -Destination $originalZip
    }
    $manifest = Get-CanonicalPackageManifest $sourceA $version $repoRoot $commit
    $comparisonSource = if (Test-Path -LiteralPath $originalZip) { $originalZip } else { $oldZip }
    $comparison = Compare-OldAndCorrected $comparisonSource $zipA $manifest (Join-Path $versionRoot "correction.json")
    if ($comparison.unexpected_divergence_count -ne 0) { throw "PACKAGE_CORRECTION_UNEXPECTED_DIVERGENCE: $version" }
    if ($version -ceq "12.1.0") {
        if ($comparison.added_count -ne 11 -or $comparison.removed_contamination_count -ne 26 -or $comparison.runtime_count -ne 11) {
            throw "PACKAGE_CORRECTION_V121_COUNTS_MISMATCH"
        }
    }
    else {
        if ($comparison.added_count -ne 13 -or $comparison.removed_contamination_count -ne 0 -or $comparison.runtime_count -ne 13) {
            throw "PACKAGE_CORRECTION_V122_COUNTS_MISMATCH"
        }
    }
    if ($ReplaceRootArtifacts) { Copy-Item -LiteralPath $zipA -Destination $oldZip -Force }
    $results.Add([pscustomobject]@{ Version = $version; Commit = $commit; Hash = $hashA.ToLowerInvariant(); Comparison = $comparison })
}

"DBYTE WINDOWS ARCHIVAL PACKAGE CORRECTION: PASS"
$results
