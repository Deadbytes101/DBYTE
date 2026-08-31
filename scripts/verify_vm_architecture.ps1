param([switch]$SkipTests)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$bytecode = Join-Path $repoRoot "crates\dbyte_bytecode\src\canonical.rs"
$compiler = Join-Path $repoRoot "crates\dbyte_compiler\src\canonical.rs"
$vm = Join-Path $repoRoot "crates\dbyte_vm\src\canonical.rs"

function Assert-True($condition, [string]$message) {
    if (-not $condition) { throw $message }
}

function Assert-Absent([string]$path, [string[]]$patterns) {
    $text = [IO.File]::ReadAllText($path)
    foreach ($pattern in $patterns) {
        if ([regex]::IsMatch($text, $pattern, [Text.RegularExpressions.RegexOptions]::CultureInvariant)) {
            throw "VM_ARCHITECTURE_FORBIDDEN_PATTERN: $([IO.Path]::GetFileName($path)) / $pattern"
        }
    }
}

$baseline = (git -C $repoRoot rev-parse "v12.2.1^{}").Trim()
Assert-True ($baseline -ceq "3843f7b3cf29951f10fe732754434b41ac53fc7f") "VM_ARCHITECTURE_BASELINE_MOVED: $baseline"

Assert-Absent $bytecode @(
    '\bCanonicalBytecodeBlock\b',
    '\bCanonicalBytecodeStatement\b',
    '\bCanonicalExprCode\b',
    'Box\s*<\s*CanonicalBytecodeBody\s*>'
)
Assert-Absent $compiler @(
    'CanonicalBytecodeBlock'
)
Assert-Absent $vm @(
    '\bfn\s+(block|statement|eval|eval_ops|call|call_closure)\s*\(',
    'self\.(block|statement|eval|eval_ops|call|call_closure)\s*\(',
    '\bcall_depth\b',
    '\bEarlyReturn\b',
    'body\s*:\s*CanonicalBytecodeBody'
)

$bytecodeText = [IO.File]::ReadAllText($bytecode)
$vmText = [IO.File]::ReadAllText($vm)
Assert-True ($bytecodeText.Contains('pub instructions: Vec<CanonicalInstruction>')) "VM_ARCHITECTURE_FLAT_STREAM_MISSING"
Assert-True ($bytecodeText.Contains('pub frame: CanonicalFrameLayout')) "VM_ARCHITECTURE_FRAME_LAYOUT_MISSING"
Assert-True ($vmText.Contains('ip: usize')) "VM_ARCHITECTURE_EXPLICIT_IP_MISSING"
Assert-True ($vmText.Contains('while !self.frames.is_empty()')) "VM_ARCHITECTURE_ITERATIVE_DISPATCH_MISSING"
Assert-True ($vmText.Contains('frame_pool: Vec<Vec<LocalSlot>>')) "VM_ARCHITECTURE_REUSABLE_FRAMES_MISSING"
Assert-True ($vmText.Contains('CodeUnit::Function(instance)')) "VM_ARCHITECTURE_DIRECT_FUNCTION_TABLE_MISSING"

if (-not $SkipTests) {
    & cargo test -p dbyte_compiler -p dbyte_vm --lib
    if ($LASTEXITCODE -ne 0) { throw "VM_ARCHITECTURE_TESTS_FAILED" }
}

Write-Host "DBYTE FLAT CANONICAL VM ARCHITECTURE: PASS"
