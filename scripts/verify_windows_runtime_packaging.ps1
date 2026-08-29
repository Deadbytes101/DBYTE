param(
    [string]$Version = "12.2.1",
    [string]$ExpectedCommit = "",
    [switch]$DoubleBuild
)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "windows_package_common.ps1")

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$proofRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot "tmp\windows_package_verifier_v$Version"))
$canonicalTmp = [IO.Path]::GetFullPath((Join-Path $repoRoot "tmp"))
if (-not $proofRoot.StartsWith($canonicalTmp + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw "PACKAGE_TEST_ROOT_INVALID"
}
if (Test-Path -LiteralPath $proofRoot) { Remove-Item -LiteralPath $proofRoot -Recurse -Force }
New-Item -ItemType Directory -Path $proofRoot -Force | Out-Null

function Assert-Fails([scriptblock]$Action, [string]$Code) {
    try { & $Action; throw "PACKAGE_NEGATIVE_TEST_DID_NOT_FAIL: $Code" }
    catch {
        if ($_.Exception.Message -eq "PACKAGE_NEGATIVE_TEST_DID_NOT_FAIL: $Code") { throw }
        if (-not $_.Exception.Message.Contains($Code)) {
            throw "PACKAGE_NEGATIVE_TEST_WRONG_DIAGNOSTIC: expected $Code, got $($_.Exception.Message)"
        }
    }
}

Assert-Fails { Assert-RuntimeTree @("README.md") $Version } "PACKAGE_RUNTIME_TREE_REQUIRED"
Assert-Fails { Assert-RuntimeTree @() $Version } "PACKAGE_RUNTIME_TREE_REQUIRED"
Assert-Fails { Assert-RuntimeTree @("runtime/runtime/v12.1.0/a.dby") $Version } "PACKAGE_RUNTIME_NESTED"
Assert-Fails { Assert-RuntimeTree @("runtime/v99.0.0/a.dby") $Version } "PACKAGE_RUNTIME_FUTURE_VERSION"
Assert-Fails { Assert-PackageRelativePath "/absolute/file" } "PACKAGE_PATH_INVALID"
Assert-Fails { Assert-PackageRelativePath "../temporary/file" } "PACKAGE_PATH_INVALID"

$dataFile = Join-Path $proofRoot "byte.bin"
[IO.File]::WriteAllBytes($dataFile, [byte[]](0x44))
$dataHash = (Get-FileHash $dataFile -Algorithm SHA256).Hash.ToLowerInvariant()
$duplicateZip = Join-Path $proofRoot "duplicate.zip"
$stream = [IO.File]::Open($duplicateZip, [IO.FileMode]::Create, [IO.FileAccess]::ReadWrite)
try {
    $archive = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create, $true)
    try {
        foreach ($index in 1..2) {
            $entry = $archive.CreateEntry("runtime/v12.1.0/a.dby")
            $entry.LastWriteTime = $script:DBytePackageFixedTimestamp
            $entry.ExternalAttributes = 0
            $entryStream = $entry.Open()
            try { $entryStream.WriteByte(0x44) } finally { $entryStream.Dispose() }
        }
    }
    finally { $archive.Dispose() }
}
finally { $stream.Dispose() }
$expectedDuplicate = @([pscustomobject]@{ Path = "runtime/v12.1.0/a.dby"; Size = 1; Sha256 = $dataHash })
Assert-Fails { Test-CanonicalPackageZip $duplicateZip $expectedDuplicate "12.1.0" } "PACKAGE_ZIP_DUPLICATE"

if ($ExpectedCommit) {
    $resolved = Resolve-PackageCommit $repoRoot $ExpectedCommit
    Assert-Fails { Resolve-PackageCommit $repoRoot ("0" * 40) } "PACKAGE_SOURCE_COMMIT_INVALID"
    if ($DoubleBuild) {
        $hashes = @()
        foreach ($name in @("a", "b")) {
            $root = Join-Path $proofRoot $name
            $source = Join-Path $root "source"
            $output = Join-Path $root "output"
            New-Item -ItemType Directory -Path $source,$output -Force | Out-Null
            $archivePath = Join-Path $root "source.zip"
            & git.exe -C $repoRoot -c core.autocrlf=false archive --format=zip -o $archivePath $resolved
            if ($LASTEXITCODE -ne 0) { throw "PACKAGE_TEST_EXPORT_FAILED" }
            Expand-Archive -LiteralPath $archivePath -DestinationPath $source
            & (Join-Path $PSScriptRoot "package_release.ps1") -Version $Version -SourceRoot $source -OutputRoot $output -ExpectedCommit $resolved -ProvenanceRepositoryRoot $repoRoot -SkipBundle
            if ($LASTEXITCODE -ne 0) { throw "PACKAGE_TEST_BUILD_FAILED: $name" }
            $packageName = "dbyte-v$Version-windows-x64"
            $zip = Join-Path $output "$packageName.zip"
            & (Join-Path $PSScriptRoot "verify_windows_package.ps1") -ZipPath $zip -ReceiptPath (Join-Path $output "$packageName.manifest.json") -SourceRoot $source -Version $Version -ExpectedCommit $resolved -ProvenanceRepositoryRoot $repoRoot | Out-Host
            if ($LASTEXITCODE -ne 0) { throw "PACKAGE_TEST_VERIFY_FAILED: $name" }
            $hashes += (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        if ($hashes[0] -cne $hashes[1]) { throw "PACKAGE_NONDETERMINISTIC: $($hashes -join ' ')" }
        [IO.File]::WriteAllLines((Join-Path $proofRoot "deterministic-sha256.txt"), $hashes)
    }
}

"DBYTE WINDOWS RUNTIME PACKAGING VERIFIER: PASS"
