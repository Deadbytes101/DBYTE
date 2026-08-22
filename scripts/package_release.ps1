param(
    [string]$Version = "11.6.0",
    [switch]$SkipBundle
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
Set-Location $repoRoot

$workspaceManifest = Get-Content -LiteralPath (Join-Path $repoRoot "Cargo.toml") -Raw
$workspaceVersionMatch = [regex]::Match($workspaceManifest, '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"')
if (-not $workspaceVersionMatch.Success) {
    throw "could not read workspace package version"
}
$workspaceVersion = $workspaceVersionMatch.Groups[1].Value
if ($Version -ne $workspaceVersion) {
    throw "release version $Version does not match workspace version $workspaceVersion"
}

$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
$releaseName = "dbyte-v$Version-windows-x64"
$releaseDir = Join-Path $repoRoot $releaseName
$zipPath = Join-Path $repoRoot "$releaseName.zip"

& $cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

if (Test-Path $releaseDir) {
    Remove-Item -Recurse -Force $releaseDir
}
if (Test-Path $zipPath) {
    Remove-Item -Force $zipPath
}

New-Item -ItemType Directory -Path $releaseDir | Out-Null
New-Item -ItemType Directory -Path (Join-Path $releaseDir "scripts") | Out-Null
New-Item -ItemType Directory -Path (Join-Path $releaseDir "benchmarks") | Out-Null

Copy-Item ".\target\release\dbyte.exe" (Join-Path $releaseDir "dbyte.exe")
Copy-Item ".\target\release\dbytec.exe" (Join-Path $releaseDir "dbytec.exe")
Copy-Item ".\README.md" $releaseDir
Copy-Item ".\INSTALL.md" $releaseDir
Copy-Item ".\LANGUAGE_SPEC.md" $releaseDir
Copy-Item ".\LICENSE" $releaseDir
Copy-Item ".\scripts\install.ps1" (Join-Path $releaseDir "scripts\install.ps1")
Copy-Item ".\benchmarks\BENCHMARKS.md" (Join-Path $releaseDir "benchmarks\BENCHMARKS.md")

# Use robust copy to avoid nesting
New-Item -ItemType Directory -Path (Join-Path $releaseDir "examples") -Force | Out-Null
Copy-Item ".\examples\*" (Join-Path $releaseDir "examples") -Recurse -Force

# Clean up examples/dbyteos/tmp junk in the release dir
$releaseTmp = Join-Path $releaseDir "examples\dbyteos\tmp"
if (Test-Path $releaseTmp) {
    Get-ChildItem -Path $releaseTmp -Exclude ".gitignore", ".gitkeep" | Remove-Item -Recurse -Force
}

# Clean up examples/dbyteos/home/deadbyte junk in the release dir
$releaseHome = Join-Path $releaseDir "examples\dbyteos\home\deadbyte"
if (Test-Path $releaseHome) {
    Get-ChildItem -Path $releaseHome -Exclude ".gitignore", ".gitkeep", "preferences.dby", "projects" | Remove-Item -Recurse -Force
    $releaseProjects = Join-Path $releaseHome "projects"
    if (Test-Path $releaseProjects) {
        Get-ChildItem -Path $releaseProjects -Exclude ".gitignore", ".gitkeep" | Remove-Item -Recurse -Force
    }
}

New-Item -ItemType Directory -Path (Join-Path $releaseDir "docs") -Force | Out-Null
Copy-Item ".\docs\*" (Join-Path $releaseDir "docs") -Recurse -Force

New-Item -ItemType Directory -Path (Join-Path $releaseDir "kernel-lab") -Force | Out-Null
Copy-Item ".\kernel-lab\*" (Join-Path $releaseDir "kernel-lab") -Recurse -Force
$releaseKernelLabTarget = Join-Path $releaseDir "kernel-lab\target"
if (Test-Path $releaseKernelLabTarget) {
    Remove-Item -Recurse -Force $releaseKernelLabTarget -ErrorAction SilentlyContinue
}

$targetBundlePath = Join-Path $releaseDir "dbyte-v$Version.bundle"
if (-not $SkipBundle) {
    $tagName = "v$Version"
    $status = git status --short
    if ($LASTEXITCODE -ne 0) { throw "git status failed" }
    if ($status) { throw "release bundle requires a clean working tree" }

    $headCommit = git rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw "could not resolve HEAD" }

    $tagType = git cat-file -t $tagName 2>$null
    if ($LASTEXITCODE -ne 0 -or $tagType -ne "tag") {
        throw "release bundle requires annotated tag $tagName"
    }

    $tagCommit = git rev-parse "$tagName^{}"
    if ($LASTEXITCODE -ne 0 -or $headCommit -ne $tagCommit) {
        throw "release bundle requires HEAD == $tagName^{}"
    }

    $tagRefs = @(git for-each-ref '--format=%(refname)' refs/tags)
    if ($LASTEXITCODE -ne 0 -or $tagRefs -notcontains "refs/tags/$tagName") {
        throw "could not enumerate release tag history"
    }

    git bundle create $targetBundlePath @tagRefs
    if ($LASTEXITCODE -ne 0) { throw "git bundle creation failed for $tagName" }
}

Compress-Archive -Path (Join-Path $releaseDir "*") -DestinationPath $zipPath

Write-Host "release package created: $zipPath"
if ($SkipBundle) {
    Write-Host "bundle backup skipped"
}
else {
    Write-Host "bundle backup created: $targetBundlePath"
}
