$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'v13_release_identity.ps1')
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$commit = $V13FrozenBaseHead
$tag = '28c074777f46bd9e8885f42c49f903ba34e19254'
# Never reset old evidence or invoke package writers. Every run is independent.
$root = Join-Path $repo ('tmp/v13_g1/boundary-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
$script:sequence = 0
function Run([string]$cwd, [string]$exe, [string[]]$arguments) {
    $script:sequence++
    $log = Join-Path $root ('command-{0:D3}.txt' -f $script:sequence)
    Push-Location $cwd
    try {
        $saved = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try { $output = & $exe @arguments 2>&1; $code = $LASTEXITCODE }
        finally { $ErrorActionPreference = $saved }
    } finally { Pop-Location }
    [IO.File]::WriteAllText($log, "CWD=$cwd`nCOMMAND=$exe $($arguments -join ' ')`nEXIT=$code`n$($output -join "`n")")
    if ($code -ne 0) { throw "BOUNDARY BLOCKED: $log (exit $code)" }
    return ($output -join "`n")
}
function Hash([string]$path) { (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash }
Push-Location $repo
try {
    if ((Run $repo git @('rev-parse', 'v12.4.0^{}')).Trim() -ne $commit) { throw 'peeled tag changed' }
    if ((Run $repo git @('rev-parse', 'v12.4.0')).Trim() -ne $tag) { throw 'tag object changed' }
    if ((Run $repo git @('cat-file', '-t', 'v12.4.0')).Trim() -ne 'tag') { throw 'not annotated' }
    $packages = @(Get-ChildItem -LiteralPath $repo -File | Where-Object { $_.Name -match '^dbyte-v.*\.(zip|json)$' } | ForEach-Object {
        [pscustomobject]@{ path = $_.FullName; sha256 = Hash $_.FullName }
    })
    $packages | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $root 'packages-before.json')
    Run $repo cargo @('build', '-q', '-p', 'dbyte_cli', '--bin', 'dbytec') | Out-Null
    Run $repo cargo @('test', '-q', '-p', 'dbyte_native', 'multiboot2::tests') | Out-Null
    $hashes = @()
    foreach ($iteration in @('first', 'second')) {
        $source = Join-Path $root $iteration
        New-Item -ItemType Directory -Path $source | Out-Null
        $archive = Join-Path $root "$iteration.tar"
        Run $repo git @('-c', 'core.autocrlf=false', 'archive', '--format=tar', '-o', $archive, $commit) | Out-Null
        Run $repo tar @('-xf', $archive, '-C', $source) | Out-Null
        Run $source cargo @('build', '-q', '-p', 'dbyte_cli', '--bin', 'dbytec') | Out-Null
        foreach ($producer in @('frozen', 'candidate')) {
            $compiler = if ($producer -eq 'frozen') { Join-Path $source 'target/debug/dbytec.exe' } else { Join-Path $repo 'target/debug/dbytec.exe' }
            foreach ($format in @('obj', 'elf')) {
                $out = Join-Path $root "$iteration-$producer.$format"
                Run $repo $compiler @('build', (Join-Path $source 'examples/native_v12_2_none/bootstrap.dby'), '--target', 'x86_64-dbyte-none', '--emit', $format, '-o', $out) | Out-Null
                $hashes += [pscustomobject]@{ iteration=$iteration; producer=$producer; format=$format; sha256=Hash $out }
            }
        }
    }
    $hashes | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'hashes.json')
    foreach ($producer in @('frozen', 'candidate')) {
        foreach ($format in @('obj', 'elf')) {
            $values = @($hashes | Where-Object { $_.producer -eq $producer -and $_.format -eq $format })
            if ($values[0].sha256 -cne $values[1].sha256) { throw "self reproduction drift: $producer $format" }
        }
    }
    foreach ($package in $packages) {
        if ((Hash $package.path) -cne $package.sha256) { throw "package changed: $($package.path)" }
    }
    'FROZEN SELF-REPRODUCTION + CANDIDATE DETERMINISM: PASS' | Set-Content -LiteralPath (Join-Path $root 'result.txt')
    Write-Output "BOUNDARY EVIDENCE: $root"
    Write-Output 'Behavioral compatibility and G1 QEMU remain PENDING.'
} finally { Pop-Location }
