$ErrorActionPreference = "Stop"
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path

function Assert-Absent([string]$path, [string[]]$patterns) {
    $text = [IO.File]::ReadAllText((Join-Path $repoRoot $path))
    foreach ($pattern in $patterns) {
        if ($text -cmatch $pattern) {
            throw "Concrete HIR authority violation in ${path}: pattern ${pattern}"
        }
    }
}

Assert-Absent "crates\dbyte_cli\src\main.rs" @(
    '\bInterpreter\b',
    '\bTypeChecker\b',
    'Compiler::',
    '\bVm::'
)
Assert-Absent "crates\dbyte_embed\src\lib.rs" @(
    '\bInterpreter\b',
    '\bTypeChecker\b',
    'Compiler::',
    '\bVm::'
)

$native = [IO.File]::ReadAllText((Join-Path $repoRoot "crates\dbyte_native\src\systems_compiler.rs"))
if ($native -notmatch '(?s)#\[cfg\(test\)\]\s*use crate::systems_semantic::') {
    throw "legacy systems semantic imports are not test-gated"
}
if ($native -notmatch 'dbyte_hir::analyze_project' -or $native -notmatch 'dbyte_hir::monomorphize_project') {
    throw "native production compiler is not routed through canonical project HIR"
}

$interp = [IO.File]::ReadAllText((Join-Path $repoRoot "crates\dbyte_interp\src\concrete.rs"))
$compiler = [IO.File]::ReadAllText((Join-Path $repoRoot "crates\dbyte_compiler\src\canonical.rs"))
$vm = [IO.File]::ReadAllText((Join-Path $repoRoot "crates\dbyte_vm\src\canonical.rs"))
foreach ($entry in @(
    @{ Name = "hosted"; Text = $interp; Token = "ConcreteHirProject" },
    @{ Name = "bytecode"; Text = $compiler; Token = "ConcreteHirProject" },
    @{ Name = "vm"; Text = $vm; Token = "CanonicalBytecodeProject" }
)) {
    if (-not $entry.Text.Contains($entry.Token)) {
        throw "$($entry.Name) canonical authority token is missing"
    }
}

$runtimeCapabilityUses = & rg -n --glob '*.dby' '__dbyte_runtime\.' $repoRoot
if ($LASTEXITCODE -gt 1) {
    throw "failed to audit private runtime capability uses"
}
$allowedRuntimeCapabilityFiles = @(
    (Join-Path $repoRoot "runtime\v12.1.0\array.dby"),
    (Join-Path $repoRoot "runtime\v12.1.0\string.dby"),
    (Join-Path $repoRoot "runtime\v12.1.0\hosted_alloc.dby"),
    (Join-Path $repoRoot "runtime\v12.3.0\buffer.dby")
)
foreach ($use in $runtimeCapabilityUses) {
    if ($use -notmatch '^(?<path>.*\.dby):\d+:') {
        throw "malformed private runtime capability audit result: $use"
    }
    $path = $Matches.path
    if ($allowedRuntimeCapabilityFiles -notcontains $path) {
        throw "private runtime capability escaped versioned runtime modules: $use"
    }
}

Write-Host "DBYTE CONCRETE HIR AUTHORITY: PASS"
