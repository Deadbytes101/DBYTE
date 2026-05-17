param(
    [string]$Version = "5.1.0"
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")
Set-Location $repoRoot

$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
$releaseName = "dbyte-v$Version-windows-x64"
$releaseDir = Join-Path $repoRoot $releaseName
$zipPath = Join-Path $repoRoot "$releaseName.zip"
$bundlePath = Join-Path $repoRoot "dbyte-v$Version.bundle"

& $cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

if (Test-Path $releaseDir) {
    Remove-Item -Recurse -Force $releaseDir
}
if (Test-Path $zipPath) {
    Remove-Item -Force $zipPath
}
if (Test-Path $bundlePath) {
    Remove-Item -Force $bundlePath
}

New-Item -ItemType Directory -Path $releaseDir | Out-Null
New-Item -ItemType Directory -Path (Join-Path $releaseDir "scripts") | Out-Null
New-Item -ItemType Directory -Path (Join-Path $releaseDir "benchmarks") | Out-Null

Copy-Item ".\target\release\dbyte.exe" (Join-Path $releaseDir "dbyte.exe")
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

New-Item -ItemType Directory -Path (Join-Path $releaseDir "docs") -Force | Out-Null
Copy-Item ".\docs\*" (Join-Path $releaseDir "docs") -Recurse -Force

git bundle create $bundlePath --all
Copy-Item $bundlePath (Join-Path $releaseDir (Split-Path $bundlePath -Leaf))

Compress-Archive -Path (Join-Path $releaseDir "*") -DestinationPath $zipPath

Write-Host "release package created: $zipPath"
Write-Host "bundle backup created: $bundlePath"
