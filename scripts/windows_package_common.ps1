$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

$script:DBytePackageFixedTimestamp = [DateTimeOffset]::new(2000, 1, 1, 0, 0, 0, [TimeSpan]::Zero)
$script:DBytePackageFixedFiles = @(
    "README.md",
    "INSTALL.md",
    "LANGUAGE_SPEC.md",
    "LICENSE",
    "scripts/install.ps1",
    "benchmarks/BENCHMARKS.md"
)
$script:DBytePackageTreeRoots = @("docs", "examples", "kernel-lab", "runtime")

function Sort-PackageStringsOrdinal([string[]]$Values) {
    $list = [System.Collections.Generic.List[string]]::new()
    foreach ($value in $Values) { $list.Add($value) }
    $list.Sort([System.Comparison[string]]{
        param($left, $right)
        return [StringComparer]::Ordinal.Compare($left, $right)
    })
    return @($list)
}

function Sort-PackageEntriesOrdinal([object[]]$Values) {
    $list = [System.Collections.Generic.List[object]]::new()
    foreach ($value in $Values) { $list.Add($value) }
    $list.Sort([System.Comparison[object]]{
        param($left, $right)
        return [StringComparer]::Ordinal.Compare([string]$left.Path, [string]$right.Path)
    })
    return @($list)
}

function Get-NormalizedPackagePath([string]$Path) {
    $normalized = $Path.Replace("\", "/")
    while ($normalized.StartsWith("./", [StringComparison]::Ordinal)) {
        $normalized = $normalized.Substring(2)
    }
    return $normalized
}

function Assert-PackageRelativePath([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path)) { throw "PACKAGE_PATH_INVALID: empty path" }
    if ($Path -cne (Get-NormalizedPackagePath $Path)) { throw "PACKAGE_PATH_INVALID: $Path" }
    if ($Path.StartsWith("/") -or $Path.Contains("\") -or $Path.Contains(":")) {
        throw "PACKAGE_PATH_INVALID: $Path"
    }
    foreach ($component in $Path.Split('/')) {
        if ($component -in @("", ".", "..")) { throw "PACKAGE_PATH_INVALID: $Path" }
    }
}

function Get-DByteWorkspaceVersion([string]$SourceRoot) {
    $manifestPath = Join-Path $SourceRoot "Cargo.toml"
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "PACKAGE_WORKSPACE_MANIFEST_REQUIRED: $manifestPath"
    }
    $content = Get-Content -LiteralPath $manifestPath -Raw
    $match = [regex]::Match($content, '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"')
    if (-not $match.Success) { throw "PACKAGE_WORKSPACE_VERSION_REQUIRED" }
    return $match.Groups[1].Value
}

function Invoke-PackageGit([string]$RepositoryRoot, [string[]]$Arguments, [string]$Description) {
    $saved = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = @(& git.exe -C $RepositoryRoot @Arguments 2>&1)
        $code = $LASTEXITCODE
    }
    finally { $ErrorActionPreference = $saved }
    if ($code -ne 0) { throw "${Description}: $($output -join "`n")" }
    return $output
}

function Resolve-PackageCommit([string]$RepositoryRoot, [string]$ExpectedCommit) {
    if ([string]::IsNullOrWhiteSpace($ExpectedCommit)) { return $null }
    $resolved = (@(Invoke-PackageGit $RepositoryRoot @("rev-parse", "$ExpectedCommit^{commit}") "PACKAGE_SOURCE_COMMIT_INVALID"))[0].Trim()
    if ($ExpectedCommit -match '^[0-9a-fA-F]{40}$' -and $resolved -cne $ExpectedCommit.ToLowerInvariant()) {
        throw "PACKAGE_SOURCE_COMMIT_MISMATCH: expected $ExpectedCommit, resolved $resolved"
    }
    return $resolved
}

function Get-PackageTrackedPaths(
    [string]$SourceRoot,
    [string]$ProvenanceRepositoryRoot,
    [string]$ResolvedCommit
) {
    $paths = [System.Collections.Generic.List[string]]::new()
    if ($ResolvedCommit) {
        $treePaths = Invoke-PackageGit $ProvenanceRepositoryRoot @(
            "ls-tree", "-r", "--name-only", $ResolvedCommit, "--",
            "docs", "examples", "kernel-lab", "runtime"
        ) "PACKAGE_TRACKED_MANIFEST_FAILED"
        foreach ($path in $script:DBytePackageFixedFiles + $treePaths) {
            if (-not [string]::IsNullOrWhiteSpace($path)) { $paths.Add((Get-NormalizedPackagePath $path.Trim())) }
        }
    }
    else {
        $gitRoot = (@(Invoke-PackageGit $SourceRoot @("rev-parse", "--show-toplevel") "PACKAGE_GIT_SOURCE_REQUIRED"))[0].Trim()
        if ((Resolve-Path -LiteralPath $gitRoot).Path -cne (Resolve-Path -LiteralPath $SourceRoot).Path) {
            throw "PACKAGE_SOURCE_ROOT_REQUIRED: source root must be the Git worktree root"
        }
        $tracked = Invoke-PackageGit $SourceRoot @(
            "ls-files", "--", "docs", "examples", "kernel-lab", "runtime",
            "README.md", "INSTALL.md", "LANGUAGE_SPEC.md", "LICENSE",
            "scripts/install.ps1", "benchmarks/BENCHMARKS.md"
        ) "PACKAGE_TRACKED_MANIFEST_FAILED"
        foreach ($path in $tracked) {
            if (-not [string]::IsNullOrWhiteSpace($path)) { $paths.Add((Get-NormalizedPackagePath $path.Trim())) }
        }
    }

    $seen = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $canonical = foreach ($path in (Sort-PackageStringsOrdinal @($paths))) {
        Assert-PackageRelativePath $path
        if (-not $seen.Add($path)) { throw "PACKAGE_MANIFEST_DUPLICATE: $path" }
        $path
    }
    return @($canonical)
}

function Assert-SourceTreeMatchesCommit([string]$SourceRoot, [string]$RepositoryRoot, [string]$Commit) {
    $treeLines = Invoke-PackageGit $RepositoryRoot @("ls-tree", "-r", $Commit) "PACKAGE_SOURCE_TREE_READ_FAILED"
    $expected = [System.Collections.Generic.Dictionary[string, string]]::new([StringComparer]::Ordinal)
    foreach ($line in $treeLines) {
        $match = [regex]::Match($line, '^[0-7]{6} blob ([0-9a-f]{40})\t(.+)$')
        if (-not $match.Success) { throw "PACKAGE_SOURCE_TREE_ENTRY_INVALID: $line" }
        $path = Get-NormalizedPackagePath $match.Groups[2].Value
        Assert-PackageRelativePath $path
        $expected.Add($path, $match.Groups[1].Value)
    }
    $actualPaths = @(
        Get-ChildItem -LiteralPath $SourceRoot -File -Recurse | ForEach-Object {
            Get-NormalizedPackagePath ([IO.Path]::GetRelativePath($SourceRoot, $_.FullName))
        }
    )
    $actualPaths = Sort-PackageStringsOrdinal $actualPaths
    if ($actualPaths.Count -ne $expected.Count) {
        $unexpected = @($actualPaths | Where-Object { -not $expected.ContainsKey($_) })
        $missing = @($expected.Keys | Where-Object { $actualPaths -notcontains $_ })
        throw "PACKAGE_SOURCE_TREE_MISMATCH: expected $($expected.Count), actual $($actualPaths.Count), unexpected [$($unexpected -join ', ')], missing [$($missing -join ', ')]"
    }
    foreach ($path in $actualPaths) {
        if (-not $expected.ContainsKey($path)) { throw "PACKAGE_SOURCE_TREE_UNEXPECTED: $path" }
    }
    foreach ($path in $actualPaths) {
        $file = Join-Path $SourceRoot ($path.Replace('/', [IO.Path]::DirectorySeparatorChar))
        $bytes = [IO.File]::ReadAllBytes($file)
        $header = [Text.Encoding]::ASCII.GetBytes("blob $($bytes.Length)`0")
        $sha1 = [Security.Cryptography.SHA1]::Create()
        try {
            [void]$sha1.TransformBlock($header, 0, $header.Length, $null, 0)
            [void]$sha1.TransformFinalBlock($bytes, 0, $bytes.Length)
            $actualHash = [Convert]::ToHexString($sha1.Hash).ToLowerInvariant()
        }
        finally { $sha1.Dispose() }
        if ($actualHash -cne $expected[$path]) {
            throw "PACKAGE_SOURCE_PROVENANCE_MISMATCH: $path"
        }
    }
}

function Assert-RuntimeTree([string[]]$Paths, [string]$Version) {
    $runtime = @($Paths | Where-Object { $_.StartsWith("runtime/", [StringComparison]::Ordinal) })
    if ($runtime.Count -eq 0) { throw "PACKAGE_RUNTIME_TREE_REQUIRED" }
    $packageVersion = [Version]::Parse($Version)
    $runtimeVersions = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    foreach ($path in $runtime) {
        if ($path.StartsWith("runtime/runtime/", [StringComparison]::Ordinal)) {
            throw "PACKAGE_RUNTIME_NESTED: $path"
        }
        $match = [regex]::Match($path, '^runtime/v([0-9]+\.[0-9]+\.[0-9]+)/[^/]+$')
        if (-not $match.Success) { throw "PACKAGE_RUNTIME_PATH_INVALID: $path" }
        $runtimeVersion = [Version]::Parse($match.Groups[1].Value)
        if ($runtimeVersion -gt $packageVersion) { throw "PACKAGE_RUNTIME_FUTURE_VERSION: $path" }
        [void]$runtimeVersions.Add($match.Groups[1].Value)
    }
    if ($packageVersion -eq [Version]::new(12, 2, 1) -and $runtimeVersions.Contains("12.2.1")) {
        throw "PACKAGE_RUNTIME_UNEXPECTED_PATCH_TREE"
    }
    if (-not $runtimeVersions.Contains("12.1.0")) { throw "PACKAGE_RUNTIME_FOUNDATION_REQUIRED" }
    if ($packageVersion -ge [Version]::new(12, 2, 0) -and -not $runtimeVersions.Contains("12.2.0")) {
        throw "PACKAGE_RUNTIME_PROVIDER_REQUIRED"
    }
}

function Get-CanonicalPackageManifest(
    [string]$SourceRoot,
    [string]$Version,
    [string]$ProvenanceRepositoryRoot = $SourceRoot,
    [string]$ExpectedCommit = ""
) {
    $source = (Resolve-Path -LiteralPath $SourceRoot).Path
    $repository = (Resolve-Path -LiteralPath $ProvenanceRepositoryRoot).Path
    $workspaceVersion = Get-DByteWorkspaceVersion $source
    if ($workspaceVersion -cne $Version) {
        throw "PACKAGE_VERSION_MISMATCH: requested $Version, source is $workspaceVersion"
    }
    $resolvedCommit = Resolve-PackageCommit $repository $ExpectedCommit
    if ($resolvedCommit) { Assert-SourceTreeMatchesCommit $source $repository $resolvedCommit }
    $paths = Get-PackageTrackedPaths $source $repository $resolvedCommit
    Assert-RuntimeTree $paths $Version

    $entries = foreach ($path in $paths) {
        $nativePath = $path.Replace('/', [IO.Path]::DirectorySeparatorChar)
        $file = Join-Path $source $nativePath
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
            throw "PACKAGE_TRACKED_FILE_MISSING: $path"
        }
        if ($resolvedCommit) {
            $expectedBlob = (@(Invoke-PackageGit $repository @("rev-parse", "${resolvedCommit}:$path") "PACKAGE_TAG_BLOB_MISSING"))[0].Trim()
            $actualBlob = (@(Invoke-PackageGit $repository @("hash-object", "--no-filters", "--", $file) "PACKAGE_SOURCE_HASH_FAILED"))[0].Trim()
            if ($actualBlob -cne $expectedBlob) {
                throw "PACKAGE_SOURCE_PROVENANCE_MISMATCH: $path"
            }
        }
        $item = Get-Item -LiteralPath $file
        [pscustomobject]@{
            Path = $path
            FilePath = $item.FullName
            Size = [int64]$item.Length
            Sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    return [pscustomobject]@{
        Version = $Version
        Commit = $resolvedCommit
        SourceRoot = $source
        Entries = @($entries)
    }
}

function Write-DeterministicPackageZip([string]$ZipPath, [object[]]$Entries) {
    $parent = Split-Path -Parent $ZipPath
    New-Item -ItemType Directory -Path $parent -Force | Out-Null
    $temporary = Join-Path $parent (".{0}.{1}.tmp" -f ([IO.Path]::GetFileName($ZipPath)), [guid]::NewGuid().ToString("N"))
    $ordered = Sort-PackageEntriesOrdinal $Entries
    $seen = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    try {
        $stream = [IO.File]::Open($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        try {
            $archive = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create, $true, [Text.Encoding]::UTF8)
            try {
                foreach ($item in $ordered) {
                    $path = [string]$item.Path
                    Assert-PackageRelativePath $path
                    if (-not $seen.Add($path)) { throw "PACKAGE_MANIFEST_DUPLICATE: $path" }
                    $entry = $archive.CreateEntry($path, [IO.Compression.CompressionLevel]::Optimal)
                    $entry.LastWriteTime = $script:DBytePackageFixedTimestamp
                    $entry.ExternalAttributes = 0
                    $input = [IO.File]::OpenRead([string]$item.FilePath)
                    try {
                        $output = $entry.Open()
                        try { $input.CopyTo($output) } finally { $output.Dispose() }
                    }
                    finally { $input.Dispose() }
                }
            }
            finally { $archive.Dispose() }
        }
        finally { $stream.Dispose() }
        Move-Item -LiteralPath $temporary -Destination $ZipPath -Force
    }
    finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    }
}

function Get-StreamSha256([IO.Stream]$Stream) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([Convert]::ToHexString($sha.ComputeHash($Stream))).ToLowerInvariant() }
    finally { $sha.Dispose() }
}

function Get-PackageZipEntries([string]$ZipPath) {
    $results = [System.Collections.Generic.List[object]]::new()
    $seen = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $stream = [IO.File]::OpenRead($ZipPath)
    try {
        $archive = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Read, $true, [Text.Encoding]::UTF8)
        try {
            foreach ($entry in $archive.Entries) {
                $path = $entry.FullName
                if ($path.EndsWith("/", [StringComparison]::Ordinal)) { continue }
                Assert-PackageRelativePath $path
                if (-not $seen.Add($path)) { throw "PACKAGE_ZIP_DUPLICATE: $path" }
                $entryStream = $entry.Open()
                try { $hash = Get-StreamSha256 $entryStream } finally { $entryStream.Dispose() }
                $results.Add([pscustomobject]@{
                    Path = $path
                    Size = [int64]$entry.Length
                    Sha256 = $hash
                    Timestamp = $entry.LastWriteTime.ToString("o")
                    Attributes = $entry.ExternalAttributes
                })
            }
        }
        finally { $archive.Dispose() }
    }
    finally { $stream.Dispose() }
    return @($results)
}

function Test-CanonicalPackageZip(
    [string]$ZipPath,
    [object[]]$ExpectedEntries,
    [string]$Version,
    [switch]$RequireFixedMetadata
) {
    if (-not (Test-Path -LiteralPath $ZipPath -PathType Leaf)) { throw "PACKAGE_ZIP_REQUIRED: $ZipPath" }
    $expected = [System.Collections.Generic.Dictionary[string, object]]::new([StringComparer]::Ordinal)
    foreach ($item in $ExpectedEntries) {
        Assert-PackageRelativePath ([string]$item.Path)
        if (-not $expected.TryAdd([string]$item.Path, $item)) { throw "PACKAGE_EXPECTED_DUPLICATE: $($item.Path)" }
    }
    $seen = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $stream = [IO.File]::OpenRead($ZipPath)
    try {
        $archive = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Read, $true, [Text.Encoding]::UTF8)
        try {
            foreach ($entry in $archive.Entries) {
                $path = $entry.FullName
                Assert-PackageRelativePath $path
                if (-not $seen.Add($path)) { throw "PACKAGE_ZIP_DUPLICATE: $path" }
                if (-not $expected.ContainsKey($path)) { throw "PACKAGE_ZIP_UNEXPECTED: $path" }
                if ($RequireFixedMetadata) {
                    $stamp = $entry.LastWriteTime
                    if ($stamp.Year -ne 2000 -or $stamp.Month -ne 1 -or $stamp.Day -ne 1 -or
                        $stamp.Hour -ne 0 -or $stamp.Minute -ne 0 -or $stamp.Second -ne 0) {
                        throw "PACKAGE_ZIP_TIMESTAMP_DRIFT: $path"
                    }
                    if ($entry.ExternalAttributes -ne 0) { throw "PACKAGE_ZIP_ATTRIBUTE_DRIFT: $path" }
                }
                $entryStream = $entry.Open()
                try { $actualHash = Get-StreamSha256 $entryStream } finally { $entryStream.Dispose() }
                if ($actualHash -cne ([string]$expected[$path].Sha256).ToLowerInvariant()) {
                    throw "PACKAGE_ZIP_HASH_MISMATCH: $path"
                }
            }
        }
        finally { $archive.Dispose() }
    }
    finally { $stream.Dispose() }
    $missing = Sort-PackageStringsOrdinal @($expected.Keys | Where-Object { -not $seen.Contains($_) })
    if ($missing.Count -ne 0) { throw "PACKAGE_ZIP_MISSING: $($missing -join ', ')" }
    Assert-RuntimeTree @($seen) $Version
    return [pscustomobject]@{
        Path = (Resolve-Path -LiteralPath $ZipPath).Path
        Sha256 = (Get-FileHash -LiteralPath $ZipPath -Algorithm SHA256).Hash.ToLowerInvariant()
        Size = (Get-Item -LiteralPath $ZipPath).Length
        EntryCount = $seen.Count
        RuntimeCount = @($seen | Where-Object { $_.StartsWith("runtime/", [StringComparison]::Ordinal) }).Count
    }
}

function Write-PackageReceipt(
    [string]$Path,
    [object]$Manifest,
    [object]$ZipEvidence,
    [object[]]$Entries
) {
    $receipt = [ordered]@{
        schema = "dbyte-windows-package-receipt-v1"
        version = $Manifest.Version
        source_commit = $Manifest.Commit
        zip_sha256 = $ZipEvidence.Sha256
        zip_size = $ZipEvidence.Size
        entry_count = $ZipEvidence.EntryCount
        runtime_count = $ZipEvidence.RuntimeCount
        files = @(Sort-PackageEntriesOrdinal $Entries | ForEach-Object {
            [ordered]@{ path = $_.Path; size = $_.Size; sha256 = $_.Sha256 }
        })
    }
    [IO.File]::WriteAllText($Path, ($receipt | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
}
