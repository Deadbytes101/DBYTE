param([string]$MilestoneVersion = "12.1.0")

$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path
$sourceRoot = Join-Path $repoRoot "runtime\v12.1.0"
$proofRoot = Join-Path $repoRoot "tmp\native_v$MilestoneVersion"
$firstRoot = Join-Path $proofRoot "first"
$secondRoot = Join-Path $proofRoot "second"
$compiler = Join-Path $repoRoot "target\debug\dbytec.exe"
$qemu = "C:\Program Files\qemu\qemu-system-i386.exe"
$objdump = (Get-Command objdump.exe -ErrorAction Stop).Source
$modules = @("arch_x86", "serial", "alloc", "array", "string", "kernel")
$expected = "DBYTE RUNTIME FOUNDATION`r`nDROP ONLINE`r`nALLOCATOR ABI ONLINE`r`nARRAY ONLINE`r`nSTRING ONLINE`r`nMOVE OWNERSHIP OK`r`nOOM TRANSACTION OK`r`nRUNTIME CHECK OK`r`n"
$ownedQemu = [System.Collections.Generic.List[int]]::new()

function Assert-True($condition, [string]$message) {
    if (-not $condition) { throw $message }
}

function Invoke-Checked([string]$executable, [string[]]$arguments, [string]$description) {
    $old = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $executable @arguments 2>&1
        $exitCode = $LASTEXITCODE
    } finally { $ErrorActionPreference = $old }
    if ($exitCode -ne 0) { throw "$description failed ($exitCode):`n$($output -join "`n")" }
    return ($output -join "`n")
}

function Assert-Same([string]$left, [string]$right, [string]$description) {
    $leftHash = (Get-FileHash -LiteralPath $left -Algorithm SHA256).Hash
    $rightHash = (Get-FileHash -LiteralPath $right -Algorithm SHA256).Hash
    Assert-True ($leftHash -ceq $rightHash) "$description is not deterministic"
    return $leftHash
}

function Read-SharedText([string]$path) {
    $stream = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
    try {
        $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::ASCII, $false, 1024, $true)
        try { return $reader.ReadToEnd() } finally { $reader.Dispose() }
    } finally { $stream.Dispose() }
}

function Build-Set([string]$destination) {
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    foreach ($module in $modules) {
        Invoke-Checked $compiler @(
            "build", (Join-Path $sourceRoot "$module.dby"),
            "--target", "i686-dbyte-none", "--emit", "obj",
            "-o", (Join-Path $destination "$module.o"), "--dump-ir"
        ) "build $module.o" | Set-Content -LiteralPath (Join-Path $destination "$module.ir.txt")
    }
    Invoke-Checked $compiler @(
        "build", (Join-Path $sourceRoot "array.dby"),
        "--target", "i686-dbyte-none", "--emit", "staticlib",
        "-o", (Join-Path $destination "libdbyte_runtime.a")
    ) "build runtime archive" | Out-Null
    Invoke-Checked $compiler @(
        "build", (Join-Path $sourceRoot "kernel.dby"),
        "--target", "i686-dbyte-none", "--emit", "elf",
        "-o", (Join-Path $destination "runtime.elf")
    ) "build integrated runtime ELF" | Out-Null
    Invoke-Checked $compiler @(
        "build", (Join-Path $sourceRoot "kernel.dby"),
        "--target", "i686-dbyte-none", "--emit", "bin",
        "-o", (Join-Path $destination "runtime.bin")
    ) "build integrated runtime BIN" | Out-Null
}

function Invoke-Proof([int]$ram, [string]$elf) {
    $runRoot = Join-Path $proofRoot "${ram}m"
    New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
    $serial = Join-Path $runRoot "serial.log"
    $trace = Join-Path $runRoot "qemu.log"
    foreach ($path in @($serial, $trace)) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
    }
    $arguments = @(
        "-kernel", $elf, "-m", "${ram}M", "-display", "none", "-monitor", "none",
        "-serial", "file:$serial", "-d", "int,cpu_reset,guest_errors", "-D", $trace,
        "-no-reboot", "-no-shutdown"
    )
    [IO.File]::WriteAllText((Join-Path $runRoot "qemu-command.txt"), "$qemu $($arguments -join ' ')")
    $process = Start-Process -FilePath $qemu -ArgumentList $arguments -PassThru -WindowStyle Hidden
    $ownedQemu.Add($process.Id)
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(12)
        $matched = $false
        while ([DateTime]::UtcNow -lt $deadline -and -not $process.HasExited) {
            Start-Sleep -Milliseconds 100
            if (Test-Path -LiteralPath $serial) {
                $actual = Read-SharedText $serial
                Assert-True ($expected.StartsWith($actual, [StringComparison]::Ordinal)) "${ram}M serial diverged"
                if ($actual -ceq $expected) { $matched = $true; break }
            }
        }
        Assert-True $matched "${ram}M runtime proof did not complete"
    } finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
    }
    $traceText = [IO.File]::ReadAllText($trace)
    Assert-True (-not $traceText.Contains("Triple fault")) "${ram}M runtime proof triple-faulted"
    Assert-True (([regex]::Matches($traceText, 'v=0[0de]')).Count -eq 0) "${ram}M runtime proof faulted"
}

Push-Location $repoRoot
try {
    if (Test-Path -LiteralPath $proofRoot) { Remove-Item -LiteralPath $proofRoot -Recurse -Force }
    New-Item -ItemType Directory -Path $proofRoot -Force | Out-Null
    Invoke-Checked "$env:USERPROFILE\.cargo\bin\cargo.exe" @("build", "-p", "dbyte_cli", "--bin", "dbytec") "build dbytec" | Out-Null
    Build-Set $firstRoot
    Build-Set $secondRoot
    $hashes = [ordered]@{}
    foreach ($name in @($modules | ForEach-Object { "$_.o" }) + @("libdbyte_runtime.a", "runtime.elf", "runtime.bin")) {
        $hashes[$name] = Assert-Same (Join-Path $firstRoot $name) (Join-Path $secondRoot $name) $name
    }
    $arrayInspect = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", (Join-Path $firstRoot "array.o")) "inspect array object"
    $stringInspect = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", (Join-Path $firstRoot "string.o")) "inspect string object"
    $finalInspect = Invoke-Checked $objdump @("-f", "-h", "-t", "-r", "-d", "-Mintel", (Join-Path $firstRoot "runtime.elf")) "inspect runtime ELF"
    $kernelIr = [IO.File]::ReadAllText((Join-Path $firstRoot "kernel.ir.txt"))
    [IO.File]::WriteAllText((Join-Path $proofRoot "array.objdump.txt"), $arrayInspect)
    [IO.File]::WriteAllText((Join-Path $proofRoot "string.objdump.txt"), $stringInspect)
    [IO.File]::WriteAllText((Join-Path $proofRoot "runtime.objdump.txt"), $finalInspect)
    Assert-True ($arrayInspect.Contains(".dbyte.drop")) "Array object lacks drop metadata"
    Assert-True ($stringInspect.Contains(".dbyte.drop")) "String object lacks drop metadata"
    Assert-True (-not $finalInspect.Contains(".dbyte.drop")) "final ELF retained drop metadata"
    Assert-True ($finalInspect.Contains("call")) "final ELF has no calls"
    Assert-True ($finalInspect -match 'call\s+e(?:ax|bx|cx|dx|si|di)\b') "final ELF lacks indirect register call required by allocator vtable dispatch"
    Assert-True ($finalInspect.Contains("<validate_utf8>")) "final ELF lacks native UTF-8 validation"
    Assert-True ($finalInspect.Contains("<kernel_initialize>")) "final ELF lacks explicit KernelAllocator adapter"
    Assert-True (([regex]::Matches($kernelIr, 'callee: Direct \{ symbol: "from_utf8" \}')).Count -eq 2) "kernel proof must execute valid and invalid UTF-8 conversions"
    Assert-True ($kernelIr -match 'callee: Direct \{ symbol: "[^"]*as_mut_slice_t\d+" \}') "kernel proof does not execute mutable slice construction"
    Assert-True ($kernelIr -match 'callee: Direct \{ symbol: "[^"]*clone_t\d+" \}') "kernel proof does not execute Array clone"
    Assert-True ($kernelIr -match 'callee: Direct \{ symbol: "[^"]*clone_in_t\d+" \}') "kernel proof does not execute Array clone_in"
    Assert-True ($kernelIr -match 'callee: Direct \{ symbol: "[^"]*_clone" \}') "kernel proof does not execute String clone"
    Assert-True ($kernelIr -match 'callee: Direct \{ symbol: "[^"]*_clone_in" \}') "kernel proof does not execute String clone_in"
    Invoke-Proof 32 (Join-Path $firstRoot "runtime.elf")
    Invoke-Proof 64 (Join-Path $firstRoot "runtime.elf")
    $hashLines = $hashes.GetEnumerator() | ForEach-Object { "$($_.Value)  $($_.Key)" }
    [IO.File]::WriteAllLines((Join-Path $proofRoot "SHA256SUMS"), $hashLines)
    "DBYTE RUNTIME FOUNDATION VERIFIER: PASS"
    $hashLines
} finally {
    Pop-Location
    $reapDeadline = [DateTime]::UtcNow.AddSeconds(2)
    do {
        $orphans = @($ownedQemu | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue })
        if ($orphans.Count -eq 0 -or [DateTime]::UtcNow -ge $reapDeadline) { break }
        Start-Sleep -Milliseconds 100
    } while ($true)
    if ($orphans.Count -ne 0) { throw "runtime verifier left owned QEMU processes: $($orphans -join ', ')" }
}
