param(
    [string]$Version = "13.0.0",
    [string]$SourceRoot = "",
    [string]$OutputRoot = "",
    [string]$ExpectedCommit = "",
    [string]$ProvenanceRepositoryRoot = "",
    [switch]$SkipBundle
)

$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "windows_package_common.ps1")

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
if ([string]::IsNullOrWhiteSpace($SourceRoot)) { $SourceRoot = $repoRoot }
if ([string]::IsNullOrWhiteSpace($OutputRoot)) { $OutputRoot = $repoRoot }
if ([string]::IsNullOrWhiteSpace($ProvenanceRepositoryRoot)) { $ProvenanceRepositoryRoot = $repoRoot }
$source = (Resolve-Path -LiteralPath $SourceRoot).Path
$output = [IO.Path]::GetFullPath($OutputRoot)
New-Item -ItemType Directory -Path $output -Force | Out-Null

$manifest = Get-CanonicalPackageManifest $source $Version $ProvenanceRepositoryRoot $ExpectedCommit
$bundleTag = $null
if (-not $SkipBundle) {
    if (-not $manifest.Commit) { throw "PACKAGE_BUNDLE_COMMIT_REQUIRED" }
    $bundleTag = "v$Version"
    $tagType = (@(Invoke-PackageGit $ProvenanceRepositoryRoot @("cat-file", "-t", $bundleTag) "PACKAGE_BUNDLE_TAG_REQUIRED"))[0].Trim()
    if ($tagType -cne "tag") { throw "PACKAGE_BUNDLE_ANNOTATED_TAG_REQUIRED: $bundleTag" }
    $tagCommit = (@(Invoke-PackageGit $ProvenanceRepositoryRoot @("rev-parse", "$bundleTag^{}") "PACKAGE_BUNDLE_TAG_REQUIRED"))[0].Trim()
    if ($tagCommit -cne $manifest.Commit) { throw "PACKAGE_BUNDLE_TAG_COMMIT_MISMATCH" }
}
$releaseName = "dbyte-v$Version-windows-x64"
$finalDirectory = Join-Path $output $releaseName
$finalZip = Join-Path $output "$releaseName.zip"
$finalReceipt = Join-Path $output "$releaseName.manifest.json"
$operationRoot = Join-Path $output (".package-{0}-{1}" -f $Version, [guid]::NewGuid().ToString("N"))
$buildRoot = Join-Path $operationRoot "target"
$staging = Join-Path $operationRoot $releaseName
$candidateZip = Join-Path $operationRoot "$releaseName.zip"

$oldTarget = $env:CARGO_TARGET_DIR
$oldRustFlags = $env:RUSTFLAGS
$oldIncremental = $env:CARGO_INCREMENTAL
$oldEpoch = $env:SOURCE_DATE_EPOCH
try {
    New-Item -ItemType Directory -Path $staging -Force | Out-Null
    $env:CARGO_TARGET_DIR = $buildRoot
    $env:CARGO_INCREMENTAL = "0"
    $env:SOURCE_DATE_EPOCH = "946684800"
    # MSVC link.exe otherwise writes wall-clock timestamps and random CodeView GUIDs.
    # /Brepro makes those PE fields content-derived; the path remap removes export-root drift.
    $remap = "--remap-path-prefix=$source=. -C link-arg=/Brepro"
    $env:RUSTFLAGS = if ([string]::IsNullOrWhiteSpace($oldRustFlags)) { $remap } else { "$oldRustFlags $remap" }
    Push-Location $source
    try {
        & cargo.exe build --release --locked
        if ($LASTEXITCODE -ne 0) { throw "PACKAGE_RELEASE_BUILD_FAILED: $LASTEXITCODE" }
    }
    finally { Pop-Location }

    $packageEntries = [System.Collections.Generic.List[object]]::new()
    foreach ($item in $manifest.Entries) {
        $destination = Join-Path $staging ($item.Path.Replace('/', [IO.Path]::DirectorySeparatorChar))
        New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
        Copy-Item -LiteralPath $item.FilePath -Destination $destination
        $packageEntries.Add([pscustomobject]@{
            Path = $item.Path
            FilePath = $destination
            Size = $item.Size
            Sha256 = $item.Sha256
        })
    }
    foreach ($binaryName in @("dbyte.exe", "dbytec.exe")) {
        $binary = Join-Path $buildRoot "release\$binaryName"
        if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw "PACKAGE_BINARY_REQUIRED: $binaryName" }
        if ($binaryName -ceq "dbyte.exe") {
            $versionOutput = @(& $binary --version 2>&1)
            if ($LASTEXITCODE -ne 0 -or ($versionOutput -join "`n") -notmatch [regex]::Escape($Version)) {
                throw "PACKAGE_BINARY_VERSION_MISMATCH: $binaryName"
            }
        }
        else {
            $oldPreference = $ErrorActionPreference
            $ErrorActionPreference = "Continue"
            try {
                $probe = @(& $binary 2>&1)
                $probeExitCode = $LASTEXITCODE
            }
            finally {
                $ErrorActionPreference = $oldPreference
            }
            if ($probeExitCode -ne 2 -or -not ($probe -join "`n").Contains("dbytec build")) {
                throw "PACKAGE_BINARY_CAPABILITY_MISMATCH: $binaryName"
            }
        }
        $destination = Join-Path $staging $binaryName
        Copy-Item -LiteralPath $binary -Destination $destination
        $packageEntries.Add([pscustomobject]@{
            Path = $binaryName
            FilePath = $destination
            Size = (Get-Item -LiteralPath $destination).Length
            Sha256 = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
        })
    }

    Write-DeterministicPackageZip $candidateZip @($packageEntries)
    $zipEvidence = Test-CanonicalPackageZip $candidateZip @($packageEntries) $Version -RequireFixedMetadata

    if (Test-Path -LiteralPath $finalDirectory) { Remove-Item -LiteralPath $finalDirectory -Recurse -Force }
    Move-Item -LiteralPath $staging -Destination $finalDirectory
    Move-Item -LiteralPath $candidateZip -Destination $finalZip -Force
    Write-PackageReceipt $finalReceipt $manifest $zipEvidence @($packageEntries)

    if (-not $SkipBundle) {
        $bundle = Join-Path $output "dbyte-v$Version.bundle"
        & git.exe -C $ProvenanceRepositoryRoot bundle create $bundle "refs/tags/$bundleTag"
        if ($LASTEXITCODE -ne 0) { throw "PACKAGE_BUNDLE_CREATE_FAILED" }
    }

    Write-Host "release package created: $finalZip"
    Write-Host "release package sha256: $((Get-FileHash -LiteralPath $finalZip -Algorithm SHA256).Hash.ToLowerInvariant())"
    Write-Host "release package entries: $($zipEvidence.EntryCount) (runtime $($zipEvidence.RuntimeCount))"
}
finally {
    $env:CARGO_TARGET_DIR = $oldTarget
    $env:RUSTFLAGS = $oldRustFlags
    $env:CARGO_INCREMENTAL = $oldIncremental
    $env:SOURCE_DATE_EPOCH = $oldEpoch
    if (Test-Path -LiteralPath $operationRoot) { Remove-Item -LiteralPath $operationRoot -Recurse -Force }
}
$global:LASTEXITCODE = 0
