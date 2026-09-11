$global:DbyteVerifyGitTimeoutMs = 30000
$verifyGitCommands = @(Get-Command git.exe -CommandType Application)
if ($verifyGitCommands.Count -eq 0) { throw 'git.exe is unavailable for verification' }
$global:DbyteVerifyGitExecutable = $verifyGitCommands[0].Source
$verifyGitLauncherDir = Split-Path $global:DbyteVerifyGitExecutable -Parent
if ((Split-Path $verifyGitLauncherDir -Leaf) -ieq 'cmd') {
    $verifyGitDirect = Join-Path (Split-Path $verifyGitLauncherDir -Parent) 'mingw64\bin\git.exe'
    if (Test-Path -LiteralPath $verifyGitDirect -PathType Leaf) {
        $global:DbyteVerifyGitExecutable = $verifyGitDirect
        $verifyGitDirectDir = Split-Path $verifyGitDirect -Parent
        $normalizedDirectDir = $verifyGitDirectDir.TrimEnd('\\')
        $pathEntries = @($env:PATH -split ';')
        if (-not ($pathEntries | Where-Object { $_ -and ($_.Trim('"').TrimEnd('\\') -ieq $normalizedDirectDir) })) {
            $env:PATH = "$verifyGitDirectDir;$env:PATH"
        }
    }
}

function ConvertTo-VerifyGitArgument([string]$value) {
    if ($value.Length -eq 0) { return '""' }
    if ($value -notmatch '[\s"]') { return $value }
    $builder = [Text.StringBuilder]::new()
    [void]$builder.Append('"')
    $slashes = 0
    foreach ($ch in $value.ToCharArray()) {
        if ($ch -eq '\') { $slashes++; continue }
        if ($ch -eq '"') {
            [void]$builder.Append(('\' * (($slashes * 2) + 1)))
            [void]$builder.Append('"')
            $slashes = 0
            continue
        }
        if ($slashes) {
            [void]$builder.Append(('\' * $slashes))
            $slashes = 0
        }
        [void]$builder.Append($ch)
    }
    if ($slashes) { [void]$builder.Append(('\' * ($slashes * 2))) }
    [void]$builder.Append('"')
    return $builder.ToString()
}

function Invoke-VerifyGitProcess(
    [string]$executable,
    [string[]]$arguments,
    [int]$timeoutMs,
    [string]$workingDirectory
) {
    $psi = [Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $executable
    $psi.Arguments = (@($arguments | ForEach-Object { ConvertTo-VerifyGitArgument ([string]$_) }) -join ' ')
    $psi.WorkingDirectory = $workingDirectory
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $psi
    try {
        [void]$process.Start()
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($timeoutMs)) {
            try { $process.Kill() } catch {}
            [void]$process.WaitForExit(5000)
            $global:LASTEXITCODE = 124
            throw "verify git timeout after ${timeoutMs}ms: $($arguments -join ' ')"
        }
        $process.WaitForExit()
        return [pscustomobject]@{
            ExitCode = $process.ExitCode
            Stdout = $stdoutTask.Result
            Stderr = $stderrTask.Result
        }
    }
    finally {
        $process.Dispose()
    }
}

function git {
    $gitArgs = @($args | ForEach-Object { [string]$_ })
    $result = Invoke-VerifyGitProcess $global:DbyteVerifyGitExecutable $gitArgs $global:DbyteVerifyGitTimeoutMs (Get-Location).Path
    $global:LASTEXITCODE = $result.ExitCode
    if ($result.Stderr) {
        foreach ($line in ($result.Stderr.TrimEnd("`r", "`n") -split "`r?`n")) {
            if ($line.Length) { Write-Error -Message $line -ErrorAction Continue }
        }
    }
    if ($result.Stdout) {
        $result.Stdout.TrimEnd("`r", "`n") -split "`r?`n"
    }
}
