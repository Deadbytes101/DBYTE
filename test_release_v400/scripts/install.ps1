param(
    [string]$InstallDir = (Join-Path $HOME ".dbyte\bin"),
    [string]$ReleaseUrl = "https://github.com/deadbyte/dbyte/releases/latest/download/dbyte.exe",
    [switch]$NoPathUpdate
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..") -ErrorAction SilentlyContinue

$localCandidates = @(
    (Join-Path $scriptDir "..\dbyte.exe"),
    (Join-Path $scriptDir "dbyte.exe")
)

if ($repoRoot) {
    $localCandidates += @(
        (Join-Path $repoRoot "target\release\dbyte.exe"),
        (Join-Path $repoRoot "dbyte.exe")
    )
}

$sourceExe = $null
foreach ($candidate in $localCandidates) {
    $resolved = Resolve-Path $candidate -ErrorAction SilentlyContinue
    if ($resolved -and (Test-Path $resolved)) {
        $sourceExe = $resolved.Path
        break
    }
}

$tempDir = $null
if (-not $sourceExe) {
    if ([string]::IsNullOrWhiteSpace($ReleaseUrl)) {
        throw "dbyte.exe not found. Run this installer from a release zip or pass -ReleaseUrl."
    }
    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("dbyte-install-" + [System.Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $tempDir | Out-Null
    $sourceExe = Join-Path $tempDir "dbyte.exe"
    Write-Host "Downloading DByte from $ReleaseUrl"
    Invoke-WebRequest -Uri $ReleaseUrl -OutFile $sourceExe
}

New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
$targetExe = Join-Path $InstallDir "dbyte.exe"
Copy-Item -Force $sourceExe $targetExe

$currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
$pathParts = @()
if ($currentPath) {
    $pathParts = $currentPath.Split(";") | Where-Object { $_ -ne "" }
}

$alreadyInPath = $false
foreach ($part in $pathParts) {
    if ($part.TrimEnd("\") -ieq $InstallDir.TrimEnd("\")) {
        $alreadyInPath = $true
        break
    }
}

if ($NoPathUpdate) {
    Write-Host "Skipping PATH update because -NoPathUpdate was set."
}
elseif (-not $alreadyInPath) {
    $newPath = (($pathParts + $InstallDir) -join ";")
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    Write-Host "Added $InstallDir to user PATH. Restart PowerShell to use dbyte globally."
}

$version = & $targetExe --version
Write-Host "Installed $version to $targetExe"

if ($tempDir) {
    Remove-Item -Recurse -Force $tempDir
}
