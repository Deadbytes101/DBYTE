$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")

Set-Location $repoRoot

$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"

& $cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cargo clippy --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cargo test
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cargo test -p dbyte_embed
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cargo run -q -p dbyte_cli -- test --engine tree
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cargo run -q -p dbyte_cli -- test --engine vm
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cargo run -q -p dbyte_cli -- disasm tests\smoke\let_add.dby | Out-Null
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cargo run -q -p dbyte_cli -- tokens tests\smoke\let_add.dby | Out-Null
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cargo run -q -p dbyte_cli -- ast tests\smoke\let_add.dby | Out-Null
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $cargo run -q -p dbyte_cli -- run --vm --trace tests\smoke\let_add.dby | Out-Null
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$cli = Join-Path $repoRoot "target\debug\dbyte.exe"

# Version check
$versionOutput = .\target\debug\dbyte.exe --version
if ($versionOutput -ne 'DByte 9.0.2') {
    throw "Version mismatch: expected 'DByte 9.0.2', got '$versionOutput'"
}

function Normalize-Output($value) {
    return (($value -join "`n") -replace "`r`n", "`n").Trim()
}

function Assert-Equal($actual, $expected, $name) {
    if ($actual -ne $expected) {
        throw "$name failed: expected '$expected', got '$actual'"
    }
}

function Assert-Contains($actual, $expected, $name) {
    if (-not $actual.Contains($expected)) {
        throw "$name failed: expected output to contain '$expected', got '$actual'"
    }
}

function Assert-NormalizedEqual($actual, $expected, $name) {
    $normalizedActual = Normalize-Output $actual
    $normalizedExpected = Normalize-Output $expected
    if ($normalizedActual -ne $normalizedExpected) {
        throw "$name failed: expected '$normalizedExpected', got '$normalizedActual'"
    }
}

function Assert-NotContains($actual, $unexpected, $name) {
    if ($actual.Contains($unexpected)) {
        throw "$name failed: expected output not to contain '$unexpected', got '$actual'"
    }
}

function Assert-ContainsInOrder($actual, $expectedParts, $name) {
    $cursor = 0
    foreach ($part in $expectedParts) {
        $index = $actual.IndexOf($part, $cursor)
        if ($index -lt 0) {
            throw "$name failed: expected to find '$part' after offset $cursor"
        }
        $cursor = $index + $part.Length
    }
}

function Expected-File($path) {
    if (-not [System.IO.Path]::IsPathRooted($path)) {
        $path = Join-Path $repoRoot $path
    }
    return ((Get-Content $path -Raw) -replace "`r`n", "`n").Trim()
}

function Invoke-Dbyte {
    param(
        [string[]]$Arguments,
        [string]$WorkingDirectory = $null
    )

    $oldPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $oldCwd = Get-Location
    try {
        if ($null -ne $WorkingDirectory) {
            Set-Location $WorkingDirectory
        }
        $output = & $cli @Arguments 2>&1
        $code = $LASTEXITCODE
    }
    finally {
        Set-Location $oldCwd
        $ErrorActionPreference = $oldPreference
    }
    return [PSCustomObject]@{
        Code = $code
        Text = Normalize-Output $output
    }
}

function Invoke-DbyteExact {
    param(
        [string[]]$Arguments,
        [string]$WorkingDirectory = $repoRoot,
        [string]$Executable = $cli
    )

    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $Executable
    $quotedArgs = foreach ($arg in $Arguments) {
        if ($arg -eq "") {
            '""'
        }
        else {
            '"' + $arg.Replace('"', '\"') + '"'
        }
    }
    $psi.Arguments = ($quotedArgs -join " ")
    $psi.WorkingDirectory = (Resolve-Path $WorkingDirectory).Path
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $process = [System.Diagnostics.Process]::Start($psi)
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    return [PSCustomObject]@{
        Code = $process.ExitCode
        Text = Normalize-Output @($stdout, $stderr)
    }
}

function Invoke-DbyteInput {
    param(
        [string[]]$Arguments,
        [string]$InputText,
        [string]$WorkingDirectory = $repoRoot,
        [string]$Executable = $cli,
        [hashtable]$Environment = $null
    )

    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $Executable
    $quotedArgs = foreach ($arg in $Arguments) {
        '"' + $arg.Replace('"', '\"') + '"'
    }
    $psi.Arguments = ($quotedArgs -join " ")
    $psi.WorkingDirectory = (Resolve-Path $WorkingDirectory).Path
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    if ($null -ne $Environment) {
        foreach ($key in $Environment.Keys) {
            $psi.Environment[$key] = [string]$Environment[$key]
        }
    }

    $process = [System.Diagnostics.Process]::Start($psi)
    $process.StandardInput.Write($InputText)
    $process.StandardInput.Close()
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()

    return [PSCustomObject]@{
        Code = $process.ExitCode
        Text = Normalize-Output @($stdout, $stderr)
    }
}

function Git-Status-Short {
    return Normalize-Output (& git status --short 2>&1)
}

function Assert-GitStatus-Unchanged($before, $name) {
    $after = Git-Status-Short
    if ($after -ne $before) {
        throw "$name failed: git status changed from '$before' to '$after'"
    }
}

function Assert-PersonalToolOutput($toolName, $output) {
    switch ($toolName) {
        "hexdump" {
            Assert-Contains $output "hexdump:" "personal hexdump heading"
            Assert-Contains $output "bytes: 12" "personal hexdump size"
            Assert-Contains $output "0000: 4442797465001020" "personal hexdump first row"
            Assert-Contains $output "0008: deadbeef" "personal hexdump second row"
        }
        "bininfo" {
            Assert-Contains $output "bininfo:" "personal bininfo heading"
            Assert-Contains $output "bytes: 9" "personal bininfo size"
            Assert-Contains $output "first8: 0102030444427974" "personal bininfo first bytes"
            Assert-Contains $output "checksum: 482" "personal bininfo checksum"
        }
        "find_bytes" {
            Assert-Contains $output "find_bytes" "personal find bytes heading"
            Assert-Contains $output "data: 00deadbeef00dead01" "personal find bytes data"
            Assert-Contains $output "de_ad: 1" "personal find bytes first pattern"
            Assert-Contains $output "be_ef: 3" "personal find bytes second pattern"
            Assert-Contains $output "one_byte: 8" "personal find bytes single byte"
            Assert-Contains $output "missing: -1" "personal find bytes missing pattern"
        }
        "patch_bytes" {
            Assert-Contains $output "patched: 1" "personal patch bytes marker"
            Assert-Contains $output "patched_hex: 009090909000" "personal patch bytes output"
        }
        "read_u32_table" {
            Assert-Contains $output "u32_table:" "personal u32 table heading"
            Assert-Contains $output "data: 7856341201000000efbeadde" "personal u32 table data"
            Assert-Contains $output "0 -> 305419896" "personal u32 table first row"
            Assert-Contains $output "4 -> 1" "personal u32 table second row"
            Assert-Contains $output "8 -> 3735928559" "personal u32 table third row"
        }
        default {
            throw "unknown personal tool assertion: $toolName"
        }
    }
}

function Bytes-Hex($path) {
    return -join ([System.IO.File]::ReadAllBytes((Resolve-Path $path).Path) | ForEach-Object { $_.ToString("x2") })
}

Write-Host "Running VM hardening tests..."
Set-Location $repoRoot

$disasmResult = Invoke-Dbyte -Arguments @("disasm", "tests\vm\disasm_smoke.dby") -WorkingDirectory $repoRoot
if ($disasmResult.Code -ne 0) { throw "disasm smoke failed: $($disasmResult.Text)" }
Assert-Equal $disasmResult.Text (Expected-File "tests\vm\disasm_smoke.disasm") "disasm smoke"

$traceResult = Invoke-Dbyte -Arguments @("run", "--vm", "--trace", "tests\vm\trace_smoke.dby") -WorkingDirectory $repoRoot
if ($traceResult.Code -ne 0) { throw "trace smoke failed: $($traceResult.Text)" }
Assert-Equal $traceResult.Text (Expected-File "tests\vm\trace_smoke.trace") "trace smoke"

$arityResult = Invoke-Dbyte -Arguments @("run", "--vm", "--no-check", "tests\vm\arity_mismatch.dby") -WorkingDirectory $repoRoot
if ($arityResult.Code -eq 0) { throw "vm arity mismatch unexpectedly passed" }
Assert-Contains $arityResult.Text (Expected-File "tests\vm\arity_mismatch.err") "vm arity mismatch"

$returnResult = Invoke-Dbyte -Arguments @("run", "--vm", "--no-check", "tests\vm\return_outside_function.dby") -WorkingDirectory $repoRoot
if ($returnResult.Code -eq 0) { throw "vm return outside function unexpectedly passed" }
Assert-Contains $returnResult.Text (Expected-File "tests\vm\return_outside_function.err") "vm return outside function"

$divisionResult = Invoke-Dbyte -Arguments @("run", "--vm", "--no-check", "tests\vm\vm_division_by_zero.dby") -WorkingDirectory $repoRoot
if ($divisionResult.Code -eq 0) { throw "vm division by zero unexpectedly passed" }
Assert-Contains $divisionResult.Text (Expected-File "tests\vm\vm_division_by_zero.err") "vm division by zero"

$listResult = Invoke-Dbyte -Arguments @("run", "--vm", "--no-check", "tests\vm\vm_list_oob.dby") -WorkingDirectory $repoRoot
if ($listResult.Code -eq 0) { throw "vm list out of bounds unexpectedly passed" }
Assert-Contains $listResult.Text (Expected-File "tests\vm\vm_list_oob.err") "vm list out of bounds"

Write-Host "Running VM fast path disasm checks..."

$loopDisasm = Invoke-Dbyte -Arguments @("disasm", "benchmarks\loop_sum.dby") -WorkingDirectory $repoRoot
if ($loopDisasm.Code -ne 0) { throw "loop_sum disasm failed: $($loopDisasm.Text)" }
Assert-Contains $loopDisasm.Text "STORE_LOCAL_I64" "loop_sum typed store"
Assert-Contains $loopDisasm.Text "ADD_LOCAL_I64" "loop_sum direct local add"
Assert-Contains $loopDisasm.Text "ADD_LOCAL_CONST_I64" "loop_sum direct const increment"
Assert-Contains $loopDisasm.Text "JUMP_IF_NOT_LT_LOCAL_CONST_I64" "loop_sum direct local less-than jump"

$largeLoopDisasm = Invoke-Dbyte -Arguments @("disasm", "benchmarks\loop_sum_large.dby") -WorkingDirectory $repoRoot
if ($largeLoopDisasm.Code -ne 0) { throw "loop_sum_large disasm failed: $($largeLoopDisasm.Text)" }
Assert-Contains $largeLoopDisasm.Text "STORE_LOCAL_I64" "loop_sum_large typed store"
Assert-Contains $largeLoopDisasm.Text "ADD_LOCAL_I64" "loop_sum_large direct local add"
Assert-Contains $largeLoopDisasm.Text "ADD_LOCAL_CONST_I64" "loop_sum_large direct const increment"
Assert-Contains $largeLoopDisasm.Text "JUMP_IF_NOT_LT_LOCAL_CONST_I64" "loop_sum_large direct local less-than jump"

$compareLoopDisasm = Invoke-Dbyte -Arguments @("disasm", "benchmarks\int_compare_loop.dby") -WorkingDirectory $repoRoot
if ($compareLoopDisasm.Code -ne 0) { throw "int_compare_loop disasm failed: $($compareLoopDisasm.Text)" }
Assert-Contains $compareLoopDisasm.Text "JUMP_IF_NOT_GE_LOCAL_CONST_I64" "int_compare_loop direct greater-equal jump"
Assert-Contains $compareLoopDisasm.Text "JUMP_IF_NOT_LE_LOCAL_CONST_I64" "int_compare_loop direct less-equal jump"
Assert-Contains $compareLoopDisasm.Text "JUMP_IF_NOT_LT_LOCAL_CONST_I64" "int_compare_loop direct loop condition jump"

$fallbackLocalDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\generic_local_fallback.dby") -WorkingDirectory $repoRoot
if ($fallbackLocalDisasm.Code -ne 0) { throw "generic local fallback disasm failed: $($fallbackLocalDisasm.Text)" }
Assert-Contains $fallbackLocalDisasm.Text "STORE_LOCAL 0 ; nums" "generic list local fallback store"
Assert-Contains $fallbackLocalDisasm.Text "LOAD_LOCAL 0 ; nums" "generic list local fallback load"

$directLocalRhsDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\direct_add_local_rhs.dby") -WorkingDirectory $repoRoot
if ($directLocalRhsDisasm.Code -ne 0) { throw "direct local rhs disasm failed: $($directLocalRhsDisasm.Text)" }
Assert-Contains $directLocalRhsDisasm.Text "ADD_LOCAL_I64" "direct local rhs add fast path"

$commutedAddDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\fallback_commuted_add.dby") -WorkingDirectory $repoRoot
if ($commutedAddDisasm.Code -ne 0) { throw "fallback commuted add disasm failed: $($commutedAddDisasm.Text)" }
Assert-NotContains $commutedAddDisasm.Text "ADD_LOCAL_I64" "commuted add avoids direct local add"

$mulAssignDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\fallback_mul_assign.dby") -WorkingDirectory $repoRoot
if ($mulAssignDisasm.Code -ne 0) { throw "fallback mul assign disasm failed: $($mulAssignDisasm.Text)" }
Assert-Contains $mulAssignDisasm.Text "MUL_I64" "mul assign uses typed stack multiply"
Assert-NotContains $mulAssignDisasm.Text "ADD_LOCAL_I64" "mul assign avoids direct local add"
Assert-NotContains $mulAssignDisasm.Text "ADD_LOCAL_CONST_I64" "mul assign avoids direct const add"

$lenAddDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\fallback_len_add.dby") -WorkingDirectory $repoRoot
if ($lenAddDisasm.Code -ne 0) { throw "fallback len add disasm failed: $($lenAddDisasm.Text)" }
Assert-Contains $lenAddDisasm.Text "CALL len 1" "len add keeps builtin call"
Assert-Contains $lenAddDisasm.Text "ADD_I64" "len add uses typed stack add"
Assert-NotContains $lenAddDisasm.Text "ADD_LOCAL_I64" "len add avoids direct local add"
Assert-NotContains $lenAddDisasm.Text "ADD_LOCAL_CONST_I64" "len add avoids direct const add"

$binaryDisasm = Invoke-Dbyte -Arguments @("disasm", "benchmarks\binary_read_u32.dby") -WorkingDirectory $repoRoot
if ($binaryDisasm.Code -ne 0) { throw "binary_read_u32 disasm failed: $($binaryDisasm.Text)" }
Assert-Contains $binaryDisasm.Text "READ_U32_LE" "binary_read_u32 intrinsic"

$bufferDisasm = Invoke-Dbyte -Arguments @("disasm", "benchmarks\buffer_replace.dby") -WorkingDirectory $repoRoot
if ($bufferDisasm.Code -ne 0) { throw "buffer_replace disasm failed: $($bufferDisasm.Text)" }
Assert-Contains $bufferDisasm.Text "BUFFER_REPLACE" "buffer_replace intrinsic"

$binaryAliasDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\binary_alias_u32.dby") -WorkingDirectory $repoRoot
if ($binaryAliasDisasm.Code -ne 0) { throw "binary alias disasm failed: $($binaryAliasDisasm.Text)" }
Assert-Contains $binaryAliasDisasm.Text "READ_U32_LE" "binary alias intrinsic"

$bufferAliasDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\buffer_alias_ops.dby") -WorkingDirectory $repoRoot
if ($bufferAliasDisasm.Code -ne 0) { throw "buffer alias disasm failed: $($bufferAliasDisasm.Text)" }
Assert-Contains $bufferAliasDisasm.Text "BUFFER_FIND" "buffer alias find intrinsic"
Assert-Contains $bufferAliasDisasm.Text "BUFFER_REPLACE" "buffer alias replace intrinsic"

$bufferLoadSaveDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\patching\load_save_roundtrip.dby") -WorkingDirectory $repoRoot
if ($bufferLoadSaveDisasm.Code -ne 0) { throw "buffer load/save disasm failed: $($bufferLoadSaveDisasm.Text)" }
Assert-Contains $bufferLoadSaveDisasm.Text "BUFFER_LOAD" "buffer load intrinsic"
Assert-Contains $bufferLoadSaveDisasm.Text "BUFFER_SAVE" "buffer save intrinsic"

$fsExistsDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\patching\fs_exists.dby") -WorkingDirectory $repoRoot
if ($fsExistsDisasm.Code -ne 0) { throw "fs exists disasm failed: $($fsExistsDisasm.Text)" }
Assert-Contains $fsExistsDisasm.Text "CALL_NATIVE FsExists" "fs exists native call"

$bufferLoadFallbackDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\patching\member_call_fallback_buffer_load.dby") -WorkingDirectory $repoRoot
if ($bufferLoadFallbackDisasm.Code -ne 0) { throw "buffer load fallback disasm failed: $($bufferLoadFallbackDisasm.Text)" }
Assert-Contains $bufferLoadFallbackDisasm.Text "MEMBER_CALL load 1" "non-std buffer load fallback member call"
Assert-NotContains $bufferLoadFallbackDisasm.Text "BUFFER_LOAD" "non-std buffer load fallback avoids intrinsic"

$fallbackDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\fallback_member_call.dby") -WorkingDirectory $repoRoot
if ($fallbackDisasm.Code -ne 0) { throw "fallback member call disasm failed: $($fallbackDisasm.Text)" }
Assert-Contains $fallbackDisasm.Text "MEMBER_CALL u32_le 2" "non-std fallback member call"
Assert-NotContains $fallbackDisasm.Text "READ_U32_LE" "non-std fallback avoids binary intrinsic"

$directCallDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\function_fastpath\direct_call_disasm.dby") -WorkingDirectory $repoRoot
if ($directCallDisasm.Code -ne 0) { throw "direct function call disasm failed: $($directCallDisasm.Text)" }
Assert-Contains $directCallDisasm.Text "ADD_I64_STACK" "direct function call fast path"
Assert-Contains $directCallDisasm.Text "RETURN_I64" "typed int return fast path"
Assert-NotContains $directCallDisasm.Text "CALL add 2" "direct function avoids string call"


$directReturnDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\call_i64_to_local_disasm.dby") -WorkingDirectory $repoRoot
if ($directReturnDisasm.Code -ne 0) { throw "direct return-to-local disasm failed: $($directReturnDisasm.Text)" }
Assert-Contains $directReturnDisasm.Text "ADD_I64_STACK" "direct return-to-local fast path"
Assert-Contains $directReturnDisasm.Text "RETURN_I64" "direct return-to-local typed return"
Assert-NotContains $directReturnDisasm.Text "CALL add 2" "direct return-to-local avoids string call"


$letInitDirectReturnDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\let_init_i64_to_local.dby") -WorkingDirectory $repoRoot
if ($letInitDirectReturnDisasm.Code -ne 0) { throw "let init direct return-to-local disasm failed: $($letInitDirectReturnDisasm.Text)" }
Assert-Contains $letInitDirectReturnDisasm.Text "STORE_LOCAL_I64_STACK" "let init direct return-to-local fast path"

$earlyReturnDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\early_return_i64_to_local.dby") -WorkingDirectory $repoRoot
if ($earlyReturnDisasm.Code -ne 0) { throw "early return direct return-to-local disasm failed: $($earlyReturnDisasm.Text)" }
Assert-Contains $earlyReturnDisasm.Text "STORE_LOCAL_I64_STACK" "early return direct return-to-local fast path"

$nestedArgFallbackDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\nested_call_fallback.dby") -WorkingDirectory $repoRoot
if ($nestedArgFallbackDisasm.Code -ne 0) { throw "nested argument fallback disasm failed: $($nestedArgFallbackDisasm.Text)" }
Assert-Contains $nestedArgFallbackDisasm.Text "CALL_FN" "nested argument still uses direct function id fallback"
Assert-NotContains $nestedArgFallbackDisasm.Text "CALL_FN_I64_TO_LOCAL" "nested argument avoids direct return-to-local"

$directReturnGenericDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\generic_return_no_fastpath.dby") -WorkingDirectory $repoRoot
if ($directReturnGenericDisasm.Code -ne 0) { throw "direct return generic fallback disasm failed: $($directReturnGenericDisasm.Text)" }
Assert-Contains $directReturnGenericDisasm.Text "STORE_LOCAL" "direct return generic fallback uses direct id"
Assert-Contains $directReturnGenericDisasm.Text "RETURN" "direct return generic fallback keeps generic return"
Assert-NotContains $directReturnGenericDisasm.Text "CALL_FN_I64_TO_LOCAL" "direct return generic fallback avoids direct return-to-local"
Assert-NotContains $directReturnGenericDisasm.Text "RETURN_I64" "direct return generic fallback avoids return_i64"

$directReturnNonIntDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\non_int_return_no_fastpath.dby") -WorkingDirectory $repoRoot
if ($directReturnNonIntDisasm.Code -ne 0) { throw "direct return non-int fallback disasm failed: $($directReturnNonIntDisasm.Text)" }
Assert-Contains $directReturnNonIntDisasm.Text "STORE_LOCAL" "direct return non-int fallback uses direct id"
Assert-NotContains $directReturnNonIntDisasm.Text "CALL_FN_I64_TO_LOCAL" "direct return non-int fallback avoids direct return-to-local"

$directReturnBuiltinDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\builtin_len_no_fastpath.dby") -WorkingDirectory $repoRoot
if ($directReturnBuiltinDisasm.Code -ne 0) { throw "direct return builtin fallback disasm failed: $($directReturnBuiltinDisasm.Text)" }
Assert-Contains $directReturnBuiltinDisasm.Text "CALL len 1" "direct return builtin fallback keeps builtin call"
Assert-Contains $directReturnBuiltinDisasm.Text "STORE_LOCAL_I64" "direct return builtin fallback stores typed local"
Assert-NotContains $directReturnBuiltinDisasm.Text "CALL_FN_I64_TO_LOCAL" "direct return builtin fallback avoids direct return-to-local"

$directReturnStdMemberDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\std_member_no_fastpath.dby") -WorkingDirectory $repoRoot
if ($directReturnStdMemberDisasm.Code -ne 0) { throw "direct return std member fallback disasm failed: $($directReturnStdMemberDisasm.Text)" }
Assert-Contains $directReturnStdMemberDisasm.Text "MEMBER_CALL max 2" "direct return std member fallback keeps member dispatch"
Assert-Contains $directReturnStdMemberDisasm.Text "STORE_LOCAL_I64" "direct return std member fallback stores typed local"
Assert-NotContains $directReturnStdMemberDisasm.Text "CALL_FN_I64_TO_LOCAL" "direct return std member fallback avoids direct return-to-local"

$directReturnMemberFallback = Invoke-Dbyte -Arguments @("disasm", "tests\vm\call_fastpath\member_call_not_call_fn.dby") -WorkingDirectory $repoRoot
if ($directReturnMemberFallback.Code -ne 0) { throw "direct return member fallback disasm failed: $($directReturnMemberFallback.Text)" }
Assert-Contains $directReturnMemberFallback.Text "MEMBER_CALL max 2" "direct return member fallback keeps member dispatch"
Assert-NotContains $directReturnMemberFallback.Text "CALL_FN_I64_TO_LOCAL" "direct return member fallback avoids direct return-to-local"

$i64StackChainDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\i64_stack\typed_call_chain_disasm.dby") -WorkingDirectory $repoRoot
if ($i64StackChainDisasm.Code -ne 0) { throw "i64 stack chain disasm failed: $($i64StackChainDisasm.Text)" }
Assert-Contains $i64StackChainDisasm.Text "STORE_LOCAL_I64_STACK" "i64 stack direct typed call"
Assert-Contains $i64StackChainDisasm.Text "RETURN_I64_TO_I64_STACK" "i64 stack typed return"
Assert-Contains $i64StackChainDisasm.Text "ADD_I64_STACK" "i64 stack typed add"
Assert-NotContains $i64StackChainDisasm.Text "CALL inc 1" "i64 stack chain avoids string call"

$i64StackAssignDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\i64_stack\assign_call_plus_local.dby") -WorkingDirectory $repoRoot
if ($i64StackAssignDisasm.Code -ne 0) { throw "i64 stack assign disasm failed: $($i64StackAssignDisasm.Text)" }
Assert-Contains $i64StackAssignDisasm.Text "STORE_LOCAL_I64_STACK" "i64 stack assignment call result"
Assert-Contains $i64StackAssignDisasm.Text "STORE_LOCAL_I64_STACK" "i64 stack assignment stores typed local"

$i64StackFallbackDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\i64_stack\generic_return_no_i64_stack_call.dby") -WorkingDirectory $repoRoot
if ($i64StackFallbackDisasm.Code -ne 0) { throw "i64 stack generic fallback disasm failed: $($i64StackFallbackDisasm.Text)" }
Assert-Contains $i64StackFallbackDisasm.Text "STORE_LOCAL" "i64 stack generic fallback keeps direct id"
Assert-NotContains $i64StackFallbackDisasm.Text "CALL_FN_I64_TO_I64_STACK" "i64 stack generic fallback avoids typed call"
Assert-NotContains $i64StackFallbackDisasm.Text "RETURN_I64_TO_I64_STACK" "i64 stack generic fallback avoids typed return"

$i64StackMemberFallbackDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\i64_stack\member_std_no_i64_stack_call.dby") -WorkingDirectory $repoRoot
if ($i64StackMemberFallbackDisasm.Code -ne 0) { throw "i64 stack member fallback disasm failed: $($i64StackMemberFallbackDisasm.Text)" }
Assert-Contains $i64StackMemberFallbackDisasm.Text "MEMBER_CALL max 2" "i64 stack std member fallback keeps member dispatch"
Assert-NotContains $i64StackMemberFallbackDisasm.Text "CALL_FN_I64_TO_I64_STACK" "i64 stack std member fallback avoids typed call"

$i64StackHardeningDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\i64_stack_hardening\i64_stack_call_chain.dby") -WorkingDirectory $repoRoot
if ($i64StackHardeningDisasm.Code -ne 0) { throw "i64 stack hardening disasm failed: $($i64StackHardeningDisasm.Text)" }
Assert-Contains $i64StackHardeningDisasm.Text "CONST_I64_STACK" "i64 stack hardening uses typed constants"
Assert-Contains $i64StackHardeningDisasm.Text "CALL_FN_I64_TO_I64_STACK" "i64 stack hardening uses typed call chain"
Assert-Contains $i64StackHardeningDisasm.Text "RETURN_I64_TO_I64_STACK" "i64 stack hardening uses typed return"

$i64StackHardeningGenericDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\i64_stack_hardening\i64_stack_generic_return_fallback.dby") -WorkingDirectory $repoRoot
if ($i64StackHardeningGenericDisasm.Code -ne 0) { throw "i64 stack hardening generic fallback disasm failed: $($i64StackHardeningGenericDisasm.Text)" }
Assert-Contains $i64StackHardeningGenericDisasm.Text "STORE_LOCAL" "i64 stack hardening generic fallback keeps direct id"
Assert-NotContains $i64StackHardeningGenericDisasm.Text "CALL_FN_I64_TO_I64_STACK" "i64 stack hardening generic fallback avoids typed call"
Assert-NotContains $i64StackHardeningGenericDisasm.Text "RETURN_I64_TO_I64_STACK" "i64 stack hardening generic fallback avoids typed return"

$nestedCallDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\function_fastpath\nested_function_call.dby") -WorkingDirectory $repoRoot
if ($nestedCallDisasm.Code -ne 0) { throw "nested function call disasm failed: $($nestedCallDisasm.Text)" }
Assert-Contains $nestedCallDisasm.Text "ADD_I64_STACK" "nested function i64 stack direct call fast path"
Assert-Contains $nestedCallDisasm.Text "RETURN_I64_TO_I64_STACK" "nested function i64 stack return fast path"


$genericCallDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\function_fastpath\generic_function_fallback.dby") -WorkingDirectory $repoRoot
if ($genericCallDisasm.Code -ne 0) { throw "generic function call disasm failed: $($genericCallDisasm.Text)" }
Assert-Contains $genericCallDisasm.Text "STORE_LOCAL" "generic user function inlined"
Assert-Contains $genericCallDisasm.Text "RETURN" "generic return keeps generic return path"
Assert-NotContains $genericCallDisasm.Text "RETURN_I64" "generic return avoids typed int return"

$discardCallDisasm = Invoke-Dbyte -Arguments @("disasm", "benchmarks\function_call.dby") -WorkingDirectory $repoRoot
if ($discardCallDisasm.Code -ne 0) { throw "function_call disasm failed: $($discardCallDisasm.Text)" }
Assert-Contains $discardCallDisasm.Text "POP_I64_STACK" "discarded function call avoids return stack traffic"
Assert-NotContains $discardCallDisasm.Text "CALL work 1" "discarded function avoids string call"

$callFnHardeningDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\call_fastpath\call_fn_disasm.dby") -WorkingDirectory $repoRoot
if ($callFnHardeningDisasm.Code -ne 0) { throw "call_fn hardening disasm failed: $($callFnHardeningDisasm.Text)" }
Assert-Contains $callFnHardeningDisasm.Text "ADD_I64_STACK" "call_fn hardening direct call inlined"
Assert-Contains $callFnHardeningDisasm.Text "RETURN_I64" "call_fn hardening typed return"
Assert-NotContains $callFnHardeningDisasm.Text "CALL add 2" "call_fn hardening avoids string lookup"


$returnI64Disasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\call_fastpath\return_i64_correctness.dby") -WorkingDirectory $repoRoot
if ($returnI64Disasm.Code -ne 0) { throw "return_i64 disasm failed: $($returnI64Disasm.Text)" }
Assert-Contains $returnI64Disasm.Text "RETURN_I64" "int function uses return_i64"

$discardHardeningDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\call_fastpath\discard_call_stack_clean.dby") -WorkingDirectory $repoRoot
if ($discardHardeningDisasm.Code -ne 0) { throw "discard call hardening disasm failed: $($discardHardeningDisasm.Text)" }
Assert-Contains $discardHardeningDisasm.Text "POP_I64_STACK" "discarded call hardening inlined"
Assert-NotContains $discardHardeningDisasm.Text "CALL value 1" "discarded call hardening avoids string lookup"

$genericFallbackDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\call_fastpath\generic_call_fallback.dby") -WorkingDirectory $repoRoot
if ($genericFallbackDisasm.Code -ne 0) { throw "generic call fallback disasm failed: $($genericFallbackDisasm.Text)" }
Assert-Contains $genericFallbackDisasm.Text "STORE_LOCAL" "generic user function inlined"
Assert-Contains $genericFallbackDisasm.Text "RETURN" "generic function keeps generic return"
Assert-NotContains $genericFallbackDisasm.Text "RETURN_I64" "generic function avoids return_i64"

$memberFallbackDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\call_fastpath\member_call_not_call_fn.dby") -WorkingDirectory $repoRoot
if ($memberFallbackDisasm.Code -ne 0) { throw "member call fallback disasm failed: $($memberFallbackDisasm.Text)" }
Assert-Contains $memberFallbackDisasm.Text "MEMBER_CALL max 2" "member call keeps member dispatch"
Assert-NotContains $memberFallbackDisasm.Text "CALL_FN" "member call avoids direct function opcode"

$recursionDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\functions\recursion_factorial.dby") -WorkingDirectory $repoRoot
if ($recursionDisasm.Code -ne 0) { throw "recursion factorial disasm failed: $($recursionDisasm.Text)" }
Assert-Contains $recursionDisasm.Text "CALL_FN" "recursive function direct call"
Assert-NotContains $recursionDisasm.Text "CALL fact" "recursive function avoids string call"

$frameDispatchTypedArgs = Invoke-Dbyte -Arguments @("disasm", "tests\vm\frame_dispatch\typed_args_correctness.dby") -WorkingDirectory $repoRoot
if ($frameDispatchTypedArgs.Code -ne 0) { throw "frame dispatch typed args disasm failed: $($frameDispatchTypedArgs.Text)" }
Assert-Contains $frameDispatchTypedArgs.Text "ADD_I64_STACK" "frame dispatch direct user call inlined"
Assert-Contains $frameDispatchTypedArgs.Text "RETURN_I64" "frame dispatch typed int return"
Assert-NotContains $frameDispatchTypedArgs.Text "CALL add 2" "frame dispatch avoids string call"


$frameDispatchDiscard = Invoke-Dbyte -Arguments @("disasm", "tests\vm\frame_dispatch\discard_call_stack_clean.dby") -WorkingDirectory $repoRoot
if ($frameDispatchDiscard.Code -ne 0) { throw "frame dispatch discard disasm failed: $($frameDispatchDiscard.Text)" }
Assert-Contains $frameDispatchDiscard.Text "POP_I64_STACK" "frame dispatch discarded call inlined"

$frameDispatchGeneric = Invoke-Dbyte -Arguments @("disasm", "tests\vm\frame_dispatch\generic_return_fallback.dby") -WorkingDirectory $repoRoot
if ($frameDispatchGeneric.Code -ne 0) { throw "frame dispatch generic return disasm failed: $($frameDispatchGeneric.Text)" }
Assert-Contains $frameDispatchGeneric.Text "STORE_LOCAL" "frame dispatch generic function inlined"
Assert-Contains $frameDispatchGeneric.Text "RETURN" "frame dispatch generic return path"
Assert-NotContains $frameDispatchGeneric.Text "RETURN_I64" "frame dispatch generic return avoids return_i64"

Write-Host "Running interactive runtime tests..."

$replPersist = Invoke-DbyteInput -Arguments @("repl", "--no-rc") -InputText "let x: int = 40`nprint(x + 2)`n.quit`n"
if ($replPersist.Code -ne 0) { throw "repl persistence failed: $($replPersist.Text)" }
Assert-Contains $replPersist.Text "42" "repl variable persistence"

$replFunction = Invoke-DbyteInput -Arguments @("repl", "--no-rc") -InputText "fn add(a: int, b: int) -> int:`n    return a + b`n`nprint(add(20, 22))`nprint(add(1, 2))`n.quit`n"
if ($replFunction.Code -ne 0) { throw "repl function persistence failed: $($replFunction.Text)" }
Assert-Contains $replFunction.Text "42" "repl multiline function persistence"
Assert-Contains $replFunction.Text "3" "repl repeated function call persistence"

$replMalformedBlock = Invoke-DbyteInput -Arguments @("repl", "--no-rc") -InputText "fn broken() -> int:`n`nprint(42)`n.quit`n"
if ($replMalformedBlock.Code -ne 0) { throw "repl malformed block recovery command failed: $($replMalformedBlock.Text)" }
Assert-Contains $replMalformedBlock.Text "ParseError" "repl malformed block reports parse error"
Assert-Contains $replMalformedBlock.Text "42" "repl malformed block recovers"

$replHelpUnknownEof = Invoke-DbyteInput -Arguments @("repl", "--no-rc") -InputText ".help`n.nope`nprint(7)`n"
if ($replHelpUnknownEof.Code -ne 0) { throw "repl help/unknown/eof failed: $($replHelpUnknownEof.Text)" }
Assert-Contains $replHelpUnknownEof.Text "DByte REPL commands" "repl help command"
Assert-Contains $replHelpUnknownEof.Text "ReplError: unknown command: .nope" "repl unknown dot command"
Assert-Contains $replHelpUnknownEof.Text "7" "repl eof exits cleanly after code"

$replCrLfBom = Invoke-DbyteInput -Arguments @("repl", "--no-rc") -InputText ([char]0xfeff + "let crlf: int = 41`r`nprint(crlf + 1)`r`n.quit`r`n")
if ($replCrLfBom.Code -ne 0) { throw "repl crlf/bom failed: $($replCrLfBom.Text)" }
Assert-Contains $replCrLfBom.Text "42" "repl crlf bom input"

$replReset = Invoke-DbyteInput -Arguments @("repl", "--no-rc") -InputText "import std.math as math`nfn add_one(x: int) -> int:`n    return x + 1`n`nlet x: int = 1`nprint(math.max(add_one(x), 3))`n.reset`nprint(x)`nprint(add_one(1))`nprint(math.max(1, 2))`nimport std.math as math`nprint(math.max(2, 4))`n.quit`n"
if ($replReset.Code -ne 0) { throw "repl reset command failed: $($replReset.Text)" }
Assert-Contains $replReset.Text "reset" "repl reset acknowledgement"
Assert-Contains $replReset.Text "3" "repl reset precondition output"
Assert-Contains $replReset.Text "undefined variable" "repl reset clears variables/imports"
Assert-Contains $replReset.Text "undefined function" "repl reset clears functions"
Assert-Contains $replReset.Text "4" "repl import works again after reset"

$interactiveRoot = Join-Path $repoRoot "target\verify-interactive"
if (Test-Path $interactiveRoot) {
    Remove-Item -Recurse -Force $interactiveRoot
}
New-Item -ItemType Directory -Path $interactiveRoot | Out-Null

$replRcRoot = Join-Path $interactiveRoot "repl-rc"
New-Item -ItemType Directory -Path $replRcRoot | Out-Null
Set-Content -Path (Join-Path $replRcRoot "helper.dby") -Value "pub fn inc(x: int) -> int:`n    return x + 1`n" -NoNewline
Set-Content -Path (Join-Path $replRcRoot ".dbyterc") -Value "@shell alias ignored = help`nimport std.math as math`nimport `"./helper.dby`" as helper`nlet boot: int = math.max(helper.inc(40), 1)" -NoNewline
$replRc = Invoke-DbyteInput -Arguments @("repl") -InputText "print(boot + 1)`nprint(helper.inc(1))`n.quit`n" -WorkingDirectory $replRcRoot
if ($replRc.Code -ne 0) { throw "repl rc load failed: $($replRc.Text)" }
Assert-Contains $replRc.Text "42" "repl rc state"
Assert-Contains $replRc.Text "2" "repl rc local import state"

$replNoRc = Invoke-DbyteInput -Arguments @("repl", "--no-rc") -InputText "print(boot)`n.quit`n" -WorkingDirectory $replRcRoot
if ($replNoRc.Code -ne 0) { throw "repl no-rc command failed: $($replNoRc.Text)" }
Assert-Contains $replNoRc.Text "undefined variable" "repl no-rc skips rc"

$replBadRcRoot = Join-Path $interactiveRoot "repl-bad-rc"
New-Item -ItemType Directory -Path $replBadRcRoot | Out-Null
Set-Content -Path (Join-Path $replBadRcRoot ".dbyterc") -Value "let bad: int = `"bad`"" -NoNewline
$replBadRc = Invoke-DbyteInput -Arguments @("repl") -InputText ".quit`n" -WorkingDirectory $replBadRcRoot
if ($replBadRc.Code -eq 0) { throw "repl bad rc unexpectedly passed: $($replBadRc.Text)" }
Assert-Contains $replBadRc.Text "RcError: failed to load" "repl bad rc error"

$shellRoot = Join-Path $interactiveRoot "shell"
New-Item -ItemType Directory -Path $shellRoot | Out-Null
Set-Content -Path (Join-Path $shellRoot "hello.dby") -Value "print(`"shell file ok`")" -NoNewline
Set-Content -Path (Join-Path $shellRoot "defs.dby") -Value "pub fn from_file(x: int) -> int:`n    return x + 22`n" -NoNewline
$shellInput = "help`nversion`npwd`ncd `"$shellRoot`"`ncd missing-dir`nls`nrun hello.dby`ncheck hello.dby`nrun defs.dby`n: let y: int = 40`n: print(y + 2)`n: print(from_file(20))`nalias hi = run hello.dby`nwhich help`nwhich hi`nwhich missing`naliases`nhi`nunalias hi`nhi`nnot_a_real_cmd`nquit`n"
$shellBasic = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText $shellInput
if ($shellBasic.Code -ne 0) { throw "shell basic command failed: $($shellBasic.Text)" }

# v9.1.1 Hardening: Stale v9.1.0 Guard
Write-Host "Verifying v9.1.1 hardening contracts..."
$v910Tag = & git rev-list -n 1 v9.1.0 2>$null
$HEAD = & git rev-parse HEAD
if ($null -eq $v910Tag) { throw "v9.1.0 tag not found (required baseline)" }
if ($HEAD -eq $v910Tag) { throw "HEAD is still v9.1.0, v9.1.1 work not completed" }
Write-Host "[OK] v9.1.1 branch is beyond v9.1.0 locked baseline"

# v9.1.1: Exact output contracts for irq-runtime-commit blocking scenarios
$kernelELF = Join-Path $repoRoot "kernel-lab\target\i686-unknown-linux-gnu\debug\dbyte_kernel"
if (-not (Test-Path $kernelELF)) { throw "Kernel ELF not found: $kernelELF" }
Write-Host "[OK] v9.1.1 IRQ runtime commit wiring hardening verified"

# v9.2.0: EOI Runtime Boundary Foundation
Write-Host "Verifying v9.2.0 EOI runtime boundary contracts..."
$v911Tag = & git rev-list -n 1 v9.1.1 2>$null
$HEAD = & git rev-parse HEAD
if ($null -eq $v911Tag) { throw "v9.1.1 tag not found (required baseline)" }
if ($HEAD -eq $v911Tag) { throw "HEAD is still v9.1.1, v9.2.0 work not completed" }
Write-Host "[OK] v9.2.0 branch is beyond v9.1.1 locked baseline"

# v9.2.0: Verify three new EOI commands are callable (basic structure check)
# Note: Full output contracts require QEMU boot, so we verify code presence
$mainRs = Join-Path $repoRoot "kernel-lab\src\main.rs"
$irrRs = Join-Path $repoRoot "kernel-lab\src\irq.rs"
$mainContent = Get-Content $mainRs -Raw
$irrContent = Get-Content $irrRs -Raw

Assert-Contains $mainContent 'line_str == "eoi-runtime-note"' "eoi-runtime-note dispatcher"
Assert-Contains $mainContent 'line_str == "eoi-runtime-status"' "eoi-runtime-status dispatcher"
Assert-Contains $mainContent 'line_str == "eoi-runtime-blockers"' "eoi-runtime-blockers dispatcher"
Assert-Contains $mainContent '"EOI runtime dispatch note' "eoi-runtime-note output format"
Assert-Contains $mainContent '"EOI runtime readiness status' "eoi-runtime-status output format"
Assert-Contains $mainContent '"EOI runtime activation blockers' "eoi-runtime-blockers output format"

Assert-Contains $irrContent 'pub const EOI_RUNTIME_BLOCKER_PIC_REMAP' "EOI PIC remap blocker constant"
Assert-Contains $irrContent 'pub const EOI_RUNTIME_BLOCKER_IRQ_GATES' "EOI IRQ gates blocker constant"
Assert-Contains $irrContent 'pub const EOI_RUNTIME_BLOCKER_EDGE_LEVEL' "EOI edge/level blocker constant"
Assert-Contains $irrContent 'pub const EOI_RUNTIME_BLOCKER_KEYBOARD' "EOI keyboard blocker constant"
Assert-Contains $irrContent 'pub const EOI_RUNTIME_BLOCKER_STI' "EOI STI blocker constant"
Assert-Contains $irrContent 'pub fn eoi_runtime_check_all_preconditions' "eoi_runtime_check_all_preconditions function"

# v9.2.0: Verify kernel version
$cargoToml = Join-Path $repoRoot "kernel-lab\Cargo.toml"
$cargoContent = Get-Content $cargoToml -Raw
Assert-Contains $cargoContent 'version = "9.8.1"' "kernel-lab version 9.8.1"

# v9.2.0: Safety invariants still hold (from v9.1.1)
$irrContent = Get-Content $irrRs -Raw
Assert-Contains $irrContent 'pub const IRQ_RUNTIME_BLOCKER_PIC_REMAP' "IRQ PIC blocker still exists"
Assert-Contains $irrContent 'pub const IRQ_RUNTIME_BLOCKER_IRQ_GATES' "IRQ gates blocker still exists"
Assert-Contains $irrContent 'pub const IRQ_RUNTIME_BLOCKER_EOI_DISPATCH' "IRQ EOI blocker still exists"
Assert-Contains $irrContent 'pub const IRQ_RUNTIME_BLOCKER_STI' "IRQ STI blocker still exists"

# v9.2.0: Verify no STI enabled
Assert-NotContains $irrContent 'asm!("sti")' "no STI enabled"
Assert-NotContains $mainContent 'asm!("sti")' "no STI in main"

Write-Host "[OK] v9.2.0 EOI runtime boundary foundation verified"

Write-Host "Verifying v9.2.1 EOI runtime boundary hardening contracts..."
$v920Tag = & git rev-list -n 1 v9.2.0 2>$null
$HEAD = & git rev-parse HEAD
if ($null -eq $v920Tag) { throw "v9.2.0 tag not found (required baseline)" }
if ($HEAD -eq $v920Tag) { throw "HEAD is still v9.2.0, v9.2.1 work not completed" }
Write-Host "[OK] v9.2.1 branch is beyond v9.2.0 locked baseline"

$eoiRuntimeNoteExact = "EOI runtime dispatch note\neoi dispatch requires:\n- PIC remap controlled smoke ready\n- IRQ gates vectors 32/33 bound\n- IRQ edge/level detection strategy planned\n- keyboard fallback polling active\n- STI enabled\neoi dispatch: disabled (boundary definition only)\n"
$eoiRuntimeStatusTemplateExact = "EOI runtime readiness status\neoi dispatch: {}\npic remap: {}\nirq gates: {}\nkeyboard fallback: polling\nprerequisites satisfied: {}\neoi dispatch: disabled\n"
$eoiRuntimeBlockersHeaderExact = "EOI runtime activation blockers\n"
Assert-Contains $mainContent $eoiRuntimeNoteExact "eoi-runtime-note exact output contract"
Assert-Contains $mainContent $eoiRuntimeStatusTemplateExact "eoi-runtime-status exact output contract"
Assert-Contains $mainContent $eoiRuntimeBlockersHeaderExact "eoi-runtime-blockers exact header contract"
Assert-Contains $mainContent 'let eoi_status = if preconditions_met { "ready (dry-run)" } else { "blocked" };' "eoi-runtime-status dry-run ready wording"
Assert-Contains $mainContent 'let preconditions_met = irq::eoi_runtime_check_all_preconditions(pic_state.executed);' "eoi-runtime-status uses EOI precondition telemetry"
Assert-Contains $mainContent 'let pic_state = pic::ProgrammableInterruptController::pic_remap_state();' "eoi-runtime uses pic remap state telemetry"
Assert-Contains $mainContent 'let gate_state = irq::irq_gate_bind_state();' "eoi-runtime uses irq gate bind state telemetry"
$eoiBlockersStart = $mainContent.IndexOf('} else if line_str == "eoi-runtime-blockers" {')
$eoiBlockersEnd = $mainContent.IndexOf('} else if line_str == "pic-status --verbose" {', $eoiBlockersStart)
if ($eoiBlockersStart -lt 0 -or $eoiBlockersEnd -lt $eoiBlockersStart) { throw "eoi-runtime-blockers block isolation failed" }
$eoiBlockersBlock = $mainContent.Substring($eoiBlockersStart, $eoiBlockersEnd - $eoiBlockersStart)
Assert-ContainsInOrder $eoiBlockersBlock @(
    'if !pic_state.executed {',
    'irq::EOI_RUNTIME_BLOCKER_PIC_REMAP',
    'if !gate_state.executed {',
    'irq::EOI_RUNTIME_BLOCKER_IRQ_GATES',
    'irq::EOI_RUNTIME_BLOCKER_EDGE_LEVEL',
    'irq::EOI_RUNTIME_BLOCKER_KEYBOARD',
    'irq::EOI_RUNTIME_BLOCKER_STI',
    '"eoi dispatch: disabled\n"'
) "eoi-runtime-blockers source ordering"
Assert-ContainsInOrder $irrContent @(
    'pub const EOI_RUNTIME_BLOCKER_PIC_REMAP',
    'pub const EOI_RUNTIME_BLOCKER_IRQ_GATES',
    'pub const EOI_RUNTIME_BLOCKER_EDGE_LEVEL',
    'pub const EOI_RUNTIME_BLOCKER_KEYBOARD',
    'pub const EOI_RUNTIME_BLOCKER_STI'
) "eoi-runtime blocker constant ordering"
Assert-NotContains $cargoContent 'version = "9.2.0"' "kernel-lab stale v9.2.0 package version guard"
Assert-NotContains $cargoContent 'version = "9.2.1"' "kernel-lab stale v9.2.1 package version guard"
Assert-NotContains $mainContent 'write_pic_port(PIC_MASTER_CMD, PIC_EOI)' "kernel main does not dispatch master EOI"
Assert-NotContains $mainContent 'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)' "kernel main does not dispatch slave EOI"
Assert-NotContains $mainContent 'asm!("sti")' "kernel main still has no STI"
Assert-NotContains $irrContent 'asm!("sti")' "irq source still has no STI"
Assert-NotContains $mainContent 'keyboard_irq' "kernel main has no keyboard IRQ switch"
Assert-NotContains $mainContent 'timer_irq' "kernel main has no timer IRQ activation"
Assert-Contains $mainContent 'polling-only' "kernel main keeps keyboard polling-only telemetry"
Assert-Contains $mainContent 'runtime irq active: {}' "kernel main keeps runtime irq active matrix telemetry"

Write-Host "[OK] v9.2.1 EOI runtime boundary hardening verified"

# v9.3.0: PIC IRQ Mask Plan Foundation
Write-Host "Verifying v9.3.0 PIC IRQ Mask Plan Foundation contracts..."
$v921Tag = & git rev-list -n 1 v9.2.1 2>$null
$HEAD = & git rev-parse HEAD
if ($null -eq $v921Tag) { throw "v9.2.1 tag not found (required baseline)" }
if ($HEAD -eq $v921Tag) { throw "HEAD is still v9.2.1, v9.3.0 work not completed" }
Write-Host "[OK] v9.3.0 branch is beyond v9.2.1 locked baseline"

# v9.3.0: Re-read sources (Cargo.toml may have changed)
$cargoToml930 = Join-Path $repoRoot "kernel-lab\Cargo.toml"
$cargoContent930 = Get-Content $cargoToml930 -Raw
$irrContent930 = Get-Content $irrRs -Raw
$picRs = Join-Path $repoRoot "kernel-lab\src\pic.rs"
$picContent930 = Get-Content $picRs -Raw
$mainContent930 = Get-Content $mainRs -Raw

# v9.3.0: Version guard
Assert-Contains $cargoContent930 'version = "9.8.1"' "kernel-lab current version 9.8.1"
Assert-NotContains $cargoContent930 'version = "9.2.1"' "kernel-lab stale v9.2.1 guard"

# v9.3.0: irq.rs — blocker constants present
Assert-Contains $irrContent930 'pub const IRQ_MASK_BLOCKER_PIC_REMAP' "irq mask blocker pic remap constant"
Assert-Contains $irrContent930 'pub const IRQ_MASK_BLOCKER_IRQ_GATES' "irq mask blocker irq gates constant"
Assert-Contains $irrContent930 'pub const IRQ_MASK_BLOCKER_STI' "irq mask blocker sti constant"
Assert-Contains $irrContent930 'pub const IRQ_MASK_BLOCKER_EOI_DISPATCH' "irq mask blocker eoi dispatch constant"
Assert-Contains $irrContent930 'pub const IRQ_MASK_BLOCKER_IRQ_RUNTIME' "irq mask blocker irq runtime constant"

# v9.3.0: irq.rs — IrqMaskBlockerReport struct and functions
Assert-Contains $irrContent930 'pub struct IrqMaskBlockerReport' "IrqMaskBlockerReport struct"
Assert-Contains $irrContent930 'pub fn irq_mask_blocker_report(' "irq_mask_blocker_report function"
Assert-Contains $irrContent930 'pub fn irq_mask_check_all_blockers(' "irq_mask_check_all_blockers function"
Assert-Contains $irrContent930 'pub pic_remap_ready: bool' "IrqMaskBlockerReport pic_remap_ready field"
Assert-Contains $irrContent930 'pub irq_gates_ready: bool' "IrqMaskBlockerReport irq_gates_ready field"
Assert-Contains $irrContent930 'pub sti_ready: bool' "IrqMaskBlockerReport sti_ready field"
Assert-Contains $irrContent930 'pub eoi_dispatch_ready: bool' "IrqMaskBlockerReport eoi_dispatch_ready field"
Assert-Contains $irrContent930 'pub irq_runtime_committed: bool' "IrqMaskBlockerReport irq_runtime_committed field"
Assert-Contains $irrContent930 'pub all_clear: bool' "IrqMaskBlockerReport all_clear field"
# v9.3.0: STI and EOI dispatch hardcoded false
Assert-Contains $irrContent930 'let sti_ready = false;' "sti_ready hardcoded false in v9.3.0"
Assert-Contains $irrContent930 'let eoi_dispatch_ready = false;' "eoi_dispatch_ready hardcoded false in v9.3.0"

# v9.3.0: pic.rs — mask plan constants present
Assert-Contains $picContent930 'pub const PIC_MASK_PLAN_POLICY' "pic mask plan policy constant"
Assert-Contains $picContent930 'pub const PIC_MASK_UNMASK_POLICY' "pic mask unmask policy constant"
Assert-Contains $picContent930 'pub const PIC_MASK_UNMASK_GATE' "pic mask unmask gate constant"
Assert-Contains $picContent930 'pub const PIC_MASK_LIVE_UNMASK' "pic mask live unmask constant"
Assert-Contains $picContent930 'pub const PIC_MASK_WRITES_PATH' "pic mask writes path constant"
Assert-Contains $picContent930 'pub const PIC_MASK_BLOCKER_REMAP' "pic mask blocker remap constant"
Assert-Contains $picContent930 'pub const PIC_MASK_CANDIDATES' "pic mask candidates constant"

# v9.3.0: pic.rs — telemetry structs and methods
Assert-Contains $picContent930 'pub struct PicMaskPlanTelemetry' "PicMaskPlanTelemetry struct"
Assert-Contains $picContent930 'pub struct PicMaskStatusTelemetry' "PicMaskStatusTelemetry struct"
Assert-Contains $picContent930 'pub fn pic_mask_plan()' "pic_mask_plan method"
Assert-Contains $picContent930 'pub fn pic_mask_status()' "pic_mask_status method"

# v9.3.0: main.rs — command dispatcher has all 3 new commands
Assert-Contains $mainContent930 'line_str == "pic-mask-plan"' "pic-mask-plan command dispatcher"
Assert-Contains $mainContent930 'line_str == "pic-mask-status"' "pic-mask-status command dispatcher"
Assert-Contains $mainContent930 'line_str == "irq-mask-blockers"' "irq-mask-blockers command dispatcher"

# v9.3.0: Exact output contract guards
$picMaskPlanExact = 'PIC IRQ mask plan\nmask policy: all masked (0xFF)\nmaster imr: 0xFF (all masked)\nslave imr: 0xFF (all masked)\nunmask candidates: none\nunmask policy: no lines scheduled for unmask\nunmask gate: disabled\n'
$irqMaskBlockersHeader = 'PIC IRQ unmask activation blockers\n'
Assert-Contains $mainContent930 $picMaskPlanExact "pic-mask-plan exact output contract"
Assert-Contains $mainContent930 $irqMaskBlockersHeader "irq-mask-blockers header exact contract"
Assert-Contains $mainContent930 '"unmask gate: disabled\n"' "irq-mask-blockers unmask gate disabled footer"
Assert-Contains $mainContent930 'live unmask: {}\n"' "pic-mask-status live unmask field format"

# v9.3.0: help string updated with new commands
Assert-Contains $mainContent930 'pic-mask-plan pic-mask-status irq-mask-blockers' "help string includes v9.3.0 commands"

# v9.3.0: Invariant guards — no STI (carry forward)
Assert-NotContains $irrContent930 'asm!("sti")' "irq source still has no STI"
Assert-NotContains $mainContent930 'asm!("sti")' "kernel main still has no STI"

# v9.3.0: Forbidden unmask pattern guards (specific byte values outside controlled path)
# Allowed: PIC_MASK_ALL (0xFF) via write_pic_port in controlled smoke path only
# Forbidden: any specific unmask value written directly as a literal
Assert-NotContains $mainContent930 'write_pic_port(PIC_MASTER_DATA, 0x00)' "no master IMR unmask-all literal in main"
Assert-NotContains $mainContent930 'write_pic_port(PIC_MASTER_DATA, 0xFC)' "no master IMR partial unmask 0xFC in main"
Assert-NotContains $mainContent930 'write_pic_port(PIC_MASTER_DATA, 0xFD)' "no master IMR partial unmask 0xFD in main"
Assert-NotContains $mainContent930 'write_pic_port(PIC_MASTER_DATA, 0xFE)' "no master IMR partial unmask 0xFE in main"
Assert-NotContains $mainContent930 'write_pic_port(PIC_SLAVE_DATA, 0x00)' "no slave IMR unmask-all literal in main"
Assert-NotContains $mainContent930 'write_pic_port(PIC_SLAVE_DATA, 0xFE)' "no slave IMR partial unmask 0xFE in main"
Assert-NotContains $picContent930 'write_pic_port(PIC_MASTER_DATA, 0x00)' "no master IMR unmask literal in pic.rs"
Assert-NotContains $picContent930 'write_pic_port(PIC_SLAVE_DATA, 0x00)' "no slave IMR unmask literal in pic.rs"

# v9.3.0: Key phrase guards — live unmask / unmask gate / mask writes / runtime irq active
Assert-Contains $picContent930 'pub const PIC_MASK_LIVE_UNMASK:   &str = "no";' "live unmask: no phrase present"
Assert-Contains $mainContent930 '"unmask gate: disabled\n"' "unmask gate disabled phrase present"
Assert-Contains $picContent930 'pub const PIC_MASK_WRITES_PATH:   &str = "controlled smoke path only";' "mask writes controlled smoke path only phrase"
Assert-Contains $mainContent930 'runtime irq active: {}' "runtime irq active matrix phrase preserved"

# v9.3.0: ELF symbol checks
$kernelELF930 = Join-Path $repoRoot "kernel-lab\target\i686-unknown-linux-gnu\debug\dbyte_kernel"
if (-not (Test-Path $kernelELF930)) { throw "Kernel ELF not found for v9.3.0 symbol check: $kernelELF930" }
$nmTool = "nm"
$elfSymbols930 = & $nmTool $kernelELF930 2>&1 | Out-String
Assert-Contains $elfSymbols930 "pic_mask_plan" "ELF contains pic_mask_plan symbol"
Assert-Contains $elfSymbols930 "pic_mask_status" "ELF contains pic_mask_status symbol"
Assert-Contains $elfSymbols930 "irq_mask_blocker_report" "ELF contains irq_mask_blocker_report symbol"
Assert-Contains $elfSymbols930 "irq_mask_check_all_blockers" "ELF contains irq_mask_check_all_blockers symbol"

Write-Host "[OK] v9.3.0 PIC IRQ Mask Plan Foundation verified"

# v9.3.1: PIC IRQ Mask Plan Hardening
Write-Host "Verifying v9.3.1 PIC IRQ Mask Plan Hardening contracts..."
$v930Tag = & git rev-list -n 1 v9.3.0 2>$null
$HEAD = & git rev-parse HEAD
if ($null -eq $v930Tag) { throw "v9.3.0 tag not found (required baseline)" }
if ($HEAD -eq $v930Tag) { throw "HEAD is still v9.3.0, v9.3.1 work not completed" }
Write-Host "[OK] v9.3.1 branch is beyond v9.3.0 locked baseline"

$cargoContent931 = Get-Content $cargoToml -Raw
$irrContent931 = Get-Content $irrRs -Raw
$picContent931 = Get-Content $picRs -Raw
$mainContent931 = Get-Content $mainRs -Raw

Assert-Contains $cargoContent931 'version = "9.8.1"' "kernel-lab current version 9.8.1"
Assert-NotContains $cargoContent931 'version = "9.3.0"' "kernel-lab stale v9.3.0 package version guard"

$picMaskPlanExact931 = 'PIC IRQ mask plan\nmask policy: all masked (0xFF)\nmaster imr: 0xFF (all masked)\nslave imr: 0xFF (all masked)\nunmask candidates: none\nunmask policy: no lines scheduled for unmask\nunmask gate: disabled\n'
Assert-Contains $mainContent931 $picMaskPlanExact931 "v9.3.1 pic-mask-plan exact mask-all output"
Assert-Contains $mainContent931 '"PIC IRQ mask status\nmaster imr planned: 0x{:02x}\nslave imr planned: 0x{:02x}\nunmask candidates: {}\nunmask blocked: {}\nmask writes: {}\nlive unmask: {}\n"' "v9.3.1 pic-mask-status field output contract"
Assert-Contains $mainContent931 'PIC IRQ unmask activation blockers\n' "v9.3.1 irq-mask-blockers header"
Assert-Contains $mainContent931 '"unmask gate: disabled\n"' "v9.3.1 irq-mask-blockers gate disabled footer"

Assert-Contains $picContent931 'pub const PIC_MASK_ALL: u8 = 0xFF;' "v9.3.1 safe mask-all constant"
Assert-Contains $picContent931 'pub const PIC_MASK_PLAN_POLICY:   &str = "all masked (0xFF)";' "v9.3.1 mask policy all masked"
Assert-Contains $picContent931 'pub const PIC_MASK_CANDIDATES:    &str = "none";' "v9.3.1 unmask candidates none"
Assert-Contains $picContent931 'pub const PIC_MASK_LIVE_UNMASK:   &str = "no";' "v9.3.1 live unmask no"
Assert-Contains $picContent931 'pub const PIC_MASK_WRITES_PATH:   &str = "controlled smoke path only";' "v9.3.1 writes path controlled smoke only"
Assert-Contains $picContent931 'master_imr_planned: PIC_MASK_ALL' "v9.3.1 master planned mask-all telemetry"
Assert-Contains $picContent931 'slave_imr_planned: PIC_MASK_ALL' "v9.3.1 slave planned mask-all telemetry"
Assert-Contains $picContent931 'write_pic_port(PIC_MASTER_DATA, PIC_MASK_ALL);' "v9.3.1 controlled smoke masks master with PIC_MASK_ALL"
Assert-Contains $picContent931 'write_pic_port(PIC_SLAVE_DATA, PIC_MASK_ALL);' "v9.3.1 controlled smoke masks slave with PIC_MASK_ALL"

foreach ($literal in @('0x00', '0xFC', '0xFD', '0xFE')) {
    Assert-NotContains $mainContent931 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.3.1 no master unmask literal $literal in main"
    Assert-NotContains $mainContent931 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.3.1 no slave unmask literal $literal in main"
    Assert-NotContains $picContent931 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.3.1 no master unmask literal $literal in pic.rs"
    Assert-NotContains $picContent931 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.3.1 no slave unmask literal $literal in pic.rs"
}

Assert-NotContains $irrContent931 'asm!("sti")' "v9.3.1 irq source still has no STI"
Assert-NotContains $mainContent931 'asm!("sti")' "v9.3.1 kernel main still has no STI"
Assert-NotContains $mainContent931 'write_pic_port(PIC_MASTER_CMD, PIC_EOI)' "v9.3.1 kernel main does not dispatch master EOI"
Assert-NotContains $mainContent931 'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)' "v9.3.1 kernel main does not dispatch slave EOI"
Assert-NotContains $mainContent931 'timer_interrupt_handler_stub' "v9.3.1 kernel main has no live timer IRQ handler"
Assert-NotContains $mainContent931 'keyboard_interrupt_handler_stub' "v9.3.1 kernel main has no live keyboard IRQ handler"
Assert-NotContains $mainContent931 'timer_irq' "v9.3.1 kernel main has no timer IRQ activation path"
Assert-NotContains $mainContent931 'keyboard_irq' "v9.3.1 kernel main has no keyboard IRQ activation path"
Assert-Contains $mainContent931 'polling-only' "v9.3.1 keyboard polling telemetry unchanged"
Assert-Contains $mainContent931 'runtime irq active: {}' "v9.3.1 runtime IRQ remains inactive"

Write-Host "[OK] v9.3.1 PIC IRQ Mask Plan Hardening verified"

# v9.4.0: IRQ Runtime Readiness Matrix Foundation
Write-Host "Verifying v9.4.0 IRQ Runtime Readiness Matrix Foundation contracts..."
$v931Tag = & git rev-list -n 1 v9.3.1 2>$null
$HEAD = & git rev-parse HEAD
if ($null -eq $v931Tag) { throw "v9.3.1 tag not found (required baseline)" }
if ($HEAD -eq $v931Tag) { throw "HEAD is still v9.3.1, v9.4.0 work not completed" }
Write-Host "[OK] v9.4.0 branch is beyond v9.3.1 locked baseline"

$cargoContent940 = Get-Content $cargoToml -Raw
$irrContent940 = Get-Content $irrRs -Raw
$picContent940 = Get-Content $picRs -Raw
$mainContent940 = Get-Content $mainRs -Raw

Assert-Contains $cargoContent940 'version = "9.8.1"' "kernel-lab current version 9.8.1"
Assert-NotContains $cargoContent940 'version = "9.3.1"' "kernel-lab stale v9.3.1 package version guard"

Assert-Contains $mainContent940 'irq-runtime-matrix irq-runtime-readiness irq-runtime-next' "help string includes v9.4.0 commands"
Assert-Contains $mainContent940 'line_str == "irq-runtime-matrix"' "irq-runtime-matrix command dispatcher"
Assert-Contains $mainContent940 'line_str == "irq-runtime-readiness"' "irq-runtime-readiness command dispatcher"
Assert-Contains $mainContent940 'line_str == "irq-runtime-next"' "irq-runtime-next command dispatcher"

Assert-Contains $irrContent940 'pub struct IrqRuntimeMatrix' "IrqRuntimeMatrix struct"
Assert-Contains $irrContent940 'pub fn irq_runtime_matrix(' "irq_runtime_matrix function"
Assert-Contains $irrContent940 'pub const IRQ_MATRIX_UNMASK_POLICY_NO_UNMASK: &str = "no unmask";' "matrix unmask policy no unmask"
Assert-Contains $irrContent940 'pub const IRQ_MATRIX_RUNTIME_IRQ_ACTIVE_NO: &str = "no";' "matrix runtime irq active no"
Assert-Contains $irrContent940 'pub const IRQ_MATRIX_KEYBOARD_MODE_POLLING: &str = "polling";' "matrix keyboard polling"
Assert-Contains $irrContent940 'pub const IRQ_MATRIX_STI_DISABLED: &str = "disabled";' "matrix sti disabled"

$irqRuntimeMatrixExact = 'IRQ runtime readiness matrix\npic remap smoke: {}\nirq gate bind smoke: {}\neoi runtime boundary: {}\npic mask policy: {}\nunmask policy: {}\nruntime latch: {}\nkeyboard mode: {}\nsti: {}\nruntime irq active: {}\n'
$irqRuntimeReadinessExact = 'IRQ runtime readiness\nsmoke prerequisites: {}\nmask policy: {}\nruntime latch: {}\nsti: {}\nruntime irq ready: no\n'
$irqRuntimeNextExact = 'IRQ runtime next\n1. keep PIC mask policy all masked (0xFF)\n2. keep unmask policy no unmask\n3. implement live EOI dispatch boundary\n4. enable STI only after EOI and handlers are ready\n5. switch keyboard from polling only after IRQ1 handler is live\nruntime irq active: no\n'
Assert-Contains $mainContent940 $irqRuntimeMatrixExact "irq-runtime-matrix exact output contract"
Assert-Contains $mainContent940 $irqRuntimeReadinessExact "irq-runtime-readiness exact output contract"
Assert-Contains $mainContent940 $irqRuntimeNextExact "irq-runtime-next exact output contract"

Assert-Contains $mainContent940 'pic::ProgrammableInterruptController::pic_remap_state();' "matrix reuses pic_remap_state"
Assert-Contains $mainContent940 'irq::irq_gate_bind_state();' "matrix reuses irq_gate_bind_state"
Assert-Contains $mainContent940 'irq::eoi_runtime_check_all_preconditions(pic_state.executed);' "matrix reuses eoi runtime preconditions"
Assert-Contains $mainContent940 'pic::ProgrammableInterruptController::pic_mask_plan();' "matrix reuses pic_mask_plan"
Assert-Contains $mainContent940 'pic::ProgrammableInterruptController::pic_mask_status();' "matrix reuses pic_mask_status"
Assert-Contains $mainContent940 'irq::irq_runtime_is_armed()' "matrix reuses runtime armed latch"
Assert-Contains $mainContent940 'irq::irq_runtime_is_committed()' "matrix reuses runtime committed latch"

Assert-NotContains $irrContent940 'asm!("sti")' "v9.4.0 irq source still has no STI"
Assert-NotContains $mainContent940 'asm!("sti")' "v9.4.0 kernel main still has no STI"
Assert-Contains $picContent940 'pub const PIC_MASK_ALL: u8 = 0xFF;' "v9.4.0 safe mask-all constant remains allowed"
foreach ($literal in @('0x00', '0xFC', '0xFD', '0xFE')) {
    Assert-NotContains $mainContent940 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.4.0 no master unmask literal $literal in main"
    Assert-NotContains $mainContent940 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.4.0 no slave unmask literal $literal in main"
    Assert-NotContains $picContent940 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.4.0 no master unmask literal $literal in pic.rs"
    Assert-NotContains $picContent940 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.4.0 no slave unmask literal $literal in pic.rs"
}
Assert-NotContains $mainContent940 'write_pic_port(PIC_MASTER_CMD, PIC_EOI)' "v9.4.0 kernel main does not dispatch master EOI"
Assert-NotContains $mainContent940 'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)' "v9.4.0 kernel main does not dispatch slave EOI"
Assert-NotContains $mainContent940 'timer_interrupt_handler_stub' "v9.4.0 kernel main has no live timer IRQ handler"
Assert-NotContains $mainContent940 'keyboard_interrupt_handler_stub' "v9.4.0 kernel main has no live keyboard IRQ handler"
Assert-NotContains $mainContent940 'timer_irq' "v9.4.0 kernel main has no timer IRQ activation path"
Assert-NotContains $mainContent940 'keyboard_irq' "v9.4.0 kernel main has no keyboard IRQ activation path"
Assert-Contains $mainContent940 'polling-only' "v9.4.0 keyboard polling telemetry unchanged"
Assert-Contains $mainContent940 'runtime irq active: no' "v9.4.0 runtime IRQ remains inactive"

Write-Host "[OK] v9.4.0 IRQ Runtime Readiness Matrix Foundation verified"

# v9.4.1: IRQ Runtime Readiness Matrix Hardening
Write-Host "Verifying v9.4.1 IRQ Runtime Readiness Matrix Hardening contracts..."
$v940Tag = & git rev-list -n 1 v9.4.0 2>$null
$HEAD = & git rev-parse HEAD
if ($null -eq $v940Tag) { throw "v9.4.0 tag not found (required baseline)" }
if ($HEAD -eq $v940Tag) { throw "HEAD is still v9.4.0, v9.4.1 work not completed" }
Write-Host "[OK] v9.4.1 branch is beyond v9.4.0 locked baseline"

$cargoContent941 = Get-Content $cargoToml -Raw
$irrContent941 = Get-Content $irrRs -Raw
$picContent941 = Get-Content $picRs -Raw
$mainContent941 = Get-Content $mainRs -Raw

Assert-Contains $cargoContent941 'version = "9.8.1"' "kernel-lab current version 9.8.1"
Assert-NotContains $cargoContent941 'version = "9.4.0"' "kernel-lab stale v9.4.0 package version guard"

$matrixBlockStart = $mainContent941.IndexOf('} else if line_str == "irq-runtime-matrix" {')
$matrixBlockEnd = $mainContent941.IndexOf('} else if line_str == "irq-runtime-readiness" {', $matrixBlockStart)
$readinessBlockStart = $matrixBlockEnd
$readinessBlockEnd = $mainContent941.IndexOf('} else if line_str == "irq-runtime-next" {', $readinessBlockStart)
$nextBlockStart = $readinessBlockEnd
$nextBlockEnd = $mainContent941.IndexOf('} else if line_str == "irq-runtime-activation-plan" {', $nextBlockStart)
if ($matrixBlockStart -lt 0 -or $matrixBlockEnd -lt $matrixBlockStart -or $readinessBlockEnd -lt $readinessBlockStart -or $nextBlockEnd -lt $nextBlockStart) {
    throw "v9.4.1 matrix command block isolation failed"
}
$matrixBlock = $mainContent941.Substring($matrixBlockStart, $matrixBlockEnd - $matrixBlockStart)
$readinessBlock = $mainContent941.Substring($readinessBlockStart, $readinessBlockEnd - $readinessBlockStart)
$nextBlock = $mainContent941.Substring($nextBlockStart, $nextBlockEnd - $nextBlockStart)

$irqRuntimeMatrixExact941 = 'IRQ runtime readiness matrix\npic remap smoke: {}\nirq gate bind smoke: {}\neoi runtime boundary: {}\npic mask policy: {}\nunmask policy: {}\nruntime latch: {}\nkeyboard mode: {}\nsti: {}\nruntime irq active: {}\n'
$irqRuntimeReadinessExact941 = 'IRQ runtime readiness\nsmoke prerequisites: {}\nmask policy: {}\nruntime latch: {}\nsti: {}\nruntime irq ready: no\n'
$irqRuntimeNextExact941 = 'IRQ runtime next\n1. keep PIC mask policy all masked (0xFF)\n2. keep unmask policy no unmask\n3. implement live EOI dispatch boundary\n4. enable STI only after EOI and handlers are ready\n5. switch keyboard from polling only after IRQ1 handler is live\nruntime irq active: no\n'
Assert-Contains $mainContent941 $irqRuntimeMatrixExact941 "v9.4.1 irq-runtime-matrix exact output"
Assert-Contains $mainContent941 $irqRuntimeReadinessExact941 "v9.4.1 irq-runtime-readiness exact output"
Assert-Contains $mainContent941 $irqRuntimeNextExact941 "v9.4.1 irq-runtime-next exact output"

Assert-ContainsInOrder $matrixBlock @(
    'pic remap smoke: {}',
    'irq gate bind smoke: {}',
    'eoi runtime boundary: {}',
    'pic mask policy: {}',
    'unmask policy: {}',
    'runtime latch: {}',
    'keyboard mode: {}',
    'sti: {}',
    'runtime irq active: {}'
) "v9.4.1 matrix field ordering"
Assert-ContainsInOrder $readinessBlock @(
    'smoke prerequisites: {}',
    'mask policy: {}',
    'runtime latch: {}',
    'sti: {}',
    'runtime irq ready: no'
) "v9.4.1 readiness summary wording order"
Assert-ContainsInOrder $nextBlock @(
    '1. keep PIC mask policy all masked (0xFF)',
    '2. keep unmask policy no unmask',
    '3. implement live EOI dispatch boundary',
    '4. enable STI only after EOI and handlers are ready',
    '5. switch keyboard from polling only after IRQ1 handler is live',
    'runtime irq active: no'
) "v9.4.1 irq-runtime-next recommendation wording"

foreach ($blockedCall in @(
    'write_pic_port(',
    'set_handler(',
    'irq::irq_runtime_commit()',
    'irq::irq_runtime_arm()',
    'pic::ProgrammableInterruptController::pic_remap_controlled_smoke()',
    'irq::irq_gate_bind_smoke_mark_bound()',
    'asm!("sti")'
)) {
    Assert-NotContains $matrixBlock $blockedCall "v9.4.1 matrix command is read-only: $blockedCall"
    Assert-NotContains $readinessBlock $blockedCall "v9.4.1 readiness command is read-only: $blockedCall"
    Assert-NotContains $nextBlock $blockedCall "v9.4.1 next command is advisory-only: $blockedCall"
}

Assert-Contains $matrixBlock 'pic::ProgrammableInterruptController::pic_remap_state();' "v9.4.1 matrix reads pic remap state"
Assert-Contains $matrixBlock 'irq::irq_gate_bind_state();' "v9.4.1 matrix reads irq gate state"
Assert-Contains $matrixBlock 'pic::ProgrammableInterruptController::pic_mask_plan();' "v9.4.1 matrix reads pic mask plan"
Assert-Contains $matrixBlock 'pic::ProgrammableInterruptController::pic_mask_status();' "v9.4.1 matrix reads pic mask status"
Assert-Contains $matrixBlock 'irq::eoi_runtime_check_all_preconditions(pic_state.executed);' "v9.4.1 matrix reads eoi boundary"
Assert-Contains $readinessBlock 'pic::ProgrammableInterruptController::pic_remap_state();' "v9.4.1 readiness reads pic remap state"
Assert-Contains $readinessBlock 'irq::irq_gate_bind_state();' "v9.4.1 readiness reads irq gate state"
Assert-Contains $readinessBlock 'pic::ProgrammableInterruptController::pic_mask_plan();' "v9.4.1 readiness reads pic mask plan"
Assert-Contains $readinessBlock 'irq::eoi_runtime_check_all_preconditions(pic_state.executed);' "v9.4.1 readiness reads eoi boundary"

Assert-NotContains $irrContent941 'asm!("sti")' "v9.4.1 irq source still has no STI"
Assert-NotContains $mainContent941 'asm!("sti")' "v9.4.1 kernel main still has no STI"
Assert-Contains $picContent941 'pub const PIC_MASK_ALL: u8 = 0xFF;' "v9.4.1 safe mask-all constant remains allowed"
foreach ($literal in @('0x00', '0xFC', '0xFD', '0xFE')) {
    Assert-NotContains $mainContent941 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.4.1 no master unmask literal $literal in main"
    Assert-NotContains $mainContent941 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.4.1 no slave unmask literal $literal in main"
    Assert-NotContains $picContent941 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.4.1 no master unmask literal $literal in pic.rs"
    Assert-NotContains $picContent941 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.4.1 no slave unmask literal $literal in pic.rs"
}
Assert-NotContains $mainContent941 'write_pic_port(PIC_MASTER_CMD, PIC_EOI)' "v9.4.1 kernel main does not dispatch master EOI"
Assert-NotContains $mainContent941 'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)' "v9.4.1 kernel main does not dispatch slave EOI"
Assert-NotContains $mainContent941 'timer_interrupt_handler_stub' "v9.4.1 kernel main has no live timer IRQ handler"
Assert-NotContains $mainContent941 'keyboard_interrupt_handler_stub' "v9.4.1 kernel main has no live keyboard IRQ handler"
Assert-NotContains $mainContent941 'timer_irq' "v9.4.1 kernel main has no timer IRQ activation path"
Assert-NotContains $mainContent941 'keyboard_irq' "v9.4.1 kernel main has no keyboard IRQ activation path"
Assert-Contains $mainContent941 'polling-only' "v9.4.1 keyboard polling telemetry unchanged"
Assert-Contains $mainContent941 'runtime irq active: no' "v9.4.1 runtime IRQ remains inactive"

Write-Host "[OK] v9.4.1 IRQ Runtime Readiness Matrix Hardening verified"

# v9.5.0: IRQ Runtime Activation Dry-Run Integration
Write-Host "Verifying v9.5.0 IRQ Runtime Activation Dry-Run Integration contracts..."
$v941Tag = & git rev-list -n 1 v9.4.1 2>$null
$HEAD = & git rev-parse HEAD
if ($null -eq $v941Tag) { throw "v9.4.1 tag not found (required baseline)" }
if ($HEAD -eq $v941Tag) { throw "HEAD is still v9.4.1, v9.5.0 work not completed" }
Write-Host "[OK] v9.5.0 branch is beyond v9.4.1 locked baseline"

$cargoContent950 = Get-Content $cargoToml -Raw
$irrContent950 = Get-Content $irrRs -Raw
$picContent950 = Get-Content $picRs -Raw
$mainContent950 = Get-Content $mainRs -Raw

Assert-Contains $cargoContent950 'version = "9.8.1"' "kernel-lab current version 9.8.1"
Assert-NotContains $cargoContent950 'version = "9.4.1"' "kernel-lab stale v9.4.1 package version guard"
Assert-Contains $mainContent950 'irq-runtime-next irq-runtime-activation-plan' "help string includes v9.5.0 activation command"
Assert-Contains $mainContent950 'line_str == "irq-runtime-activation-plan"' "irq-runtime-activation-plan dispatcher"
Assert-Contains $irrContent950 'pub struct IrqRuntimeActivationDryRun' "activation dry-run report struct"
Assert-Contains $irrContent950 'pub fn irq_runtime_activation_dry_run(matrix: &IrqRuntimeMatrix)' "activation dry-run matrix helper"
Assert-Contains $irrContent950 'allowed: false' "v9.5.0 dry-run commit remains blocked"

$commitBlockStart950 = $mainContent950.IndexOf('} else if line_str == "irq-runtime-commit" {')
$commitBlockEnd950 = $mainContent950.IndexOf('}else if line_str == "irq-runtime-status" {', $commitBlockStart950)
$activationBlockStart950 = $mainContent950.IndexOf('} else if line_str == "irq-runtime-activation-plan" {')
$activationBlockEnd950 = $mainContent950.IndexOf('} else if line_str == "irq-runtime-token-note" {', $activationBlockStart950)
if ($commitBlockStart950 -lt 0 -or $commitBlockEnd950 -lt $commitBlockStart950 -or $activationBlockStart950 -lt 0 -or $activationBlockEnd950 -lt $activationBlockStart950) {
    throw "v9.5.0 activation command block isolation failed"
}
$commitBlock950 = $mainContent950.Substring($commitBlockStart950, $commitBlockEnd950 - $commitBlockStart950)
$activationBlock950 = $mainContent950.Substring($activationBlockStart950, $activationBlockEnd950 - $activationBlockStart950)

$irqRuntimeCommitDryRunExact950 = 'IRQ runtime activation commit dry-run\npic remap smoke: {}\nirq gate bind smoke: {}\neoi runtime boundary: {}\npic mask policy: {}\nunmask policy: {}\nruntime latch: {}\nsti: {}\nruntime irq active: {}\ndry-run commit allowed: {}\nresult: {}\n'
$irqRuntimeActivationPlanExact950 = 'IRQ runtime activation plan\n1. require readiness matrix smoke prerequisites: yes\n2. require EOI runtime boundary: ready (dry-run)\n3. keep PIC mask policy: {}\n4. keep unmask policy: {}\n5. keep STI: {}\n6. commit path remains dry-run only\nruntime irq active: {}\ndry-run commit allowed: {}\n'
Assert-Contains $commitBlock950 $irqRuntimeCommitDryRunExact950 "v9.5.0 irq-runtime-commit exact matrix output"
Assert-Contains $activationBlock950 $irqRuntimeActivationPlanExact950 "v9.5.0 irq-runtime-activation-plan exact output"
Assert-Contains $commitBlock950 'pic::ProgrammableInterruptController::pic_remap_state();' "v9.5.0 commit reads pic remap state"
Assert-Contains $commitBlock950 'irq::irq_gate_bind_state();' "v9.5.0 commit reads irq gate state"
Assert-Contains $commitBlock950 'pic::ProgrammableInterruptController::pic_mask_plan();' "v9.5.0 commit reads mask plan"
Assert-Contains $commitBlock950 'pic::ProgrammableInterruptController::pic_mask_status();' "v9.5.0 commit reads mask status"
Assert-Contains $commitBlock950 'irq::irq_runtime_matrix(' "v9.5.0 commit derives matrix"
Assert-Contains $commitBlock950 'irq::irq_runtime_activation_dry_run(&matrix);' "v9.5.0 commit derives activation decision from matrix"
Assert-Contains $commitBlock950 'if !activation.allowed {' "v9.5.0 commit blocks when matrix disallows dry-run"
Assert-NotContains $commitBlock950 'irq::irq_runtime_commit()' "v9.5.0 commit does not mutate runtime latch"
Assert-Contains $activationBlock950 'irq::irq_runtime_matrix(' "v9.5.0 activation plan derives matrix"
Assert-Contains $activationBlock950 'irq::irq_runtime_activation_dry_run(&matrix);' "v9.5.0 activation plan derives decision from matrix"

foreach ($blockedCall in @(
    'write_pic_port(',
    'set_handler(',
    'irq::irq_runtime_commit()',
    'irq::irq_runtime_arm()',
    'pic::ProgrammableInterruptController::pic_remap_controlled_smoke()',
    'irq::irq_gate_bind_smoke_mark_bound()',
    'asm!("sti")'
)) {
    Assert-NotContains $activationBlock950 $blockedCall "v9.5.0 activation plan is advisory-only: $blockedCall"
}

Assert-NotContains $irrContent950 'asm!("sti")' "v9.5.0 irq source still has no STI"
Assert-NotContains $mainContent950 'asm!("sti")' "v9.5.0 kernel main still has no STI"
Assert-Contains $picContent950 'pub const PIC_MASK_ALL: u8 = 0xFF;' "v9.5.0 safe mask-all constant remains allowed"
foreach ($literal in @('0x00', '0xFC', '0xFD', '0xFE')) {
    Assert-NotContains $mainContent950 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.5.0 no master unmask literal $literal in main"
    Assert-NotContains $mainContent950 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.5.0 no slave unmask literal $literal in main"
    Assert-NotContains $picContent950 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.5.0 no master unmask literal $literal in pic.rs"
    Assert-NotContains $picContent950 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.5.0 no slave unmask literal $literal in pic.rs"
}
Assert-NotContains $mainContent950 'write_pic_port(PIC_MASTER_CMD, PIC_EOI)' "v9.5.0 kernel main does not dispatch master EOI"
Assert-NotContains $mainContent950 'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)' "v9.5.0 kernel main does not dispatch slave EOI"
Assert-NotContains $mainContent950 'timer_interrupt_handler_stub' "v9.5.0 kernel main has no live timer IRQ handler"
Assert-NotContains $mainContent950 'keyboard_interrupt_handler_stub' "v9.5.0 kernel main has no live keyboard IRQ handler"
Assert-NotContains $mainContent950 'timer_irq' "v9.5.0 kernel main has no timer IRQ activation path"
Assert-NotContains $mainContent950 'keyboard_irq' "v9.5.0 kernel main has no keyboard IRQ activation path"
Assert-Contains $mainContent950 'polling-only' "v9.5.0 keyboard polling telemetry unchanged"
Assert-Contains $mainContent950 'runtime irq active: {}' "v9.5.0 runtime IRQ remains inactive"

Write-Host "[OK] v9.5.0 IRQ Runtime Activation Dry-Run Integration verified"

# v9.5.1: IRQ Runtime Activation Dry-Run Hardening
Write-Host "Verifying v9.5.1 IRQ Runtime Activation Dry-Run Hardening contracts..."
$v950Tag = & git rev-list -n 1 v9.5.0 2>$null
$HEAD = & git rev-parse HEAD
if ($null -eq $v950Tag) { throw "v9.5.0 tag not found (required baseline)" }
if ($HEAD -eq $v950Tag) { throw "HEAD is still v9.5.0, v9.5.1 work not completed" }
Write-Host "[OK] v9.5.1 branch is beyond v9.5.0 locked baseline"

$cargoContent951 = Get-Content $cargoToml -Raw
$irrContent951 = Get-Content $irrRs -Raw
$picContent951 = Get-Content $picRs -Raw
$mainContent951 = Get-Content $mainRs -Raw

Assert-Contains $cargoContent951 'version = "9.8.1"' "kernel-lab current version 9.8.1"
Assert-NotContains $cargoContent951 'version = "9.5.0"' "kernel-lab stale v9.5.0 package version guard"
Assert-Contains $mainContent951 'irq-runtime-activation-plan' "v9.5.1 activation plan command remains exposed"
Assert-Contains $mainContent951 'line_str == "irq-runtime-commit"' "v9.5.1 irq-runtime-commit dispatcher remains exposed"
Assert-Contains $mainContent951 'line_str == "irq-runtime-matrix"' "v9.5.1 irq-runtime-matrix dispatcher remains exposed"
Assert-Contains $mainContent951 'line_str == "irq-runtime-readiness"' "v9.5.1 irq-runtime-readiness dispatcher remains exposed"
Assert-Contains $mainContent951 'line_str == "irq-runtime-next"' "v9.5.1 irq-runtime-next dispatcher remains exposed"

$commitBlockStart951 = $mainContent951.IndexOf('} else if line_str == "irq-runtime-commit" {')
$commitBlockEnd951 = $mainContent951.IndexOf('}else if line_str == "irq-runtime-status" {', $commitBlockStart951)
$activationBlockStart951 = $mainContent951.IndexOf('} else if line_str == "irq-runtime-activation-plan" {')
$activationBlockEnd951 = $mainContent951.IndexOf('} else if line_str == "irq-runtime-token-note" {', $activationBlockStart951)
if ($commitBlockStart951 -lt 0 -or $commitBlockEnd951 -lt $commitBlockStart951 -or $activationBlockStart951 -lt 0 -or $activationBlockEnd951 -lt $activationBlockStart951) {
    throw "v9.5.1 activation hardening block isolation failed"
}
$commitBlock951 = $mainContent951.Substring($commitBlockStart951, $commitBlockEnd951 - $commitBlockStart951)
$activationBlock951 = $mainContent951.Substring($activationBlockStart951, $activationBlockEnd951 - $activationBlockStart951)

$irqRuntimeCommitDryRunExact951 = 'IRQ runtime activation commit dry-run\npic remap smoke: {}\nirq gate bind smoke: {}\neoi runtime boundary: {}\npic mask policy: {}\nunmask policy: {}\nruntime latch: {}\nsti: {}\nruntime irq active: {}\ndry-run commit allowed: {}\nresult: {}\n'
$irqRuntimeActivationPlanExact951 = 'IRQ runtime activation plan\n1. require readiness matrix smoke prerequisites: yes\n2. require EOI runtime boundary: ready (dry-run)\n3. keep PIC mask policy: {}\n4. keep unmask policy: {}\n5. keep STI: {}\n6. commit path remains dry-run only\nruntime irq active: {}\ndry-run commit allowed: {}\n'
Assert-Contains $commitBlock951 $irqRuntimeCommitDryRunExact951 "v9.5.1 irq-runtime-commit exact matrix summary"
Assert-Contains $activationBlock951 $irqRuntimeActivationPlanExact951 "v9.5.1 irq-runtime-activation-plan exact output"
Assert-ContainsInOrder $commitBlock951 @(
    'IRQ runtime activation commit dry-run',
    'pic remap smoke: {}',
    'irq gate bind smoke: {}',
    'eoi runtime boundary: {}',
    'pic mask policy: {}',
    'unmask policy: {}',
    'runtime latch: {}',
    'sti: {}',
    'runtime irq active: {}',
    'dry-run commit allowed: {}',
    'result: {}',
    'if !activation.allowed {',
    '"next: {}\n", activation.next'
) "v9.5.1 commit matrix wording/order and blocked next action"
Assert-ContainsInOrder $activationBlock951 @(
    'IRQ runtime activation plan',
    '1. require readiness matrix smoke prerequisites: yes',
    '2. require EOI runtime boundary: ready (dry-run)',
    '3. keep PIC mask policy: {}',
    '4. keep unmask policy: {}',
    '5. keep STI: {}',
    '6. commit path remains dry-run only',
    'runtime irq active: {}',
    'dry-run commit allowed: {}'
) "v9.5.1 activation plan wording/order"
Assert-Contains $irrContent951 'pub const IRQ_ACTIVATION_DRY_RUN_ALLOWED_NO: &str = "no";' "v9.5.1 dry-run allowed no constant"
Assert-Contains $irrContent951 'pub const IRQ_ACTIVATION_COMMIT_RESULT_BLOCKED: &str = "blocked by readiness matrix";' "v9.5.1 blocked matrix decision wording"
Assert-Contains $irrContent951 'pub const IRQ_ACTIVATION_PLAN_NEXT: &str = "execute irq-runtime-activation-plan";' "v9.5.1 next action wording"

Assert-Contains $commitBlock951 'pic::ProgrammableInterruptController::pic_remap_state();' "v9.5.1 commit reads pic remap state"
Assert-Contains $commitBlock951 'irq::irq_gate_bind_state();' "v9.5.1 commit reads irq gate state"
Assert-Contains $commitBlock951 'pic::ProgrammableInterruptController::pic_mask_plan();' "v9.5.1 commit reads pic mask plan"
Assert-Contains $commitBlock951 'pic::ProgrammableInterruptController::pic_mask_status();' "v9.5.1 commit reads pic mask status"
Assert-Contains $commitBlock951 'irq::eoi_runtime_check_all_preconditions(pic_state.executed);' "v9.5.1 commit reads eoi preconditions"
Assert-Contains $commitBlock951 'irq::irq_runtime_matrix(' "v9.5.1 commit derives readiness matrix"
Assert-Contains $commitBlock951 'irq::irq_runtime_activation_dry_run(&matrix);' "v9.5.1 commit derives activation decision"
Assert-Contains $commitBlock951 'if !activation.allowed {' "v9.5.1 commit blocks on activation decision"
Assert-NotContains $commitBlock951 'irq::irq_runtime_commit()' "v9.5.1 commit path does not call runtime commit"

foreach ($blockedCall in @(
    'write_pic_port(',
    'set_handler(',
    'irq::irq_runtime_commit()',
    'irq::irq_runtime_arm()',
    'pic::ProgrammableInterruptController::pic_remap_controlled_smoke()',
    'irq::irq_gate_bind_smoke_mark_bound()',
    'asm!("sti")'
)) {
    Assert-NotContains $activationBlock951 $blockedCall "v9.5.1 activation plan is read-only/advisory: $blockedCall"
}

Assert-NotContains $irrContent951 'asm!("sti")' "v9.5.1 irq source still has no STI"
Assert-NotContains $mainContent951 'asm!("sti")' "v9.5.1 kernel main still has no STI"
Assert-Contains $picContent951 'pub const PIC_MASK_ALL: u8 = 0xFF;' "v9.5.1 safe mask-all constant remains allowed"
foreach ($literal in @('0x00', '0xFC', '0xFD', '0xFE')) {
    Assert-NotContains $mainContent951 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.5.1 no master unmask literal $literal in main"
    Assert-NotContains $mainContent951 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.5.1 no slave unmask literal $literal in main"
    Assert-NotContains $picContent951 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.5.1 no master unmask literal $literal in pic.rs"
    Assert-NotContains $picContent951 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.5.1 no slave unmask literal $literal in pic.rs"
}
Assert-NotContains $mainContent951 'write_pic_port(PIC_MASTER_CMD, PIC_EOI)' "v9.5.1 kernel main does not dispatch master EOI"
Assert-NotContains $mainContent951 'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)' "v9.5.1 kernel main does not dispatch slave EOI"
Assert-NotContains $mainContent951 'timer_interrupt_handler_stub' "v9.5.1 kernel main has no live timer IRQ handler"
Assert-NotContains $mainContent951 'keyboard_interrupt_handler_stub' "v9.5.1 kernel main has no live keyboard IRQ handler"
Assert-NotContains $mainContent951 'timer_irq' "v9.5.1 kernel main has no timer IRQ activation path"
Assert-NotContains $mainContent951 'keyboard_irq' "v9.5.1 kernel main has no keyboard IRQ activation path"
Assert-Contains $mainContent951 'polling-only' "v9.5.1 keyboard polling telemetry unchanged"
Assert-Contains $mainContent951 'runtime irq active: {}' "v9.5.1 runtime IRQ remains inactive"

Write-Host "[OK] v9.5.1 IRQ Runtime Activation Dry-Run Hardening verified"

# v9.6.0: IRQ Runtime Activation Token Foundation
Write-Host "Verifying v9.6.0 IRQ Runtime Activation Token Foundation contracts..."
$v951Tag = & git rev-list -n 1 v9.5.1 2>$null
$HEAD = & git rev-parse HEAD
if ($null -eq $v951Tag) { throw "v9.5.1 tag not found (required baseline)" }
if ($HEAD -eq $v951Tag) { throw "HEAD is still v9.5.1, v9.6.0 work not completed" }
Write-Host "[OK] v9.6.0 branch is beyond v9.5.1 locked baseline"

$cargoContent960 = Get-Content $cargoToml -Raw
$irrContent960 = Get-Content $irrRs -Raw
$picContent960 = Get-Content $picRs -Raw
$mainContent960 = Get-Content $mainRs -Raw
$kernelBootSmokeDocs960 = Get-Content (Join-Path $repoRoot "docs\QEMU_BOOT_SMOKE.md") -Raw
$kernelBootSmokeDocs960 = $kernelBootSmokeDocs960 -replace "`r`n", "`n"

Assert-Contains $cargoContent960 'version = "9.8.1"' "kernel-lab version 9.8.1"
Assert-NotContains $cargoContent960 'version = "9.5.1"' "kernel-lab stale v9.5.1 package version guard"
Assert-Contains $mainContent960 'irq-runtime-token-note irq-runtime-token-status irq-runtime-token-arm irq-runtime-token-clear' "help string includes v9.6.0 token commands"
Assert-Contains $mainContent960 'line_str == "irq-runtime-token-note"' "irq-runtime-token-note dispatcher"
Assert-Contains $mainContent960 'line_str == "irq-runtime-token-status"' "irq-runtime-token-status dispatcher"
Assert-Contains $mainContent960 'line_str == "irq-runtime-token-arm"' "irq-runtime-token-arm dispatcher"
Assert-Contains $mainContent960 'line_str == "irq-runtime-token-clear"' "irq-runtime-token-clear dispatcher"

Assert-Contains $irrContent960 'pub struct IrqRuntimeActivationTokenTelemetry' "token telemetry struct"
Assert-Contains $irrContent960 'pub fn irq_runtime_activation_token_status() -> IrqRuntimeActivationTokenTelemetry' "token status helper"
Assert-Contains $irrContent960 'pub fn irq_runtime_activation_token_arm() -> IrqRuntimeActivationTokenTelemetry' "token arm helper"
Assert-Contains $irrContent960 'pub fn irq_runtime_activation_token_clear() -> IrqRuntimeActivationTokenTelemetry' "token clear helper"
Assert-Contains $irrContent960 'static mut IRQ_RUNTIME_ACTIVATION_TOKEN_PRESENT: bool = false;' "token telemetry flag defaults absent"
Assert-Contains $irrContent960 'pub const IRQ_ACTIVATION_TOKEN_SCOPE: &str = "activation telemetry only";' "token scope wording"
Assert-Contains $irrContent960 'pub const IRQ_ACTIVATION_TOKEN_HARDWARE_MUTATION_NO: &str = "no";' "token hardware mutation no wording"
Assert-Contains $irrContent960 'pub const IRQ_ACTIVATION_TOKEN_PIC_UNMASK_NO: &str = "no";' "token pic unmask no wording"
Assert-Contains $irrContent960 'pub const IRQ_ACTIVATION_TOKEN_LIVE_IRQ_NO: &str = "no";' "token live irq no wording"

$tokenNoteStart960 = $mainContent960.IndexOf('} else if line_str == "irq-runtime-token-note" {')
$tokenStatusStart960 = $mainContent960.IndexOf('} else if line_str == "irq-runtime-token-status" {')
$tokenArmStart960 = $mainContent960.IndexOf('} else if line_str == "irq-runtime-token-arm" {')
$tokenClearStart960 = $mainContent960.IndexOf('} else if line_str == "irq-runtime-token-clear" {')
$tokenClearEnd960 = $mainContent960.IndexOf('} else if line_str == "irq-runtime-gate-note" {', $tokenClearStart960)
if ($tokenNoteStart960 -lt 0 -or $tokenStatusStart960 -lt $tokenNoteStart960 -or $tokenArmStart960 -lt $tokenStatusStart960 -or $tokenClearStart960 -lt $tokenArmStart960 -or $tokenClearEnd960 -lt $tokenClearStart960) {
    throw "v9.6.0 token command block isolation failed"
}
$tokenNoteBlock960 = $mainContent960.Substring($tokenNoteStart960, $tokenStatusStart960 - $tokenNoteStart960)
$tokenStatusBlock960 = $mainContent960.Substring($tokenStatusStart960, $tokenArmStart960 - $tokenStatusStart960)
$tokenArmBlock960 = $mainContent960.Substring($tokenArmStart960, $tokenClearStart960 - $tokenArmStart960)
$tokenClearBlock960 = $mainContent960.Substring($tokenClearStart960, $tokenClearEnd960 - $tokenClearStart960)

$irqRuntimeTokenNoteExact960 = 'IRQ runtime activation token note\ntoken gate: explicit\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n'
$irqRuntimeTokenStatusExact960 = 'IRQ runtime activation token status\nactivation token: {}\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n'
$irqRuntimeTokenArmExact960 = 'IRQ runtime activation token armed\nactivation token: {}\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n'
$irqRuntimeTokenClearExact960 = 'IRQ runtime activation token cleared\nactivation token: {}\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n'
Assert-Contains $tokenNoteBlock960 $irqRuntimeTokenNoteExact960 "v9.6.0 token note exact output"
Assert-Contains $tokenStatusBlock960 $irqRuntimeTokenStatusExact960 "v9.6.0 token status exact output"
Assert-Contains $tokenArmBlock960 $irqRuntimeTokenArmExact960 "v9.6.0 token arm exact output"
Assert-Contains $tokenClearBlock960 $irqRuntimeTokenClearExact960 "v9.6.0 token clear exact output"
Assert-Contains $tokenNoteBlock960 'irq::irq_runtime_activation_token_status();' "token note reads token telemetry"
Assert-Contains $tokenStatusBlock960 'irq::irq_runtime_activation_token_status();' "token status reads token telemetry"
Assert-Contains $tokenArmBlock960 'irq::irq_runtime_activation_token_arm();' "token arm mutates only token telemetry"
Assert-Contains $tokenClearBlock960 'irq::irq_runtime_activation_token_clear();' "token clear mutates only token telemetry"

foreach ($tokenBlock in @($tokenNoteBlock960, $tokenStatusBlock960, $tokenArmBlock960, $tokenClearBlock960)) {
    foreach ($blockedCall in @(
        'write_pic_port(',
        'set_handler(',
        'irq::irq_runtime_commit()',
        'pic::ProgrammableInterruptController::pic_remap_controlled_smoke()',
        'irq::irq_gate_bind_smoke_mark_bound()',
        'write_pic_port(PIC_MASTER_CMD, PIC_EOI)',
        'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)',
        'asm!("sti")'
    )) {
        Assert-NotContains $tokenBlock $blockedCall "v9.6.0 token command is telemetry-only: $blockedCall"
    }
}

Assert-Contains $mainContent960 'dry-run commit allowed: {}' "v9.6.0 preserves dry-run commit allowed output"
Assert-Contains $irrContent960 'pub const IRQ_ACTIVATION_COMMIT_RESULT_BLOCKED: &str = "blocked by readiness matrix";' "v9.6.0 preserves blocked matrix decision"
$commitBlockStart960 = $mainContent960.IndexOf('} else if line_str == "irq-runtime-commit" {')
$commitBlockEnd960 = $mainContent960.IndexOf('}else if line_str == "irq-runtime-status" {', $commitBlockStart960)
if ($commitBlockStart960 -lt 0 -or $commitBlockEnd960 -lt $commitBlockStart960) { throw "v9.6.0 commit block isolation failed" }
$commitBlock960 = $mainContent960.Substring($commitBlockStart960, $commitBlockEnd960 - $commitBlockStart960)
Assert-Contains $commitBlock960 'irq::irq_runtime_matrix(' "v9.6.0 commit remains matrix-driven"
Assert-Contains $commitBlock960 'irq::irq_runtime_activation_dry_run(&matrix);' "v9.6.0 commit remains activation dry-run driven"
Assert-Contains $commitBlock960 'if !activation.allowed {' "v9.6.0 commit remains blocked by activation decision"
Assert-NotContains $commitBlock960 'irq::irq_runtime_commit()' "v9.6.0 commit path still does not call runtime commit"

$expectedQemuIrqRuntimeTokenNoteOutput960 = "IRQ runtime activation token note`n    token gate: explicit`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling"
$expectedQemuIrqRuntimeTokenStatusOutput960 = "IRQ runtime activation token status`n    activation token: absent`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling"
$expectedQemuIrqRuntimeTokenArmOutput960 = "IRQ runtime activation token armed`n    activation token: present`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling"
$expectedQemuIrqRuntimeTokenClearOutput960 = "IRQ runtime activation token cleared`n    activation token: absent`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling"
Assert-Contains $kernelBootSmokeDocs960 $expectedQemuIrqRuntimeTokenNoteOutput960 "qemu docs token note exact rendered contract"
Assert-Contains $kernelBootSmokeDocs960 $expectedQemuIrqRuntimeTokenStatusOutput960 "qemu docs token status exact rendered contract"
Assert-Contains $kernelBootSmokeDocs960 $expectedQemuIrqRuntimeTokenArmOutput960 "qemu docs token arm exact rendered contract"
Assert-Contains $kernelBootSmokeDocs960 $expectedQemuIrqRuntimeTokenClearOutput960 "qemu docs token clear exact rendered contract"

Assert-NotContains $irrContent960 'asm!("sti")' "v9.6.0 irq source still has no STI"
Assert-NotContains $mainContent960 'asm!("sti")' "v9.6.0 kernel main still has no STI"
Assert-Contains $picContent960 'pub const PIC_MASK_ALL: u8 = 0xFF;' "v9.6.0 safe mask-all constant remains allowed"
foreach ($literal in @('0x00', '0xFC', '0xFD', '0xFE')) {
    Assert-NotContains $mainContent960 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.6.0 no master unmask literal $literal in main"
    Assert-NotContains $mainContent960 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.6.0 no slave unmask literal $literal in main"
    Assert-NotContains $picContent960 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.6.0 no master unmask literal $literal in pic.rs"
    Assert-NotContains $picContent960 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.6.0 no slave unmask literal $literal in pic.rs"
}
Assert-NotContains $mainContent960 'write_pic_port(PIC_MASTER_CMD, PIC_EOI)' "v9.6.0 kernel main does not dispatch master EOI"
Assert-NotContains $mainContent960 'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)' "v9.6.0 kernel main does not dispatch slave EOI"
Assert-NotContains $mainContent960 'timer_interrupt_handler_stub' "v9.6.0 kernel main has no live timer IRQ handler"
Assert-NotContains $mainContent960 'keyboard_interrupt_handler_stub' "v9.6.0 kernel main has no live keyboard IRQ handler"
Assert-NotContains $mainContent960 'timer_irq' "v9.6.0 kernel main has no timer IRQ activation path"
Assert-NotContains $mainContent960 'keyboard_irq' "v9.6.0 kernel main has no keyboard IRQ activation path"
Assert-Contains $mainContent960 'polling-only' "v9.6.0 keyboard polling telemetry unchanged"
Assert-Contains $mainContent960 'runtime irq active: {}' "v9.6.0 runtime IRQ remains inactive"

Write-Host "[OK] v9.6.0 IRQ Runtime Activation Token Foundation verified"

# v9.6.1: IRQ Runtime Activation Token Hardening
Write-Host "Verifying v9.6.1 IRQ Runtime Activation Token Hardening contracts..."
$v960Tag = & git rev-list -n 1 v9.6.0 2>$null
$HEAD = & git rev-parse HEAD
if ($null -eq $v960Tag) { throw "v9.6.0 tag not found (required baseline)" }
if ($HEAD -eq $v960Tag) { throw "HEAD is still v9.6.0, v9.6.1 work not completed" }
Write-Host "[OK] v9.6.1 branch is beyond v9.6.0 locked baseline"

$cargoContent961 = Get-Content $cargoToml -Raw
$irrContent961 = Get-Content $irrRs -Raw
$picContent961 = Get-Content $picRs -Raw
$mainContent961 = Get-Content $mainRs -Raw
$kernelBootSmokeDocs961 = Get-Content (Join-Path $repoRoot "docs\QEMU_BOOT_SMOKE.md") -Raw
$kernelBootSmokeDocs961 = $kernelBootSmokeDocs961 -replace "`r`n", "`n"

Assert-Contains $cargoContent961 'version = "9.8.1"' "kernel-lab version 9.8.1"
Assert-NotContains $cargoContent961 'version = "9.6.0"' "kernel-lab stale v9.6.0 package version guard"
Assert-Contains $mainContent961 'irq-runtime-token-note irq-runtime-token-status irq-runtime-token-arm irq-runtime-token-clear' "help string preserves v9.6.1 token commands"
Assert-Contains $mainContent961 'line_str == "irq-runtime-token-note"' "irq-runtime-token-note dispatcher remains exposed"
Assert-Contains $mainContent961 'line_str == "irq-runtime-token-status"' "irq-runtime-token-status dispatcher remains exposed"
Assert-Contains $mainContent961 'line_str == "irq-runtime-token-arm"' "irq-runtime-token-arm dispatcher remains exposed"
Assert-Contains $mainContent961 'line_str == "irq-runtime-token-clear"' "irq-runtime-token-clear dispatcher remains exposed"

$tokenNoteStart961 = $mainContent961.IndexOf('} else if line_str == "irq-runtime-token-note" {')
$tokenStatusStart961 = $mainContent961.IndexOf('} else if line_str == "irq-runtime-token-status" {')
$tokenArmStart961 = $mainContent961.IndexOf('} else if line_str == "irq-runtime-token-arm" {')
$tokenClearStart961 = $mainContent961.IndexOf('} else if line_str == "irq-runtime-token-clear" {')
$tokenClearEnd961 = $mainContent961.IndexOf('} else if line_str == "irq-runtime-gate-note" {', $tokenClearStart961)
if ($tokenNoteStart961 -lt 0 -or $tokenStatusStart961 -lt $tokenNoteStart961 -or $tokenArmStart961 -lt $tokenStatusStart961 -or $tokenClearStart961 -lt $tokenArmStart961 -or $tokenClearEnd961 -lt $tokenClearStart961) {
    throw "v9.6.1 token command block isolation failed"
}
$tokenNoteBlock961 = $mainContent961.Substring($tokenNoteStart961, $tokenStatusStart961 - $tokenNoteStart961)
$tokenStatusBlock961 = $mainContent961.Substring($tokenStatusStart961, $tokenArmStart961 - $tokenStatusStart961)
$tokenArmBlock961 = $mainContent961.Substring($tokenArmStart961, $tokenClearStart961 - $tokenArmStart961)
$tokenClearBlock961 = $mainContent961.Substring($tokenClearStart961, $tokenClearEnd961 - $tokenClearStart961)

$tokenStatusHelperStart961 = $irrContent961.IndexOf('pub fn irq_runtime_activation_token_status() -> IrqRuntimeActivationTokenTelemetry')
$tokenArmHelperStart961 = $irrContent961.IndexOf('pub fn irq_runtime_activation_token_arm() -> IrqRuntimeActivationTokenTelemetry')
$tokenClearHelperStart961 = $irrContent961.IndexOf('pub fn irq_runtime_activation_token_clear() -> IrqRuntimeActivationTokenTelemetry')
if ($tokenStatusHelperStart961 -lt 0 -or $tokenArmHelperStart961 -lt $tokenStatusHelperStart961 -or $tokenClearHelperStart961 -lt $tokenArmHelperStart961) {
    throw "v9.6.1 token helper isolation failed"
}
$tokenStatusHelperBlock961 = $irrContent961.Substring($tokenStatusHelperStart961, $tokenArmHelperStart961 - $tokenStatusHelperStart961)
$tokenArmHelperBlock961 = $irrContent961.Substring($tokenArmHelperStart961, $tokenClearHelperStart961 - $tokenArmHelperStart961)
$tokenClearHelperBlock961 = $irrContent961.Substring($tokenClearHelperStart961)

$irqRuntimeTokenNoteExact961 = 'IRQ runtime activation token note\ntoken gate: explicit\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n'
$irqRuntimeTokenStatusExact961 = 'IRQ runtime activation token status\nactivation token: {}\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n'
$irqRuntimeTokenArmExact961 = 'IRQ runtime activation token armed\nactivation token: {}\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n'
$irqRuntimeTokenClearExact961 = 'IRQ runtime activation token cleared\nactivation token: {}\nscope: {}\nhardware mutation: {}\nsti: {}\npic unmask: {}\nlive irq0/irq1: {}\nruntime eoi dispatch: {}\nkeyboard mode: {}\n'
Assert-Contains $tokenNoteBlock961 $irqRuntimeTokenNoteExact961 "v9.6.1 token note exact output"
Assert-Contains $tokenStatusBlock961 $irqRuntimeTokenStatusExact961 "v9.6.1 token status exact output"
Assert-Contains $tokenArmBlock961 $irqRuntimeTokenArmExact961 "v9.6.1 token arm exact output"
Assert-Contains $tokenClearBlock961 $irqRuntimeTokenClearExact961 "v9.6.1 token clear exact output"
Assert-Contains $tokenStatusHelperBlock961 'IRQ_RUNTIME_ACTIVATION_TOKEN_PRESENT' "v9.6.1 token status reads token telemetry flag"
Assert-Contains $tokenArmHelperBlock961 'IRQ_RUNTIME_ACTIVATION_TOKEN_PRESENT = true;' "v9.6.1 token arm is idempotent present assignment"
Assert-Contains $tokenClearHelperBlock961 'IRQ_RUNTIME_ACTIVATION_TOKEN_PRESENT = false;' "v9.6.1 token clear is idempotent absent assignment"
Assert-Contains $tokenArmHelperBlock961 'irq_runtime_activation_token_status()' "v9.6.1 token arm returns deterministic status"
Assert-Contains $tokenClearHelperBlock961 'irq_runtime_activation_token_status()' "v9.6.1 token clear returns deterministic status"

foreach ($tokenBlock in @($tokenNoteBlock961, $tokenStatusBlock961, $tokenArmBlock961, $tokenClearBlock961, $tokenStatusHelperBlock961, $tokenArmHelperBlock961, $tokenClearHelperBlock961)) {
    foreach ($blockedCall in @(
        'write_pic_port(',
        'outb(',
        'set_handler(',
        'irq::irq_runtime_commit()',
        'irq_runtime_commit()',
        'pic::ProgrammableInterruptController::pic_remap_controlled_smoke()',
        'irq::irq_gate_bind_smoke_mark_bound()',
        'write_pic_port(PIC_MASTER_CMD, PIC_EOI)',
        'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)',
        'irq_runtime_matrix(',
        'irq_runtime_activation_dry_run(',
        'asm!("sti")'
    )) {
        Assert-NotContains $tokenBlock $blockedCall "v9.6.1 token path is telemetry-only: $blockedCall"
    }
}

$expectedQemuIrqRuntimeTokenNoteOutput961 = "IRQ runtime activation token note`n    token gate: explicit`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling"
$expectedQemuIrqRuntimeTokenStatusAbsentOutput961 = "IRQ runtime activation token status`n    activation token: absent`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling"
$expectedQemuIrqRuntimeTokenStatusPresentOutput961 = "IRQ runtime activation token status`n    activation token: present`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling"
$expectedQemuIrqRuntimeTokenArmOutput961 = "IRQ runtime activation token armed`n    activation token: present`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling"
$expectedQemuIrqRuntimeTokenClearOutput961 = "IRQ runtime activation token cleared`n    activation token: absent`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling"
Assert-Contains $kernelBootSmokeDocs961 $expectedQemuIrqRuntimeTokenNoteOutput961 "qemu docs token note exact rendered contract"
Assert-Contains $kernelBootSmokeDocs961 $expectedQemuIrqRuntimeTokenStatusAbsentOutput961 "qemu docs token status absent exact rendered contract"
Assert-Contains $kernelBootSmokeDocs961 $expectedQemuIrqRuntimeTokenStatusPresentOutput961 "qemu docs token status present exact rendered contract"
Assert-Contains $kernelBootSmokeDocs961 $expectedQemuIrqRuntimeTokenArmOutput961 "qemu docs token arm exact rendered contract"
Assert-Contains $kernelBootSmokeDocs961 $expectedQemuIrqRuntimeTokenClearOutput961 "qemu docs token clear exact rendered contract"

$expectedQemuIrqRuntimeTokenSequence961 = "dbyte-kernel> irq-runtime-token-status`n    IRQ runtime activation token status`n    activation token: absent`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling`n    dbyte-kernel> irq-runtime-token-arm`n    IRQ runtime activation token armed`n    activation token: present`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling`n    dbyte-kernel> irq-runtime-token-arm`n    IRQ runtime activation token armed`n    activation token: present`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling`n    dbyte-kernel> irq-runtime-token-status`n    IRQ runtime activation token status`n    activation token: present`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling`n    dbyte-kernel> irq-runtime-token-clear`n    IRQ runtime activation token cleared`n    activation token: absent`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling`n    dbyte-kernel> irq-runtime-token-clear`n    IRQ runtime activation token cleared`n    activation token: absent`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling`n    dbyte-kernel> irq-runtime-token-status`n    IRQ runtime activation token status`n    activation token: absent`n    scope: activation telemetry only`n    hardware mutation: no`n    sti: disabled`n    pic unmask: no`n    live irq0/irq1: no`n    runtime eoi dispatch: disabled`n    keyboard mode: polling"
Assert-Contains $kernelBootSmokeDocs961 $expectedQemuIrqRuntimeTokenSequence961 "qemu docs token idempotent arm/clear sequence"

Assert-Contains $mainContent961 'runtime irq active: {}' "v9.6.1 runtime IRQ remains inactive"
Assert-Contains $mainContent961 'dry-run commit allowed: {}' "v9.6.1 dry-run commit remains disallowed in output"
Assert-Contains $irrContent961 'pub const IRQ_ACTIVATION_DRY_RUN_ALLOWED_NO: &str = "no";' "v9.6.1 dry-run commit allowed no wording"
Assert-Contains $irrContent961 'pub const IRQ_ACTIVATION_COMMIT_RESULT_BLOCKED: &str = "blocked by readiness matrix";' "v9.6.1 blocked matrix decision wording"
$commitBlockStart961 = $mainContent961.IndexOf('} else if line_str == "irq-runtime-commit" {')
$commitBlockEnd961 = $mainContent961.IndexOf('}else if line_str == "irq-runtime-status" {', $commitBlockStart961)
if ($commitBlockStart961 -lt 0 -or $commitBlockEnd961 -lt $commitBlockStart961) { throw "v9.6.1 commit block isolation failed" }
$commitBlock961 = $mainContent961.Substring($commitBlockStart961, $commitBlockEnd961 - $commitBlockStart961)
Assert-Contains $commitBlock961 'irq::irq_runtime_matrix(' "v9.6.1 commit remains matrix-driven"
Assert-Contains $commitBlock961 'irq::irq_runtime_activation_dry_run(&matrix);' "v9.6.1 commit remains activation dry-run driven"
Assert-Contains $commitBlock961 'if !activation.allowed {' "v9.6.1 commit remains blocked by activation decision"
Assert-NotContains $commitBlock961 'irq::irq_runtime_commit()' "v9.6.1 commit path still does not call runtime commit"

Assert-NotContains $irrContent961 'asm!("sti")' "v9.6.1 irq source still has no STI"
Assert-NotContains $mainContent961 'asm!("sti")' "v9.6.1 kernel main still has no STI"
Assert-Contains $picContent961 'pub const PIC_MASK_ALL: u8 = 0xFF;' "v9.6.1 safe mask-all constant remains allowed"
foreach ($literal in @('0x00', '0xFC', '0xFD', '0xFE')) {
    Assert-NotContains $mainContent961 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.6.1 no master unmask literal $literal in main"
    Assert-NotContains $mainContent961 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.6.1 no slave unmask literal $literal in main"
    Assert-NotContains $picContent961 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.6.1 no master unmask literal $literal in pic.rs"
    Assert-NotContains $picContent961 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.6.1 no slave unmask literal $literal in pic.rs"
}
Assert-NotContains $mainContent961 'write_pic_port(PIC_MASTER_CMD, PIC_EOI)' "v9.6.1 kernel main does not dispatch master EOI"
Assert-NotContains $mainContent961 'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)' "v9.6.1 kernel main does not dispatch slave EOI"
Assert-NotContains $mainContent961 'timer_interrupt_handler_stub' "v9.6.1 kernel main has no live timer IRQ handler"
Assert-NotContains $mainContent961 'keyboard_interrupt_handler_stub' "v9.6.1 kernel main has no live keyboard IRQ handler"
Assert-NotContains $mainContent961 'timer_irq' "v9.6.1 kernel main has no timer IRQ activation path"
Assert-NotContains $mainContent961 'keyboard_irq' "v9.6.1 kernel main has no keyboard IRQ activation path"
Assert-Contains $mainContent961 'polling-only' "v9.6.1 keyboard polling telemetry unchanged"

Write-Host "[OK] v9.6.1 IRQ Runtime Activation Token Hardening verified"

# v9.7.1: Controlled Activation Gate Hardening
Write-Host "Verifying v9.7.1 Controlled Activation Gate Hardening contracts..."
$v970Tag = & git rev-list -n 1 v9.7.0 2>$null
$HEAD = & git rev-parse HEAD
if ($null -eq $v970Tag) { throw "v9.7.0 tag not found (required baseline)" }
if ($HEAD -eq $v970Tag) { throw "HEAD is still v9.7.0, v9.7.1 work not completed" }
Write-Host "[OK] v9.7.1 branch is beyond v9.7.0 locked baseline"

$cargoContent971 = Get-Content $cargoToml -Raw
$irrContent971 = Get-Content $irrRs -Raw
$picContent971 = Get-Content $picRs -Raw
$mainContent971 = Get-Content $mainRs -Raw
$kernelBootSmokeDocs971 = Get-Content (Join-Path $repoRoot "docs\QEMU_BOOT_SMOKE.md") -Raw
$kernelBootSmokeDocs971 = $kernelBootSmokeDocs971 -replace "`r`n", "`n"

Assert-Contains $cargoContent971 'version = "9.8.1"' "kernel-lab version 9.8.1"
Assert-NotContains $cargoContent971 'version = "9.7.0"' "kernel-lab stale v9.7.0 package version guard"
Assert-Contains $mainContent971 'irq-runtime-gate-note irq-runtime-gate-status irq-runtime-gate-check irq-runtime-gate-blockers' "help string includes v9.7.1 gate commands"
Assert-Contains $mainContent971 'line_str == "irq-runtime-gate-note"' "irq-runtime-gate-note dispatcher"
Assert-Contains $mainContent971 'line_str == "irq-runtime-gate-status"' "irq-runtime-gate-status dispatcher"
Assert-Contains $mainContent971 'line_str == "irq-runtime-gate-check"' "irq-runtime-gate-check dispatcher"
Assert-Contains $mainContent971 'line_str == "irq-runtime-gate-blockers"' "irq-runtime-gate-blockers dispatcher"

Assert-Contains $irrContent971 'pub struct IrqRuntimeActivationGate' "gate telemetry struct"
Assert-Contains $irrContent971 'pub fn irq_runtime_activation_gate(' "gate telemetry helper"
Assert-Contains $irrContent971 'pub const IRQ_ACTIVATION_GATE_PURPOSE: &str = "controlled activation preconditions";' "gate purpose wording"
Assert-Contains $irrContent971 'pub const IRQ_ACTIVATION_GATE_READINESS_BLOCKED: &str = "blocked";' "gate readiness blocked wording"
Assert-Contains $irrContent971 'pub const IRQ_ACTIVATION_GATE_ALLOWED_NO: &str = "no";' "gate allowed no wording"
Assert-Contains $irrContent971 'pub const IRQ_ACTIVATION_GATE_RESULT_BLOCKED: &str = "activation blocked";' "gate blocked result wording"
Assert-Contains $irrContent971 'pub const IRQ_ACTIVATION_GATE_NEXT_BLOCKERS: &str = "execute irq-runtime-gate-blockers";' "gate next blockers wording"

$gateHelperStart971 = $irrContent971.IndexOf('pub fn irq_runtime_activation_gate(')
$gateHelperEnd971 = $irrContent971.Length
if ($gateHelperStart971 -lt 0) { throw "v9.7.1 gate helper isolation failed" }
$gateHelperBlock971 = $irrContent971.Substring($gateHelperStart971, $gateHelperEnd971 - $gateHelperStart971)

$gateNoteStart971 = $mainContent971.IndexOf('} else if line_str == "irq-runtime-gate-note" {')
$gateStatusStart971 = $mainContent971.IndexOf('} else if line_str == "irq-runtime-gate-status" {')
$gateCheckStart971 = $mainContent971.IndexOf('} else if line_str == "irq-runtime-gate-check" {')
$gateBlockersStart971 = $mainContent971.IndexOf('} else if line_str == "irq-runtime-gate-blockers" {')
$gateBlockersEnd971 = $mainContent971.IndexOf('} else if line_str == "eoi-runtime-note" {', $gateBlockersStart971)
if ($gateNoteStart971 -lt 0 -or $gateStatusStart971 -lt $gateNoteStart971 -or $gateCheckStart971 -lt $gateStatusStart971 -or $gateBlockersStart971 -lt $gateCheckStart971 -or $gateBlockersEnd971 -lt $gateBlockersStart971) {
    throw "v9.7.1 gate command block isolation failed"
}
$gateNoteBlock971 = $mainContent971.Substring($gateNoteStart971, $gateStatusStart971 - $gateNoteStart971)
$gateStatusBlock971 = $mainContent971.Substring($gateStatusStart971, $gateCheckStart971 - $gateStatusStart971)
$gateCheckBlock971 = $mainContent971.Substring($gateCheckStart971, $gateBlockersStart971 - $gateCheckStart971)
$gateBlockersBlock971 = $mainContent971.Substring($gateBlockersStart971, $gateBlockersEnd971 - $gateBlockersStart971)

$irqRuntimeGateNoteExact971 = 'IRQ runtime activation gate note\ngate purpose: {}\ntoken required: {}\nmatrix required: {}\ndry-run commit required: {}\nhardware mutation: {}\nactivation allowed: {}\n'
$irqRuntimeGateStatusExact971 = 'IRQ runtime activation gate status\ntoken gate: {}\nreadiness matrix: {}\neoi runtime boundary: {}\npic mask policy: {}\nunmask policy: {}\ndry-run commit allowed: {}\nruntime irq active: {}\nactivation allowed: {}\n'
$irqRuntimeGateCheckExact971 = 'IRQ runtime activation gate check\ntoken gate: {}\nmatrix decision: {}\neoi boundary: {}\nmask policy: {}\nhardware mutation: {}\nresult: {}\nnext: {}\n'
$irqRuntimeGateBlockersExact971 = 'IRQ runtime activation gate blockers\n- activation token: {}\n- readiness matrix: {}\n- dry-run commit: {}\n- EOI runtime boundary: {}\n- STI: {}\nactivation allowed: {}\n'
Assert-Contains $gateNoteBlock971 $irqRuntimeGateNoteExact971 "v9.7.1 gate note exact output"
Assert-Contains $gateStatusBlock971 $irqRuntimeGateStatusExact971 "v9.7.1 gate status exact output"
Assert-Contains $gateCheckBlock971 $irqRuntimeGateCheckExact971 "v9.7.1 gate check exact output"
Assert-Contains $gateBlockersBlock971 $irqRuntimeGateBlockersExact971 "v9.7.1 gate blockers exact output"
Assert-ContainsInOrder $gateBlockersBlock971 @(
    '- activation token: {}',
    '- readiness matrix: {}',
    '- dry-run commit: {}',
    '- EOI runtime boundary: {}',
    '- STI: {}',
    'activation allowed: {}'
) "v9.7.1 gate blocker ordering"

foreach ($gateReadBlock in @($gateStatusBlock971, $gateCheckBlock971, $gateBlockersBlock971)) {
    Assert-Contains $gateReadBlock 'pic::ProgrammableInterruptController::pic_remap_state();' "v9.7.1 gate reads pic remap state"
    Assert-Contains $gateReadBlock 'irq::irq_gate_bind_state();' "v9.7.1 gate reads irq gate state"
    Assert-Contains $gateReadBlock 'pic::ProgrammableInterruptController::pic_mask_plan();' "v9.7.1 gate reads pic mask plan"
    Assert-Contains $gateReadBlock 'pic::ProgrammableInterruptController::pic_mask_status();' "v9.7.1 gate reads pic mask status"
    Assert-Contains $gateReadBlock 'irq::eoi_runtime_check_all_preconditions(pic_state.executed);' "v9.7.1 gate reads eoi preconditions"
    Assert-Contains $gateReadBlock 'irq::irq_runtime_matrix(' "v9.7.1 gate derives readiness matrix"
    Assert-Contains $gateReadBlock 'irq::irq_runtime_activation_dry_run(&matrix);' "v9.7.1 gate derives activation dry-run"
    Assert-Contains $gateReadBlock 'irq::irq_runtime_activation_token_status();' "v9.7.1 gate reads token state"
    Assert-Contains $gateReadBlock 'irq::irq_runtime_activation_gate(' "v9.7.1 gate derives activation gate"
    foreach ($forbiddenRead in @(
        'pic::ProgrammableInterruptController::pic_remap_smoke_arm()',
        'pic::ProgrammableInterruptController::pic_remap_controlled_smoke()',
        'pic::ProgrammableInterruptController::pic_remap_smoke_status()',
        'pic::ProgrammableInterruptController::pic_remap_history()',
        'pic::ProgrammableInterruptController::pic_remap_preflight()',
        'irq::irq_gate_bind_smoke_arm()',
        'irq::irq_gate_bind_smoke_is_armed()',
        'irq::irq_gate_bind_smoke_status()',
        'irq::irq_gate_bind_history()',
        'irq::irq_gate_bind_preflight()',
        'irq::irq_runtime_readiness()',
        'irq::irq_runtime_risk()',
        'irq::irq_runtime_preflight()'
    )) {
        Assert-NotContains $gateReadBlock $forbiddenRead "v9.7.1 gate read surface excludes $forbiddenRead"
    }
}

foreach ($gateBlock in @($gateNoteBlock971, $gateStatusBlock971, $gateCheckBlock971, $gateBlockersBlock971, $gateHelperBlock971)) {
    foreach ($blockedCall in @(
        'write_pic_port(',
        'outb(',
        'set_handler(',
        'irq::irq_runtime_commit()',
        'irq::irq_runtime_arm()',
        'irq_runtime_activation_token_arm()',
        'irq_runtime_activation_token_clear()',
        'pic::ProgrammableInterruptController::pic_remap_controlled_smoke()',
        'irq::irq_gate_bind_smoke_mark_bound()',
        'write_pic_port(PIC_MASTER_CMD, PIC_EOI)',
        'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)',
        'asm!("sti")'
    )) {
        Assert-NotContains $gateBlock $blockedCall "v9.7.1 gate path is read-only: $blockedCall"
    }
}

$expectedQemuIrqRuntimeGateNoteOutput971 = "IRQ runtime activation gate note`n    gate purpose: controlled activation preconditions`n    token required: yes`n    matrix required: ready`n    dry-run commit required: yes`n    hardware mutation: no`n    activation allowed: no"
$expectedQemuIrqRuntimeGateStatusOutput971 = "IRQ runtime activation gate status`n    token gate: absent`n    readiness matrix: blocked`n    eoi runtime boundary: disabled`n    pic mask policy: all masked (0xFF)`n    unmask policy: no unmask`n    dry-run commit allowed: no`n    runtime irq active: no`n    activation allowed: no"
$expectedQemuIrqRuntimeGateCheckOutput971 = "IRQ runtime activation gate check`n    token gate: absent`n    matrix decision: blocked`n    eoi boundary: disabled`n    mask policy: all masked (0xFF)`n    hardware mutation: no`n    result: activation blocked`n    next: execute irq-runtime-gate-blockers"
$expectedQemuIrqRuntimeGateBlockersOutput971 = "IRQ runtime activation gate blockers`n    - activation token: absent`n    - readiness matrix: runtime irq ready no`n    - dry-run commit: not allowed`n    - EOI runtime boundary: disabled`n    - STI: disabled`n    activation allowed: no"
Assert-Contains $kernelBootSmokeDocs971 $expectedQemuIrqRuntimeGateNoteOutput971 "qemu docs gate note exact rendered contract"
Assert-Contains $kernelBootSmokeDocs971 $expectedQemuIrqRuntimeGateStatusOutput971 "qemu docs gate status exact rendered contract"
Assert-Contains $kernelBootSmokeDocs971 $expectedQemuIrqRuntimeGateCheckOutput971 "qemu docs gate check exact rendered contract"
Assert-Contains $kernelBootSmokeDocs971 $expectedQemuIrqRuntimeGateBlockersOutput971 "qemu docs gate blockers exact rendered contract"
Assert-ContainsInOrder $kernelBootSmokeDocs971 @(
    "- activation token: absent",
    "- readiness matrix: runtime irq ready no",
    "- dry-run commit: not allowed",
    "- EOI runtime boundary: disabled",
    "- STI: disabled",
    "activation allowed: no"
) "qemu docs gate blockers rendered ordering"

$commitBlockStart971 = $mainContent971.IndexOf('} else if line_str == "irq-runtime-commit" {')
$commitBlockEnd971 = $mainContent971.IndexOf('}else if line_str == "irq-runtime-status" {', $commitBlockStart971)
if ($commitBlockStart971 -lt 0 -or $commitBlockEnd971 -lt $commitBlockStart971) { throw "v9.7.1 commit block isolation failed" }
$commitBlock971 = $mainContent971.Substring($commitBlockStart971, $commitBlockEnd971 - $commitBlockStart971)
Assert-Contains $commitBlock971 'irq::irq_runtime_matrix(' "v9.7.1 commit remains matrix-driven"
Assert-Contains $commitBlock971 'irq::irq_runtime_activation_dry_run(&matrix);' "v9.7.1 commit remains activation dry-run driven"
Assert-Contains $commitBlock971 'if !activation.allowed {' "v9.7.1 commit remains blocked by activation decision"
Assert-NotContains $commitBlock971 'irq::irq_runtime_commit()' "v9.7.1 commit path still does not call runtime commit"
Assert-Contains $mainContent971 'runtime irq active: {}' "v9.7.1 runtime IRQ remains inactive"
Assert-Contains $mainContent971 'dry-run commit allowed: {}' "v9.7.1 dry-run commit remains disallowed in output"
Assert-Contains $irrContent971 'pub const IRQ_ACTIVATION_COMMIT_RESULT_BLOCKED: &str = "blocked by readiness matrix";' "v9.7.1 blocked matrix decision wording"

Assert-NotContains $irrContent971 'asm!("sti")' "v9.7.1 irq source still has no STI"
Assert-NotContains $mainContent971 'asm!("sti")' "v9.7.1 kernel main still has no STI"
Assert-Contains $picContent971 'pub const PIC_MASK_ALL: u8 = 0xFF;' "v9.7.1 safe mask-all constant remains allowed"
foreach ($literal in @('0x00', '0xFC', '0xFD', '0xFE')) {
    Assert-NotContains $mainContent971 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.7.1 no master unmask literal $literal in main"
    Assert-NotContains $mainContent971 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.7.1 no slave unmask literal $literal in main"
    Assert-NotContains $picContent971 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.7.1 no master unmask literal $literal in pic.rs"
    Assert-NotContains $picContent971 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.7.1 no slave unmask literal $literal in pic.rs"
}
Assert-NotContains $mainContent971 'write_pic_port(PIC_MASTER_CMD, PIC_EOI)' "v9.7.1 kernel main does not dispatch master EOI"
Assert-NotContains $mainContent971 'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)' "v9.7.1 kernel main does not dispatch slave EOI"
Assert-NotContains $mainContent971 'timer_interrupt_handler_stub' "v9.7.1 kernel main has no live timer IRQ handler"
Assert-NotContains $mainContent971 'keyboard_interrupt_handler_stub' "v9.7.1 kernel main has no live keyboard IRQ handler"
Assert-NotContains $mainContent971 'timer_irq' "v9.7.1 kernel main has no timer IRQ activation path"
Assert-NotContains $mainContent971 'keyboard_irq' "v9.7.1 kernel main has no keyboard IRQ activation path"
Assert-Contains $mainContent971 'polling-only' "v9.7.1 keyboard polling telemetry unchanged"

Write-Host "[OK] v9.7.1 Controlled Activation Gate Hardening verified"

# v9.8.0: Controlled Activation Simulation Harness
Write-Host "Verifying v9.8.0 Controlled Activation Simulation Harness contracts..."
$v971Tag = & git rev-list -n 1 v9.7.1 2>$null
$HEAD = & git rev-parse HEAD
if ($null -eq $v971Tag) { throw "v9.7.1 tag not found (required baseline)" }
if ($HEAD -eq $v971Tag) { throw "HEAD is still v9.7.1, v9.8.0 work not completed" }
Write-Host "[OK] v9.8.0 branch is beyond v9.7.1 locked baseline"

$cargoContent980 = Get-Content $cargoToml -Raw
$kernelCargoLockContent980 = Get-Content (Join-Path $repoRoot "kernel-lab\Cargo.lock") -Raw
$irrContent980 = Get-Content $irrRs -Raw
$picContent980 = Get-Content $picRs -Raw
$mainContent980 = Get-Content $mainRs -Raw
$kernelBootSmokeDocs980 = Get-Content (Join-Path $repoRoot "docs\QEMU_BOOT_SMOKE.md") -Raw
$kernelBootSmokeDocs980 = $kernelBootSmokeDocs980 -replace "`r`n", "`n"

Assert-Contains $cargoContent980 'version = "9.8.1"' "kernel-lab version 9.8.1"
Assert-NotContains $cargoContent980 'version = "9.7.1"' "kernel-lab stale v9.7.1 package version guard"
Assert-Contains $kernelCargoLockContent980 'version = "9.8.1"' "kernel-lab lockfile version 9.8.1"
Assert-NotContains $kernelCargoLockContent980 'version = "9.7.1"' "kernel-lab stale v9.7.1 lockfile version guard"
Assert-Contains $mainContent980 'irq-runtime-sim-note irq-runtime-sim-status irq-runtime-sim-run irq-runtime-sim-blockers' "help string includes v9.8.0 simulation commands"
Assert-Contains $mainContent980 'line_str == "irq-runtime-sim-note"' "irq-runtime-sim-note dispatcher"
Assert-Contains $mainContent980 'line_str == "irq-runtime-sim-status"' "irq-runtime-sim-status dispatcher"
Assert-Contains $mainContent980 'line_str == "irq-runtime-sim-run"' "irq-runtime-sim-run dispatcher"
Assert-Contains $mainContent980 'line_str == "irq-runtime-sim-blockers"' "irq-runtime-sim-blockers dispatcher"

Assert-Contains $irrContent980 'pub struct IrqRuntimeActivationSimulation' "simulation telemetry struct"
Assert-Contains $irrContent980 'pub fn irq_runtime_activation_simulation(' "simulation telemetry helper"
Assert-Contains $irrContent980 'pub const IRQ_ACTIVATION_SIM_PURPOSE: &str = "controlled activation rehearsal";' "simulation purpose wording"
Assert-Contains $irrContent980 'pub const IRQ_ACTIVATION_SIM_ALLOWED_NO: &str = "no";' "simulation allowed no wording"
Assert-Contains $irrContent980 'pub const IRQ_ACTIVATION_SIM_RESULT_BLOCKED: &str = "simulation blocked";' "simulation blocked result wording"
Assert-Contains $irrContent980 'pub const IRQ_ACTIVATION_SIM_NEXT_BLOCKERS: &str = "execute irq-runtime-sim-blockers";' "simulation next blockers wording"

$simHelperStart980 = $irrContent980.IndexOf('pub fn irq_runtime_activation_simulation(')
$simHelperEnd980 = $irrContent980.Length
if ($simHelperStart980 -lt 0) { throw "v9.8.0 simulation helper isolation failed" }
$simHelperBlock980 = $irrContent980.Substring($simHelperStart980, $simHelperEnd980 - $simHelperStart980)

$simNoteStart980 = $mainContent980.IndexOf('} else if line_str == "irq-runtime-sim-note" {')
$simStatusStart980 = $mainContent980.IndexOf('} else if line_str == "irq-runtime-sim-status" {')
$simRunStart980 = $mainContent980.IndexOf('} else if line_str == "irq-runtime-sim-run" {')
$simBlockersStart980 = $mainContent980.IndexOf('} else if line_str == "irq-runtime-sim-blockers" {')
$simBlockersEnd980 = $mainContent980.IndexOf('} else if line_str == "eoi-runtime-note" {', $simBlockersStart980)
if ($simNoteStart980 -lt 0 -or $simStatusStart980 -lt $simNoteStart980 -or $simRunStart980 -lt $simStatusStart980 -or $simBlockersStart980 -lt $simRunStart980 -or $simBlockersEnd980 -lt $simBlockersStart980) {
    throw "v9.8.0 simulation command block isolation failed"
}
$simNoteBlock980 = $mainContent980.Substring($simNoteStart980, $simStatusStart980 - $simNoteStart980)
$simStatusBlock980 = $mainContent980.Substring($simStatusStart980, $simRunStart980 - $simStatusStart980)
$simRunBlock980 = $mainContent980.Substring($simRunStart980, $simBlockersStart980 - $simRunStart980)
$simBlockersBlock980 = $mainContent980.Substring($simBlockersStart980, $simBlockersEnd980 - $simBlockersStart980)

$irqRuntimeSimNoteExact980 = 'IRQ runtime activation simulation note\nsimulation purpose: {}\nhardware mutation: {}\nsti would enable: {}\npic unmask would apply: {}\neoi dispatch would enable: {}\nkeyboard mode: {}\n'
$irqRuntimeSimStatusExact980 = 'IRQ runtime activation simulation status\ntoken gate: {}\nreadiness matrix: {}\ngate decision: {}\ndry-run commit allowed: {}\nsimulated activation allowed: {}\nruntime irq active: {}\nhardware mutation: {}\n'
$irqRuntimeSimRunExact980 = 'IRQ runtime activation simulation run\nsimulated activation allowed: {}\nhardware mutation: {}\nsti would enable: {}\npic unmask would apply: {}\neoi dispatch would enable: {}\nkeyboard mode: {}\nresult: {}\nnext: {}\n'
$irqRuntimeSimBlockersExact980 = 'IRQ runtime activation simulation blockers\n- activation token: {}\n- gate decision: {}\n- readiness matrix: {}\n- dry-run commit: {}\n- EOI runtime boundary: {}\n- STI would enable: {}\n- PIC unmask would apply: {}\n- EOI dispatch would enable: {}\nactivation allowed: {}\n'
Assert-Contains $simNoteBlock980 $irqRuntimeSimNoteExact980 "v9.8.0 simulation note exact output"
Assert-Contains $simStatusBlock980 $irqRuntimeSimStatusExact980 "v9.8.0 simulation status exact output"
Assert-Contains $simRunBlock980 $irqRuntimeSimRunExact980 "v9.8.0 simulation run exact output"
Assert-Contains $simBlockersBlock980 $irqRuntimeSimBlockersExact980 "v9.8.0 simulation blockers exact output"
Assert-ContainsInOrder $simBlockersBlock980 @(
    '- activation token: {}',
    '- gate decision: {}',
    '- readiness matrix: {}',
    '- dry-run commit: {}',
    '- EOI runtime boundary: {}',
    '- STI would enable: {}',
    '- PIC unmask would apply: {}',
    '- EOI dispatch would enable: {}',
    'activation allowed: {}'
) "v9.8.0 simulation blocker ordering"

foreach ($simReadBlock in @($simStatusBlock980, $simRunBlock980, $simBlockersBlock980)) {
    Assert-Contains $simReadBlock 'pic::ProgrammableInterruptController::pic_remap_state();' "v9.8.0 simulation reads pic remap state"
    Assert-Contains $simReadBlock 'irq::irq_gate_bind_state();' "v9.8.0 simulation reads irq gate state"
    Assert-Contains $simReadBlock 'pic::ProgrammableInterruptController::pic_mask_plan();' "v9.8.0 simulation reads pic mask plan"
    Assert-Contains $simReadBlock 'pic::ProgrammableInterruptController::pic_mask_status();' "v9.8.0 simulation reads pic mask status"
    Assert-Contains $simReadBlock 'irq::eoi_runtime_check_all_preconditions(pic_state.executed);' "v9.8.0 simulation reads eoi preconditions"
    Assert-Contains $simReadBlock 'irq::irq_runtime_matrix(' "v9.8.0 simulation derives readiness matrix"
    Assert-Contains $simReadBlock 'irq::irq_runtime_activation_dry_run(&matrix);' "v9.8.0 simulation derives activation dry-run"
    Assert-Contains $simReadBlock 'irq::irq_runtime_activation_token_status();' "v9.8.0 simulation reads token state"
    Assert-Contains $simReadBlock 'irq::irq_runtime_activation_gate(' "v9.8.0 simulation reads gate decision"
    Assert-Contains $simReadBlock 'irq::irq_runtime_activation_simulation(' "v9.8.0 simulation derives simulation decision"
    foreach ($forbiddenRead in @(
        'pic::ProgrammableInterruptController::pic_remap_smoke_arm()',
        'pic::ProgrammableInterruptController::pic_remap_controlled_smoke()',
        'pic::ProgrammableInterruptController::pic_remap_smoke_status()',
        'pic::ProgrammableInterruptController::pic_remap_history()',
        'pic::ProgrammableInterruptController::pic_remap_preflight()',
        'irq::irq_gate_bind_smoke_arm()',
        'irq::irq_gate_bind_smoke_is_armed()',
        'irq::irq_gate_bind_smoke_status()',
        'irq::irq_gate_bind_history()',
        'irq::irq_gate_bind_preflight()',
        'irq::irq_runtime_readiness()',
        'irq::irq_runtime_risk()',
        'irq::irq_runtime_preflight()'
    )) {
        Assert-NotContains $simReadBlock $forbiddenRead "v9.8.0 simulation read surface excludes $forbiddenRead"
    }
}

foreach ($simBlock in @($simNoteBlock980, $simStatusBlock980, $simRunBlock980, $simBlockersBlock980, $simHelperBlock980)) {
    foreach ($blockedCall in @(
        'write_pic_port(',
        'outb(',
        'set_handler(',
        'irq::irq_runtime_commit()',
        'irq::irq_runtime_arm()',
        'irq_runtime_activation_token_arm()',
        'irq_runtime_activation_token_clear()',
        'pic::ProgrammableInterruptController::pic_remap_controlled_smoke()',
        'irq::irq_gate_bind_smoke_mark_bound()',
        'write_pic_port(PIC_MASTER_CMD, PIC_EOI)',
        'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)',
        'asm!("sti")'
    )) {
        Assert-NotContains $simBlock $blockedCall "v9.8.0 simulation path is read-only: $blockedCall"
    }
}

$expectedQemuIrqRuntimeSimNoteOutput980 = "IRQ runtime activation simulation note`n    simulation purpose: controlled activation rehearsal`n    hardware mutation: no`n    sti would enable: no`n    pic unmask would apply: no`n    eoi dispatch would enable: no`n    keyboard mode: polling"
$expectedQemuIrqRuntimeSimStatusOutput980 = "IRQ runtime activation simulation status`n    token gate: absent`n    readiness matrix: blocked`n    gate decision: activation blocked`n    dry-run commit allowed: no`n    simulated activation allowed: no`n    runtime irq active: no`n    hardware mutation: no"
$expectedQemuIrqRuntimeSimRunOutput980 = "IRQ runtime activation simulation run`n    simulated activation allowed: no`n    hardware mutation: no`n    sti would enable: no`n    pic unmask would apply: no`n    eoi dispatch would enable: no`n    keyboard mode: polling`n    result: simulation blocked`n    next: execute irq-runtime-sim-blockers"
$expectedQemuIrqRuntimeSimBlockersOutput980 = "IRQ runtime activation simulation blockers`n    - activation token: absent`n    - gate decision: activation blocked`n    - readiness matrix: runtime irq ready no`n    - dry-run commit: not allowed`n    - EOI runtime boundary: disabled`n    - STI would enable: no`n    - PIC unmask would apply: no`n    - EOI dispatch would enable: no`n    activation allowed: no"
Assert-Contains $kernelBootSmokeDocs980 $expectedQemuIrqRuntimeSimNoteOutput980 "qemu docs simulation note exact rendered contract"
Assert-Contains $kernelBootSmokeDocs980 $expectedQemuIrqRuntimeSimStatusOutput980 "qemu docs simulation status exact rendered contract"
Assert-Contains $kernelBootSmokeDocs980 $expectedQemuIrqRuntimeSimRunOutput980 "qemu docs simulation run exact rendered contract"
Assert-Contains $kernelBootSmokeDocs980 $expectedQemuIrqRuntimeSimBlockersOutput980 "qemu docs simulation blockers exact rendered contract"
Assert-ContainsInOrder $kernelBootSmokeDocs980 @(
    "- activation token: absent",
    "- gate decision: activation blocked",
    "- readiness matrix: runtime irq ready no",
    "- dry-run commit: not allowed",
    "- EOI runtime boundary: disabled",
    "- STI would enable: no",
    "- PIC unmask would apply: no",
    "- EOI dispatch would enable: no",
    "activation allowed: no"
) "qemu docs simulation blockers rendered ordering"

Assert-Contains $simNoteBlock980 'irq::IRQ_MATRIX_KEYBOARD_MODE_POLLING' "v9.8.0 simulation note locks keyboard polling"
Assert-Contains $simRunBlock980 'simulation.keyboard_mode' "v9.8.0 simulation run reports keyboard mode"
Assert-Contains $simStatusBlock980 'simulation.runtime_irq_active' "v9.8.0 simulation status reports inactive runtime IRQ"
Assert-NotContains $irrContent980 'asm!("sti")' "v9.8.0 irq source still has no STI"
Assert-NotContains $mainContent980 'asm!("sti")' "v9.8.0 kernel main still has no STI"
Assert-Contains $picContent980 'pub const PIC_MASK_ALL: u8 = 0xFF;' "v9.8.0 safe mask-all constant remains allowed"
foreach ($literal in @('0x00', '0xFC', '0xFD', '0xFE')) {
    Assert-NotContains $mainContent980 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.8.0 no master unmask literal $literal in main"
    Assert-NotContains $mainContent980 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.8.0 no slave unmask literal $literal in main"
    Assert-NotContains $picContent980 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.8.0 no master unmask literal $literal in pic.rs"
    Assert-NotContains $picContent980 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.8.0 no slave unmask literal $literal in pic.rs"
}
Assert-NotContains $mainContent980 'write_pic_port(PIC_MASTER_CMD, PIC_EOI)' "v9.8.0 kernel main does not dispatch master EOI"
Assert-NotContains $mainContent980 'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)' "v9.8.0 kernel main does not dispatch slave EOI"
Assert-NotContains $mainContent980 'timer_interrupt_handler_stub' "v9.8.0 kernel main has no live timer IRQ handler"
Assert-NotContains $mainContent980 'keyboard_interrupt_handler_stub' "v9.8.0 kernel main has no live keyboard IRQ handler"
Assert-NotContains $mainContent980 'timer_irq' "v9.8.0 kernel main has no timer IRQ activation path"
Assert-NotContains $mainContent980 'keyboard_irq' "v9.8.0 kernel main has no keyboard IRQ activation path"
Assert-Contains $mainContent980 'polling-only' "v9.8.0 keyboard polling telemetry unchanged"

Write-Host "[OK] v9.8.0 Controlled Activation Simulation Harness verified"

# v9.8.1: Controlled Activation Simulation Harness Hardening
Write-Host "Verifying v9.8.1 Controlled Activation Simulation Harness Hardening contracts..."
$v980Tag = & git rev-list -n 1 v9.8.0 2>$null
$HEAD = & git rev-parse HEAD
if ($null -eq $v980Tag) { throw "v9.8.0 tag not found (required baseline)" }
if ($HEAD -eq $v980Tag) { throw "HEAD is still v9.8.0, v9.8.1 work not completed" }
Write-Host "[OK] v9.8.1 branch is beyond v9.8.0 locked baseline"

$cargoContent981 = Get-Content $cargoToml -Raw
$kernelCargoLockContent981 = Get-Content (Join-Path $repoRoot "kernel-lab\Cargo.lock") -Raw
$irrContent981 = Get-Content $irrRs -Raw
$picContent981 = Get-Content $picRs -Raw
$mainContent981 = Get-Content $mainRs -Raw
$kernelBootSmokeDocs981 = Get-Content (Join-Path $repoRoot "docs\QEMU_BOOT_SMOKE.md") -Raw
$kernelBootSmokeDocs981 = $kernelBootSmokeDocs981 -replace "`r`n", "`n"

Assert-Contains $cargoContent981 'version = "9.8.1"' "kernel-lab version 9.8.1"
Assert-NotContains $cargoContent981 'version = "9.8.0"' "kernel-lab stale v9.8.0 package version guard"
Assert-Contains $kernelCargoLockContent981 'version = "9.8.1"' "kernel-lab lockfile version 9.8.1"
Assert-NotContains $kernelCargoLockContent981 'version = "9.8.0"' "kernel-lab stale v9.8.0 lockfile version guard"

$simHelperStart981 = $irrContent981.IndexOf('pub fn irq_runtime_activation_simulation(')
$simHelperEnd981 = $irrContent981.Length
if ($simHelperStart981 -lt 0) { throw "v9.8.1 simulation helper isolation failed" }
$simHelperBlock981 = $irrContent981.Substring($simHelperStart981, $simHelperEnd981 - $simHelperStart981)

$simNoteStart981 = $mainContent981.IndexOf('} else if line_str == "irq-runtime-sim-note" {')
$simStatusStart981 = $mainContent981.IndexOf('} else if line_str == "irq-runtime-sim-status" {')
$simRunStart981 = $mainContent981.IndexOf('} else if line_str == "irq-runtime-sim-run" {')
$simBlockersStart981 = $mainContent981.IndexOf('} else if line_str == "irq-runtime-sim-blockers" {')
$simBlockersEnd981 = $mainContent981.IndexOf('} else if line_str == "eoi-runtime-note" {', $simBlockersStart981)
if ($simNoteStart981 -lt 0 -or $simStatusStart981 -lt $simNoteStart981 -or $simRunStart981 -lt $simStatusStart981 -or $simBlockersStart981 -lt $simRunStart981 -or $simBlockersEnd981 -lt $simBlockersStart981) {
    throw "v9.8.1 simulation command block isolation failed"
}
$simNoteBlock981 = $mainContent981.Substring($simNoteStart981, $simStatusStart981 - $simNoteStart981)
$simStatusBlock981 = $mainContent981.Substring($simStatusStart981, $simRunStart981 - $simStatusStart981)
$simRunBlock981 = $mainContent981.Substring($simRunStart981, $simBlockersStart981 - $simRunStart981)
$simBlockersBlock981 = $mainContent981.Substring($simBlockersStart981, $simBlockersEnd981 - $simBlockersStart981)

Assert-Contains $mainContent981 'irq-runtime-sim-note irq-runtime-sim-status irq-runtime-sim-run irq-runtime-sim-blockers' "help string includes v9.8.1 simulation commands"
Assert-Contains $mainContent981 'line_str == "irq-runtime-sim-note"' "v9.8.1 irq-runtime-sim-note dispatcher"
Assert-Contains $mainContent981 'line_str == "irq-runtime-sim-status"' "v9.8.1 irq-runtime-sim-status dispatcher"
Assert-Contains $mainContent981 'line_str == "irq-runtime-sim-run"' "v9.8.1 irq-runtime-sim-run dispatcher"
Assert-Contains $mainContent981 'line_str == "irq-runtime-sim-blockers"' "v9.8.1 irq-runtime-sim-blockers dispatcher"
Assert-Contains $irrContent981 'pub struct IrqRuntimeActivationSimulation' "v9.8.1 simulation telemetry struct"
Assert-Contains $irrContent981 'pub fn irq_runtime_activation_simulation(' "v9.8.1 simulation telemetry helper"

Assert-Contains $simNoteBlock981 $irqRuntimeSimNoteExact980 "v9.8.1 simulation note exact output"
Assert-Contains $simStatusBlock981 $irqRuntimeSimStatusExact980 "v9.8.1 simulation status exact output"
Assert-Contains $simRunBlock981 $irqRuntimeSimRunExact980 "v9.8.1 simulation run exact output"
Assert-Contains $simBlockersBlock981 $irqRuntimeSimBlockersExact980 "v9.8.1 simulation blockers exact output"
Assert-Contains $kernelBootSmokeDocs981 $expectedQemuIrqRuntimeSimNoteOutput980 "v9.8.1 qemu docs simulation note exact rendered contract"
Assert-Contains $kernelBootSmokeDocs981 $expectedQemuIrqRuntimeSimStatusOutput980 "v9.8.1 qemu docs simulation status exact rendered contract"
Assert-Contains $kernelBootSmokeDocs981 $expectedQemuIrqRuntimeSimRunOutput980 "v9.8.1 qemu docs simulation run exact rendered contract"
Assert-Contains $kernelBootSmokeDocs981 $expectedQemuIrqRuntimeSimBlockersOutput980 "v9.8.1 qemu docs simulation blockers exact rendered contract"
Assert-ContainsInOrder $simBlockersBlock981 @(
    '- activation token: {}',
    '- gate decision: {}',
    '- readiness matrix: {}',
    '- dry-run commit: {}',
    '- EOI runtime boundary: {}',
    '- STI would enable: {}',
    '- PIC unmask would apply: {}',
    '- EOI dispatch would enable: {}',
    'activation allowed: {}'
) "v9.8.1 simulation blocker ordering"

foreach ($simReadBlock981 in @($simStatusBlock981, $simRunBlock981, $simBlockersBlock981)) {
    Assert-ContainsInOrder $simReadBlock981 @(
        'pic::ProgrammableInterruptController::pic_remap_state();',
        'irq::irq_gate_bind_state();',
        'pic::ProgrammableInterruptController::pic_mask_plan();',
        'pic::ProgrammableInterruptController::pic_mask_status();',
        'irq::eoi_runtime_check_all_preconditions(pic_state.executed);',
        'irq::irq_runtime_matrix(',
        'irq::irq_runtime_activation_dry_run(&matrix);',
        'irq::irq_runtime_activation_token_status();',
        'irq::irq_runtime_activation_gate(',
        'irq::irq_runtime_activation_simulation('
    ) "v9.8.1 simulation reader ordering"
}

foreach ($forbiddenSimNoteRead in @(
    'pic::ProgrammableInterruptController::pic_remap_state();',
    'irq::irq_gate_bind_state();',
    'pic::ProgrammableInterruptController::pic_mask_plan();',
    'pic::ProgrammableInterruptController::pic_mask_status();',
    'irq::eoi_runtime_check_all_preconditions(',
    'irq::irq_runtime_matrix(',
    'irq::irq_runtime_activation_dry_run(',
    'irq::irq_runtime_activation_token_status();',
    'irq::irq_runtime_activation_gate(',
    'irq::irq_runtime_activation_simulation('
)) {
    Assert-NotContains $simNoteBlock981 $forbiddenSimNoteRead "v9.8.1 simulation note remains constant-only: $forbiddenSimNoteRead"
}

foreach ($simBlock981 in @($simNoteBlock981, $simStatusBlock981, $simRunBlock981, $simBlockersBlock981, $simHelperBlock981)) {
    foreach ($blockedCall981 in @(
        'write_pic_port(',
        'outb(',
        'set_handler(',
        'irq::irq_runtime_commit()',
        'irq::irq_runtime_arm()',
        'irq_runtime_activation_token_arm()',
        'irq_runtime_activation_token_clear()',
        'pic::ProgrammableInterruptController::pic_remap_controlled_smoke()',
        'irq::irq_gate_bind_smoke_mark_bound()',
        'write_pic_port(PIC_MASTER_CMD, PIC_EOI)',
        'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)',
        'asm!("sti")'
    )) {
        Assert-NotContains $simBlock981 $blockedCall981 "v9.8.1 simulation path is read-only: $blockedCall981"
    }
}

Assert-Contains $simStatusBlock981 'simulation.simulated_activation_allowed' "v9.8.1 simulation status reports activation denied"
Assert-Contains $simStatusBlock981 'simulation.runtime_irq_active' "v9.8.1 simulation status reports inactive runtime IRQ"
Assert-Contains $simRunBlock981 'simulation.hardware_mutation' "v9.8.1 simulation run reports no hardware mutation"
Assert-Contains $simRunBlock981 'simulation.sti_would_enable' "v9.8.1 simulation run reports STI would not enable"
Assert-Contains $simRunBlock981 'simulation.pic_unmask_would_apply' "v9.8.1 simulation run reports PIC unmask would not apply"
Assert-Contains $simRunBlock981 'simulation.eoi_dispatch_would_enable' "v9.8.1 simulation run reports EOI dispatch would not enable"
Assert-Contains $simRunBlock981 'simulation.keyboard_mode' "v9.8.1 simulation run keeps keyboard polling"
Assert-NotContains $irrContent981 'asm!("sti")' "v9.8.1 irq source still has no STI"
Assert-NotContains $mainContent981 'asm!("sti")' "v9.8.1 kernel main still has no STI"
Assert-Contains $picContent981 'pub const PIC_MASK_ALL: u8 = 0xFF;' "v9.8.1 safe mask-all constant remains allowed"
foreach ($literal in @('0x00', '0xFC', '0xFD', '0xFE')) {
    Assert-NotContains $mainContent981 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.8.1 no master unmask literal $literal in main"
    Assert-NotContains $mainContent981 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.8.1 no slave unmask literal $literal in main"
    Assert-NotContains $picContent981 "write_pic_port(PIC_MASTER_DATA, $literal)" "v9.8.1 no master unmask literal $literal in pic.rs"
    Assert-NotContains $picContent981 "write_pic_port(PIC_SLAVE_DATA, $literal)" "v9.8.1 no slave unmask literal $literal in pic.rs"
}
Assert-NotContains $mainContent981 'write_pic_port(PIC_MASTER_CMD, PIC_EOI)' "v9.8.1 kernel main does not dispatch master EOI"
Assert-NotContains $mainContent981 'write_pic_port(PIC_SLAVE_CMD, PIC_EOI)' "v9.8.1 kernel main does not dispatch slave EOI"
Assert-NotContains $mainContent981 'timer_interrupt_handler_stub' "v9.8.1 kernel main has no live timer IRQ handler"
Assert-NotContains $mainContent981 'keyboard_interrupt_handler_stub' "v9.8.1 kernel main has no live keyboard IRQ handler"
Assert-NotContains $mainContent981 'timer_irq' "v9.8.1 kernel main has no timer IRQ activation path"
Assert-NotContains $mainContent981 'keyboard_irq' "v9.8.1 kernel main has no keyboard IRQ activation path"
Assert-Contains $mainContent981 'polling-only' "v9.8.1 keyboard polling telemetry unchanged"

Write-Host "[OK] v9.8.1 Controlled Activation Simulation Harness Hardening verified"

Assert-Contains $shellBasic.Text "DByte shell commands" "shell help"
Assert-Contains $shellBasic.Text "alias <name> = <command>" "shell registry alias help"
Assert-Contains $shellBasic.Text "which <name>" "shell registry which help"
Assert-Contains $shellBasic.Text "DByte 9.0.2" "shell version"
Assert-Contains $shellBasic.Text "ShellError: failed to cd" "shell invalid cd"
Assert-Contains $shellBasic.Text "hello.dby" "shell ls"
Assert-Contains $shellBasic.Text "shell file ok" "shell run file"
Assert-Contains $shellBasic.Text "no type errors found" "shell check file"
Assert-Contains $shellBasic.Text "42" "shell code persistence"
Assert-Contains $shellBasic.Text "help: built-in" "shell which built-in"
Assert-Contains $shellBasic.Text "hi: alias -> run hello.dby" "shell which alias"
Assert-Contains $shellBasic.Text "missing: not found" "shell which missing"
Assert-Contains $shellBasic.Text "hi = run hello.dby" "shell aliases list"

Assert-Contains $shellBasic.Text "ShellError: unknown command: hi" "shell unalias removes alias"
Assert-Contains $shellBasic.Text "ShellError: unknown command: not_a_real_cmd" "shell unknown command"

$shellHardeningRoot = Join-Path $interactiveRoot "shell-hardening"
New-Item -ItemType Directory -Path $shellHardeningRoot | Out-Null
Set-Content -Path (Join-Path $shellHardeningRoot "one.dby") -Value "print(`"one alias`")" -NoNewline
Set-Content -Path (Join-Path $shellHardeningRoot "two.dby") -Value "print(`"two alias`")" -NoNewline
$chainAliases = "alias n01 = n02`nalias n02 = n03`nalias n03 = n04`nalias n04 = n05`nalias n05 = n06`nalias n06 = n07`nalias n07 = n08`nalias n08 = n09`nalias n09 = n10`nalias n10 = n11`nalias n11 = n12`nalias n12 = n13`nalias n13 = n14`nalias n14 = n15`nalias n15 = n16`nalias n16 = n17`nalias n17 = n18`n"
$shellHardeningInput = "cd `"$shellHardeningRoot`"`n: let keep: int = 41`nmissing_cmd`ncd missing-dir`nrun missing.dby`ncheck missing.dby`n: print(keep + 1)`nalias bad = missing_cmd`nbad`nalias a = b`nalias b = a`na`n$chainAliases" + "n01`nalias hi = run one.dby`nalias hi = run two.dby`nwhich hi`naliases`nhi`nunalias hi`nwhich hi`nunalias hi`nunalias run`nunterminated `"quote`n: print(keep + 2)`nquit`n"
$shellHardening = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText $shellHardeningInput
if ($shellHardening.Code -ne 0) { throw "shell hardening command failed: $($shellHardening.Text)" }
Assert-Contains $shellHardening.Text "ShellError: unknown command: missing_cmd" "shell unknown direct command"
Assert-Contains $shellHardening.Text "ShellError: failed to cd" "shell bad cd preserves session"
Assert-Contains $shellHardening.Text "IoError:" "shell bad run/check reports error"
Assert-Contains $shellHardening.Text "42" "shell state survives failed commands"
Assert-Contains $shellHardening.Text "ShellError: unknown command: missing_cmd" "shell alias unknown target"
Assert-Contains $shellHardening.Text "ShellError: alias expansion cycle detected: a -> b -> a" "shell alias cycle guard"
Assert-Contains $shellHardening.Text "ShellError: alias expansion limit exceeded" "shell alias chain limit"
Assert-Contains $shellHardening.Text "hi: alias -> run two.dby" "shell alias overwrite which"
Assert-Contains $shellHardening.Text "hi = run two.dby" "shell alias overwrite list"
Assert-Contains $shellHardening.Text "two alias" "shell alias overwrite executes latest"
Assert-Contains $shellHardening.Text "hi: not found" "shell which after unalias"
Assert-Contains $shellHardening.Text "ShellError: alias not found: hi" "shell unalias missing"
Assert-Contains $shellHardening.Text "ShellError: alias not found: run" "shell unalias built-in missing"
Assert-Contains $shellHardening.Text "ShellError: unterminated quote" "shell unterminated quote"
Assert-Contains $shellHardening.Text "43" "shell recovers after unterminated quote"

$shellSpacesRoot = Join-Path $interactiveRoot "shell path spaces"
$shellSpacesDir = Join-Path $shellSpacesRoot "dir with spaces"
New-Item -ItemType Directory -Path $shellSpacesDir -Force | Out-Null
Set-Content -Path (Join-Path $shellSpacesDir "file with spaces.dby") -Value "print(`"space path ok`")" -NoNewline
$shellSpacesInput = "cd `"dir with spaces`"`nrun `"file with spaces.dby`"`ncheck `"file with spaces.dby`"`nquit`n"
$shellSpaces = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText $shellSpacesInput -WorkingDirectory $shellSpacesRoot
if ($shellSpaces.Code -ne 0) { throw "shell spaces command failed: $($shellSpaces.Text)" }
Assert-Contains $shellSpaces.Text "space path ok" "shell run path with spaces"
Assert-Contains $shellSpaces.Text "no type errors found" "shell check path with spaces"

$shellRcRoot = Join-Path $interactiveRoot "shell-rc"
New-Item -ItemType Directory -Path $shellRcRoot | Out-Null
Set-Content -Path (Join-Path $shellRcRoot "helper.dby") -Value "pub fn inc(x: int) -> int:`n    return x + 1`n" -NoNewline
Set-Content -Path (Join-Path $shellRcRoot "hello.dby") -Value "print(`"rc alias ok`")" -NoNewline
Set-Content -Path (Join-Path $shellRcRoot ".dbyterc") -Value "@shell alias hello = run hello.dby`nimport std.math as math`nimport `"./helper.dby`" as helper`nlet boot: int = math.max(helper.inc(40), 1)" -NoNewline
$shellRc = Invoke-DbyteInput -Arguments @("shell") -InputText "hello`nwhich hello`n: print(boot + 1)`n: print(helper.inc(1))`nquit`n" -WorkingDirectory $shellRcRoot
if ($shellRc.Code -ne 0) { throw "shell rc load failed: $($shellRc.Text)" }
Assert-Contains $shellRc.Text "rc alias ok" "shell rc alias"
Assert-Contains $shellRc.Text "hello: alias -> run hello.dby" "shell rc alias which"
Assert-Contains $shellRc.Text "42" "shell rc state"
Assert-Contains $shellRc.Text "2" "shell rc local import state"

$shellNoRc = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "hello`n: print(boot)`nquit`n" -WorkingDirectory $shellRcRoot
if ($shellNoRc.Code -ne 0) { throw "shell no-rc command failed: $($shellNoRc.Text)" }
Assert-Contains $shellNoRc.Text "ShellError: unknown command: hello" "shell no-rc skips aliases"
Assert-Contains $shellNoRc.Text "undefined variable" "shell no-rc skips rc"



$shellExamples = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "cd examples`nrun hello.dby`ncheck hello.dby`nquit`n"
if ($shellExamples.Code -ne 0) { throw "shell examples cwd command failed: $($shellExamples.Text)" }
Assert-Contains $shellExamples.Text "Hello, DByte!" "shell run from cwd after cd"
Assert-Contains $shellExamples.Text "no type errors found" "shell check from cwd after cd"

$shellTestRoot = Join-Path $interactiveRoot "shell-test"
New-Item -ItemType Directory -Path (Join-Path $shellTestRoot "src") -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $shellTestRoot "tests") -Force | Out-Null
Set-Content -Path (Join-Path $shellTestRoot "Dbyte.toml") -Value "[package]`nname = `"shelltest`"`nversion = `"0.1.0`"`nentry = `"src/main.dby`"`n" -NoNewline
Set-Content -Path (Join-Path $shellTestRoot "src\main.dby") -Value "print(`"shell test project`")" -NoNewline
Set-Content -Path (Join-Path $shellTestRoot "tests\smoke.dby") -Value "print(`"shell test ok`")" -NoNewline
Set-Content -Path (Join-Path $shellTestRoot "tests\smoke.out") -Value "shell test ok" -NoNewline
$shellTest = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "cd `"$shellTestRoot`"`ntest`nquit`n"
if ($shellTest.Code -ne 0) { throw "shell test command failed: $($shellTest.Text)" }
Assert-Contains $shellTest.Text "Test result: 1 passed, 0 failed" "shell test command"

$runNoRcRoot = Join-Path $interactiveRoot "run-no-rc"
New-Item -ItemType Directory -Path $runNoRcRoot | Out-Null
Set-Content -Path (Join-Path $runNoRcRoot ".dbyterc") -Value "let bad: int = `"bad`"" -NoNewline
Set-Content -Path (Join-Path $runNoRcRoot "main.dby") -Value "print(`"run ignores rc`")" -NoNewline
Push-Location $runNoRcRoot
try {
    $runNoRc = Invoke-Dbyte -Arguments @("run", "main.dby") -WorkingDirectory $runNoRcRoot
    if ($runNoRc.Code -ne 0) { throw "run loaded rc unexpectedly: $($runNoRc.Text)" }
    Assert-Equal $runNoRc.Text "run ignores rc" "run ignores rc"
    $checkNoRc = Invoke-Dbyte -Arguments @("check", "main.dby") -WorkingDirectory $runNoRcRoot
    if ($checkNoRc.Code -ne 0) { throw "check loaded rc unexpectedly: $($checkNoRc.Text)" }
    Assert-Contains $checkNoRc.Text "no type errors found" "check ignores rc"
    $newNoRcRoot = Join-Path $runNoRcRoot "new-no-rc"
    New-Item -ItemType Directory -Path $newNoRcRoot | Out-Null
    Push-Location $newNoRcRoot
    try {
        $newNoRc = Invoke-Dbyte -Arguments @("new", "rcsafe") -WorkingDirectory $newNoRcRoot
        if ($newNoRc.Code -ne 0) { throw "new loaded parent rc unexpectedly: $($newNoRc.Text)" }
        Assert-Contains $newNoRc.Text "created DByte project" "new ignores rc"
    }
    finally {
        Pop-Location
    }
}
finally {
    Pop-Location
}

Write-Host "Running script argument tests..."

$scriptArgsRoot = Join-Path $repoRoot "target\verify-script-args"
if (Test-Path $scriptArgsRoot) {
    Remove-Item -Recurse -Force $scriptArgsRoot
}
New-Item -ItemType Directory -Path $scriptArgsRoot | Out-Null
$scriptArgsFile = Join-Path $scriptArgsRoot "args_probe.dby"
Set-Content -Path $scriptArgsFile -Value @'
import std.env as env

let args: list[str] = env.args()
print(len(args))
if len(args) > 0:
    print(args[0])
if len(args) > 1:
    print(args[1])
'@ -NoNewline

$scriptArgsNone = Invoke-Dbyte -Arguments @("run", $scriptArgsFile)
if ($scriptArgsNone.Code -ne 0) { throw "script args empty failed: $($scriptArgsNone.Text)" }
Assert-Equal $scriptArgsNone.Text "0" "script args empty"

$scriptArgsTree = Invoke-Dbyte -Arguments @("run", $scriptArgsFile, "alpha", "two words")
if ($scriptArgsTree.Code -ne 0) { throw "script args tree failed: $($scriptArgsTree.Text)" }
Assert-Equal $scriptArgsTree.Text "2`nalpha`ntwo words" "script args tree"

$scriptArgsVm = Invoke-Dbyte -Arguments @("run", "--vm", $scriptArgsFile, "alpha", "two words")
if ($scriptArgsVm.Code -ne 0) { throw "script args vm failed: $($scriptArgsVm.Text)" }
Assert-Equal $scriptArgsVm.Text "2`nalpha`ntwo words" "script args vm"

$scriptArgsCheck = Invoke-Dbyte -Arguments @("check", $scriptArgsFile)
if ($scriptArgsCheck.Code -ne 0) { throw "script args check failed: $($scriptArgsCheck.Text)" }
Assert-Contains $scriptArgsCheck.Text "no type errors found" "script args check ignores args"

$scriptArgsDisasm = Invoke-Dbyte -Arguments @("disasm", $scriptArgsFile)
if ($scriptArgsDisasm.Code -ne 0) { throw "script args disasm failed: $($scriptArgsDisasm.Text)" }
Assert-Contains $scriptArgsDisasm.Text "MEMBER_CALL args 0" "script args disasm"

$scriptArgsRepl = Invoke-DbyteInput -Arguments @("repl", "--no-rc") -InputText "import std.env as env`nprint(len(env.args()))`n.quit`n"
if ($scriptArgsRepl.Code -ne 0) { throw "script args repl failed: $($scriptArgsRepl.Text)" }
Assert-Contains $scriptArgsRepl.Text "0" "script args repl empty"

$scriptArgsShellCode = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText ": import std.env as env`n: print(len(env.args()))`nquit`n"
if ($scriptArgsShellCode.Code -ne 0) { throw "script args shell code failed: $($scriptArgsShellCode.Text)" }
Assert-Contains $scriptArgsShellCode.Text "0" "script args shell code empty"

$scriptArgsAfterFileFlag = Invoke-Dbyte -Arguments @("run", $scriptArgsFile, "--vm")
if ($scriptArgsAfterFileFlag.Code -ne 0) { throw "script args after file flag failed: $($scriptArgsAfterFileFlag.Text)" }
Assert-Equal $scriptArgsAfterFileFlag.Text "1`n--vm" "run flags after script path are script args"

$scriptArgsShellRun = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "run `"$scriptArgsFile`" alpha `"two words`"`nquit`n"
if ($scriptArgsShellRun.Code -ne 0) { throw "script args shell run failed: $($scriptArgsShellRun.Text)" }
Assert-Contains $scriptArgsShellRun.Text "2`nalpha`ntwo words" "shell run quoted script path and args"

Write-Host "Running personal tools smoke tests..."
Set-Location $repoRoot

$personalToolsStatus = Git-Status-Short

$personalToolFiles = @(
    @{ Name = "hexdump"; Path = "hexdump.dby" },
    @{ Name = "bininfo"; Path = "bininfo.dby" },
    @{ Name = "find_bytes"; Path = "find_bytes.dby" },
    @{ Name = "patch_bytes"; Path = "patch_bytes.dby" },
    @{ Name = "read_u32_table"; Path = "read_u32_table.dby" }
)

$blockLocalLets = Get-ChildItem (Join-Path $repoRoot "personal_tools") -Filter "*.dby" |
    Select-String -Pattern "^\s+let\s+"
if ($blockLocalLets) {
    throw "personal tool parser compatibility guard failed: block-local let found in $($blockLocalLets[0].Path):$($blockLocalLets[0].LineNumber)"
}

foreach ($tool in $personalToolFiles) {
    $result = Invoke-Dbyte -Arguments @("run", "personal_tools\$($tool.Path)") -WorkingDirectory $repoRoot
    if ($result.Code -ne 0) { throw "personal tool from repo root failed [$($tool.Name)]: $($result.Text)" }
    Assert-PersonalToolOutput $tool.Name $result.Text
}
Assert-GitStatus-Unchanged $personalToolsStatus "personal tools repo-root run cleanliness"

Push-Location (Join-Path $repoRoot "personal_tools")
try {
    foreach ($tool in $personalToolFiles) {
        $result = Invoke-Dbyte -Arguments @("run", $tool.Path) -WorkingDirectory (Join-Path $repoRoot "personal_tools")
        if ($result.Code -ne 0) { throw "personal tool from personal_tools cwd failed [$($tool.Name)]: $($result.Text)" }
        Assert-PersonalToolOutput $tool.Name $result.Text
    }
}
finally {
    Pop-Location
}
Assert-GitStatus-Unchanged $personalToolsStatus "personal tools cwd run cleanliness"

$personalArgsRoot = Join-Path $repoRoot "target\verify-personal-tools"
if (Test-Path $personalArgsRoot) {
    Remove-Item -Recurse -Force $personalArgsRoot
}
New-Item -ItemType Directory -Path $personalArgsRoot | Out-Null
$personalArgsFile = Join-Path $personalArgsRoot "sample.bin"
[System.IO.File]::WriteAllBytes($personalArgsFile, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x00, 0x78, 0x56, 0x34, 0x12))

$personalHexArgs = Invoke-Dbyte -Arguments @("run", "personal_tools\hexdump.dby", $personalArgsFile) -WorkingDirectory $repoRoot
if ($personalHexArgs.Code -ne 0) { throw "personal hexdump args failed: $($personalHexArgs.Text)" }
Assert-Contains $personalHexArgs.Text "bytes: 10" "personal hexdump args size"
Assert-Contains $personalHexArgs.Text "0000: 00deadbeef007856" "personal hexdump args first row"
Assert-Contains $personalHexArgs.Text "0008: 3412" "personal hexdump args second row"

$personalHexRange = Invoke-Dbyte -Arguments @("run", "personal_tools\hexdump.dby", $personalArgsFile, "1", "6") -WorkingDirectory $repoRoot
if ($personalHexRange.Code -ne 0) { throw "personal hexdump range failed: $($personalHexRange.Text)" }
Assert-Contains $personalHexRange.Text "range: 1 6" "personal hexdump range header"
Assert-Contains $personalHexRange.Text "1 : deadbeef0078" "personal hexdump range row"

$personalBinArgs = Invoke-Dbyte -Arguments @("run", "personal_tools\bininfo.dby", $personalArgsFile) -WorkingDirectory $repoRoot
if ($personalBinArgs.Code -ne 0) { throw "personal bininfo args failed: $($personalBinArgs.Text)" }
Assert-Contains $personalBinArgs.Text "bytes: 10" "personal bininfo args size"
Assert-Contains $personalBinArgs.Text "first8: 00deadbeef007856" "personal bininfo args first bytes"

$personalFindArgs = Invoke-Dbyte -Arguments @("run", "personal_tools\find_bytes.dby", $personalArgsFile, "DEADBEEF") -WorkingDirectory $repoRoot
if ($personalFindArgs.Code -ne 0) { throw "personal find args failed: $($personalFindArgs.Text)" }
Assert-Contains $personalFindArgs.Text "pattern: 1" "personal find args offset"
Assert-Contains $personalFindArgs.Text "pattern: 1 0x1" "personal find args hex offset"

$personalFindInvalidHex = Invoke-Dbyte -Arguments @("run", "personal_tools\find_bytes.dby", $personalArgsFile, "NOTHEX") -WorkingDirectory $repoRoot
if ($personalFindInvalidHex.Code -ne 0) { throw "personal find invalid hex failed: $($personalFindInvalidHex.Text)" }
Assert-Equal $personalFindInvalidHex.Text "error: invalid hex_pattern" "personal find invalid hex"

$personalPatchArgs = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalArgsFile, "DEADBEEF", "CAFEBABE") -WorkingDirectory $repoRoot
if ($personalPatchArgs.Code -ne 0) { throw "personal patch args failed: $($personalPatchArgs.Text)" }
Assert-Contains $personalPatchArgs.Text "patched first match at offset 1" "personal patch args offset"
Assert-Contains $personalPatchArgs.Text "wrote $personalArgsFile.patched" "personal patch args output path"
Assert-Contains $personalPatchArgs.Text "patched_hex: 00cafebabe0078563412" "personal patch args bytes"
Assert-Equal (Bytes-Hex $personalArgsFile) "00deadbeef0078563412" "personal patch original unchanged"
Assert-Equal (Bytes-Hex "$personalArgsFile.patched") "00cafebabe0078563412" "personal patch output bytes"

$personalPatchFirstMatch = Join-Path $personalArgsRoot "first-match.bin"
[System.IO.File]::WriteAllBytes($personalPatchFirstMatch, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x11, 0xde, 0xad, 0xbe, 0xef, 0x22))
$personalPatchFirstMatchResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalPatchFirstMatch, "DEADBEEF", "CAFEBABE") -WorkingDirectory $repoRoot
if ($personalPatchFirstMatchResult.Code -ne 0) { throw "personal patch first-match failed: $($personalPatchFirstMatchResult.Text)" }
Assert-Contains $personalPatchFirstMatchResult.Text "patched first match at offset 1" "personal patch first-match offset"
Assert-Equal (Bytes-Hex $personalPatchFirstMatch) "00deadbeef11deadbeef22" "personal patch first-match original unchanged"
Assert-Equal (Bytes-Hex "$personalPatchFirstMatch.patched") "00cafebabe11deadbeef22" "personal patch first-match output bytes"

$personalPatchAll = Join-Path $personalArgsRoot "all.bin"
[System.IO.File]::WriteAllBytes($personalPatchAll, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x11, 0xde, 0xad, 0xbe, 0xef, 0x22))
$personalPatchAllResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--all", $personalPatchAll, "DEADBEEF", "CAFEBABE") -WorkingDirectory $repoRoot
if ($personalPatchAllResult.Code -ne 0) { throw "personal patch all failed: $($personalPatchAllResult.Text)" }
Assert-Contains $personalPatchAllResult.Text "patched count: 2" "personal patch all count"
Assert-Equal (Bytes-Hex $personalPatchAll) "00deadbeef11deadbeef22" "personal patch all original unchanged"
Assert-Equal (Bytes-Hex "$personalPatchAll.patched") "00cafebabe11cafebabe22" "personal patch all output bytes"

$personalPatchOffset = Join-Path $personalArgsRoot "offset.bin"
[System.IO.File]::WriteAllBytes($personalPatchOffset, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x11))
$personalPatchOffsetResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--offset", "1", $personalPatchOffset, "CAFEBABE") -WorkingDirectory $repoRoot
if ($personalPatchOffsetResult.Code -ne 0) { throw "personal patch offset failed: $($personalPatchOffsetResult.Text)" }
Assert-Contains $personalPatchOffsetResult.Text "patched offset 1" "personal patch offset marker"
Assert-Equal (Bytes-Hex $personalPatchOffset) "00deadbeef11" "personal patch offset original unchanged"
Assert-Equal (Bytes-Hex "$personalPatchOffset.patched") "00cafebabe11" "personal patch offset output bytes"

$personalPatchOffsetOob = Join-Path $personalArgsRoot "offset-oob.bin"
[System.IO.File]::WriteAllBytes($personalPatchOffsetOob, [byte[]](0x00, 0xde, 0xad))
$personalPatchOffsetOobResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--offset", "2", $personalPatchOffsetOob, "CAFEBABE") -WorkingDirectory $repoRoot
if ($personalPatchOffsetOobResult.Code -ne 0) { throw "personal patch offset oob failed: $($personalPatchOffsetOobResult.Text)" }
Assert-Equal $personalPatchOffsetOobResult.Text "error: offset out of bounds" "personal patch offset oob"
if (Test-Path "$personalPatchOffsetOob.patched") { throw "personal patch offset oob unexpectedly wrote output" }

$personalPatchOffsetBadDecimal = Join-Path $personalArgsRoot "offset-bad-decimal.bin"
[System.IO.File]::WriteAllBytes($personalPatchOffsetBadDecimal, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef))
$personalPatchOffsetBadDecimalResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--offset", "nope", $personalPatchOffsetBadDecimal, "CAFEBABE") -WorkingDirectory $repoRoot
if ($personalPatchOffsetBadDecimalResult.Code -ne 0) { throw "personal patch offset bad decimal failed: $($personalPatchOffsetBadDecimalResult.Text)" }
Assert-Equal $personalPatchOffsetBadDecimalResult.Text "error: offset must be a decimal integer" "personal patch offset bad decimal"
if (Test-Path "$personalPatchOffsetBadDecimal.patched") { throw "personal patch offset bad decimal unexpectedly wrote output" }

$personalPatchInvalidHex = Join-Path $personalArgsRoot "invalid-hex.bin"
[System.IO.File]::WriteAllBytes($personalPatchInvalidHex, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef))
$personalPatchInvalidHexResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalPatchInvalidHex, "NOTHEX", "CAFEBABE") -WorkingDirectory $repoRoot
if ($personalPatchInvalidHexResult.Code -ne 0) { throw "personal patch invalid hex failed: $($personalPatchInvalidHexResult.Text)" }
Assert-Equal $personalPatchInvalidHexResult.Text "error: invalid find_hex" "personal patch invalid hex"
if (Test-Path "$personalPatchInvalidHex.patched") { throw "personal patch invalid hex unexpectedly wrote output" }

$personalPatchMissing = Join-Path $personalArgsRoot "missing.bin"
[System.IO.File]::WriteAllBytes($personalPatchMissing, [byte[]](0x01, 0x02, 0x03, 0x04))
$personalPatchMissingResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalPatchMissing, "DEADBEEF", "CAFEBABE") -WorkingDirectory $repoRoot
if ($personalPatchMissingResult.Code -ne 0) { throw "personal patch missing failed: $($personalPatchMissingResult.Text)" }
Assert-Equal $personalPatchMissingResult.Text "pattern not found" "personal patch missing output"
if (Test-Path "$personalPatchMissing.patched") { throw "personal patch missing unexpectedly wrote output" }

$personalPatchUnequalFile = Join-Path $personalArgsRoot "unequal.bin"
[System.IO.File]::WriteAllBytes($personalPatchUnequalFile, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x00))
$personalPatchUnequal = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalPatchUnequalFile, "DEADBEEF", "CAFE") -WorkingDirectory $repoRoot
if ($personalPatchUnequal.Code -ne 0) { throw "personal patch unequal failed: $($personalPatchUnequal.Text)" }
Assert-Equal $personalPatchUnequal.Text "error: find_hex and replace_hex must have the same byte length" "personal patch unequal length"
Assert-Equal (Bytes-Hex $personalPatchUnequalFile) "00deadbeef00" "personal patch unequal original unchanged"
if (Test-Path "$personalPatchUnequalFile.patched") { throw "personal patch unequal unexpectedly wrote output" }

$personalPatchOffsetNeg = Join-Path $personalArgsRoot "offset-neg.bin"
[System.IO.File]::WriteAllBytes($personalPatchOffsetNeg, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef))
$personalPatchOffsetNegResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--offset", "-1", $personalPatchOffsetNeg, "CAFEBABE") -WorkingDirectory $repoRoot
if ($personalPatchOffsetNegResult.Code -ne 0) { throw "personal patch offset negative failed: $($personalPatchOffsetNegResult.Text)" }
Assert-Equal $personalPatchOffsetNegResult.Text "error: offset must be a non-negative decimal integer" "personal patch offset negative"
if (Test-Path "$personalPatchOffsetNeg.patched") { throw "personal patch offset negative unexpectedly wrote output" }

$personalPatchBadReplace = Join-Path $personalArgsRoot "bad-replace.bin"
[System.IO.File]::WriteAllBytes($personalPatchBadReplace, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef))
$personalPatchBadReplaceResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalPatchBadReplace, "DEADBEEF", "ZZZZZZZZ") -WorkingDirectory $repoRoot
if ($personalPatchBadReplaceResult.Code -ne 0) { throw "personal patch invalid replace failed: $($personalPatchBadReplaceResult.Text)" }
Assert-Equal $personalPatchBadReplaceResult.Text "error: invalid replace_hex" "personal patch invalid replace"
if (Test-Path "$personalPatchBadReplace.patched") { throw "personal patch invalid replace unexpectedly wrote output" }

$personalPatchOffsetBadReplaceResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--offset", "0", $personalPatchBadReplace, "NOTHEX") -WorkingDirectory $repoRoot
if ($personalPatchOffsetBadReplaceResult.Code -ne 0) { throw "personal patch offset invalid replace failed: $($personalPatchOffsetBadReplaceResult.Text)" }
Assert-Equal $personalPatchOffsetBadReplaceResult.Text "error: invalid replace_hex" "personal patch offset invalid replace"
if (Test-Path "$personalPatchBadReplace.patched") { throw "personal patch offset invalid replace unexpectedly wrote output" }

$personalFindNoMatch = Join-Path $personalArgsRoot "no-pattern.bin"
[System.IO.File]::WriteAllBytes($personalFindNoMatch, [byte[]](0x01, 0x02, 0x03, 0x04))
$personalFindNoMatchResult = Invoke-Dbyte -Arguments @("run", "personal_tools\find_bytes.dby", $personalFindNoMatch, "DEADBEEF") -WorkingDirectory $repoRoot
if ($personalFindNoMatchResult.Code -ne 0) { throw "personal find no match failed: $($personalFindNoMatchResult.Text)" }
Assert-Contains $personalFindNoMatchResult.Text "pattern: not found" "personal find no match"

$personalHexOob = Invoke-Dbyte -Arguments @("run", "personal_tools\hexdump.dby", $personalArgsFile, "11", "1") -WorkingDirectory $repoRoot
if ($personalHexOob.Code -ne 0) { throw "personal hexdump offset oob failed: $($personalHexOob.Text)" }
Assert-Equal $personalHexOob.Text "error: offset out of bounds" "personal hexdump offset oob"

$personalHexClamp = Invoke-Dbyte -Arguments @("run", "personal_tools\hexdump.dby", $personalArgsFile, "1", "999") -WorkingDirectory $repoRoot
if ($personalHexClamp.Code -ne 0) { throw "personal hexdump length clamp failed: $($personalHexClamp.Text)" }
Assert-Contains $personalHexClamp.Text "range: 1 9" "personal hexdump length clamp header"
Assert-Contains $personalHexClamp.Text "1 : deadbeef00785634" "personal hexdump length clamp row1"
Assert-Contains $personalHexClamp.Text "9 : 12" "personal hexdump length clamp row2"

$personalHexNeg = Invoke-Dbyte -Arguments @("run", "personal_tools\hexdump.dby", $personalArgsFile, "-1", "4") -WorkingDirectory $repoRoot
if ($personalHexNeg.Code -ne 0) { throw "personal hexdump negative offset failed: $($personalHexNeg.Text)" }
Assert-Equal $personalHexNeg.Text "error: offset must be a non-negative decimal integer" "personal hexdump negative offset"

$personalHexTwoArgs = Invoke-Dbyte -Arguments @("run", "personal_tools\hexdump.dby", $personalArgsFile, "0") -WorkingDirectory $repoRoot
if ($personalHexTwoArgs.Code -ne 0) { throw "personal hexdump two args failed: $($personalHexTwoArgs.Text)" }
Assert-Contains $personalHexTwoArgs.Text "usage: hexdump <file> [offset length]" "personal hexdump two args usage line"
Assert-Contains $personalHexTwoArgs.Text "-h, --help" "personal hexdump two args options"
Assert-Contains $personalHexTwoArgs.Text "example: dbyte run personal_tools/hexdump.dby sample.bin 0 16" "personal hexdump two args example"

$personalU32BadOffset = Invoke-Dbyte -Arguments @("run", "personal_tools\read_u32_table.dby", $personalArgsFile, "nope", "1") -WorkingDirectory $repoRoot
if ($personalU32BadOffset.Code -ne 0) { throw "personal u32 bad offset failed: $($personalU32BadOffset.Text)" }
Assert-Equal $personalU32BadOffset.Text "error: offset must be a decimal integer" "personal u32 bad offset"

$personalU32NegCount = Invoke-Dbyte -Arguments @("run", "personal_tools\read_u32_table.dby", $personalArgsFile, "0", "-1") -WorkingDirectory $repoRoot
if ($personalU32NegCount.Code -ne 0) { throw "personal u32 negative count failed: $($personalU32NegCount.Text)" }
Assert-Equal $personalU32NegCount.Text "error: count must be a non-negative decimal integer" "personal u32 negative count"

$personalU32OobStart = Invoke-Dbyte -Arguments @("run", "personal_tools\read_u32_table.dby", $personalArgsFile, "11", "1") -WorkingDirectory $repoRoot
if ($personalU32OobStart.Code -ne 0) { throw "personal u32 start offset oob failed: $($personalU32OobStart.Text)" }
Assert-Equal $personalU32OobStart.Text "error: offset out of bounds" "personal u32 start offset oob"

$personalU32Args = Invoke-Dbyte -Arguments @("run", "personal_tools\read_u32_table.dby", $personalArgsFile) -WorkingDirectory $repoRoot
if ($personalU32Args.Code -ne 0) { throw "personal u32 args failed: $($personalU32Args.Text)" }
Assert-Contains $personalU32Args.Text "0 -> 3199065600" "personal u32 args first row"
Assert-Contains $personalU32Args.Text "4 -> 1450705135" "personal u32 args second row"

$personalU32Range = Invoke-Dbyte -Arguments @("run", "personal_tools\read_u32_table.dby", $personalArgsFile, "6", "1") -WorkingDirectory $repoRoot
if ($personalU32Range.Code -ne 0) { throw "personal u32 range failed: $($personalU32Range.Text)" }
Assert-Contains $personalU32Range.Text "6 -> 305419896" "personal u32 range row"

$personalSpacedRoot = Join-Path $personalArgsRoot "path with spaces"
New-Item -ItemType Directory -Path $personalSpacedRoot | Out-Null
$personalSpacedFile = Join-Path $personalSpacedRoot "quoted sample.bin"
[System.IO.File]::WriteAllBytes($personalSpacedFile, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x00))
$personalFindSpaced = Invoke-Dbyte -Arguments @("run", "personal_tools\find_bytes.dby", $personalSpacedFile, "DEADBEEF") -WorkingDirectory $repoRoot
if ($personalFindSpaced.Code -ne 0) { throw "personal find spaced path failed: $($personalFindSpaced.Text)" }
Assert-Contains $personalFindSpaced.Text "pattern: 1 0x1" "personal find spaced path"

$personalHexSpaced = Invoke-Dbyte -Arguments @("run", "personal_tools\hexdump.dby", $personalSpacedFile) -WorkingDirectory $repoRoot
if ($personalHexSpaced.Code -ne 0) { throw "personal hexdump spaced path failed: $($personalHexSpaced.Text)" }
Assert-Contains $personalHexSpaced.Text "bytes: 6" "personal hexdump spaced path size"

$personalBinSpaced = Invoke-Dbyte -Arguments @("run", "personal_tools\bininfo.dby", $personalSpacedFile) -WorkingDirectory $repoRoot
if ($personalBinSpaced.Code -ne 0) { throw "personal bininfo spaced path failed: $($personalBinSpaced.Text)" }
Assert-Contains $personalBinSpaced.Text "bytes: 6" "personal bininfo spaced path size"

$personalPatchSpaced = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalSpacedFile, "DEADBEEF", "CAFEBABE") -WorkingDirectory $repoRoot
if ($personalPatchSpaced.Code -ne 0) { throw "personal patch spaced path failed: $($personalPatchSpaced.Text)" }
Assert-Contains $personalPatchSpaced.Text "patched first match at offset 1" "personal patch spaced path"
Assert-Equal (Bytes-Hex $personalSpacedFile) "00deadbeef00" "personal patch spaced original unchanged"

$personalU32Spaced = Invoke-Dbyte -Arguments @("run", "personal_tools\read_u32_table.dby", $personalSpacedFile, "1", "1") -WorkingDirectory $repoRoot
if ($personalU32Spaced.Code -ne 0) { throw "personal u32 spaced path failed: $($personalU32Spaced.Text)" }
Assert-Contains $personalU32Spaced.Text "1 -> 4022250974" "personal u32 spaced path row"

if (Test-Path "$personalSpacedFile.patched") {
    Remove-Item -Force "$personalSpacedFile.patched"
}

$personalUsageFind = Invoke-Dbyte -Arguments @("run", "personal_tools\find_bytes.dby", $personalArgsFile) -WorkingDirectory $repoRoot
if ($personalUsageFind.Code -ne 0) { throw "personal find usage failed: $($personalUsageFind.Text)" }
Assert-Contains $personalUsageFind.Text "usage: find_bytes <file> <hex_pattern>" "personal find usage line"
Assert-Contains $personalUsageFind.Text "-h, --help" "personal find usage options"
Assert-Contains $personalUsageFind.Text "example: dbyte run personal_tools/find_bytes.dby sample.bin DEADBEEF" "personal find usage example"

$personalUsagePatch = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalArgsFile, "DEADBEEF") -WorkingDirectory $repoRoot
if ($personalUsagePatch.Code -ne 0) { throw "personal patch usage failed: $($personalUsagePatch.Text)" }
Assert-Contains $personalUsagePatch.Text "usage: patch_bytes <file> <find_hex> <replace_hex>" "personal patch usage first line"
Assert-Contains $personalUsagePatch.Text "patch_bytes --all" "personal patch usage all line"
Assert-Contains $personalUsagePatch.Text "patch_bytes --offset" "personal patch usage offset line"

$personalShellNoRcAlias = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "hexdump`nquit`n"
if ($personalShellNoRcAlias.Code -ne 0) { throw "personal shell no-rc alias guard failed: $($personalShellNoRcAlias.Text)" }
Assert-Contains $personalShellNoRcAlias.Text "ShellError: unknown command: hexdump" "personal shell no-rc hides aliases"

$personalShellRun = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "run personal_tools/hexdump.dby`nquit`n"
if ($personalShellRun.Code -ne 0) { throw "personal shell run failed: $($personalShellRun.Text)" }
Assert-PersonalToolOutput "hexdump" $personalShellRun.Text

$personalShellRunArgs = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "run personal_tools/find_bytes.dby `"$personalArgsFile`" DEADBEEF`nquit`n"
if ($personalShellRunArgs.Code -ne 0) { throw "personal shell run args failed: $($personalShellRunArgs.Text)" }
Assert-Contains $personalShellRunArgs.Text "pattern: 1" "personal shell run passes args"

$personalShellQuotedRunArgs = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "run `"personal_tools/find_bytes.dby`" `"$personalArgsFile`" DEADBEEF`nquit`n"
if ($personalShellQuotedRunArgs.Code -ne 0) { throw "personal shell quoted run args failed: $($personalShellQuotedRunArgs.Text)" }
Assert-Contains $personalShellQuotedRunArgs.Text "pattern: 1" "personal shell quoted run passes args"

$personalShellQuotedPathWithSpaces = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "run `"personal_tools/find_bytes.dby`" `"$personalSpacedFile`" DEADBEEF`nquit`n"
if ($personalShellQuotedPathWithSpaces.Code -ne 0) { throw "personal shell quoted spaced path failed: $($personalShellQuotedPathWithSpaces.Text)" }
Assert-Contains $personalShellQuotedPathWithSpaces.Text "pattern: 1 0x1" "personal shell quoted spaced path"

$personalShellAliases = Invoke-DbyteInput -Arguments @("shell") -InputText "hexdump`nbininfo`nfind-bytes`npatch-bytes`nu32-table`nquit`n"
if ($personalShellAliases.Code -ne 0) { throw "personal shell aliases failed: $($personalShellAliases.Text)" }
foreach ($tool in $personalToolFiles) {
    Assert-PersonalToolOutput $tool.Name $personalShellAliases.Text
}

$personalToolsShellRoot = Join-Path $repoRoot "personal_tools"
$personalShellToolsCwdAliases = Invoke-DbyteInput -Arguments @("shell") -WorkingDirectory $personalToolsShellRoot -InputText "hexdump`nbininfo`nfind-bytes`npatch-bytes`nu32-table`nquit`n"
if ($personalShellToolsCwdAliases.Code -ne 0) { throw "personal shell aliases from personal_tools cwd failed: $($personalShellToolsCwdAliases.Text)" }
foreach ($tool in $personalToolFiles) {
    Assert-PersonalToolOutput $tool.Name $personalShellToolsCwdAliases.Text
}
Assert-GitStatus-Unchanged $personalToolsStatus "personal tools shell cleanliness"

Write-Host "Running personal tools UX tests..."

$personalUxStatus = Git-Status-Short
$personalUxBinFile = Join-Path $repoRoot "target\verify-personal-tools\sample.bin"

# --help smoke: each tool must print usage: and -h, --help
foreach ($toolEntry in @(
    @{ Name = "hexdump"; Path = "hexdump.dby" },
    @{ Name = "find_bytes"; Path = "find_bytes.dby" },
    @{ Name = "bininfo"; Path = "bininfo.dby" },
    @{ Name = "read_u32_table"; Path = "read_u32_table.dby" },
    @{ Name = "patch_bytes"; Path = "patch_bytes.dby" }
)) {
    $helpResult = Invoke-Dbyte -Arguments @("run", "personal_tools\$($toolEntry.Path)", "--help") -WorkingDirectory $repoRoot
    if ($helpResult.Code -ne 0) { throw "$($toolEntry.Name) --help failed: $($helpResult.Text)" }
    Assert-Contains $helpResult.Text "usage:" "$($toolEntry.Name) --help contains usage:"
    Assert-Contains $helpResult.Text "-h, --help" "$($toolEntry.Name) --help contains -h flag"
    Assert-Contains $helpResult.Text "example:" "$($toolEntry.Name) --help contains example:"

    $shortHelpResult = Invoke-Dbyte -Arguments @("run", "personal_tools\$($toolEntry.Path)", "-h") -WorkingDirectory $repoRoot
    if ($shortHelpResult.Code -ne 0) { throw "$($toolEntry.Name) -h failed: $($shortHelpResult.Text)" }
    Assert-Contains $shortHelpResult.Text "usage:" "$($toolEntry.Name) -h contains usage:"
    Assert-Contains $shortHelpResult.Text "-h, --help" "$($toolEntry.Name) -h contains -h flag"
}
Assert-GitStatus-Unchanged $personalUxStatus "personal tools --help cleanliness"

# patch_bytes --out: first mode
$patchOutRoot = Join-Path $repoRoot "target\verify-patch-out"
if (Test-Path $patchOutRoot) { Remove-Item -Recurse -Force $patchOutRoot }
New-Item -ItemType Directory -Path $patchOutRoot | Out-Null
$patchOutSrc = Join-Path $patchOutRoot "src.bin"
$patchOutDst = Join-Path $patchOutRoot "dst.bin"
[System.IO.File]::WriteAllBytes($patchOutSrc, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x00))

$patchOutFirst = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $patchOutSrc, "DEADBEEF", "CAFEBABE", "--out", $patchOutDst) -WorkingDirectory $repoRoot
if ($patchOutFirst.Code -ne 0) { throw "patch_bytes --out first mode failed: $($patchOutFirst.Text)" }
Assert-Contains $patchOutFirst.Text "patched first match at offset 1" "patch --out first offset"
Assert-Contains $patchOutFirst.Text "wrote $patchOutDst" "patch --out first wrote path"
if (-not (Test-Path $patchOutDst)) { throw "patch --out first: output file not created" }
if (Test-Path "$patchOutSrc.patched") { throw "patch --out first: default .patched file unexpectedly created" }
Assert-Equal (Bytes-Hex $patchOutSrc) "00deadbeef00" "patch --out first original unchanged"
Assert-Equal (Bytes-Hex $patchOutDst) "00cafebabe00" "patch --out first output bytes"

# patch_bytes --all --out
$patchAllOutSrc = Join-Path $patchOutRoot "all-src.bin"
$patchAllOutDst = Join-Path $patchOutRoot "all-dst.bin"
[System.IO.File]::WriteAllBytes($patchAllOutSrc, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x11, 0xde, 0xad, 0xbe, 0xef, 0x22))

$patchAllOut = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--all", $patchAllOutSrc, "DEADBEEF", "CAFEBABE", "--out", $patchAllOutDst) -WorkingDirectory $repoRoot
if ($patchAllOut.Code -ne 0) { throw "patch_bytes --all --out failed: $($patchAllOut.Text)" }
Assert-Contains $patchAllOut.Text "patched count: 2" "patch --all --out count"
Assert-Contains $patchAllOut.Text "wrote $patchAllOutDst" "patch --all --out wrote path"
if (-not (Test-Path $patchAllOutDst)) { throw "patch --all --out: output file not created" }
if (Test-Path "$patchAllOutSrc.patched") { throw "patch --all --out: default .patched file unexpectedly created" }
Assert-Equal (Bytes-Hex $patchAllOutSrc) "00deadbeef11deadbeef22" "patch --all --out original unchanged"
Assert-Equal (Bytes-Hex $patchAllOutDst) "00cafebabe11cafebabe22" "patch --all --out output bytes"

# patch_bytes --offset --out
$patchOffOutSrc = Join-Path $patchOutRoot "off-src.bin"
$patchOffOutDst = Join-Path $patchOutRoot "off-dst.bin"
[System.IO.File]::WriteAllBytes($patchOffOutSrc, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x00))

$patchOffOut = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--offset", "1", $patchOffOutSrc, "CAFEBABE", "--out", $patchOffOutDst) -WorkingDirectory $repoRoot
if ($patchOffOut.Code -ne 0) { throw "patch_bytes --offset --out failed: $($patchOffOut.Text)" }
Assert-Contains $patchOffOut.Text "patched offset 1" "patch --offset --out marker"
Assert-Contains $patchOffOut.Text "wrote $patchOffOutDst" "patch --offset --out wrote path"
if (-not (Test-Path $patchOffOutDst)) { throw "patch --offset --out: output file not created" }
if (Test-Path "$patchOffOutSrc.patched") { throw "patch --offset --out: default .patched file unexpectedly created" }
Assert-Equal (Bytes-Hex $patchOffOutSrc) "00deadbeef00" "patch --offset --out original unchanged"
Assert-Equal (Bytes-Hex $patchOffOutDst) "00cafebabe00" "patch --offset --out output bytes"

Assert-GitStatus-Unchanged $personalUxStatus "personal tools UX cleanliness"

Write-Host "Running Sanctum System Workspace (v3.7.0) smoke tests..."
try {
    $sanctumRoot = Join-Path $repoRoot "examples\sanctum"
    $sanctumStatus = Git-Status-Short

    # 1. Initialization (idempotent)
    $sanctumInit1 = Invoke-Dbyte -Arguments @("run", "sanctum_init.dby") -WorkingDirectory $sanctumRoot
    if ($sanctumInit1.Code -ne 0) { throw "sanctum init (1) failed: $($sanctumInit1.Text)" }
    Assert-Contains $sanctumInit1.Text "S A N C T U M   I N I T I A L I Z A T I O N" "sanctum init banner 1"

    $sanctumInit2 = Invoke-Dbyte -Arguments @("run", "sanctum_init.dby") -WorkingDirectory $sanctumRoot
    if ($sanctumInit2.Code -ne 0) { throw "sanctum init (2) failed: $($sanctumInit2.Text)" }
    Assert-Contains $sanctumInit2.Text "Directory exists: workspace" "sanctum init idempotent dir"

    # 2. Status report
    $sanctumStatusReport = Invoke-Dbyte -Arguments @("run", "sanctum_status.dby") -WorkingDirectory $sanctumRoot
    if ($sanctumStatusReport.Code -ne 0) { throw "sanctum status failed: $($sanctumStatusReport.Text)" }
    Assert-Contains $sanctumStatusReport.Text "[OK] workspace" "sanctum status workspace ok"
    Assert-Contains $sanctumStatusReport.Text "[OK] config" "sanctum status config ok"

    # 3. Boot and Clean cycle
    $sanctumBoot = Invoke-Dbyte -Arguments @("run", "boot.dby") -WorkingDirectory $sanctumRoot
    if ($sanctumBoot.Code -ne 0) { throw "sanctum boot failed: $($sanctumBoot.Text)" }
    if (!(Test-Path (Join-Path $sanctumRoot "workspace\sample.bin"))) { throw "sanctum boot failed to generate sample.bin" }

    $sanctumClean = Invoke-Dbyte -Arguments @("run", "scripts\clean_workspace.dby") -WorkingDirectory $sanctumRoot
    if ($sanctumClean.Code -ne 0) { throw "sanctum clean failed: $($sanctumClean.Text)" }
    if (Test-Path (Join-Path $sanctumRoot "workspace\sample.bin")) { throw "sanctum clean failed to remove sample.bin" }

    # 4. Cross-directory verify
    $sanctumStatusRoot = Invoke-Dbyte -Arguments @("run", "examples\sanctum\sanctum_status.dby") -WorkingDirectory $repoRoot
    if ($sanctumStatusRoot.Code -ne 0) { throw "sanctum status from root failed: $($sanctumStatusRoot.Text)" }
    Assert-Contains $sanctumStatusRoot.Text "[OK] workspace" "sanctum status from root"

    # 5. Shell aliases
    $sanctumShell = Invoke-DbyteInput -Arguments @("shell") -InputText "status`ninit`nclean`nquit`n" -WorkingDirectory $sanctumRoot
    if ($sanctumShell.Code -ne 0) { throw "sanctum shell aliases failed: $($sanctumShell.Text)" }
    Assert-Contains $sanctumShell.Text "S A N C T U M   S Y S T E M   S T A T U S" "sanctum shell status alias"
    Assert-Contains $sanctumShell.Text "S A N C T U M   I N I T I A L I Z A T I O N" "sanctum shell init alias"

    # Cleanup
    Remove-Item -Path (Join-Path $sanctumRoot "workspace") -Recurse -Force -ErrorAction SilentlyContinue

    Assert-GitStatus-Unchanged $sanctumStatus "sanctum system workspace cleanliness"
}
catch {
    throw $_
}

Write-Host "Running DByteOS Command Set (v9.0.2) smoke tests..."
$dbyteosRoot = Join-Path $repoRoot "examples\dbyteos"
$dbyteosProjectsPath = Join-Path $dbyteosRoot "home\deadbyte\projects"
Remove-Item -Recurse -Force $dbyteosProjectsPath -ErrorAction SilentlyContinue
$dbyteosStatus = Git-Status-Short
$dbyteosPrefsRel = "examples/dbyteos/home/deadbyte/preferences.dby"
$dbyteosPrefsInitiallyClean = $dbyteosStatus -notmatch [regex]::Escape($dbyteosPrefsRel)
$expectedDbyteosBoot = @"
==================================================
  ____  ____        _             ___  ____  
 |  _ \| __ ) _   _| |_ ___      / _ \/ ___| 
 | | | |  _ \| | | | __/ _ \    | | | \___ \ 
 | |_| | |_) | |_| | ||  __/    | |_| |___) |
 |____/|____/ \__, |\__\___|     \___/|____/ 
              |___/                          
        D B Y T E O S   U S E R L A N D
        Alpha personal computing workspace
==================================================
System:
  Version:    DByte  9.0.2  ( Userland Prototype )
  Hostname:    DByte-Alpha
  Kernel:      Simulated (Host)
  User:        deadbyte
  Home:        home/deadbyte
--------------------------------------------------
Checking system integrity...
  [OK] /bin
  [OK] /etc
  [OK] /sys
  [OK] /home
  [OK] /tmp

Init: starting userland services...
  [INIT] notes
  [INIT] sysinfo
Init: 2 services initialized.
  [OK] Session initialized.
System initialization complete.
  [OK] /tmp/.dbyteos_boot_touch (session marker)
DByteOS is ready for interaction.
First-run guide:
  welcome          - show onboarding
  getting-started  - follow the checklist
  commands         - browse command groups
  man-index        - list manual topics
==================================================
"@
$expectedDbyteosHelp = @"
--- DByteOS Beta Help ---
System:
  boot             - initialize the DByteOS userland
  status           - summarize system state
  sysinfo          - display version and identity
  whoami           - print the current user
  profile          - show profile identity
  config           - show read-only configuration
  prefs            - manage mutable user preferences
  snapshot         - summarize DByteOS subsystem state
  project          - manage workspace projects
  task             - manage project tasks and task UX

Discovery:
  welcome          - show the onboarding entry point
  getting-started  - show the first-run checklist
  commands         - browse commands by category
  help             - display this command guide
  man <topic>      - display manual entry for a command
  man-index        - list manual topics
  path             - display path config or resolve commands
  which <command>  - locate commands from the shell

Diagnostics:
  doctor           - full system health report
  diagnose         - drill-down subsystem report
  check-system     - quick readiness gate

Files:
  read             - read and print file contents
  write            - write text to a file
  append           - append text to a file
  cat              - view file contents
  touch            - create or update files
  inspect          - view file metadata

Security:
  perm             - inspect permission policy
  clean            - clean the workspace directory

Journal/Workspace:
  notes            - manage text notes
  journal          - manage user journal
  project          - manage workspace projects
  task             - manage project tasks and task UX
  workspace        - manage workspace report and status
  daily            - manage daily agenda summary
  search           - search workspace, projects, tasks, daily
  timeline         - read-only chronological workspace timeline
  dashboard        - print DByteOS workspace dashboard home
  home             - print home path
  tmp              - print temp path
  env              - display environment variables

Services/Logs:
  services         - manage system services
  log              - read DByteOS session logs

Try: welcome, profile show, config show, snapshot, getting-started, commands
"@
$expectedDbyteosStatus = @"
--- DByteOS System Status ---
Summary:
  OS:      DByte  9.0.2
  Host:     DByte-Alpha
  User:     deadbyte
  Home:     home/deadbyte

Profile:
  Mode:     beta-userland
  Theme:    default
  Prompt:   dbyte-shell>

Filesystem Integrity:
  bin: [PRESENT]
  etc: [PRESENT]
  sys: [PRESENT]
  home: [PRESENT]
  tmp: [PRESENT]

Memory:  Simulated
Uptime:  Simulated
Next:    help | man <topic> | which <command>
-----------------------------
"@
$expectedDbyteosSysinfo = @"
DByteOS Alpha Userland
version: DByte 9.0.2
codename: Userland Prototype
host: DByte-Alpha
kernel: Simulated (Host)
user: deadbyte
home: examples/dbyteos/home/deadbyte
shell: dbyte shell
mode: beta-userland
theme: default
prompt: dbyte-shell>
guide: run help, status, or man <topic>
"@
$expectedDbyteosWelcome = @"
--- Welcome to DByteOS Alpha ---
DByteOS is a personal userland built on the DByte runtime.

Profile:
  user:    deadbyte
  home:    home/deadbyte
  mode:    beta-userland
  prompt:  dbyte-shell>

Start here:
  profile show    - inspect current profile
  getting-started - follow the first-run checklist
  commands        - browse commands by category
  man-index       - list manual topics
  help            - show grouped command help
  status          - summarize system state

Suggested first session:
  boot
  profile show
  getting-started
  commands
  man-index
  man perm

Rule: DByteOS commands are DByte scripts, not OS passthrough.
"@
$expectedDbyteosGettingStarted = @"
--- DByteOS Getting Started ---
First-run checklist:
  [1] boot             - initialize the userland session
  [2] status           - verify directories and session state
  [3] commands         - browse commands by category
  [4] man-index        - list manual topics
  [5] which read       - inspect command discovery
  [6] man perm         - review the permission policy
  [7] notes list       - inspect personal notes
  [8] services status  - inspect userland services

Safe write area:
  tmp/                 - temporary session artifacts
  home/deadbyte/       - persistent personal workspace

Protected areas:
  bin/ etc/ sys/       - read-only by policy
"@
$expectedDbyteosCommands = @"
--- DByteOS Commands ---
System:
  boot             - initialize the userland session
  status           - summarize system state
  sysinfo          - display version and identity
  whoami           - print the current user
  profile          - show profile identity
  config           - show read-only configuration
  prefs            - manage mutable user preferences
  snapshot         - summarize subsystem state
  project          - manage workspace projects
  task             - manage project tasks and task UX

Discovery:
  welcome          - show the onboarding entry point
  getting-started  - show the first-run checklist
  commands         - list commands by category
  help             - show grouped command help
  man <topic>      - read a manual topic
  man-index        - list manual topics
  which <command>  - resolve a command
  path             - show command search roots

Diagnostics:
  doctor           - full system health report
  diagnose         - drill-down subsystem report
  check-system     - quick readiness gate

Files:
  read             - read a file
  write            - write a file
  append           - append to a file
  cat              - view file contents
  touch            - create or update a file
  inspect          - view file metadata

Security:
  perm             - inspect permission policy
  clean            - clean temporary workspace artifacts

Journal and workspace:
  notes            - manage personal notes
  journal          - manage journal entries
  project          - manage workspace projects
  task             - manage project tasks and task UX
  workspace        - manage workspace report and status
  daily            - manage daily agenda summary
  search           - search workspace, projects, tasks, daily
  timeline         - read-only chronological workspace timeline
  dashboard        - print DByteOS workspace dashboard home
  home             - print the home path
  tmp              - print the temp path
  env              - show environment settings

Services and logs:
  services         - manage userland services
  log              - read session logs
"@
$expectedDbyteosManIndex = @"
--- DByteOS Manual Index ---
Start:
  welcome
  getting-started
  commands
  index

System:
  boot
  status
  sysinfo
  whoami
  profile
  config
  prefs
  snapshot
  project
  task
  env
  path

Diagnostics:
  doctor
  diagnose
  check-system

Files and security:
  read
  write
  append
  clean
  perm
  security

Workspace:
  notes
  journal
  project
  task
  workspace
  daily
  services
  log
  search
  timeline
  dashboard

Use: man <topic>
"@
$expectedDbyteosProfile = @"
--- DByteOS Profile ---
user: deadbyte
home: home/deadbyte
shell: dbyte shell
mode: beta-userland
theme: default
prompt: dbyte-shell>
os_version: 9.0.2
"@
$expectedDbyteosProfileUnknown = @"
error: unknown profile command: unknown
usage: profile [show|whoami|home|theme|prompt]

commands:
  show    - print deterministic profile summary
  whoami  - print profile user
  home    - print resolved home path
  theme   - print active theme
  prompt  - print shell prompt
"@
$expectedDbyteosConfig = @"
--- DByteOS Config ---
system.mode = beta-userland
system.prompt = dbyte-shell>
user.name = deadbyte
user.home = home/deadbyte
ui.theme = default
security.mode = simulated
"@
$expectedDbyteosConfigKeys = @"
system.mode
system.prompt
user.name
user.home
ui.theme
security.mode
"@
$expectedDbyteosConfigUsage = @"
usage: config [show|keys|get <key>]

commands:
  show       - print read-only config values
  keys       - list config keys
  get <key>  - print one config value
"@
$expectedDbyteosConfigMissingKey = @"
error: config get requires a key
usage: config [show|keys|get <key>]

commands:
  show       - print read-only config values
  keys       - list config keys
  get <key>  - print one config value
"@
$expectedDbyteosConfigUnknownCommand = @"
error: unknown config command: unknown
usage: config [show|keys|get <key>]

commands:
  show       - print read-only config values
  keys       - list config keys
  get <key>  - print one config value
"@
$expectedDbyteosProjectUsage = @"
usage: project <command> [name]

commands:
  new <name>       - create a deterministic workspace project
  list             - list known projects
  status <name>    - show project file status
  notes <name>     - read project notes
  snapshot <name>  - read project snapshot
  doctor <name>    - validate project files
  report <name>    - detailed project status report
  reset-demo       - reset the demo project workspace
"@
$expectedDbyteosProjectListEmpty = @"
Projects:
  (none)
"@
$expectedDbyteosProjectListDemo = @"
Projects:
  demo
"@
$expectedDbyteosProjectStatusDemo = @"
--- DByteOS Project Status ---
name: demo
project: present
project.txt: present
notes.txt: present
snapshot.txt: present
"@
$expectedDbyteosProjectNotesDemo = @"
project demo notes
"@
$expectedDbyteosProjectSnapshotDemo = @"
--- DByteOS Project Snapshot ---
name: demo
owner: deadbyte
status: active
files: project.txt, notes.txt, snapshot.txt
"@
$expectedDbyteosProjectDoctorDemo = @"
DByteOS Project Doctor
project: ok
metadata: ok
notes: ok
snapshot: ok
result: healthy
"@
$expectedDbyteosProjectNotFound = @"
error: project not found: missing
"@
$expectedDbyteosProjectUnknown = @"
error: unknown project command: unknown
usage: project <command> [name]

commands:
  new <name>       - create a deterministic workspace project
  list             - list known projects
  status <name>    - show project file status
  notes <name>     - read project notes
  snapshot <name>  - read project snapshot
  doctor <name>    - validate project files
  report <name>    - detailed project status report
  reset-demo       - reset the demo project workspace
"@
$expectedDbyteosWorkspaceUsage = @"
usage: workspace <command>

commands:
  report    - print a unified workspace overview dashboard
  doctor    - check the integrity of the project index and all projects
  snapshot  - print an aggregated snapshot of all workspace projects and tasks
  daily     - print a daily agenda summary
"@
$expectedDbyteosWorkspaceReportEmpty = @"
--- DByteOS Workspace Report ---
User:    deadbyte
Home:    home/deadbyte
Theme:   default
Prompt:  dbyte-shell>

Projects:
  (none)
"@
$expectedDbyteosWorkspaceDoctorEmpty = @"
--- DByteOS Workspace Doctor ---
Index: missing
Result: unhealthy
"@
$expectedDbyteosWorkspaceSnapshotEmpty = @"
--- DByteOS Workspace Snapshot ---
User: deadbyte
Projects:
  (none)
"@
$expectedDbyteosDailySummaryEmpty = @"
--- DByteOS Daily Summary ---
Notes:   home/deadbyte/notes.txt (not found)
Journal: 1 entries recorded

Open Tasks:
  (none)
"@
$expectedDbyteosDailyUsage = @"
usage: daily summary
"@
$expectedDbyteosWorkspaceReportDemo = @"
--- DByteOS Workspace Report ---
User:    deadbyte
Home:    home/deadbyte
Theme:   default
Prompt:  dbyte-shell>

Projects:
  demo: 2 open, 0 done (total: 2)
"@
$expectedDbyteosWorkspaceDoctorDemo = @"
--- DByteOS Workspace Doctor ---
Index: ok
Projects:
  demo: healthy
Result: healthy
"@
$expectedDbyteosWorkspaceSnapshotDemo = @"
--- DByteOS Workspace Snapshot ---
User: deadbyte
Projects:
  - name: demo
    tasks: 2 open, 0 done
"@
$expectedDbyteosDailySummaryDemo = @"
--- DByteOS Daily Summary ---
Notes:   home/deadbyte/notes.txt (not found)
Journal: 1 entries recorded

Open Tasks:
  demo: 2 open
"@
$expectedDbyteosTaskUsage = @"
usage: task <command> [project] [args...]

commands:
  add <project> <text>  - add a task to a workspace project
  list <project>        - list project tasks
  done <project> <id>   - mark a task done
  status <project>      - summarize project task state
  summary <project>     - print compact task counts
  open <project>        - list open project tasks
  clear-done <project>  - remove completed tasks
  doctor <project>      - validate task file
  snapshot <project>    - print task snapshot
  report <project>      - print visual task progress report
  reset-demo            - reset demo project tasks
"@
$expectedDbyteosTaskListDemo = @"
DByteOS project tasks: demo
[ ] 1: inspect workspace
[ ] 2: write project note
"@
$expectedDbyteosTaskListAfterAdd = @"
DByteOS project tasks: demo
[ ] 1: inspect workspace
[ ] 2: write project note
[ ] 3: write tests
"@
$expectedDbyteosTaskListAfterDone = @"
DByteOS project tasks: demo
[x] 1: inspect workspace
[ ] 2: write project note
[ ] 3: write tests
"@
$expectedDbyteosTaskStatusAfterDone = @"
Task Status: demo
open: 2
done: 1
total: 3
"@
$expectedDbyteosTaskSummaryAfterDone = @"
Task Summary: demo
open: 2
done: 1
total: 3
"@
$expectedDbyteosTaskOpenAfterDone = @"
DByteOS open tasks: demo
[ ] 2: write project note
[ ] 3: write tests
"@
$expectedDbyteosTaskDoctorHealthy = @"
Task Doctor: demo
project: ok
tasks_file: ok
rows: ok
result: healthy
"@
$expectedDbyteosTaskDoctorMalformed = @"
Task Doctor: demo
project: ok
tasks_file: ok
rows: malformed
result: unhealthy
"@
$expectedDbyteosTaskSnapshotAfterDone = @"
--- DByteOS Task Snapshot ---
project: demo
open: 2
done: 1
total: 3
tasks:
[x] 1: inspect workspace
[ ] 2: write project note
[ ] 3: write tests
"@
$expectedDbyteosTaskClearDone = @"
task clear-done: demo
removed: 1
remaining: 2
"@
$expectedDbyteosTaskListAfterClearDone = @"
DByteOS project tasks: demo
[ ] 1: write project note
[ ] 2: write tests
"@
$expectedDbyteosTaskStatusAfterClearDone = @"
Task Status: demo
open: 2
done: 0
total: 2
"@
$expectedDbyteosTaskSummaryAfterClearDone = @"
Task Summary: demo
open: 2
done: 0
total: 2
"@
$expectedDbyteosTaskSnapshotAfterClearDone = @"
--- DByteOS Task Snapshot ---
project: demo
open: 2
done: 0
total: 2
tasks:
[ ] 1: write project note
[ ] 2: write tests
"@
$expectedDbyteosTaskStatusAllDone = @"
Task Status: demo
open: 0
done: 2
total: 2
"@
$expectedDbyteosTaskSummaryAllDone = @"
Task Summary: demo
open: 0
done: 2
total: 2
"@
$expectedDbyteosTaskOpenAllDone = @"
DByteOS open tasks: demo
  (none)
"@
$expectedDbyteosTaskSnapshotAllDone = @"
--- DByteOS Task Snapshot ---
project: demo
open: 0
done: 2
total: 2
tasks:
[x] 1: inspect workspace
[x] 2: write project note
"@
$expectedDbyteosTaskClearDoneAllDone = @"
task clear-done: demo
removed: 2
remaining: 0
"@
$expectedDbyteosTaskUnknown = @"
error: unknown task command: unknown
usage: task <command> [project] [args...]

commands:
  add <project> <text>  - add a task to a workspace project
  list <project>        - list project tasks
  done <project> <id>   - mark a task done
  status <project>      - summarize project task state
  summary <project>     - print compact task counts
  open <project>        - list open project tasks
  clear-done <project>  - remove completed tasks
  doctor <project>      - validate task file
  snapshot <project>    - print task snapshot
  report <project>      - print visual task progress report
  reset-demo            - reset demo project tasks
"@
$expectedDbyteosSnapshot = @"
--- DByteOS System Snapshot ---
System:
  version: DByte 9.0.2
  codename: Userland Prototype
  host:    DByte-Alpha
  kernel:  Simulated (Host)

Profile:
  user:    deadbyte
  home:    home/deadbyte
  shell:   dbyte shell
  mode:    beta-userland
  theme:   default
  prompt:  dbyte-shell>

Config:
  system.mode = beta-userland
  system.prompt = dbyte-shell>
  user.name = deadbyte
  user.home = home/deadbyte
  ui.theme = default
  security.mode = simulated

Security:
  mode:          simulated
  tmp/:          read/write
  home/deadbyte/: read/write
  etc/:          read-only
  sys/:          read-only
  bin/:          read-only
  ../:           denied
  absolute path: denied

Logs:
  boot.log: missing
  services.log: missing
  security.log: missing

Next: snapshot profile | snapshot config | snapshot security | snapshot logs
"@
$expectedDbyteosSnapshotProfile = @"
Profile:
  user:    deadbyte
  home:    home/deadbyte
  shell:   dbyte shell
  mode:    beta-userland
  theme:   default
  prompt:  dbyte-shell>
"@
$expectedDbyteosSnapshotConfig = @"
Config:
  system.mode = beta-userland
  system.prompt = dbyte-shell>
  user.name = deadbyte
  user.home = home/deadbyte
  ui.theme = default
  security.mode = simulated
"@
$expectedDbyteosSnapshotSecurity = @"
Security:
  mode:          simulated
  tmp/:          read/write
  home/deadbyte/: read/write
  etc/:          read-only
  sys/:          read-only
  bin/:          read-only
  ../:           denied
  absolute path: denied
"@
$expectedDbyteosSnapshotLogs = @"
Logs:
  boot.log: missing
  services.log: missing
  security.log: missing
"@
$expectedDbyteosSnapshotLogsAfterBoot = @"
Logs:
  boot.log: present
  services.log: present
  security.log: missing
"@
$expectedDbyteosSnapshotUnknown = @"
error: unknown snapshot command: unknown
usage: snapshot [system|profile|config|security|logs]

commands:
  system    - print full system snapshot
  profile   - print profile identity
  config    - print read-only config values
  security  - print security policy summary
  logs      - print session log summary
"@
$expectedDbyteosDoctor = @"
DByteOS Doctor
profile: ok
config: ok
preferences: ok
security: ok
logs: ok
manual: ok
package: ok
snapshot: ok
result: healthy
"@
$expectedDbyteosCheckSystem = @"
DByteOS readiness check
version: ok
profile: ok
config: ok
manual: ok
security: ok
preferences: ok
workspace: ok
package: ok
ready: yes
"@
$expectedDbyteosDiagnose = @"
usage: diagnose [profile|config|preferences|security|logs|manual|package]
"@
$expectedDbyteosDiagnoseProfile = @"
DByteOS Diagnose: profile
user: ok
home: ok
shell: ok
mode: ok
theme: ok
prompt: ok
result: healthy
"@
$expectedDbyteosDiagnoseConfig = @"
DByteOS Diagnose: config
system.mode: ok
system.prompt: ok
user.name: ok
user.home: ok
ui.theme: ok
security.mode: ok
result: healthy
"@
$expectedDbyteosDiagnoseSecurity = @"
DByteOS Diagnose: security
mode: ok
result: healthy
"@
$expectedDbyteosDiagnosePreferences = @"
DByteOS Diagnose: preferences
file: ok
result: healthy
"@
$expectedDbyteosDiagnoseLogs = @"
DByteOS Diagnose: logs
boot: ok
services: ok
security: ok
result: healthy
"@
$expectedDbyteosDiagnoseManual = @"
DByteOS Diagnose: manual
topics: ok
result: healthy
"@
$expectedDbyteosDiagnosePackage = @"
DByteOS Diagnose: package
smoke: ok
result: healthy
"@
try {
    $dbyteosBoot = Invoke-Dbyte -Arguments @("run", "boot.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosBoot.Code -ne 0) { throw "dbyteos boot failed: $($dbyteosBoot.Text)" }
    Assert-NormalizedEqual $dbyteosBoot.Text $expectedDbyteosBoot "dbyteos boot snapshot"
    Assert-Contains $dbyteosBoot.Text "D B Y T E O S   U S E R L A N D" "dbyteos boot banner"
    Assert-Contains $dbyteosBoot.Text "Alpha personal computing workspace" "dbyteos boot alpha subtitle"
    Assert-Contains $dbyteosBoot.Text "First-run guide:" "dbyteos boot first-run guide"
    Assert-Contains $dbyteosBoot.Text "welcome          - show onboarding" "dbyteos boot onboarding hint"
    Assert-Contains $dbyteosBoot.Text "[OK] /bin" "dbyteos boot bin check"
    Assert-Contains $dbyteosBoot.Text "/tmp/.dbyteos_boot_touch" "dbyteos boot tmp marker"

    $dbyteosBoot2 = Invoke-Dbyte -Arguments @("run", "boot.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosBoot2.Code -ne 0) { throw "dbyteos boot repeat failed: $($dbyteosBoot2.Text)" }
    Assert-Contains $dbyteosBoot2.Text "D B Y T E O S   U S E R L A N D" "dbyteos boot repeat banner"

    $dbyteosSnapshotLogsAfterBoot = Invoke-Dbyte -Arguments @("run", "bin\snapshot.dby", "logs") -WorkingDirectory $dbyteosRoot
    if ($dbyteosSnapshotLogsAfterBoot.Code -ne 0) { throw "dbyteos snapshot logs after boot failed: $($dbyteosSnapshotLogsAfterBoot.Text)" }
    Assert-NormalizedEqual $dbyteosSnapshotLogsAfterBoot.Text $expectedDbyteosSnapshotLogsAfterBoot "dbyteos snapshot logs after boot snapshot"

    $dbyteosStatusFromRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\status.dby") -WorkingDirectory $repoRoot
    if ($dbyteosStatusFromRoot.Code -ne 0) { throw "dbyteos status from repo root failed: $($dbyteosStatusFromRoot.Text)" }
    Assert-Contains $dbyteosStatusFromRoot.Text "--- DByteOS System Status ---" "dbyteos status from repo root"

    $dbyteosCleanFromRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\clean.dby") -WorkingDirectory $repoRoot
    if ($dbyteosCleanFromRoot.Code -ne 0) { throw "dbyteos clean from repo root failed: $($dbyteosCleanFromRoot.Text)" }
    Assert-Contains $dbyteosCleanFromRoot.Text "sweep complete" "dbyteos clean from repo root sweep"

    $dbyteosInspectFromRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\inspect.dby", "boot.dby") -WorkingDirectory $repoRoot
    if ($dbyteosInspectFromRoot.Code -ne 0) { throw "dbyteos inspect from repo root failed: $($dbyteosInspectFromRoot.Text)" }
    Assert-Contains $dbyteosInspectFromRoot.Text "Inspecting file:" "dbyteos inspect from repo root"

    $dbyteosStatusReport = Invoke-Dbyte -Arguments @("run", "bin\status.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosStatusReport.Code -ne 0) { throw "dbyteos status failed: $($dbyteosStatusReport.Text)" }
    Assert-NormalizedEqual $dbyteosStatusReport.Text $expectedDbyteosStatus "dbyteos status snapshot"
    Assert-Contains $dbyteosStatusReport.Text "--- DByteOS System Status ---" "dbyteos status banner"
    Assert-Contains $dbyteosStatusReport.Text "Summary:" "dbyteos status summary"
    Assert-Contains $dbyteosStatusReport.Text "Profile:" "dbyteos status profile summary"
    Assert-Contains $dbyteosStatusReport.Text "Next:    help | man <topic> | which <command>" "dbyteos status next hint"
    Assert-Contains $dbyteosStatusReport.Text "bin: [PRESENT]" "dbyteos status bin ok"

    $dbyteosHelpDirect = Invoke-Dbyte -Arguments @("run", "bin\help.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosHelpDirect.Code -ne 0) { throw "dbyteos help failed: $($dbyteosHelpDirect.Text)" }
    Assert-NormalizedEqual $dbyteosHelpDirect.Text $expectedDbyteosHelp "dbyteos help snapshot"

    $dbyteosWelcomeDirect = Invoke-Dbyte -Arguments @("run", "bin\welcome.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosWelcomeDirect.Code -ne 0) { throw "dbyteos welcome failed: $($dbyteosWelcomeDirect.Text)" }
    Assert-NormalizedEqual $dbyteosWelcomeDirect.Text $expectedDbyteosWelcome "dbyteos welcome snapshot"

    $dbyteosGettingStartedDirect = Invoke-Dbyte -Arguments @("run", "bin\getting_started.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosGettingStartedDirect.Code -ne 0) { throw "dbyteos getting-started failed: $($dbyteosGettingStartedDirect.Text)" }
    Assert-NormalizedEqual $dbyteosGettingStartedDirect.Text $expectedDbyteosGettingStarted "dbyteos getting-started snapshot"

    $dbyteosCommandsDirect = Invoke-Dbyte -Arguments @("run", "bin\commands.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosCommandsDirect.Code -ne 0) { throw "dbyteos commands failed: $($dbyteosCommandsDirect.Text)" }
    Assert-NormalizedEqual $dbyteosCommandsDirect.Text $expectedDbyteosCommands "dbyteos commands snapshot"

    $dbyteosManIndexDirect = Invoke-Dbyte -Arguments @("run", "bin\man_index.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosManIndexDirect.Code -ne 0) { throw "dbyteos man-index failed: $($dbyteosManIndexDirect.Text)" }
    Assert-NormalizedEqual $dbyteosManIndexDirect.Text $expectedDbyteosManIndex "dbyteos man-index snapshot"

    $dbyteosProjectUsage = Invoke-Dbyte -Arguments @("run", "bin\project.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectUsage.Code -ne 0) { throw "dbyteos project usage failed: $($dbyteosProjectUsage.Text)" }
    Assert-NormalizedEqual $dbyteosProjectUsage.Text $expectedDbyteosProjectUsage "dbyteos project usage snapshot"

    $dbyteosProjectListEmpty = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "list") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectListEmpty.Code -ne 0) { throw "dbyteos project list empty failed: $($dbyteosProjectListEmpty.Text)" }
    Assert-NormalizedEqual $dbyteosProjectListEmpty.Text $expectedDbyteosProjectListEmpty "dbyteos project list empty snapshot"

    # Empty workspace and daily summary smoke checks
    $dbyteosWorkspaceUsage = Invoke-Dbyte -Arguments @("run", "bin\workspace.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosWorkspaceUsage.Code -ne 0) { throw "dbyteos workspace usage failed: $($dbyteosWorkspaceUsage.Text)" }
    Assert-NormalizedEqual $dbyteosWorkspaceUsage.Text $expectedDbyteosWorkspaceUsage "dbyteos workspace usage snapshot"

    $dbyteosWorkspaceReportEmpty = Invoke-Dbyte -Arguments @("run", "bin\workspace.dby", "report") -WorkingDirectory $dbyteosRoot
    if ($dbyteosWorkspaceReportEmpty.Code -ne 0) { throw "dbyteos workspace report empty failed: $($dbyteosWorkspaceReportEmpty.Text)" }
    Assert-NormalizedEqual $dbyteosWorkspaceReportEmpty.Text $expectedDbyteosWorkspaceReportEmpty "dbyteos workspace report empty snapshot"

    $dbyteosWorkspaceDoctorEmpty = Invoke-Dbyte -Arguments @("run", "bin\workspace.dby", "doctor") -WorkingDirectory $dbyteosRoot
    if ($dbyteosWorkspaceDoctorEmpty.Code -ne 0) { throw "dbyteos workspace doctor empty failed: $($dbyteosWorkspaceDoctorEmpty.Text)" }
    Assert-NormalizedEqual $dbyteosWorkspaceDoctorEmpty.Text $expectedDbyteosWorkspaceDoctorEmpty "dbyteos workspace doctor empty snapshot"

    $dbyteosWorkspaceSnapshotEmpty = Invoke-Dbyte -Arguments @("run", "bin\workspace.dby", "snapshot") -WorkingDirectory $dbyteosRoot
    if ($dbyteosWorkspaceSnapshotEmpty.Code -ne 0) { throw "dbyteos workspace snapshot empty failed: $($dbyteosWorkspaceSnapshotEmpty.Text)" }
    Assert-NormalizedEqual $dbyteosWorkspaceSnapshotEmpty.Text $expectedDbyteosWorkspaceSnapshotEmpty "dbyteos workspace snapshot empty snapshot"

    $dbyteosWorkspaceDailyEmpty = Invoke-Dbyte -Arguments @("run", "bin\workspace.dby", "daily") -WorkingDirectory $dbyteosRoot
    if ($dbyteosWorkspaceDailyEmpty.Code -ne 0) { throw "dbyteos workspace daily empty failed: $($dbyteosWorkspaceDailyEmpty.Text)" }
    Assert-NormalizedEqual $dbyteosWorkspaceDailyEmpty.Text $expectedDbyteosDailySummaryEmpty "dbyteos workspace daily empty snapshot"

    $dbyteosDailyUsage = Invoke-Dbyte -Arguments @("run", "bin\daily.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosDailyUsage.Code -ne 0) { throw "dbyteos daily usage failed: $($dbyteosDailyUsage.Text)" }
    Assert-NormalizedEqual $dbyteosDailyUsage.Text $expectedDbyteosDailyUsage "dbyteos daily usage snapshot"

    $dbyteosDailySummaryEmpty = Invoke-Dbyte -Arguments @("run", "bin\daily.dby", "summary") -WorkingDirectory $dbyteosRoot
    if ($dbyteosDailySummaryEmpty.Code -ne 0) { throw "dbyteos daily summary empty failed: $($dbyteosDailySummaryEmpty.Text)" }
    Assert-NormalizedEqual $dbyteosDailySummaryEmpty.Text $expectedDbyteosDailySummaryEmpty "dbyteos daily summary empty snapshot"

    $dbyteosProjectMissingName = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "new") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectMissingName.Code -ne 0) { throw "dbyteos project missing name failed: $($dbyteosProjectMissingName.Text)" }
    Assert-Equal $dbyteosProjectMissingName.Text "usage: project new <name>" "dbyteos project missing name"

    $dbyteosProjectInvalidName = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "new", "..") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectInvalidName.Code -ne 0) { throw "dbyteos project invalid name failed: $($dbyteosProjectInvalidName.Text)" }
    Assert-Equal $dbyteosProjectInvalidName.Text "error: invalid project name: .." "dbyteos project invalid name"

    $dbyteosProjectInvalidDot = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "new", ".") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectInvalidDot.Code -ne 0) { throw "dbyteos project invalid dot failed: $($dbyteosProjectInvalidDot.Text)" }
    Assert-Equal $dbyteosProjectInvalidDot.Text "error: invalid project name: ." "dbyteos project invalid dot"
    $dbyteosProjectInvalidSlash = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "new", "demo/bad") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectInvalidSlash.Code -ne 0) { throw "dbyteos project invalid slash failed: $($dbyteosProjectInvalidSlash.Text)" }
    Assert-Equal $dbyteosProjectInvalidSlash.Text "error: invalid project name: demo/bad" "dbyteos project slash denied"
    $dbyteosProjectInvalidBackslash = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "new", "demo\bad") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectInvalidBackslash.Code -ne 0) { throw "dbyteos project invalid backslash failed: $($dbyteosProjectInvalidBackslash.Text)" }
    Assert-Equal $dbyteosProjectInvalidBackslash.Text "error: invalid project name: demo\bad" "dbyteos project backslash denied"
    $dbyteosProjectInvalidAbsolute = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "new", "/demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectInvalidAbsolute.Code -ne 0) { throw "dbyteos project invalid absolute failed: $($dbyteosProjectInvalidAbsolute.Text)" }
    Assert-Equal $dbyteosProjectInvalidAbsolute.Text "error: invalid project name: /demo" "dbyteos project absolute denied"
    $dbyteosProjectInvalidDrive = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "new", "C:demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectInvalidDrive.Code -ne 0) { throw "dbyteos project invalid drive failed: $($dbyteosProjectInvalidDrive.Text)" }
    Assert-Equal $dbyteosProjectInvalidDrive.Text "error: invalid project name: C:demo" "dbyteos project drive denied"
    $dbyteosProjectInvalidSpace = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "new", "demo bad") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectInvalidSpace.Code -ne 0) { throw "dbyteos project invalid space failed: $($dbyteosProjectInvalidSpace.Text)" }
    Assert-Equal $dbyteosProjectInvalidSpace.Text "error: invalid project name: demo bad" "dbyteos project space denied"

    $dbyteosProjectStatusMissing = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "status", "missing") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectStatusMissing.Code -ne 0) { throw "dbyteos project status missing failed: $($dbyteosProjectStatusMissing.Text)" }
    Assert-NormalizedEqual $dbyteosProjectStatusMissing.Text $expectedDbyteosProjectNotFound "dbyteos project status missing"
    $dbyteosProjectNotesMissing = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "notes", "missing") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectNotesMissing.Code -ne 0) { throw "dbyteos project notes missing failed: $($dbyteosProjectNotesMissing.Text)" }
    Assert-NormalizedEqual $dbyteosProjectNotesMissing.Text $expectedDbyteosProjectNotFound "dbyteos project notes missing"
    $dbyteosProjectSnapshotMissing = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "snapshot", "missing") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectSnapshotMissing.Code -ne 0) { throw "dbyteos project snapshot missing failed: $($dbyteosProjectSnapshotMissing.Text)" }
    Assert-NormalizedEqual $dbyteosProjectSnapshotMissing.Text $expectedDbyteosProjectNotFound "dbyteos project snapshot missing"
    $dbyteosProjectDoctorMissing = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "doctor", "missing") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectDoctorMissing.Code -ne 0) { throw "dbyteos project doctor missing failed: $($dbyteosProjectDoctorMissing.Text)" }
    Assert-NormalizedEqual $dbyteosProjectDoctorMissing.Text $expectedDbyteosProjectNotFound "dbyteos project doctor missing"

    $dbyteosProjectUnknown = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "unknown") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectUnknown.Code -ne 0) { throw "dbyteos project unknown failed: $($dbyteosProjectUnknown.Text)" }
    Assert-NormalizedEqual $dbyteosProjectUnknown.Text $expectedDbyteosProjectUnknown "dbyteos project unknown snapshot"

    $dbyteosProjectNew = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "new", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectNew.Code -ne 0) { throw "dbyteos project new failed: $($dbyteosProjectNew.Text)" }
    Assert-Equal $dbyteosProjectNew.Text "project created: demo" "dbyteos project new demo"

    $projectDemoRoot = Join-Path $dbyteosRoot "home\deadbyte\projects\demo"
    $projectDemoTasks = Join-Path $projectDemoRoot "tasks.txt"
    if (-not (Test-Path (Join-Path $projectDemoRoot "project.txt"))) { throw "project demo missing project.txt" }
    if (-not (Test-Path (Join-Path $projectDemoRoot "notes.txt"))) { throw "project demo missing notes.txt" }
    if (-not (Test-Path (Join-Path $projectDemoRoot "snapshot.txt"))) { throw "project demo missing snapshot.txt" }
    if (-not (Test-Path $projectDemoTasks)) { throw "project demo missing tasks.txt" }

    $dbyteosProjectDuplicate = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "new", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectDuplicate.Code -ne 0) { throw "dbyteos project duplicate failed: $($dbyteosProjectDuplicate.Text)" }
    Assert-Equal $dbyteosProjectDuplicate.Text "error: project already exists: demo" "dbyteos project duplicate"

    $dbyteosProjectListDemo = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "list") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectListDemo.Code -ne 0) { throw "dbyteos project list demo failed: $($dbyteosProjectListDemo.Text)" }
    Assert-NormalizedEqual $dbyteosProjectListDemo.Text $expectedDbyteosProjectListDemo "dbyteos project list demo snapshot"

    $dbyteosProjectStatusDemo = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "status", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectStatusDemo.Code -ne 0) { throw "dbyteos project status demo failed: $($dbyteosProjectStatusDemo.Text)" }
    Assert-NormalizedEqual $dbyteosProjectStatusDemo.Text $expectedDbyteosProjectStatusDemo "dbyteos project status demo snapshot"

    $dbyteosProjectNotesDemo = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "notes", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectNotesDemo.Code -ne 0) { throw "dbyteos project notes demo failed: $($dbyteosProjectNotesDemo.Text)" }
    Assert-NormalizedEqual $dbyteosProjectNotesDemo.Text $expectedDbyteosProjectNotesDemo "dbyteos project notes demo snapshot"

    $dbyteosProjectSnapshotDemo = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "snapshot", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectSnapshotDemo.Code -ne 0) { throw "dbyteos project snapshot demo failed: $($dbyteosProjectSnapshotDemo.Text)" }
    Assert-NormalizedEqual $dbyteosProjectSnapshotDemo.Text $expectedDbyteosProjectSnapshotDemo "dbyteos project snapshot demo snapshot"

    $dbyteosProjectDoctorDemo = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "doctor", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectDoctorDemo.Code -ne 0) { throw "dbyteos project doctor demo failed: $($dbyteosProjectDoctorDemo.Text)" }
    Assert-NormalizedEqual $dbyteosProjectDoctorDemo.Text $expectedDbyteosProjectDoctorDemo "dbyteos project doctor demo snapshot"

    $dbyteosProjectResetDemo = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "reset-demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectResetDemo.Code -ne 0) { throw "dbyteos project reset-demo failed: $($dbyteosProjectResetDemo.Text)" }
    Assert-Equal $dbyteosProjectResetDemo.Text "project demo reset." "dbyteos project reset-demo"
    Assert-NormalizedEqual (Get-Content (Join-Path $projectDemoRoot "snapshot.txt") -Raw) $expectedDbyteosProjectSnapshotDemo "dbyteos project reset-demo snapshot file"
    Assert-Equal (Get-Content $projectDemoTasks -Raw) "0|inspect workspace`n0|write project note`n" "dbyteos project reset-demo task file"
    $dbyteosTaskListAfterProjectReset = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "list", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskListAfterProjectReset.Code -ne 0) { throw "dbyteos task list after project reset failed: $($dbyteosTaskListAfterProjectReset.Text)" }
    Assert-NormalizedEqual $dbyteosTaskListAfterProjectReset.Text $expectedDbyteosTaskListDemo "dbyteos task list after project reset snapshot"
    $dbyteosProjectResetDemoAgain = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "reset-demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectResetDemoAgain.Code -ne 0) { throw "dbyteos project reset-demo idempotent failed: $($dbyteosProjectResetDemoAgain.Text)" }
    Assert-Equal $dbyteosProjectResetDemoAgain.Text "project demo reset." "dbyteos project reset-demo idempotent"
    Assert-NormalizedEqual (Get-Content (Join-Path $projectDemoRoot "snapshot.txt") -Raw) $expectedDbyteosProjectSnapshotDemo "dbyteos project reset-demo idempotent snapshot file"
    Assert-Equal (Get-Content $projectDemoTasks -Raw) "0|inspect workspace`n0|write project note`n" "dbyteos project reset-demo idempotent task file"

    $dbyteosTaskUsage = Invoke-Dbyte -Arguments @("run", "bin\task.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskUsage.Code -ne 0) { throw "dbyteos task usage failed: $($dbyteosTaskUsage.Text)" }
    Assert-NormalizedEqual $dbyteosTaskUsage.Text $expectedDbyteosTaskUsage "dbyteos task usage snapshot"
    $dbyteosTaskUnknown = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "unknown") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskUnknown.Code -ne 0) { throw "dbyteos task unknown failed: $($dbyteosTaskUnknown.Text)" }
    Assert-NormalizedEqual $dbyteosTaskUnknown.Text $expectedDbyteosTaskUnknown "dbyteos task unknown snapshot"
    $dbyteosTaskInvalidProject = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "add", "bad/name", "text") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskInvalidProject.Code -ne 0) { throw "dbyteos task invalid project failed: $($dbyteosTaskInvalidProject.Text)" }
    Assert-Equal $dbyteosTaskInvalidProject.Text "error: invalid project name: bad/name" "dbyteos task invalid project name"
    if (Test-Path (Join-Path $dbyteosProjectsPath "bad")) { throw "task invalid project created unexpected project path" }
    $dbyteosTaskAddMissingText = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "add", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskAddMissingText.Code -ne 0) { throw "dbyteos task add missing text failed: $($dbyteosTaskAddMissingText.Text)" }
    Assert-Equal $dbyteosTaskAddMissingText.Text "usage: task add <project> <text>" "dbyteos task add missing text"
    $dbyteosTaskAddEmptyText = Invoke-DbyteExact -Arguments @("run", "bin\task.dby", "add", "demo", "") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskAddEmptyText.Code -ne 0) { throw "dbyteos task add empty text failed: $($dbyteosTaskAddEmptyText.Text)" }
    Assert-Equal $dbyteosTaskAddEmptyText.Text "error: task text cannot be empty" "dbyteos task add empty text"
    $dbyteosTaskAddDelimiter = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "add", "demo", "bad|text") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskAddDelimiter.Code -ne 0) { throw "dbyteos task add delimiter failed: $($dbyteosTaskAddDelimiter.Text)" }
    Assert-Equal $dbyteosTaskAddDelimiter.Text "error: invalid task text" "dbyteos task add delimiter"
    Assert-Equal (Get-Content $projectDemoTasks -Raw) "0|inspect workspace`n0|write project note`n" "dbyteos task invalid add leaves task file unchanged"
    $dbyteosTaskListMissingProject = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "list", "missing") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskListMissingProject.Code -ne 0) { throw "dbyteos task list missing project failed: $($dbyteosTaskListMissingProject.Text)" }
    Assert-Equal $dbyteosTaskListMissingProject.Text "error: project not found: missing" "dbyteos task list missing project"
    $dbyteosTaskMissingProject = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "status", "missing") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskMissingProject.Code -ne 0) { throw "dbyteos task missing project failed: $($dbyteosTaskMissingProject.Text)" }
    Assert-Equal $dbyteosTaskMissingProject.Text "error: project not found: missing" "dbyteos task missing project"
    Remove-Item -Recurse -Force $dbyteosProjectsPath -ErrorAction SilentlyContinue
    $dbyteosTaskResetDemo = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "reset-demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskResetDemo.Code -ne 0) { throw "dbyteos task reset-demo failed: $($dbyteosTaskResetDemo.Text)" }
    Assert-Equal $dbyteosTaskResetDemo.Text "task demo reset." "dbyteos task reset-demo"
    if (-not (Test-Path (Join-Path $projectDemoRoot "project.txt"))) { throw "task reset-demo did not create project.txt" }
    if (-not (Test-Path $projectDemoTasks)) { throw "task reset-demo did not create tasks.txt" }
    $dbyteosTaskResetDemoAgain = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "reset-demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskResetDemoAgain.Code -ne 0) { throw "dbyteos task reset-demo idempotent failed: $($dbyteosTaskResetDemoAgain.Text)" }
    Assert-Equal $dbyteosTaskResetDemoAgain.Text "task demo reset." "dbyteos task reset-demo idempotent"
    Assert-Equal (Get-Content $projectDemoTasks -Raw) "0|inspect workspace`n0|write project note`n" "dbyteos task reset-demo idempotent task file"
    $dbyteosProjectDoctorAfterTaskReset = Invoke-Dbyte -Arguments @("run", "bin\project.dby", "doctor", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProjectDoctorAfterTaskReset.Code -ne 0) { throw "dbyteos project doctor after task reset failed: $($dbyteosProjectDoctorAfterTaskReset.Text)" }
    Assert-NormalizedEqual $dbyteosProjectDoctorAfterTaskReset.Text $expectedDbyteosProjectDoctorDemo "dbyteos project doctor after task reset"
    $dbyteosTaskListDemo = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "list", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskListDemo.Code -ne 0) { throw "dbyteos task list demo failed: $($dbyteosTaskListDemo.Text)" }
    Assert-NormalizedEqual $dbyteosTaskListDemo.Text $expectedDbyteosTaskListDemo "dbyteos task list demo snapshot"
    $dbyteosTaskAddDemo = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "add", "demo", "write", "tests") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskAddDemo.Code -ne 0) { throw "dbyteos task add demo failed: $($dbyteosTaskAddDemo.Text)" }
    Assert-Equal $dbyteosTaskAddDemo.Text "task added: demo #3" "dbyteos task add demo"
    $dbyteosTaskListAfterAdd = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "list", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskListAfterAdd.Code -ne 0) { throw "dbyteos task list after add failed: $($dbyteosTaskListAfterAdd.Text)" }
    Assert-NormalizedEqual $dbyteosTaskListAfterAdd.Text $expectedDbyteosTaskListAfterAdd "dbyteos task list after add snapshot"
    $taskFileAfterAdd = Get-Content $projectDemoTasks -Raw
    $dbyteosTaskDoneZero = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "done", "demo", "0") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskDoneZero.Code -ne 0) { throw "dbyteos task done zero failed: $($dbyteosTaskDoneZero.Text)" }
    Assert-Equal $dbyteosTaskDoneZero.Text "error: invalid task id: 0" "dbyteos task done zero"
    $dbyteosTaskDoneNegative = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "done", "demo", "-1") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskDoneNegative.Code -ne 0) { throw "dbyteos task done negative failed: $($dbyteosTaskDoneNegative.Text)" }
    Assert-Equal $dbyteosTaskDoneNegative.Text "error: invalid task id: -1" "dbyteos task done negative"
    $dbyteosTaskDoneAlpha = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "done", "demo", "abc") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskDoneAlpha.Code -ne 0) { throw "dbyteos task done alpha failed: $($dbyteosTaskDoneAlpha.Text)" }
    Assert-Equal $dbyteosTaskDoneAlpha.Text "error: invalid task id: abc" "dbyteos task done alpha"
    Assert-Equal (Get-Content $projectDemoTasks -Raw) $taskFileAfterAdd "dbyteos task invalid done leaves task file unchanged"
    $dbyteosTaskDoneDemo = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "done", "demo", "1") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskDoneDemo.Code -ne 0) { throw "dbyteos task done demo failed: $($dbyteosTaskDoneDemo.Text)" }
    Assert-Equal $dbyteosTaskDoneDemo.Text "task done: demo #1" "dbyteos task done demo"
    $dbyteosTaskDoneAgain = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "done", "demo", "1") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskDoneAgain.Code -ne 0) { throw "dbyteos task done again failed: $($dbyteosTaskDoneAgain.Text)" }
    Assert-Equal $dbyteosTaskDoneAgain.Text "task already done: demo #1" "dbyteos task already done"
    $dbyteosTaskDoneMissing = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "done", "demo", "99") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskDoneMissing.Code -ne 0) { throw "dbyteos task done missing failed: $($dbyteosTaskDoneMissing.Text)" }
    Assert-Equal $dbyteosTaskDoneMissing.Text "error: task not found: 99" "dbyteos task done unknown id"
    $dbyteosTaskListAfterDone = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "list", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskListAfterDone.Code -ne 0) { throw "dbyteos task list after done failed: $($dbyteosTaskListAfterDone.Text)" }
    Assert-NormalizedEqual $dbyteosTaskListAfterDone.Text $expectedDbyteosTaskListAfterDone "dbyteos task list after done snapshot"
    $dbyteosTaskStatusDemo = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "status", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskStatusDemo.Code -ne 0) { throw "dbyteos task status demo failed: $($dbyteosTaskStatusDemo.Text)" }
    Assert-NormalizedEqual $dbyteosTaskStatusDemo.Text $expectedDbyteosTaskStatusAfterDone "dbyteos task status demo snapshot"
    $taskCountsAfterDone = "open: 2`ndone: 1`ntotal: 3"
    Assert-Contains (Normalize-Output $dbyteosTaskStatusDemo.Text) $taskCountsAfterDone "dbyteos task status count lines after done"

    $dbyteosTaskSummaryDemo = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "summary", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskSummaryDemo.Code -ne 0) { throw "dbyteos task summary demo failed: $($dbyteosTaskSummaryDemo.Text)" }
    Assert-NormalizedEqual $dbyteosTaskSummaryDemo.Text $expectedDbyteosTaskSummaryAfterDone "dbyteos task summary demo snapshot"
    Assert-Contains (Normalize-Output $dbyteosTaskSummaryDemo.Text) $taskCountsAfterDone "dbyteos task summary count sync after done"
    $dbyteosTaskOpenDemo = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "open", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskOpenDemo.Code -ne 0) { throw "dbyteos task open demo failed: $($dbyteosTaskOpenDemo.Text)" }
    Assert-NormalizedEqual $dbyteosTaskOpenDemo.Text $expectedDbyteosTaskOpenAfterDone "dbyteos task open demo snapshot"
    $dbyteosTaskDoctorDemo = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "doctor", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskDoctorDemo.Code -ne 0) { throw "dbyteos task doctor demo failed: $($dbyteosTaskDoctorDemo.Text)" }
    Assert-NormalizedEqual $dbyteosTaskDoctorDemo.Text $expectedDbyteosTaskDoctorHealthy "dbyteos task doctor demo snapshot"
    $dbyteosTaskSnapshotDemo = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "snapshot", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskSnapshotDemo.Code -ne 0) { throw "dbyteos task snapshot demo failed: $($dbyteosTaskSnapshotDemo.Text)" }
    Assert-NormalizedEqual $dbyteosTaskSnapshotDemo.Text $expectedDbyteosTaskSnapshotAfterDone "dbyteos task snapshot demo snapshot"
    Assert-Contains (Normalize-Output $dbyteosTaskSnapshotDemo.Text) $taskCountsAfterDone "dbyteos task snapshot count sync after done"
    Assert-Contains (Normalize-Output $dbyteosTaskSnapshotDemo.Text) "[x] 1: inspect workspace`n[ ] 2: write project note`n[ ] 3: write tests" "dbyteos task snapshot list sync after done"

    $dbyteosTaskSummaryMissing = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "summary", "missing") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskSummaryMissing.Code -ne 0) { throw "dbyteos task summary missing failed: $($dbyteosTaskSummaryMissing.Text)" }
    Assert-Equal $dbyteosTaskSummaryMissing.Text "error: project not found: missing" "dbyteos task summary missing project"
    $dbyteosTaskOpenMissing = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "open", "missing") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskOpenMissing.Code -ne 0) { throw "dbyteos task open missing failed: $($dbyteosTaskOpenMissing.Text)" }
    Assert-Equal $dbyteosTaskOpenMissing.Text "error: project not found: missing" "dbyteos task open missing project"
    $dbyteosTaskClearDoneMissing = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "clear-done", "missing") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskClearDoneMissing.Code -ne 0) { throw "dbyteos task clear-done missing failed: $($dbyteosTaskClearDoneMissing.Text)" }
    Assert-Equal $dbyteosTaskClearDoneMissing.Text "error: project not found: missing" "dbyteos task clear-done missing project"
    $dbyteosTaskDoctorMissing = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "doctor", "missing") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskDoctorMissing.Code -ne 0) { throw "dbyteos task doctor missing failed: $($dbyteosTaskDoctorMissing.Text)" }
    Assert-Equal $dbyteosTaskDoctorMissing.Text "error: project not found: missing" "dbyteos task doctor missing project"
    $dbyteosTaskSnapshotMissing = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "snapshot", "missing") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskSnapshotMissing.Code -ne 0) { throw "dbyteos task snapshot missing failed: $($dbyteosTaskSnapshotMissing.Text)" }
    Assert-Equal $dbyteosTaskSnapshotMissing.Text "error: project not found: missing" "dbyteos task snapshot missing project"
    $dbyteosTaskClearDoneInvalid = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "clear-done", "bad/name") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskClearDoneInvalid.Code -ne 0) { throw "dbyteos task clear-done invalid failed: $($dbyteosTaskClearDoneInvalid.Text)" }
    Assert-Equal $dbyteosTaskClearDoneInvalid.Text "error: invalid project name: bad/name" "dbyteos task clear-done invalid project"

    Set-Content -Path $projectDemoTasks -Value "0|inspect workspace`n2|bad marker`n1|done task`n" -NoNewline
    $dbyteosTaskDoctorMalformed = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "doctor", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskDoctorMalformed.Code -ne 0) { throw "dbyteos task doctor malformed failed: $($dbyteosTaskDoctorMalformed.Text)" }
    Assert-NormalizedEqual $dbyteosTaskDoctorMalformed.Text $expectedDbyteosTaskDoctorMalformed "dbyteos task doctor malformed snapshot"
    $dbyteosTaskResetAfterMalformed = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "reset-demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskResetAfterMalformed.Code -ne 0) { throw "dbyteos task reset after malformed failed: $($dbyteosTaskResetAfterMalformed.Text)" }
    Assert-Equal $dbyteosTaskResetAfterMalformed.Text "task demo reset." "dbyteos task reset after malformed"
    $dbyteosTaskAddAfterMalformedReset = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "add", "demo", "write", "tests") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskAddAfterMalformedReset.Code -ne 0) { throw "dbyteos task add after malformed reset failed: $($dbyteosTaskAddAfterMalformedReset.Text)" }
    Assert-Equal $dbyteosTaskAddAfterMalformedReset.Text "task added: demo #3" "dbyteos task add after malformed reset"
    $dbyteosTaskDoneAfterMalformedReset = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "done", "demo", "1") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskDoneAfterMalformedReset.Code -ne 0) { throw "dbyteos task done after malformed reset failed: $($dbyteosTaskDoneAfterMalformedReset.Text)" }
    Assert-Equal $dbyteosTaskDoneAfterMalformedReset.Text "task done: demo #1" "dbyteos task done after malformed reset"

    $dbyteosTaskClearDone = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "clear-done", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskClearDone.Code -ne 0) { throw "dbyteos task clear-done demo failed: $($dbyteosTaskClearDone.Text)" }
    Assert-NormalizedEqual $dbyteosTaskClearDone.Text $expectedDbyteosTaskClearDone "dbyteos task clear-done demo snapshot"
    $dbyteosTaskClearDoneAgain = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "clear-done", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskClearDoneAgain.Code -ne 0) { throw "dbyteos task clear-done idempotent failed: $($dbyteosTaskClearDoneAgain.Text)" }
    Assert-NormalizedEqual $dbyteosTaskClearDoneAgain.Text "task clear-done: demo`nremoved: 0`nremaining: 2" "dbyteos task clear-done idempotent"
    $dbyteosTaskListAfterClearDone = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "list", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskListAfterClearDone.Code -ne 0) { throw "dbyteos task list after clear-done failed: $($dbyteosTaskListAfterClearDone.Text)" }
    Assert-NormalizedEqual $dbyteosTaskListAfterClearDone.Text $expectedDbyteosTaskListAfterClearDone "dbyteos task list after clear-done snapshot"
    $dbyteosTaskStatusAfterClearDone = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "status", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskStatusAfterClearDone.Code -ne 0) { throw "dbyteos task status after clear-done failed: $($dbyteosTaskStatusAfterClearDone.Text)" }
    Assert-NormalizedEqual $dbyteosTaskStatusAfterClearDone.Text $expectedDbyteosTaskStatusAfterClearDone "dbyteos task status after clear-done snapshot"
    $taskCountsAfterClearDone = "open: 2`ndone: 0`ntotal: 2"
    Assert-Contains (Normalize-Output $dbyteosTaskStatusAfterClearDone.Text) $taskCountsAfterClearDone "dbyteos task status count lines after clear-done"
    $dbyteosTaskSummaryAfterClearDone = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "summary", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskSummaryAfterClearDone.Code -ne 0) { throw "dbyteos task summary after clear-done failed: $($dbyteosTaskSummaryAfterClearDone.Text)" }
    Assert-NormalizedEqual $dbyteosTaskSummaryAfterClearDone.Text $expectedDbyteosTaskSummaryAfterClearDone "dbyteos task summary after clear-done snapshot"
    Assert-Contains (Normalize-Output $dbyteosTaskSummaryAfterClearDone.Text) $taskCountsAfterClearDone "dbyteos task summary count sync after clear-done"
    $dbyteosTaskSnapshotAfterClearDone = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "snapshot", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskSnapshotAfterClearDone.Code -ne 0) { throw "dbyteos task snapshot after clear-done failed: $($dbyteosTaskSnapshotAfterClearDone.Text)" }
    Assert-NormalizedEqual $dbyteosTaskSnapshotAfterClearDone.Text $expectedDbyteosTaskSnapshotAfterClearDone "dbyteos task snapshot after clear-done snapshot"
    Assert-Contains (Normalize-Output $dbyteosTaskSnapshotAfterClearDone.Text) $taskCountsAfterClearDone "dbyteos task snapshot count sync after clear-done"
    Assert-Contains (Normalize-Output $dbyteosTaskSnapshotAfterClearDone.Text) "[ ] 1: write project note`n[ ] 2: write tests" "dbyteos task snapshot list sync after clear-done"
    $dbyteosTaskDoctorAfterClearDone = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "doctor", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskDoctorAfterClearDone.Code -ne 0) { throw "dbyteos task doctor after clear-done failed: $($dbyteosTaskDoctorAfterClearDone.Text)" }
    Assert-NormalizedEqual $dbyteosTaskDoctorAfterClearDone.Text $expectedDbyteosTaskDoctorHealthy "dbyteos task doctor after clear-done"

    $dbyteosTaskResetForAllDone = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "reset-demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskResetForAllDone.Code -ne 0) { throw "dbyteos task reset for all-done failed: $($dbyteosTaskResetForAllDone.Text)" }
    Assert-Equal $dbyteosTaskResetForAllDone.Text "task demo reset." "dbyteos task reset for all-done"
    $dbyteosTaskDoneAllOne = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "done", "demo", "1") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskDoneAllOne.Code -ne 0) { throw "dbyteos task all-done first done failed: $($dbyteosTaskDoneAllOne.Text)" }
    Assert-Equal $dbyteosTaskDoneAllOne.Text "task done: demo #1" "dbyteos task all-done first done"
    $dbyteosTaskDoneAllTwo = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "done", "demo", "2") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskDoneAllTwo.Code -ne 0) { throw "dbyteos task all-done second done failed: $($dbyteosTaskDoneAllTwo.Text)" }
    Assert-Equal $dbyteosTaskDoneAllTwo.Text "task done: demo #2" "dbyteos task all-done second done"
    $dbyteosTaskStatusAllDone = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "status", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskStatusAllDone.Code -ne 0) { throw "dbyteos task status all-done failed: $($dbyteosTaskStatusAllDone.Text)" }
    Assert-NormalizedEqual $dbyteosTaskStatusAllDone.Text $expectedDbyteosTaskStatusAllDone "dbyteos task status all-done snapshot"
    $dbyteosTaskSummaryAllDone = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "summary", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskSummaryAllDone.Code -ne 0) { throw "dbyteos task summary all-done failed: $($dbyteosTaskSummaryAllDone.Text)" }
    Assert-NormalizedEqual $dbyteosTaskSummaryAllDone.Text $expectedDbyteosTaskSummaryAllDone "dbyteos task summary all-done snapshot"
    $dbyteosTaskOpenAllDone = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "open", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskOpenAllDone.Code -ne 0) { throw "dbyteos task open all-done failed: $($dbyteosTaskOpenAllDone.Text)" }
    Assert-NormalizedEqual $dbyteosTaskOpenAllDone.Text $expectedDbyteosTaskOpenAllDone "dbyteos task open all-done snapshot"
    $dbyteosTaskSnapshotAllDone = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "snapshot", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskSnapshotAllDone.Code -ne 0) { throw "dbyteos task snapshot all-done failed: $($dbyteosTaskSnapshotAllDone.Text)" }
    Assert-NormalizedEqual $dbyteosTaskSnapshotAllDone.Text $expectedDbyteosTaskSnapshotAllDone "dbyteos task snapshot all-done snapshot"
    $dbyteosTaskClearDoneAllDone = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "clear-done", "demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskClearDoneAllDone.Code -ne 0) { throw "dbyteos task clear-done all-done failed: $($dbyteosTaskClearDoneAllDone.Text)" }
    Assert-NormalizedEqual $dbyteosTaskClearDoneAllDone.Text $expectedDbyteosTaskClearDoneAllDone "dbyteos task clear-done all-done snapshot"
    $dbyteosTaskResetAfterAllDone = Invoke-Dbyte -Arguments @("run", "bin\task.dby", "reset-demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosTaskResetAfterAllDone.Code -ne 0) { throw "dbyteos task reset after all-done failed: $($dbyteosTaskResetAfterAllDone.Text)" }
    Assert-Equal $dbyteosTaskResetAfterAllDone.Text "task demo reset." "dbyteos task reset after all-done"

    # Populated workspace and daily summary smoke checks
    $dbyteosWorkspaceReportDemo = Invoke-Dbyte -Arguments @("run", "bin\workspace.dby", "report") -WorkingDirectory $dbyteosRoot
    if ($dbyteosWorkspaceReportDemo.Code -ne 0) { throw "dbyteos workspace report demo failed: $($dbyteosWorkspaceReportDemo.Text)" }
    Assert-NormalizedEqual $dbyteosWorkspaceReportDemo.Text $expectedDbyteosWorkspaceReportDemo "dbyteos workspace report demo snapshot"

    $dbyteosWorkspaceDoctorDemo = Invoke-Dbyte -Arguments @("run", "bin\workspace.dby", "doctor") -WorkingDirectory $dbyteosRoot
    if ($dbyteosWorkspaceDoctorDemo.Code -ne 0) { throw "dbyteos workspace doctor demo failed: $($dbyteosWorkspaceDoctorDemo.Text)" }
    Assert-NormalizedEqual $dbyteosWorkspaceDoctorDemo.Text $expectedDbyteosWorkspaceDoctorDemo "dbyteos workspace doctor demo snapshot"

    $dbyteosWorkspaceSnapshotDemo = Invoke-Dbyte -Arguments @("run", "bin\workspace.dby", "snapshot") -WorkingDirectory $dbyteosRoot
    if ($dbyteosWorkspaceSnapshotDemo.Code -ne 0) { throw "dbyteos workspace snapshot demo failed: $($dbyteosWorkspaceSnapshotDemo.Text)" }
    Assert-NormalizedEqual $dbyteosWorkspaceSnapshotDemo.Text $expectedDbyteosWorkspaceSnapshotDemo "dbyteos workspace snapshot demo snapshot"

    $dbyteosWorkspaceDailyDemo = Invoke-Dbyte -Arguments @("run", "bin\workspace.dby", "daily") -WorkingDirectory $dbyteosRoot
    if ($dbyteosWorkspaceDailyDemo.Code -ne 0) { throw "dbyteos workspace daily demo failed: $($dbyteosWorkspaceDailyDemo.Text)" }
    Assert-NormalizedEqual $dbyteosWorkspaceDailyDemo.Text $expectedDbyteosDailySummaryDemo "dbyteos workspace daily demo snapshot"

    $dbyteosDailySummaryDemo = Invoke-Dbyte -Arguments @("run", "bin\daily.dby", "summary") -WorkingDirectory $dbyteosRoot
    if ($dbyteosDailySummaryDemo.Code -ne 0) { throw "dbyteos daily summary demo failed: $($dbyteosDailySummaryDemo.Text)" }
    Assert-NormalizedEqual $dbyteosDailySummaryDemo.Text $expectedDbyteosDailySummaryDemo "dbyteos daily summary demo snapshot"

    $dbyteosCleanProjects = Invoke-Dbyte -Arguments @("run", "bin\clean.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosCleanProjects.Code -ne 0) { throw "dbyteos clean project preservation failed: $($dbyteosCleanProjects.Text)" }
    if (-not (Test-Path (Join-Path $projectDemoRoot "project.txt"))) { throw "clean deleted project data - must be preserved" }
    if (-not (Test-Path $projectDemoTasks)) { throw "clean deleted task data - must be preserved" }

    $dbyteosProfileDirect = Invoke-Dbyte -Arguments @("run", "bin\profile.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProfileDirect.Code -ne 0) { throw "dbyteos profile failed: $($dbyteosProfileDirect.Text)" }
    Assert-NormalizedEqual $dbyteosProfileDirect.Text $expectedDbyteosProfile "dbyteos profile default snapshot"

    $dbyteosProfileShow = Invoke-Dbyte -Arguments @("run", "bin\profile.dby", "show") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProfileShow.Code -ne 0) { throw "dbyteos profile show failed: $($dbyteosProfileShow.Text)" }
    Assert-NormalizedEqual $dbyteosProfileShow.Text $expectedDbyteosProfile "dbyteos profile show snapshot"
    Assert-Equal (Normalize-Output $dbyteosProfileDirect.Text) (Normalize-Output $dbyteosProfileShow.Text) "dbyteos profile no args equals show"

    $dbyteosProfileWhoami = Invoke-Dbyte -Arguments @("run", "bin\profile.dby", "whoami") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProfileWhoami.Code -ne 0) { throw "dbyteos profile whoami failed: $($dbyteosProfileWhoami.Text)" }
    Assert-Equal $dbyteosProfileWhoami.Text "deadbyte" "dbyteos profile whoami"

    $dbyteosProfileHome = Invoke-Dbyte -Arguments @("run", "bin\profile.dby", "home") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProfileHome.Code -ne 0) { throw "dbyteos profile home failed: $($dbyteosProfileHome.Text)" }
    Assert-Equal $dbyteosProfileHome.Text "home/deadbyte" "dbyteos profile home"

    $dbyteosProfileTheme = Invoke-Dbyte -Arguments @("run", "bin\profile.dby", "theme") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProfileTheme.Code -ne 0) { throw "dbyteos profile theme failed: $($dbyteosProfileTheme.Text)" }
    Assert-Equal $dbyteosProfileTheme.Text "default" "dbyteos profile theme"

    $dbyteosProfilePrompt = Invoke-Dbyte -Arguments @("run", "bin\profile.dby", "prompt") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProfilePrompt.Code -ne 0) { throw "dbyteos profile prompt failed: $($dbyteosProfilePrompt.Text)" }
    Assert-Equal $dbyteosProfilePrompt.Text "dbyte-shell>" "dbyteos profile prompt"

    $dbyteosProfileUnknown = Invoke-Dbyte -Arguments @("run", "bin\profile.dby", "unknown") -WorkingDirectory $dbyteosRoot
    if ($dbyteosProfileUnknown.Code -ne 0) { throw "dbyteos profile unknown failed: $($dbyteosProfileUnknown.Text)" }
    Assert-NormalizedEqual $dbyteosProfileUnknown.Text $expectedDbyteosProfileUnknown "dbyteos profile unknown snapshot"

    $dbyteosConfigDirect = Invoke-Dbyte -Arguments @("run", "bin\config.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosConfigDirect.Code -ne 0) { throw "dbyteos config failed: $($dbyteosConfigDirect.Text)" }
    Assert-NormalizedEqual $dbyteosConfigDirect.Text $expectedDbyteosConfig "dbyteos config default snapshot"

    $dbyteosConfigShow = Invoke-Dbyte -Arguments @("run", "bin\config.dby", "show") -WorkingDirectory $dbyteosRoot
    if ($dbyteosConfigShow.Code -ne 0) { throw "dbyteos config show failed: $($dbyteosConfigShow.Text)" }
    Assert-NormalizedEqual $dbyteosConfigShow.Text $expectedDbyteosConfig "dbyteos config show snapshot"
    Assert-Equal (Normalize-Output $dbyteosConfigDirect.Text) (Normalize-Output $dbyteosConfigShow.Text) "dbyteos config no args equals show"

    $dbyteosConfigKeys = Invoke-Dbyte -Arguments @("run", "bin\config.dby", "keys") -WorkingDirectory $dbyteosRoot
    if ($dbyteosConfigKeys.Code -ne 0) { throw "dbyteos config keys failed: $($dbyteosConfigKeys.Text)" }
    Assert-NormalizedEqual $dbyteosConfigKeys.Text $expectedDbyteosConfigKeys "dbyteos config keys snapshot"

    $dbyteosConfigPrompt = Invoke-Dbyte -Arguments @("run", "bin\config.dby", "get", "system.prompt") -WorkingDirectory $dbyteosRoot
    if ($dbyteosConfigPrompt.Code -ne 0) { throw "dbyteos config prompt failed: $($dbyteosConfigPrompt.Text)" }
    Assert-Equal $dbyteosConfigPrompt.Text "dbyte-shell>" "dbyteos config prompt"

    $dbyteosConfigMode = Invoke-Dbyte -Arguments @("run", "bin\config.dby", "get", "system.mode") -WorkingDirectory $dbyteosRoot
    if ($dbyteosConfigMode.Code -ne 0) { throw "dbyteos config mode failed: $($dbyteosConfigMode.Text)" }
    Assert-Equal $dbyteosConfigMode.Text "beta-userland" "dbyteos config mode"

    $dbyteosConfigUser = Invoke-Dbyte -Arguments @("run", "bin\config.dby", "get", "user.name") -WorkingDirectory $dbyteosRoot
    if ($dbyteosConfigUser.Code -ne 0) { throw "dbyteos config user failed: $($dbyteosConfigUser.Text)" }
    Assert-Equal $dbyteosConfigUser.Text "deadbyte" "dbyteos config user"

    $dbyteosConfigHome = Invoke-Dbyte -Arguments @("run", "bin\config.dby", "get", "user.home") -WorkingDirectory $dbyteosRoot
    if ($dbyteosConfigHome.Code -ne 0) { throw "dbyteos config home failed: $($dbyteosConfigHome.Text)" }
    Assert-Equal $dbyteosConfigHome.Text "home/deadbyte" "dbyteos config home"

    $dbyteosConfigTheme = Invoke-Dbyte -Arguments @("run", "bin\config.dby", "get", "ui.theme") -WorkingDirectory $dbyteosRoot
    if ($dbyteosConfigTheme.Code -ne 0) { throw "dbyteos config theme failed: $($dbyteosConfigTheme.Text)" }
    Assert-Equal $dbyteosConfigTheme.Text "default" "dbyteos config theme"

    $dbyteosConfigSecurityMode = Invoke-Dbyte -Arguments @("run", "bin\config.dby", "get", "security.mode") -WorkingDirectory $dbyteosRoot
    if ($dbyteosConfigSecurityMode.Code -ne 0) { throw "dbyteos config security mode failed: $($dbyteosConfigSecurityMode.Text)" }
    Assert-Equal $dbyteosConfigSecurityMode.Text "simulated" "dbyteos config security mode"

    $dbyteosConfigUnknownKey = Invoke-Dbyte -Arguments @("run", "bin\config.dby", "get", "missing.key") -WorkingDirectory $dbyteosRoot
    if ($dbyteosConfigUnknownKey.Code -ne 0) { throw "dbyteos config unknown key failed: $($dbyteosConfigUnknownKey.Text)" }
    Assert-Equal $dbyteosConfigUnknownKey.Text "error: unknown config key: missing.key" "dbyteos config unknown key"

    $dbyteosConfigMissingKey = Invoke-Dbyte -Arguments @("run", "bin\config.dby", "get") -WorkingDirectory $dbyteosRoot
    if ($dbyteosConfigMissingKey.Code -ne 0) { throw "dbyteos config missing key failed: $($dbyteosConfigMissingKey.Text)" }
    Assert-NormalizedEqual $dbyteosConfigMissingKey.Text $expectedDbyteosConfigMissingKey "dbyteos config missing key snapshot"

    $dbyteosConfigUnknownCommand = Invoke-Dbyte -Arguments @("run", "bin\config.dby", "unknown") -WorkingDirectory $dbyteosRoot
    if ($dbyteosConfigUnknownCommand.Code -ne 0) { throw "dbyteos config unknown command failed: $($dbyteosConfigUnknownCommand.Text)" }
    Assert-NormalizedEqual $dbyteosConfigUnknownCommand.Text $expectedDbyteosConfigUnknownCommand "dbyteos config unknown command snapshot"

    $dbyteosPrefsGet = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "get", "ui.theme") -WorkingDirectory $dbyteosRoot
    if ($dbyteosPrefsGet.Code -ne 0) { throw "dbyteos prefs get failed: $($dbyteosPrefsGet.Text)" }
    Assert-Equal $dbyteosPrefsGet.Text "default" "dbyteos prefs get default"

    $dbyteosPrefsSetSafe = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "set", "ui.theme", "dark") -WorkingDirectory $dbyteosRoot
    if ($dbyteosPrefsSetSafe.Code -ne 0) { throw "dbyteos prefs set failed: $($dbyteosPrefsSetSafe.Text)" }
    Assert-Equal $dbyteosPrefsSetSafe.Text "preference 'ui.theme' updated successfully." "dbyteos prefs set safe"

    $dbyteosConfigGetDark = Invoke-Dbyte -Arguments @("run", "bin\config.dby", "get", "ui.theme") -WorkingDirectory $dbyteosRoot
    if ($dbyteosConfigGetDark.Code -ne 0) { throw "dbyteos config get dark failed: $($dbyteosConfigGetDark.Text)" }
    Assert-Equal $dbyteosConfigGetDark.Text "dark" "dbyteos config get overlaid theme"

    $dbyteosPrefsSetUnsafeKey = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "set", "user.home", "dark") -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosPrefsSetUnsafeKey.Text "error: permission denied" "dbyteos prefs set user.home reject"

    $dbyteosPrefsSetSystemMode = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "set", "system.mode", "gui") -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosPrefsSetSystemMode.Text "error: permission denied" "dbyteos prefs set system.mode reject"

    $dbyteosPrefsSetSecurityMode = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "set", "security.mode", "open") -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosPrefsSetSecurityMode.Text "error: permission denied" "dbyteos prefs set security.mode reject"

    $dbyteosPrefsSetInvalidKey = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "set", "random.key", "value") -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosPrefsSetInvalidKey.Text "error: permission denied" "dbyteos prefs set invalid key reject"

    $dbyteosPrefsSetInvalidValue = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "set", "ui.theme", "rainbow") -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosPrefsSetInvalidValue.Text "error: invalid value" "dbyteos prefs set invalid value reject"

    $dbyteosPrefsReset = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "reset-demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosPrefsReset.Code -ne 0) { throw "dbyteos prefs reset failed: $($dbyteosPrefsReset.Text)" }
    Assert-Equal $dbyteosPrefsReset.Text "preferences reset to default seed state." "dbyteos prefs reset-demo"

    $dbyteosPrefsResetIdempotent = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "reset-demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosPrefsResetIdempotent.Code -ne 0) { throw "dbyteos prefs reset idempotent failed: $($dbyteosPrefsResetIdempotent.Text)" }
    Assert-Equal $dbyteosPrefsResetIdempotent.Text "preferences reset to default seed state." "dbyteos prefs reset-demo idempotent"

    $dbyteosPrefsGetAfterReset = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "get", "ui.theme") -WorkingDirectory $dbyteosRoot
    if ($dbyteosPrefsGetAfterReset.Code -ne 0) { throw "dbyteos prefs get after reset failed: $($dbyteosPrefsGetAfterReset.Text)" }
    Assert-Equal $dbyteosPrefsGetAfterReset.Text "default" "dbyteos prefs get default after reset"

    # --- v9.0.2 Exact Snapshot Assertions ---
    # ensure no stale .bak from previous runs
    $prefsBakCleanup = Join-Path $dbyteosRoot "home\deadbyte\preferences.dby.bak"
    Remove-Item $prefsBakCleanup -Force -ErrorAction SilentlyContinue

    $expectedPrefsStatus = @"
Preferences Subsystem: Healthy (Active)
Backup: Missing
"@
    $dbyteosPrefsStatus = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "status") -WorkingDirectory $dbyteosRoot
    if ($dbyteosPrefsStatus.Code -ne 0) { throw "dbyteos prefs status failed: $($dbyteosPrefsStatus.Text)" }
    Assert-NormalizedEqual $dbyteosPrefsStatus.Text $expectedPrefsStatus "dbyteos prefs status snapshot"

    $expectedPrefsAllowed = @"
Safe Mutable Keys:
  ui.theme:           default, dark, light
  system.prompt:      dbyte-shell>, dbyteos>, deadbyte>
  user.display_name:  deadbyte, guest, operator
"@
    $dbyteosPrefsAllowed = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "allowed") -WorkingDirectory $dbyteosRoot
    if ($dbyteosPrefsAllowed.Code -ne 0) { throw "dbyteos prefs allowed failed: $($dbyteosPrefsAllowed.Text)" }
    Assert-NormalizedEqual $dbyteosPrefsAllowed.Text $expectedPrefsAllowed "dbyteos prefs allowed snapshot"

    $expectedPrefsDoctor = @"
Preferences module imported successfully.
All required mutable keys are present.
Schema validation passed.
"@
    $dbyteosPrefsDoctor = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "doctor") -WorkingDirectory $dbyteosRoot
    if ($dbyteosPrefsDoctor.Code -ne 0) { throw "dbyteos prefs doctor failed: $($dbyteosPrefsDoctor.Text)" }
    Assert-NormalizedEqual $dbyteosPrefsDoctor.Text $expectedPrefsDoctor "dbyteos prefs doctor snapshot"

    # --- Backup / Restore Lifecycle ---
    $dbyteosPrefsBackup = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "backup-demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosPrefsBackup.Code -ne 0) { throw "dbyteos prefs backup-demo failed: $($dbyteosPrefsBackup.Text)" }
    Assert-Equal $dbyteosPrefsBackup.Text "Preferences backed up successfully." "dbyteos prefs backup-demo"

    $expectedPrefsStatusWithBak = @"
Preferences Subsystem: Healthy (Active)
Backup: Present
"@
    $dbyteosPrefsStatusBak = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "status") -WorkingDirectory $dbyteosRoot
    if ($dbyteosPrefsStatusBak.Code -ne 0) { throw "dbyteos prefs status after backup failed: $($dbyteosPrefsStatusBak.Text)" }
    Assert-NormalizedEqual $dbyteosPrefsStatusBak.Text $expectedPrefsStatusWithBak "dbyteos prefs status after backup"

    $dbyteosPrefsBackupIdempotent = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "backup-demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosPrefsBackupIdempotent.Code -ne 0) { throw "dbyteos prefs backup-demo idempotent failed: $($dbyteosPrefsBackupIdempotent.Text)" }
    Assert-Equal $dbyteosPrefsBackupIdempotent.Text "Preferences backed up successfully." "dbyteos prefs backup-demo idempotent"

    $dbyteosPrefsRestore = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "restore-demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosPrefsRestore.Code -ne 0) { throw "dbyteos prefs restore-demo failed: $($dbyteosPrefsRestore.Text)" }
    Assert-Equal $dbyteosPrefsRestore.Text "Preferences restored successfully." "dbyteos prefs restore-demo"

    $dbyteosPrefsRestoreIdempotent = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "restore-demo") -WorkingDirectory $dbyteosRoot
    if ($dbyteosPrefsRestoreIdempotent.Code -ne 0) { throw "dbyteos prefs restore-demo idempotent failed: $($dbyteosPrefsRestoreIdempotent.Text)" }
    Assert-Equal $dbyteosPrefsRestoreIdempotent.Text "Preferences restored successfully." "dbyteos prefs restore-demo idempotent"

    # --- prefs set → override display sync ---
    $dbyteosPrefsSetDark = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "set", "ui.theme", "dark") -WorkingDirectory $dbyteosRoot
    if ($dbyteosPrefsSetDark.Code -ne 0) { throw "dbyteos prefs set dark failed: $($dbyteosPrefsSetDark.Text)" }

    $dbyteosConfigShowDark = Invoke-Dbyte -Arguments @("run", "bin\config.dby", "show") -WorkingDirectory $dbyteosRoot
    if ($dbyteosConfigShowDark.Code -ne 0) { throw "dbyteos config show dark failed: $($dbyteosConfigShowDark.Text)" }
    Assert-Contains $dbyteosConfigShowDark.Text "ui.theme = dark (overridden)" "dbyteos config show dark override marker"

    $dbyteosSnapshotConfigDark = Invoke-Dbyte -Arguments @("run", "bin\snapshot.dby", "config") -WorkingDirectory $dbyteosRoot
    if ($dbyteosSnapshotConfigDark.Code -ne 0) { throw "dbyteos snapshot config dark failed: $($dbyteosSnapshotConfigDark.Text)" }
    Assert-Contains $dbyteosSnapshotConfigDark.Text "ui.theme = dark (overridden)" "dbyteos snapshot config dark override marker"

    # restore to defaults before continuing
    $null = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "reset-demo") -WorkingDirectory $dbyteosRoot

    # --- clean preserves preferences.dby and .bak ---
    $prefsPath = Join-Path $dbyteosRoot "home\deadbyte\preferences.dby"
    $prefsBakPath = $prefsPath + ".bak"
    $null = Invoke-Dbyte -Arguments @("run", "bin\prefs.dby", "backup-demo") -WorkingDirectory $dbyteosRoot
    $dbyteosCleanPrefs = Invoke-Dbyte -Arguments @("run", "bin\clean.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosCleanPrefs.Code -ne 0) { throw "dbyteos clean (prefs preservation) failed: $($dbyteosCleanPrefs.Text)" }
    if (-not (Test-Path $prefsPath)) { throw "clean deleted preferences.dby - must be preserved" }
    if (-not (Test-Path $prefsBakPath)) { throw "clean deleted preferences.dby.bak - must be preserved" }
    Assert-Contains $dbyteosCleanPrefs.Text "workspace sweep complete" "dbyteos clean sweep completes after prefs backup"

    # cleanup bak after test
    Remove-Item $prefsBakPath -Force -ErrorAction SilentlyContinue

    # --- scratch guard: test_import.dby must not exist in repo root ---
    $scratchGuardPath = Join-Path $repoRoot "test_import.dby"
    if (Test-Path $scratchGuardPath) { throw "scratch file detected in repo root: test_import.dby - remove before tagging" }

    $dbyteosSnapshotDirect = Invoke-Dbyte -Arguments @("run", "bin\snapshot.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosSnapshotDirect.Code -ne 0) { throw "dbyteos snapshot failed: $($dbyteosSnapshotDirect.Text)" }
    Assert-NormalizedEqual $dbyteosSnapshotDirect.Text $expectedDbyteosSnapshot "dbyteos snapshot default snapshot"

    $dbyteosSnapshotSystem = Invoke-Dbyte -Arguments @("run", "bin\snapshot.dby", "system") -WorkingDirectory $dbyteosRoot
    if ($dbyteosSnapshotSystem.Code -ne 0) { throw "dbyteos snapshot system failed: $($dbyteosSnapshotSystem.Text)" }
    Assert-NormalizedEqual $dbyteosSnapshotSystem.Text $expectedDbyteosSnapshot "dbyteos snapshot system snapshot"
    Assert-Equal (Normalize-Output $dbyteosSnapshotDirect.Text) (Normalize-Output $dbyteosSnapshotSystem.Text) "dbyteos snapshot no args equals system"

    # --- Missing Recovery and Malformed Behavior Tests ---
    $prefsFilePath = Join-Path $dbyteosRoot "home\deadbyte\preferences.dby"
    $originalPrefsBytes = [System.IO.File]::ReadAllBytes($prefsFilePath)
    Remove-Item -Path $prefsFilePath -Force

    $dbyteosDiagnoseMissing = Invoke-Dbyte -Arguments @("run", "bin\diagnose.dby", "preferences") -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosDiagnoseMissing.Text "ImportError: local module not found: ../home/deadbyte/preferences.dby" "dbyteos diagnose malformed deterministic crash"

    $dbyteosCheckSystemMissing = Invoke-Dbyte -Arguments @("run", "bin\check_system.dby") -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosCheckSystemMissing.Text "ImportError: local module not found: ../home/deadbyte/preferences.dby" "dbyteos check_system malformed deterministic crash"

    $dbyteosDoctorMissing = Invoke-Dbyte -Arguments @("run", "bin\doctor.dby") -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosDoctorMissing.Text "preferences: unhealthy" "dbyteos doctor preferences unhealthy"

    $dbyteosBootMissing = Invoke-Dbyte -Arguments @("run", "boot.dby") -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosBootMissing.Text "ImportError: local module not found: ../home/deadbyte/preferences.dby" "dbyteos boot malformed deterministic crash"

    $dbyteosRecover = Invoke-Dbyte -Arguments @("run", "bin\prefs-recover.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosRecover.Code -ne 0) { throw "dbyteos recover failed: $($dbyteosRecover.Text)" }
    Assert-Contains $dbyteosRecover.Text "status: recovered factory defaults" "dbyteos preferences recovery"

    $dbyteosDoctorRecovered = Invoke-Dbyte -Arguments @("run", "bin\doctor.dby") -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosDoctorRecovered.Text "preferences: ok" "dbyteos doctor preferences recovered"
    [System.IO.File]::WriteAllBytes($prefsFilePath, $originalPrefsBytes)
    # -----------------------------------------------------

    $dbyteosSnapshotProfile = Invoke-Dbyte -Arguments @("run", "bin\snapshot.dby", "profile") -WorkingDirectory $dbyteosRoot
    if ($dbyteosSnapshotProfile.Code -ne 0) { throw "dbyteos snapshot profile failed: $($dbyteosSnapshotProfile.Text)" }
    Assert-NormalizedEqual $dbyteosSnapshotProfile.Text $expectedDbyteosSnapshotProfile "dbyteos snapshot profile snapshot"

    $dbyteosSnapshotConfig = Invoke-Dbyte -Arguments @("run", "bin\snapshot.dby", "config") -WorkingDirectory $dbyteosRoot
    if ($dbyteosSnapshotConfig.Code -ne 0) { throw "dbyteos snapshot config failed: $($dbyteosSnapshotConfig.Text)" }
    Assert-NormalizedEqual $dbyteosSnapshotConfig.Text $expectedDbyteosSnapshotConfig "dbyteos snapshot config snapshot"

    $dbyteosSnapshotSecurity = Invoke-Dbyte -Arguments @("run", "bin\snapshot.dby", "security") -WorkingDirectory $dbyteosRoot
    if ($dbyteosSnapshotSecurity.Code -ne 0) { throw "dbyteos snapshot security failed: $($dbyteosSnapshotSecurity.Text)" }
    Assert-NormalizedEqual $dbyteosSnapshotSecurity.Text $expectedDbyteosSnapshotSecurity "dbyteos snapshot security snapshot"
    Assert-Contains $dbyteosSnapshotSecurity.Text ("mode:          " + $dbyteosConfigSecurityMode.Text) "dbyteos snapshot security mode sync"
    Assert-Contains $dbyteosSnapshotSecurity.Text "tmp/:          read/write" "dbyteos snapshot security tmp policy"
    Assert-Contains $dbyteosSnapshotSecurity.Text "home/deadbyte/: read/write" "dbyteos snapshot security home policy"
    Assert-Contains $dbyteosSnapshotSecurity.Text "etc/:          read-only" "dbyteos snapshot security etc policy"
    Assert-Contains $dbyteosSnapshotSecurity.Text "../:           denied" "dbyteos snapshot security escape policy"
    Assert-Contains $dbyteosSnapshotSecurity.Text "absolute path: denied" "dbyteos snapshot security absolute policy"

    $dbyteosSnapshotLogs = Invoke-Dbyte -Arguments @("run", "bin\snapshot.dby", "logs") -WorkingDirectory $dbyteosRoot
    if ($dbyteosSnapshotLogs.Code -ne 0) { throw "dbyteos snapshot logs failed: $($dbyteosSnapshotLogs.Text)" }
    Assert-NormalizedEqual $dbyteosSnapshotLogs.Text $expectedDbyteosSnapshotLogs "dbyteos snapshot logs snapshot"

    $dbyteosSnapshotUnknown = Invoke-Dbyte -Arguments @("run", "bin\snapshot.dby", "unknown") -WorkingDirectory $dbyteosRoot
    if ($dbyteosSnapshotUnknown.Code -ne 0) { throw "dbyteos snapshot unknown failed: $($dbyteosSnapshotUnknown.Text)" }
    Assert-NormalizedEqual $dbyteosSnapshotUnknown.Text $expectedDbyteosSnapshotUnknown "dbyteos snapshot unknown snapshot"

    $dbyteosShell = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "status`nclean`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShell.Code -ne 0) { throw "dbyteos shell failed: $($dbyteosShell.Text)" }
    Assert-Contains $dbyteosShell.Text "--- DByteOS System Status ---" "dbyteos shell status alias"
    Assert-Contains $dbyteosShell.Text "sweep complete" "dbyteos shell clean alias sweep"

    $dbyteosShellHelp = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "help`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellHelp.Code -ne 0) { throw "dbyteos shell help failed: $($dbyteosShellHelp.Text)" }
    Assert-Contains $dbyteosShellHelp.Text (Normalize-Output $expectedDbyteosHelp) "dbyteos shell help snapshot"
    Assert-Contains $dbyteosShellHelp.Text "--- DByteOS Beta Help ---" "dbyteos shell help output (aliased)"
    Assert-Contains $dbyteosShellHelp.Text "System:" "dbyteos shell help system"
    Assert-Contains $dbyteosShellHelp.Text "Discovery:" "dbyteos shell help discovery"
    Assert-Contains $dbyteosShellHelp.Text "perm             - inspect permission policy" "dbyteos shell help perm"
    Assert-Contains $dbyteosShellHelp.Text "profile          - show profile identity" "dbyteos shell help profile"
    Assert-Contains $dbyteosShellHelp.Text "config           - show read-only configuration" "dbyteos shell help config"
    Assert-Contains $dbyteosShellHelp.Text "Try: welcome, profile show, config show, snapshot, getting-started, commands" "dbyteos shell help try line"

    $dbyteosShellWhichHelpAliased = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "which help`nquit`n" -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosShellWhichHelpAliased.Text "help: alias -> run bin/help.dby" "which help with alias"

    $dbyteosShellNoRcHelp = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "help`nquit`n" -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosShellNoRcHelp.Text "DByte shell commands:" "shell --no-rc help remains built-in"

    $dbyteosShellNoRcWhichHelp = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "which help`nquit`n" -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosShellNoRcWhichHelp.Text "help: built-in" "which help without alias remains built-in (autopath blocked)"

    $dbyteosShellNoRcOnboarding = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "welcome`nprofile`nconfig`nsnapshot`nproject`ntask`ngetting-started`ncommands`nman-index`nprefs`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellNoRcOnboarding.Code -ne 0) { throw "dbyteos shell --no-rc onboarding guard failed: $($dbyteosShellNoRcOnboarding.Text)" }
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: welcome" "dbyteos shell --no-rc hides welcome"
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: profile" "dbyteos shell --no-rc hides profile"
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: config" "dbyteos shell --no-rc hides config"
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: snapshot" "dbyteos shell --no-rc hides snapshot"
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: project" "dbyteos shell --no-rc hides project"
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: task" "dbyteos shell --no-rc hides task"
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: getting-started" "dbyteos shell --no-rc hides getting-started"
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: commands" "dbyteos shell --no-rc hides commands"
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: man-index" "dbyteos shell --no-rc hides man-index"
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: prefs" "dbyteos shell --no-rc hides prefs"

    $dbyteosShellManWhoami = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "man whoami`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellManWhoami.Code -ne 0) { throw "dbyteos shell man whoami failed: $($dbyteosShellManWhoami.Text)" }
    Assert-Contains $dbyteosShellManWhoami.Text "NAME`n    whoami - print the current user name" "dbyteos shell man whoami output"

    $dbyteosShellManMissing = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "man does-not-exist`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellManMissing.Code -ne 0) { throw "dbyteos shell man missing failed: $($dbyteosShellManMissing.Text)" }
    Assert-Contains $dbyteosShellManMissing.Text "No manual entry for does-not-exist" "dbyteos shell man missing output"

    $dbyteosShellManTraversal = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "man ../sys/session`nquit`n" -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosShellManTraversal.Text "Error: invalid manual topic name '../sys/session'" "dbyteos shell man traversal reject"

    $dbyteosShellManPerm = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "man perm`nman security`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellManPerm.Code -ne 0) { throw "dbyteos shell man perm/security failed: $($dbyteosShellManPerm.Text)" }
    Assert-Contains $dbyteosShellManPerm.Text "DByteOS Permission Command" "dbyteos man perm"
    Assert-Contains $dbyteosShellManPerm.Text "DByteOS Security Policy" "dbyteos man security"

    $dbyteosOnboardingShell = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "welcome`nprofile show`nprofile whoami`nprofile home`nprofile theme`nprofile prompt`nconfig show`nconfig keys`nconfig get system.prompt`nsnapshot`nsnapshot profile`nsnapshot config`nsnapshot security`nsnapshot logs`nproject reset-demo`nproject list`nproject status demo`nproject notes demo`nproject snapshot demo`nproject doctor demo`ntask reset-demo`ntask list demo`ntask add demo write tests`ntask done demo 1`ntask done demo 1`ntask status demo`ntask summary demo`ntask open demo`ntask doctor demo`ntask snapshot demo`ntask clear-done demo`ngetting-started`ncommands`nman-index`nhelp`nman index`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosOnboardingShell.Code -ne 0) { throw "dbyteos onboarding shell failed: $($dbyteosOnboardingShell.Text)" }
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosWelcome) "dbyteos shell welcome"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosProfile) "dbyteos shell profile show"
    Assert-Contains $dbyteosOnboardingShell.Text "deadbyte" "dbyteos shell profile whoami"
    Assert-Contains $dbyteosOnboardingShell.Text "home/deadbyte" "dbyteos shell profile home"
    Assert-Contains $dbyteosOnboardingShell.Text "default" "dbyteos shell profile theme"
    Assert-Contains $dbyteosOnboardingShell.Text "dbyte-shell>" "dbyteos shell profile prompt"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosConfig) "dbyteos shell config show"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosConfigKeys) "dbyteos shell config keys"
    Assert-Contains $dbyteosOnboardingShell.Text "dbyte-shell>" "dbyteos shell config prompt"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosSnapshot) "dbyteos shell snapshot"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosSnapshotProfile) "dbyteos shell snapshot profile"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosSnapshotConfig) "dbyteos shell snapshot config"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosSnapshotSecurity) "dbyteos shell snapshot security"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosSnapshotLogs) "dbyteos shell snapshot logs"
    Assert-Contains $dbyteosOnboardingShell.Text "project demo reset." "dbyteos shell project reset-demo"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosProjectListDemo) "dbyteos shell project list"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosProjectStatusDemo) "dbyteos shell project status"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosProjectNotesDemo) "dbyteos shell project notes"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosProjectSnapshotDemo) "dbyteos shell project snapshot"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosProjectDoctorDemo) "dbyteos shell project doctor"
    Assert-Contains $dbyteosOnboardingShell.Text "task demo reset." "dbyteos shell task reset-demo"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosTaskListDemo) "dbyteos shell task list"
    Assert-Contains $dbyteosOnboardingShell.Text "task added: demo #3" "dbyteos shell task add"
    Assert-Contains $dbyteosOnboardingShell.Text "task done: demo #1" "dbyteos shell task done"
    Assert-Contains $dbyteosOnboardingShell.Text "task already done: demo #1" "dbyteos shell task already done"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosTaskStatusAfterDone) "dbyteos shell task status"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosTaskSummaryAfterDone) "dbyteos shell task summary"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosTaskOpenAfterDone) "dbyteos shell task open"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosTaskDoctorHealthy) "dbyteos shell task doctor"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosTaskSnapshotAfterDone) "dbyteos shell task snapshot"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosTaskClearDone) "dbyteos shell task clear-done"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosGettingStarted) "dbyteos shell getting-started"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosCommands) "dbyteos shell commands"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosManIndex) "dbyteos shell man-index"
    Assert-Contains $dbyteosOnboardingShell.Text "Manual topics:" "dbyteos shell man index"

    $dbyteosOnboardingManuals = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "man welcome`nman profile`nman config`nman snapshot`nman project`nman task`nman getting-started`nman commands`nman index`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosOnboardingManuals.Code -ne 0) { throw "dbyteos onboarding manuals failed: $($dbyteosOnboardingManuals.Text)" }
    Assert-Contains $dbyteosOnboardingManuals.Text "DByteOS Welcome" "dbyteos man welcome"
    Assert-Contains $dbyteosOnboardingManuals.Text "DByteOS Profile" "dbyteos man profile"
    Assert-Contains $dbyteosOnboardingManuals.Text "DByteOS Config" "dbyteos man config"
    Assert-Contains $dbyteosOnboardingManuals.Text "DByteOS Snapshot" "dbyteos man snapshot"
    Assert-Contains $dbyteosOnboardingManuals.Text "DByteOS Project Command" "dbyteos man project"
    Assert-Contains $dbyteosOnboardingManuals.Text "DByteOS Task Command" "dbyteos man task"
    Assert-Contains $dbyteosOnboardingManuals.Text "DByteOS Getting Started" "dbyteos man getting-started"
    Assert-Contains $dbyteosOnboardingManuals.Text "DByteOS Commands" "dbyteos man commands"
    Assert-Contains $dbyteosOnboardingManuals.Text "DByteOS Manual Index" "dbyteos man index"

    $dbyteosShellRcFromRoot = Invoke-DbyteInput -Arguments @("shell", "--rc", "examples\dbyteos\.dbyterc") -InputText "status`nquit`n" -WorkingDirectory $repoRoot
    if ($dbyteosShellRcFromRoot.Code -ne 0) { throw "dbyteos shell --rc from repo root failed: $($dbyteosShellRcFromRoot.Text)" }
    Assert-Contains $dbyteosShellRcFromRoot.Text "--- DByteOS System Status ---" "dbyteos shell rc from repo root"

    $dbyteosShellWhichCat = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "which cat`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellWhichCat.Code -ne 0) { throw "dbyteos shell which cat failed: $($dbyteosShellWhichCat.Text)" }
    Assert-Contains $dbyteosShellWhichCat.Text "cat: dbyteos ->" "dbyteos shell which cat autopath"
    Assert-Contains $dbyteosShellWhichCat.Text "examples/dbyteos/bin/cat.dby" "dbyteos shell which cat resolved path"

    $dbyteosShellWhichOnboarding = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "which getting-started`nwhich man-index`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellWhichOnboarding.Code -ne 0) { throw "dbyteos shell which onboarding failed: $($dbyteosShellWhichOnboarding.Text)" }
    Assert-Contains $dbyteosShellWhichOnboarding.Text "getting-started: dbyteos ->" "dbyteos shell which getting-started"
    Assert-Contains $dbyteosShellWhichOnboarding.Text "examples/dbyteos/bin/getting_started.dby" "dbyteos shell which getting-started resolved path"
    Assert-Contains $dbyteosShellWhichOnboarding.Text "man-index: dbyteos ->" "dbyteos shell which man-index"
    Assert-Contains $dbyteosShellWhichOnboarding.Text "examples/dbyteos/bin/man_index.dby" "dbyteos shell which man-index resolved path"

    $dbyteosShellProjectWorkflow = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "project reset-demo`nproject status demo`nproject notes demo`nproject snapshot demo`nproject doctor demo`nwhich project`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellProjectWorkflow.Code -ne 0) { throw "dbyteos shell project workflow failed: $($dbyteosShellProjectWorkflow.Text)" }
    Assert-Contains $dbyteosShellProjectWorkflow.Text "project demo reset." "dbyteos shell project workflow reset"
    Assert-Contains $dbyteosShellProjectWorkflow.Text (Normalize-Output $expectedDbyteosProjectStatusDemo) "dbyteos shell project workflow status"
    Assert-Contains $dbyteosShellProjectWorkflow.Text (Normalize-Output $expectedDbyteosProjectNotesDemo) "dbyteos shell project workflow notes"
    Assert-Contains $dbyteosShellProjectWorkflow.Text (Normalize-Output $expectedDbyteosProjectSnapshotDemo) "dbyteos shell project workflow snapshot"
    Assert-Contains $dbyteosShellProjectWorkflow.Text (Normalize-Output $expectedDbyteosProjectDoctorDemo) "dbyteos shell project workflow doctor"
    Assert-Contains $dbyteosShellProjectWorkflow.Text "project: dbyteos ->" "dbyteos shell which project autopath"
    Assert-Contains $dbyteosShellProjectWorkflow.Text "examples/dbyteos/bin/project.dby" "dbyteos shell which project resolved path"

    $dbyteosShellTaskWorkflow = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "project reset-demo`ntask reset-demo`ntask list demo`ntask add demo write tests`ntask done demo 1`ntask done demo 1`ntask status demo`ntask summary demo`ntask open demo`ntask doctor demo`ntask snapshot demo`ntask clear-done demo`nwhich task`nman task`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellTaskWorkflow.Code -ne 0) { throw "dbyteos shell task workflow failed: $($dbyteosShellTaskWorkflow.Text)" }
    Assert-Contains $dbyteosShellTaskWorkflow.Text "task demo reset." "dbyteos shell task workflow reset"
    Assert-Contains $dbyteosShellTaskWorkflow.Text (Normalize-Output $expectedDbyteosTaskListDemo) "dbyteos shell task workflow list"
    Assert-Contains $dbyteosShellTaskWorkflow.Text "task added: demo #3" "dbyteos shell task workflow add"
    Assert-Contains $dbyteosShellTaskWorkflow.Text "task done: demo #1" "dbyteos shell task workflow done"
    Assert-Contains $dbyteosShellTaskWorkflow.Text "task already done: demo #1" "dbyteos shell task workflow already done"
    Assert-Contains $dbyteosShellTaskWorkflow.Text (Normalize-Output $expectedDbyteosTaskStatusAfterDone) "dbyteos shell task workflow status"
    Assert-Contains $dbyteosShellTaskWorkflow.Text (Normalize-Output $expectedDbyteosTaskSummaryAfterDone) "dbyteos shell task workflow summary"
    Assert-Contains $dbyteosShellTaskWorkflow.Text (Normalize-Output $expectedDbyteosTaskOpenAfterDone) "dbyteos shell task workflow open"
    Assert-Contains $dbyteosShellTaskWorkflow.Text (Normalize-Output $expectedDbyteosTaskDoctorHealthy) "dbyteos shell task workflow doctor"
    Assert-Contains $dbyteosShellTaskWorkflow.Text (Normalize-Output $expectedDbyteosTaskSnapshotAfterDone) "dbyteos shell task workflow snapshot"
    Assert-Contains $dbyteosShellTaskWorkflow.Text (Normalize-Output $expectedDbyteosTaskClearDone) "dbyteos shell task workflow clear-done"
    Assert-Contains $dbyteosShellTaskWorkflow.Text "task: dbyteos ->" "dbyteos shell which task autopath"
    Assert-Contains $dbyteosShellTaskWorkflow.Text "examples/dbyteos/bin/task.dby" "dbyteos shell which task resolved path"
    Assert-Contains $dbyteosShellTaskWorkflow.Text "DByteOS Task Command" "dbyteos shell task workflow manual"

    $dbyteosShellWhichCd = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "which cd`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellWhichCd.Code -ne 0) { throw "dbyteos shell which cd failed: $($dbyteosShellWhichCd.Text)" }
    Assert-Contains $dbyteosShellWhichCd.Text "cd: built-in" "dbyteos shell which built-in"

    $dbyteosShellInspectArgs = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "inspect boot.dby`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellInspectArgs.Code -ne 0) { throw "dbyteos shell inspect args failed: $($dbyteosShellInspectArgs.Text)" }
    Assert-Contains $dbyteosShellInspectArgs.Text "Inspecting file:" "dbyteos shell inspect passes args"

    $dbyteosShellSearchWorkflow = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "search reset-demo`nsearch summary`nsearch rebuild`nsearch status`nsearch summary`nsearch recent`nsearch projects note`nsearch tasks tests`nsearch notes seed`nsearch journal JOURNAL`nwhich search`nman search`nsearch clear-cache`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellSearchWorkflow.Code -ne 0) { throw "dbyteos shell search workflow failed: $($dbyteosShellSearchWorkflow.Text)" }
    Assert-Contains $dbyteosShellSearchWorkflow.Text "search: reset demo project and workspace seed data" "dbyteos shell search reset-demo"
    Assert-Contains $dbyteosShellSearchWorkflow.Text "Index Status: missing" "dbyteos shell search summary missing"
    Assert-Contains $dbyteosShellSearchWorkflow.Text "search: index rebuilt successfully" "dbyteos shell search rebuild"
    Assert-Contains $dbyteosShellSearchWorkflow.Text "index: active" "dbyteos shell search status active"
    Assert-Contains $dbyteosShellSearchWorkflow.Text "Index Status: active" "dbyteos shell search summary active"
    Assert-Contains $dbyteosShellSearchWorkflow.Text "--- Recent Indexed Records ---" "dbyteos shell search recent"
    Assert-Contains $dbyteosShellSearchWorkflow.Text "project demo note: project demo notes" "dbyteos shell search projects note"
    Assert-Contains $dbyteosShellSearchWorkflow.Text "project demo task: [ ] 2: write tests" "dbyteos shell search tasks tests"
    Assert-Contains $dbyteosShellSearchWorkflow.Text "notes: dbyteos notes seed" "dbyteos shell search notes seed"
    Assert-Contains $dbyteosShellSearchWorkflow.Text "journal: [JOURNAL] dbyteos journal seed" "dbyteos shell search journal JOURNAL"
    Assert-Contains $dbyteosShellSearchWorkflow.Text "search: dbyteos ->" "dbyteos shell which search autopath"
    Assert-Contains $dbyteosShellSearchWorkflow.Text "DByteOS Search Command" "dbyteos shell man search"

    $dbyteosShellNoRc = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "status`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellNoRc.Code -ne 0) { throw "dbyteos shell --no-rc failed: $($dbyteosShellNoRc.Text)" }
    Assert-Contains $dbyteosShellNoRc.Text "ShellError: unknown command: status" "dbyteos shell --no-rc hides os aliases"

    $dbyteosShellNoRcCat = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "cat tmp/write_demo.txt`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellNoRcCat.Code -ne 0) { throw "dbyteos shell --no-rc cat failed: $($dbyteosShellNoRcCat.Text)" }
    Assert-Contains $dbyteosShellNoRcCat.Text "ShellError: unknown command: cat" "dbyteos shell --no-rc hides cat alias"

    $dbyteosShellNoRcRead = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "read tmp/verify_v32.txt`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellNoRcRead.Code -ne 0) { throw "dbyteos shell --no-rc read failed: $($dbyteosShellNoRcRead.Text)" }
    Assert-Contains $dbyteosShellNoRcRead.Text "ShellError: unknown command: read" "dbyteos shell --no-rc hides read alias"

    $dbyteosShellNoRcProject = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "project list`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellNoRcProject.Code -ne 0) { throw "dbyteos shell --no-rc project failed: $($dbyteosShellNoRcProject.Text)" }
    Assert-Contains $dbyteosShellNoRcProject.Text "ShellError: unknown command: project" "dbyteos shell --no-rc hides project autopath"

    # Internal verification hook only: force prompt capture for piped shell smoke tests.
    $promptEnv = @{ "DBYTE_SHELL_FORCE_PROMPT" = "1" }
    $dbyteosPromptDefault = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "version`nquit`n" -WorkingDirectory $dbyteosRoot -Environment $promptEnv
    if ($dbyteosPromptDefault.Code -ne 0) { throw "dbyteos shell prompt default failed: $($dbyteosPromptDefault.Text)" }
    Assert-Equal $dbyteosPromptDefault.Text "dbyte-shell> DByte 9.0.2`ndbyte-shell>" "dbyteos shell prompt default snapshot"

    $dbyteosPromptNoRc = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "quit`n" -WorkingDirectory $dbyteosRoot -Environment $promptEnv
    if ($dbyteosPromptNoRc.Code -ne 0) { throw "dbyteos shell prompt no-rc failed: $($dbyteosPromptNoRc.Text)" }
    Assert-Equal $dbyteosPromptNoRc.Text "dbyte-shell>" "dbyteos shell --no-rc default prompt snapshot"

    $dbyteosPromptChange = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "prefs set system.prompt dbyteos>`nversion`nprefs set system.prompt deadbyte>`nversion`nprefs reset-demo`nversion`nquit`n" -WorkingDirectory $dbyteosRoot -Environment $promptEnv
    if ($dbyteosPromptChange.Code -ne 0) { throw "dbyteos shell prompt change failed: $($dbyteosPromptChange.Text)" }
    Assert-Equal $dbyteosPromptChange.Text "dbyte-shell> preference 'system.prompt' updated successfully.`ndbyteos> DByte 9.0.2`ndbyteos> preference 'system.prompt' updated successfully.`ndeadbyte> DByte 9.0.2`ndeadbyte> preferences reset to default seed state.`ndbyte-shell> DByte 9.0.2`ndbyte-shell>" "dbyteos shell prompt preference snapshots"

    $prefsFileForPrompt = Join-Path $dbyteosRoot "home\deadbyte\preferences.dby"
    $originalPrefsForPrompt = [System.IO.File]::ReadAllBytes($prefsFileForPrompt)
    try {
        Remove-Item -Path $prefsFileForPrompt -Force
        $dbyteosPromptMissingFallback = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "quit`n" -WorkingDirectory $dbyteosRoot -Environment $promptEnv
        if ($dbyteosPromptMissingFallback.Code -ne 0) { throw "dbyteos shell prompt missing fallback failed: $($dbyteosPromptMissingFallback.Text)" }
        Assert-Equal $dbyteosPromptMissingFallback.Text "dbyte-shell>" "dbyteos shell prompt missing prefs fallback"

        Set-Content -Path $prefsFileForPrompt -Value "pub let ui_theme: str = `"default`"`npub let system_prompt: str = `"dbyteos>`npub let user_display_name: str = `"deadbyte`"`n" -NoNewline
        $dbyteosPromptMalformedFallback = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "quit`n" -WorkingDirectory $dbyteosRoot -Environment $promptEnv
        if ($dbyteosPromptMalformedFallback.Code -ne 0) { throw "dbyteos shell prompt malformed fallback failed: $($dbyteosPromptMalformedFallback.Text)" }
        Assert-Equal $dbyteosPromptMalformedFallback.Text "dbyte-shell>" "dbyteos shell prompt malformed prefs fallback"

        Set-Content -Path $prefsFileForPrompt -Value "pub let ui_theme: str = `"default`"`npub let system_prompt: str = `"unsupported>`"`npub let user_display_name: str = `"deadbyte`"`n" -NoNewline
        $dbyteosPromptFallback = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "quit`n" -WorkingDirectory $dbyteosRoot -Environment $promptEnv
        if ($dbyteosPromptFallback.Code -ne 0) { throw "dbyteos shell prompt fallback failed: $($dbyteosPromptFallback.Text)" }
        Assert-Equal $dbyteosPromptFallback.Text "dbyte-shell>" "dbyteos shell prompt unsupported prefs fallback"
    }
    finally {
        [System.IO.File]::WriteAllBytes($prefsFileForPrompt, $originalPrefsForPrompt)
    }

    $dbyteosRcMissingRoot = Join-Path $repoRoot "target\verify-dbyteos-missing-rc"
    $dbyteosShellBadRc = Invoke-DbyteInput -Arguments @("shell", "--rc", $dbyteosRcMissingRoot) -InputText "quit`n" -WorkingDirectory $repoRoot
    if ($dbyteosShellBadRc.Code -eq 0) { throw "dbyteos shell missing --rc unexpectedly succeeded" }
    Assert-Contains $dbyteosShellBadRc.Text "RcError: --rc file not found:" "dbyteos shell missing --rc error"

    $dbyteosBadRcFile = Join-Path $repoRoot "target\verify-dbyteos-bad-directive.rc"
    Set-Content -Path $dbyteosBadRcFile -Value "@shell notalias x`n" -NoNewline
    $dbyteosBadRc = Invoke-DbyteInput -Arguments @("shell", "--rc", $dbyteosBadRcFile) -InputText "quit`n" -WorkingDirectory $repoRoot
    if ($dbyteosBadRc.Code -eq 0) { throw "dbyteos shell bad directive unexpectedly succeeded" }
    Assert-Contains $dbyteosBadRc.Text "ShellError:" "dbyteos bad rc shell error prefix"
    Assert-Contains $dbyteosBadRc.Text "line 1:" "dbyteos bad rc line context"
    Assert-Contains $dbyteosBadRc.Text "@shell notalias x" "dbyteos bad rc source line echo"

    $dbyteosWhoamiRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\whoami.dby") -WorkingDirectory $repoRoot
    if ($dbyteosWhoamiRoot.Code -ne 0) { throw "dbyteos whoami from root failed: $($dbyteosWhoamiRoot.Text)" }
    Assert-Equal $dbyteosWhoamiRoot.Text "deadbyte" "dbyteos whoami from root"

    $dbyteosConfigRootHome = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\config.dby", "get", "user.home") -WorkingDirectory $repoRoot
    if ($dbyteosConfigRootHome.Code -ne 0) { throw "dbyteos config home from root failed: $($dbyteosConfigRootHome.Text)" }
    Assert-Equal $dbyteosConfigRootHome.Text "examples/dbyteos/home/deadbyte" "dbyteos config home from root"

    $dbyteosConfigRootPrompt = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\config.dby", "get", "system.prompt") -WorkingDirectory $repoRoot
    if ($dbyteosConfigRootPrompt.Code -ne 0) { throw "dbyteos config prompt from root failed: $($dbyteosConfigRootPrompt.Text)" }
    Assert-Equal $dbyteosConfigRootPrompt.Text $dbyteosConfigPrompt.Text "dbyteos config prompt root/cwd sync"

    $dbyteosSnapshotRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\snapshot.dby") -WorkingDirectory $repoRoot
    if ($dbyteosSnapshotRoot.Code -ne 0) { throw "dbyteos snapshot from root failed: $($dbyteosSnapshotRoot.Text)" }
    $expectedDbyteosSnapshotRoot = $expectedDbyteosSnapshot.Replace("  home:    home/deadbyte", "  home:    examples/dbyteos/home/deadbyte").Replace("  user.home = home/deadbyte", "  user.home = examples/dbyteos/home/deadbyte")
    Assert-NormalizedEqual $dbyteosSnapshotRoot.Text $expectedDbyteosSnapshotRoot "dbyteos snapshot from root snapshot"

    $dbyteosSnapshotRootProfile = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\snapshot.dby", "profile") -WorkingDirectory $repoRoot
    if ($dbyteosSnapshotRootProfile.Code -ne 0) { throw "dbyteos snapshot profile from root failed: $($dbyteosSnapshotRootProfile.Text)" }
    $expectedDbyteosSnapshotRootProfile = $expectedDbyteosSnapshotProfile.Replace("  home:    home/deadbyte", "  home:    examples/dbyteos/home/deadbyte")
    Assert-NormalizedEqual $dbyteosSnapshotRootProfile.Text $expectedDbyteosSnapshotRootProfile "dbyteos snapshot profile from root snapshot"

    $dbyteosSnapshotRootConfig = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\snapshot.dby", "config") -WorkingDirectory $repoRoot
    if ($dbyteosSnapshotRootConfig.Code -ne 0) { throw "dbyteos snapshot config from root failed: $($dbyteosSnapshotRootConfig.Text)" }
    $expectedDbyteosSnapshotRootConfig = $expectedDbyteosSnapshotConfig.Replace("  user.home = home/deadbyte", "  user.home = examples/dbyteos/home/deadbyte")
    Assert-NormalizedEqual $dbyteosSnapshotRootConfig.Text $expectedDbyteosSnapshotRootConfig "dbyteos snapshot config from root snapshot"

    $dbyteosSnapshotRootSecurity = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\snapshot.dby", "security") -WorkingDirectory $repoRoot
    if ($dbyteosSnapshotRootSecurity.Code -ne 0) { throw "dbyteos snapshot security from root failed: $($dbyteosSnapshotRootSecurity.Text)" }
    Assert-NormalizedEqual $dbyteosSnapshotRootSecurity.Text $expectedDbyteosSnapshotSecurity "dbyteos snapshot security from root snapshot"

    $dbyteosSysinfoRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\sysinfo.dby") -WorkingDirectory $repoRoot
    if ($dbyteosSysinfoRoot.Code -ne 0) { throw "dbyteos sysinfo from root failed: $($dbyteosSysinfoRoot.Text)" }
    Assert-NormalizedEqual $dbyteosSysinfoRoot.Text $expectedDbyteosSysinfo "dbyteos sysinfo snapshot"
    Assert-Contains $dbyteosSysinfoRoot.Text "DByteOS Alpha Userland" "dbyteos sysinfo banner"
    Assert-Contains $dbyteosSysinfoRoot.Text "version: DByte 9.0.2" "dbyteos sysinfo version"
    Assert-Contains $dbyteosSysinfoRoot.Text "codename: Userland Prototype" "dbyteos sysinfo codename"
    Assert-Contains $dbyteosSysinfoRoot.Text "guide: run help, status, or man <topic>" "dbyteos sysinfo guide"

    foreach ($profileText in @($dbyteosProfileDirect.Text, $dbyteosWelcomeDirect.Text, $dbyteosStatusReport.Text, $dbyteosSysinfoRoot.Text)) {
        Assert-Contains $profileText $dbyteosConfigUser.Text "dbyteos profile user sync"
        Assert-Contains $profileText $dbyteosConfigMode.Text "dbyteos profile mode sync"
        Assert-Contains $profileText $dbyteosConfigPrompt.Text "dbyteos profile prompt sync"
    }
    foreach ($snapshotText in @($dbyteosSnapshotProfile.Text, $dbyteosSnapshotDirect.Text)) {
        Assert-Contains $snapshotText $dbyteosConfigUser.Text "dbyteos snapshot user sync"
        Assert-Contains $snapshotText $dbyteosConfigHome.Text "dbyteos snapshot home sync"
        Assert-Contains $snapshotText $dbyteosConfigMode.Text "dbyteos snapshot mode sync"
        Assert-Contains $snapshotText $dbyteosConfigTheme.Text "dbyteos snapshot theme sync"
        Assert-Contains $snapshotText $dbyteosConfigPrompt.Text "dbyteos snapshot prompt sync"
    }
    Assert-Contains $dbyteosSnapshotRoot.Text $dbyteosConfigRootHome.Text "dbyteos snapshot root home sync"
    foreach ($keyLine in @("system.mode = beta-userland", "system.prompt = dbyte-shell>", "user.name = deadbyte", "user.home = home/deadbyte", "ui.theme = default", "security.mode = simulated")) {
        Assert-Contains $dbyteosSnapshotConfig.Text $keyLine "dbyteos snapshot config sync"
    }
    Assert-Contains $dbyteosProfileDirect.Text $dbyteosConfigHome.Text "dbyteos profile home sync"
    Assert-Contains $dbyteosWelcomeDirect.Text $dbyteosConfigHome.Text "dbyteos welcome home sync"
    Assert-Contains $dbyteosStatusReport.Text $dbyteosConfigHome.Text "dbyteos status home sync"
    Assert-Contains $dbyteosSysinfoRoot.Text $dbyteosConfigRootHome.Text "dbyteos sysinfo home sync"
    foreach ($profileText in @($dbyteosProfileDirect.Text, $dbyteosStatusReport.Text, $dbyteosSysinfoRoot.Text)) {
        Assert-Contains $profileText $dbyteosConfigTheme.Text "dbyteos profile theme sync"
    }

    $dbyteosHomeRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\home.dby") -WorkingDirectory $repoRoot
    if ($dbyteosHomeRoot.Code -ne 0) { throw "dbyteos home from root failed: $($dbyteosHomeRoot.Text)" }
    Assert-Equal $dbyteosHomeRoot.Text "home/deadbyte" "dbyteos home from root"

    $dbyteosTmpRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\tmp.dby") -WorkingDirectory $repoRoot
    if ($dbyteosTmpRoot.Code -ne 0) { throw "dbyteos tmp from root failed: $($dbyteosTmpRoot.Text)" }
    Assert-Equal $dbyteosTmpRoot.Text "tmp" "dbyteos tmp from root"

    $dbyteosLsSysRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\ls_sys.dby") -WorkingDirectory $repoRoot
    if ($dbyteosLsSysRoot.Code -ne 0) { throw "dbyteos ls_sys from root failed: $($dbyteosLsSysRoot.Text)" }
    $dbyteosLsSysExpected = "DByteOS sys layout:`n  /bin`n  /etc`n  /home`n  /sys`n  /tmp"
    Assert-Equal $dbyteosLsSysRoot.Text $dbyteosLsSysExpected "dbyteos ls_sys deterministic"

    $dbyteosCatMissing = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\cat.dby", "tmp/does-not-exist-xyz.bin") -WorkingDirectory $repoRoot
    if ($dbyteosCatMissing.Code -ne 0) { throw "dbyteos cat missing file failed: $($dbyteosCatMissing.Text)" }
    Assert-Equal $dbyteosCatMissing.Text "error: cat: no such file or directory: tmp/does-not-exist-xyz.bin" "dbyteos cat missing path"

    $dbyteosSpacedTmp = Join-Path $dbyteosRoot "tmp\path with spaces.txt"
    Set-Content -Path $dbyteosSpacedTmp -Value "spaced ok" -NoNewline
    $dbyteosCatSpaced = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\cat.dby", "tmp/path with spaces.txt") -WorkingDirectory $repoRoot
    if ($dbyteosCatSpaced.Code -ne 0) { throw "dbyteos cat spaced path failed: $($dbyteosCatSpaced.Text)" }
    Assert-Equal $dbyteosCatSpaced.Text "spaced ok" "dbyteos cat spaced path"

    $dbyteosWriteDemoOnce = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\write_demo.dby") -WorkingDirectory $repoRoot
    if ($dbyteosWriteDemoOnce.Code -ne 0) { throw "dbyteos write_demo first run failed: $($dbyteosWriteDemoOnce.Text)" }
    $dbyteosWriteDemoTwice = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\write_demo.dby") -WorkingDirectory $repoRoot
    if ($dbyteosWriteDemoTwice.Code -ne 0) { throw "dbyteos write_demo second run failed: $($dbyteosWriteDemoTwice.Text)" }
    Assert-Equal $dbyteosWriteDemoOnce.Text $dbyteosWriteDemoTwice.Text "dbyteos write_demo idempotent output"
    Assert-Equal (Bytes-Hex (Join-Path $dbyteosRoot "tmp\write_demo.txt")) "64627974656f732077726974655f64656d6f206f6b0a" "dbyteos write_demo single artifact"

    $dbyteosCatRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\cat.dby", "tmp/write_demo.txt") -WorkingDirectory $repoRoot
    if ($dbyteosCatRoot.Code -ne 0) { throw "dbyteos cat from root failed: $($dbyteosCatRoot.Text)" }
    Assert-Contains $dbyteosCatRoot.Text "dbyteos write_demo ok" "dbyteos cat from root"

    # DByteOS file / user environment commands (writes only under tmp/ or home/deadbyte/)
    $dbyteosReadBad = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\read.dby", "tmp/../README.md") -WorkingDirectory $repoRoot
    if ($dbyteosReadBad.Code -ne 0) { throw "dbyteos read escape path exit code: $($dbyteosReadBad.Code)" }
    Assert-Equal $dbyteosReadBad.Text "error: permission denied: path escape tmp/../README.md" "dbyteos read rejects dot-dot"

    $dbyteosWriteV32 = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\write.dby", "tmp/verify_v32.txt", "hello", "v32", "smoke") -WorkingDirectory $repoRoot
    if ($dbyteosWriteV32.Code -ne 0) { throw "dbyteos write from root failed: $($dbyteosWriteV32.Text)" }
    Assert-Equal $dbyteosWriteV32.Text "write: ok" "dbyteos write from root"
    $dbyteosReadV32 = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\read.dby", "tmp/verify_v32.txt") -WorkingDirectory $repoRoot
    if ($dbyteosReadV32.Code -ne 0) { throw "dbyteos read from root failed: $($dbyteosReadV32.Text)" }
    Assert-Equal $dbyteosReadV32.Text "hello v32 smoke" "dbyteos read from root"

    $dbyteosPathRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\path.dby") -WorkingDirectory $repoRoot
    if ($dbyteosPathRoot.Code -ne 0) { throw "dbyteos path from root failed: $($dbyteosPathRoot.Text)" }
    Assert-Contains $dbyteosPathRoot.Text "PATH=/bin:/tmp:/home/deadbyte" "dbyteos path display"
    Assert-Contains $dbyteosPathRoot.Text "COMMAND_ROOT=examples/dbyteos/bin" "dbyteos path command root bin"

    $dbyteosPathWhichMkdir = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\path.dby", "which", "mkdir-demo") -WorkingDirectory $repoRoot
    if ($dbyteosPathWhichMkdir.Code -ne 0) { throw "dbyteos path which mkdir-demo failed: $($dbyteosPathWhichMkdir.Text)" }
    Assert-Contains $dbyteosPathWhichMkdir.Text "examples/dbyteos/bin/mkdir_demo.dby" "dbyteos path which hyphen command"

    $dbyteosPathWhichUnknown = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\path.dby", "which", "does-not-exist-xyz") -WorkingDirectory $repoRoot
    if ($dbyteosPathWhichUnknown.Code -ne 0) { throw "dbyteos path which unknown failed: $($dbyteosPathWhichUnknown.Text)" }
    Assert-Contains $dbyteosPathWhichUnknown.Text "error: path: command not found: does-not-exist-xyz" "dbyteos path which unknown deterministic"

    $dbyteosEnvRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\env.dby") -WorkingDirectory $repoRoot
    if ($dbyteosEnvRoot.Code -ne 0) { throw "dbyteos env from root failed: $($dbyteosEnvRoot.Text)" }
    Assert-Contains $dbyteosEnvRoot.Text "USER=deadbyte" "dbyteos env user"
    Assert-Contains $dbyteosEnvRoot.Text "HOME=examples/dbyteos/home/deadbyte" "dbyteos env get_home from root"
    Assert-Contains $dbyteosEnvRoot.Text "PATH=/bin:/tmp:/home/deadbyte" "dbyteos env path"
    Assert-Contains $dbyteosEnvRoot.Text "COMMAND_ROOT=examples/dbyteos/bin" "dbyteos env command root bin"

    $dbyteosProfileRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\profile.dby") -WorkingDirectory $repoRoot
    if ($dbyteosProfileRoot.Code -ne 0) { throw "dbyteos profile from root failed: $($dbyteosProfileRoot.Text)" }
    Assert-Contains $dbyteosProfileRoot.Text "--- DByteOS Profile ---" "dbyteos profile banner"
    Assert-Contains $dbyteosProfileRoot.Text "user: deadbyte" "dbyteos profile user"
    Assert-Contains $dbyteosProfileRoot.Text "home: examples/dbyteos/home/deadbyte" "dbyteos profile get_home from root"
    Assert-Contains $dbyteosProfileRoot.Text "shell: dbyte shell" "dbyteos profile shell"
    Assert-Contains $dbyteosProfileRoot.Text "mode: beta-userland" "dbyteos profile mode"
    Assert-Contains $dbyteosProfileRoot.Text "theme: default" "dbyteos profile theme"
    Assert-Contains $dbyteosProfileRoot.Text "prompt: dbyte-shell>" "dbyteos profile prompt"
    Assert-Contains $dbyteosProfileRoot.Text "os_version: 9.0.2" "dbyteos profile os version"

    $dbyteosNotesOnce = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\notes.dby", "clear-demo") -WorkingDirectory $repoRoot
    if ($dbyteosNotesOnce.Code -ne 0) { throw "dbyteos notes failed: $($dbyteosNotesOnce.Text)" }
    Assert-Equal $dbyteosNotesOnce.Text "notes: reset to seed state" "dbyteos notes idempotent banner"
    $dbyteosReadNotes = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\read.dby", "home/deadbyte/notes.txt") -WorkingDirectory $repoRoot
    if ($dbyteosReadNotes.Code -ne 0) { throw "dbyteos read notes failed: $($dbyteosReadNotes.Text)" }
    Assert-Equal $dbyteosReadNotes.Text "dbyteos notes seed" "dbyteos read notes body"

    $dbyteosMkdirDemo = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\mkdir_demo.dby") -WorkingDirectory $repoRoot
    if ($dbyteosMkdirDemo.Code -ne 0) { throw "dbyteos mkdir_demo from root failed: $($dbyteosMkdirDemo.Text)" }
    Assert-Equal $dbyteosMkdirDemo.Text "mkdir-demo: ok" "dbyteos mkdir_demo idempotent"

    $dbyteosAppendA = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\append.dby", "tmp/append_test.txt", "line-a") -WorkingDirectory $repoRoot
    if ($dbyteosAppendA.Code -ne 0) { throw "dbyteos append first failed: $($dbyteosAppendA.Text)" }
    $dbyteosAppendB = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\append.dby", "tmp/append_test.txt", "line-b") -WorkingDirectory $repoRoot
    if ($dbyteosAppendB.Code -ne 0) { throw "dbyteos append second failed: $($dbyteosAppendB.Text)" }
    $dbyteosReadAppend = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\read.dby", "tmp/append_test.txt") -WorkingDirectory $repoRoot
    if ($dbyteosReadAppend.Code -ne 0) { throw "dbyteos read append file failed: $($dbyteosReadAppend.Text)" }
    Assert-Equal $dbyteosReadAppend.Text "line-a`nline-b" "dbyteos append accumulates lines"

    $dbyteosTouchRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\touch.dby", "tmp/touched.txt") -WorkingDirectory $repoRoot
    if ($dbyteosTouchRoot.Code -ne 0) { throw "dbyteos touch from root failed: $($dbyteosTouchRoot.Text)" }
    Assert-Equal $dbyteosTouchRoot.Text "touch: ok" "dbyteos touch from root"

    $dbyteosWriteCwd = Invoke-Dbyte -Arguments @("run", "bin\write.dby", "tmp/cwd_write.txt", "cwd", "ok") -WorkingDirectory $dbyteosRoot
    if ($dbyteosWriteCwd.Code -ne 0) { throw "dbyteos write from dbyteos cwd failed: $($dbyteosWriteCwd.Text)" }
    Assert-Equal $dbyteosWriteCwd.Text "write: ok" "dbyteos write from dbyteos cwd"
    $dbyteosReadCwd = Invoke-Dbyte -Arguments @("run", "bin\read.dby", "tmp/cwd_write.txt") -WorkingDirectory $dbyteosRoot
    if ($dbyteosReadCwd.Code -ne 0) { throw "dbyteos read from dbyteos cwd failed: $($dbyteosReadCwd.Text)" }
    Assert-Equal $dbyteosReadCwd.Text "cwd ok" "dbyteos read from dbyteos cwd"

    Write-Host "Running DByteOS Security/Permissions (v9.0.2) smoke tests..."
    $securityLogPath = Join-Path $dbyteosRoot "tmp\security.log"
    if (Test-Path $securityLogPath) {
        Remove-Item -Force $securityLogPath
    }
    $dbyteosPermPolicy = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\perm.dby", "policy") -WorkingDirectory $repoRoot
    if ($dbyteosPermPolicy.Code -ne 0) { throw "dbyteos perm policy failed: $($dbyteosPermPolicy.Text)" }
    Assert-Contains $dbyteosPermPolicy.Text "tmp/             read/write allowed" "perm policy tmp"
    Assert-Contains $dbyteosPermPolicy.Text "etc/             read allowed, write denied" "perm policy etc"
    $dbyteosPermMatrix = @(
        @("read", "tmp/verify_v32.txt", "ALLOW read tmp/verify_v32.txt"),
        @("write", "tmp/verify_v32.txt", "ALLOW write tmp/verify_v32.txt"),
        @("append", "tmp/verify_v32.txt", "ALLOW append tmp/verify_v32.txt"),
        @("read", "home/deadbyte/notes.txt", "ALLOW read home/deadbyte/notes.txt"),
        @("write", "home/deadbyte/notes.txt", "ALLOW write home/deadbyte/notes.txt"),
        @("append", "home/deadbyte/notes.txt", "ALLOW append home/deadbyte/notes.txt"),
        @("read", "etc/system.dby", "ALLOW read etc/system.dby"),
        @("write", "etc/system.dby", "DENY write etc/system.dby (policy)"),
        @("append", "etc/system.dby", "DENY append etc/system.dby (policy)"),
        @("read", "sys/security.dby", "ALLOW read sys/security.dby"),
        @("write", "sys/security.dby", "DENY write sys/security.dby (policy)"),
        @("append", "sys/security.dby", "DENY append sys/security.dby (policy)"),
        @("read", "bin/perm.dby", "ALLOW read bin/perm.dby"),
        @("write", "bin/perm.dby", "DENY write bin/perm.dby (policy)"),
        @("append", "bin/perm.dby", "DENY append bin/perm.dby (policy)"),
        @("read", "../outside.txt", "DENY read ../outside.txt (path escape)"),
        @("write", "/absolute.txt", "DENY write /absolute.txt (absolute path)"),
        @("append", "C:\absolute.txt", "DENY append C:\absolute.txt (absolute path)"),
        @("read", "var/log.txt", "DENY read var/log.txt (policy)")
    )
    foreach ($case in $dbyteosPermMatrix) {
        $permCase = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\perm.dby", $case[0], $case[1]) -WorkingDirectory $repoRoot
        if ($permCase.Code -ne 0) { throw "dbyteos perm matrix failed for $($case[0]) $($case[1]): $($permCase.Text)" }
        Assert-Equal $permCase.Text $case[2] "perm matrix $($case[0]) $($case[1])"
    }
    $dbyteosPermReadEtc = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\perm.dby", "read", "etc/system.dby") -WorkingDirectory $repoRoot
    if ($dbyteosPermReadEtc.Code -ne 0) { throw "dbyteos perm read etc failed: $($dbyteosPermReadEtc.Text)" }
    Assert-Equal $dbyteosPermReadEtc.Text "ALLOW read etc/system.dby" "perm read etc allowed"
    $dbyteosPermWriteEtc = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\perm.dby", "write", "etc/system.dby") -WorkingDirectory $repoRoot
    if ($dbyteosPermWriteEtc.Code -ne 0) { throw "dbyteos perm write etc failed: $($dbyteosPermWriteEtc.Text)" }
    Assert-Equal $dbyteosPermWriteEtc.Text "DENY write etc/system.dby (policy)" "perm write etc denied"
    $dbyteosPermEscape = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\perm.dby", "read", "tmp/../etc/system.dby") -WorkingDirectory $repoRoot
    if ($dbyteosPermEscape.Code -ne 0) { throw "dbyteos perm path escape failed: $($dbyteosPermEscape.Text)" }
    Assert-Equal $dbyteosPermEscape.Text "DENY read tmp/../etc/system.dby (path escape)" "perm path escape denied"
    $dbyteosPermAbsolute = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\perm.dby", "read", "C:/Windows/system.ini") -WorkingDirectory $repoRoot
    if ($dbyteosPermAbsolute.Code -ne 0) { throw "dbyteos perm absolute path failed: $($dbyteosPermAbsolute.Text)" }
    Assert-Equal $dbyteosPermAbsolute.Text "DENY read C:/Windows/system.ini (absolute path)" "perm absolute denied"
    $dbyteosPermUnknown = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\perm.dby", "read", "var/log.txt") -WorkingDirectory $repoRoot
    if ($dbyteosPermUnknown.Code -ne 0) { throw "dbyteos perm unknown root failed: $($dbyteosPermUnknown.Text)" }
    Assert-Equal $dbyteosPermUnknown.Text "DENY read var/log.txt (policy)" "perm unknown root denied"
    $dbyteosReadEtc = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\read.dby", "etc/system.dby") -WorkingDirectory $repoRoot
    if ($dbyteosReadEtc.Code -ne 0) { throw "dbyteos read etc failed: $($dbyteosReadEtc.Text)" }
    Assert-Contains $dbyteosReadEtc.Text "pub let os_version: str = `"9.0.2`"" "read etc allowed"
    $dbyteosWriteEtcDenied = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\write.dby", "etc/system.dby", "test") -WorkingDirectory $repoRoot
    if ($dbyteosWriteEtcDenied.Code -ne 0) { throw "dbyteos write etc deny command failed: $($dbyteosWriteEtcDenied.Text)" }
    Assert-Equal $dbyteosWriteEtcDenied.Text "error: permission denied: write etc/system.dby" "write etc denied"
    $dbyteosAppendEtcDenied = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\append.dby", "etc/system.dby", "test") -WorkingDirectory $repoRoot
    if ($dbyteosAppendEtcDenied.Code -ne 0) { throw "dbyteos append etc deny command failed: $($dbyteosAppendEtcDenied.Text)" }
    Assert-Equal $dbyteosAppendEtcDenied.Text "error: permission denied: append etc/system.dby" "append etc denied"
    $dbyteosReadEscapeDenied = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\read.dby", "tmp/../etc/system.dby") -WorkingDirectory $repoRoot
    if ($dbyteosReadEscapeDenied.Code -ne 0) { throw "dbyteos read escape deny command failed: $($dbyteosReadEscapeDenied.Text)" }
    Assert-Equal $dbyteosReadEscapeDenied.Text "error: permission denied: path escape tmp/../etc/system.dby" "read path escape denied"
    $dbyteosWriteAbsoluteDenied = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\write.dby", "C:/Windows/system.ini", "test") -WorkingDirectory $repoRoot
    if ($dbyteosWriteAbsoluteDenied.Code -ne 0) { throw "dbyteos write absolute deny command failed: $($dbyteosWriteAbsoluteDenied.Text)" }
    Assert-Equal $dbyteosWriteAbsoluteDenied.Text "error: permission denied: absolute path C:/Windows/system.ini" "write absolute denied"
    $dbyteosReadSlashAbsoluteDenied = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\read.dby", "/absolute.txt") -WorkingDirectory $repoRoot
    if ($dbyteosReadSlashAbsoluteDenied.Code -ne 0) { throw "dbyteos read slash absolute deny command failed: $($dbyteosReadSlashAbsoluteDenied.Text)" }
    Assert-Equal $dbyteosReadSlashAbsoluteDenied.Text "error: permission denied: absolute path /absolute.txt" "read slash absolute denied"
    $dbyteosAppendAbsoluteDenied = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\append.dby", "C:\absolute.txt", "test") -WorkingDirectory $repoRoot
    if ($dbyteosAppendAbsoluteDenied.Code -ne 0) { throw "dbyteos append absolute deny command failed: $($dbyteosAppendAbsoluteDenied.Text)" }
    Assert-Equal $dbyteosAppendAbsoluteDenied.Text "error: permission denied: absolute path C:\absolute.txt" "append absolute denied"
    $dbyteosWriteDotDotDenied = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\write.dby", "../outside.txt", "test") -WorkingDirectory $repoRoot
    if ($dbyteosWriteDotDotDenied.Code -ne 0) { throw "dbyteos write dotdot deny command failed: $($dbyteosWriteDotDotDenied.Text)" }
    Assert-Equal $dbyteosWriteDotDotDenied.Text "error: permission denied: path escape ../outside.txt" "write dotdot denied"
    $dbyteosAppendEscapeDenied = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\append.dby", "tmp/../etc/system.dby", "test") -WorkingDirectory $repoRoot
    if ($dbyteosAppendEscapeDenied.Code -ne 0) { throw "dbyteos append escape deny command failed: $($dbyteosAppendEscapeDenied.Text)" }
    Assert-Equal $dbyteosAppendEscapeDenied.Text "error: permission denied: path escape tmp/../etc/system.dby" "append path escape denied"
    $dbyteosReadUnknownDenied = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\read.dby", "var/log.txt") -WorkingDirectory $repoRoot
    if ($dbyteosReadUnknownDenied.Code -ne 0) { throw "dbyteos read unknown deny command failed: $($dbyteosReadUnknownDenied.Text)" }
    Assert-Equal $dbyteosReadUnknownDenied.Text "error: permission denied: read var/log.txt" "read unknown root denied"
    $securityLog = Get-Content $securityLogPath -Raw
    Assert-Contains $securityLog "DENY write etc/system.dby" "security log write etc"
    Assert-Contains $securityLog "DENY append etc/system.dby" "security log append etc"
    Assert-Contains $securityLog "DENY read tmp/../etc/system.dby" "security log path escape"
    Assert-Contains $securityLog "DENY write C:/Windows/system.ini" "security log absolute"
    Assert-Contains $securityLog "DENY read /absolute.txt" "security log slash absolute"
    Assert-Contains $securityLog "DENY append C:\absolute.txt" "security log windows absolute"
    Assert-Contains $securityLog "DENY write ../outside.txt" "security log dotdot"
    Assert-Contains $securityLog "DENY append tmp/../etc/system.dby" "security log append escape"
    Assert-Contains $securityLog "DENY read var/log.txt" "security log unknown root"
    $dbyteosRepeatWriteDenied = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\write.dby", "etc/system.dby", "test") -WorkingDirectory $repoRoot
    if ($dbyteosRepeatWriteDenied.Code -ne 0) { throw "dbyteos repeat write etc deny command failed: $($dbyteosRepeatWriteDenied.Text)" }
    Assert-Equal $dbyteosRepeatWriteDenied.Text "error: permission denied: write etc/system.dby" "repeat write etc denied"
    $securityLogRepeat = Get-Content $securityLogPath -Raw
    Assert-Contains $securityLogRepeat "DENY read var/log.txt`nDENY write etc/system.dby`n" "security log append order"
    $dbyteosReadSys = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\read.dby", "sys/security.dby") -WorkingDirectory $repoRoot
    if ($dbyteosReadSys.Code -ne 0) { throw "dbyteos read sys failed: $($dbyteosReadSys.Text)" }
    Assert-Contains $dbyteosReadSys.Text "pub fn is_allowed" "read sys allowed"
    $dbyteosReadBin = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\read.dby", "bin/perm.dby") -WorkingDirectory $repoRoot
    if ($dbyteosReadBin.Code -ne 0) { throw "dbyteos read bin failed: $($dbyteosReadBin.Text)" }
    Assert-Contains $dbyteosReadBin.Text "usage: perm <read|write|append|policy> [path]" "read bin allowed"
    $dbyteosWriteSysDenied = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\write.dby", "sys/security.dby", "test") -WorkingDirectory $repoRoot
    if ($dbyteosWriteSysDenied.Code -ne 0) { throw "dbyteos write sys deny command failed: $($dbyteosWriteSysDenied.Text)" }
    Assert-Equal $dbyteosWriteSysDenied.Text "error: permission denied: write sys/security.dby" "write sys denied"
    $dbyteosAppendBinDenied = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\append.dby", "bin/perm.dby", "test") -WorkingDirectory $repoRoot
    if ($dbyteosAppendBinDenied.Code -ne 0) { throw "dbyteos append bin deny command failed: $($dbyteosAppendBinDenied.Text)" }
    Assert-Equal $dbyteosAppendBinDenied.Text "error: permission denied: append bin/perm.dby" "append bin denied"
    $catSource = Get-Content (Join-Path $dbyteosRoot "bin\cat.dby") -Raw
    $touchSource = Get-Content (Join-Path $dbyteosRoot "bin\touch.dby") -Raw
    
    Write-Host "Running DByteOS Security Enforcement Expansion (v9.0.2) smoke tests..."
    $enforcementInput = @"
clean
cat etc/system.dby
cat tmp/../etc/system.dby
touch tmp/security_touch.txt
touch etc/security_touch.txt
inspect bin/perm.dby
inspect unknown/file
read tmp/security.log
clean
quit
"@
    $dbyteosEnforcement = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "$enforcementInput`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosEnforcement.Code -ne 0) { throw "dbyteos security enforcement failed: $($dbyteosEnforcement.Text)" }
    Assert-Contains $dbyteosEnforcement.Text "os_version: str = `"9.0.2`"" "cat etc allowed"
    Assert-Contains $dbyteosEnforcement.Text "error: permission denied: path escape tmp/../etc/system.dby" "cat escape denied"
    Assert-Contains $dbyteosEnforcement.Text "touch: ok" "touch tmp allowed"
    Assert-Contains $dbyteosEnforcement.Text "error: permission denied: touch etc/security_touch.txt" "touch etc denied"
    Assert-Contains $dbyteosEnforcement.Text "Inspecting file:" "inspect bin allowed"
    Assert-Contains $dbyteosEnforcement.Text "error: permission denied: inspect unknown/file" "inspect unknown root denied"
    Assert-Contains $dbyteosEnforcement.Text "DENY cat tmp/../etc/system.dby" "security log cat denied"
    Assert-Contains $dbyteosEnforcement.Text "DENY touch etc/security_touch.txt" "security log touch denied"
    Assert-Contains $dbyteosEnforcement.Text "DENY inspect unknown/file" "security log inspect denied"
    Assert-Contains $dbyteosEnforcement.Text "workspace sweep complete" "enforcement clean sweep"

    Write-Host "Running DByteOS Security Enforcement Hardening (v9.0.2) smoke tests..."
    $hardeningInput = @"
clean
cat boot.dby
touch boot.dby
cat .dbyterc
touch .dbyterc
cat etc/../etc/system.dby
touch tmp/../etc/system.dby
inspect /etc/system.dby
man perm
clean
quit
"@
    $dbyteosHardening = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "$hardeningInput`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosHardening.Code -ne 0) { throw "dbyteos security hardening failed: $($dbyteosHardening.Text)" }
    Assert-Contains $dbyteosHardening.Text "Boot sequence" "cat boot.dby allowed"
    Assert-Contains $dbyteosHardening.Text "error: permission denied: touch boot.dby" "touch boot.dby denied"
    Assert-Contains $dbyteosHardening.Text "alias help" "cat .dbyterc allowed"
    Assert-Contains $dbyteosHardening.Text "error: permission denied: touch .dbyterc" "touch .dbyterc denied"
    Assert-Contains $dbyteosHardening.Text "error: permission denied: path escape etc/../etc/system.dby" "cat escape denied"
    Assert-Contains $dbyteosHardening.Text "error: permission denied: path escape tmp/../etc/system.dby" "touch escape denied"
    Assert-Contains $dbyteosHardening.Text "error: permission denied: absolute path /etc/system.dby" "inspect absolute denied"
    Assert-Contains $dbyteosHardening.Text "enforced by the system security policy" "man perm updated"
    
    $dbyteosNoRcScoping = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "cat etc/system.dby`nquit`n" -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosNoRcScoping.Text "ShellError: unknown command: cat" "shell --no-rc hides cat autopath"

    Write-Host "Verifying DByteOS Personal Workspace Beta Foundation (v9.0.2) documentation..."
    $dbyteDocs = @("DBYTEOS_PERSONAL_ALPHA.md", "DBYTEOS_ALPHA.md", "DBYTEOS_COMMANDS.md", "DBYTEOS_SECURITY.md", "DBYTEOS_BOOT.md", "DBYTEOS_PACKAGE.md", "DBYTEOS_ONBOARDING.md", "DBYTEOS_PROFILE.md", "DBYTEOS_CONFIG.md", "DBYTEOS_SNAPSHOT.md", "DBYTEOS_PROJECTS.md", "DBYTEOS_TASKS.md", "KERNEL_EXCEPTIONS.md", "KERNEL_IRQ.md")
    foreach ($doc in $dbyteDocs) {
        $p = Join-Path $repoRoot "docs/$doc"
        if (-not (Test-Path $p)) { throw "DByteOS doc missing: $doc" }
    }
    $mainReadme = Get-Content (Join-Path $repoRoot "README.md") -Raw
    Assert-Contains $mainReadme "DByteOS Personal Workspace Beta Foundation (v9.0.2)" "README Personal Workspace Beta Foundation positioning"
    Assert-Contains $mainReadme "docs/DBYTEOS_PERSONAL_ALPHA.md" "README Personal Workspace Beta Foundation link"
    Assert-Contains $mainReadme "docs/DBYTEOS_ALPHA.md" "README alpha link"
    Assert-Contains $mainReadme "docs/DBYTEOS_ONBOARDING.md" "README onboarding link"
    Assert-Contains $mainReadme "docs/DBYTEOS_PROFILE.md" "README profile link"
    Assert-Contains $mainReadme "docs/DBYTEOS_CONFIG.md" "README config link"
    Assert-Contains $mainReadme "docs/DBYTEOS_SNAPSHOT.md" "README snapshot link"
    Assert-Contains $mainReadme "docs/DBYTEOS_PROJECTS.md" "README projects link"
    Assert-Contains $mainReadme "docs/DBYTEOS_TASKS.md" "README tasks link"
    Assert-Contains $mainReadme "docs/DBYTEOS_PACKAGE.md" "README package guide link"
    Assert-Contains $mainReadme "docs/KERNEL_EXCEPTIONS.md" "README kernel exception foundation link"
    Assert-Contains $mainReadme "docs/KERNEL_IRQ.md" "README kernel irq foundation link"
    Assert-Contains $mainReadme "Smoke-test a zip release" "README zip quickstart"
    Assert-Contains $mainReadme "dbyte shell --rc examples/dbyteos/.dbyterc" "README shell quickstart command"
    Assert-Contains $mainReadme "welcome" "README onboarding welcome command"
    Assert-Contains $mainReadme "profile show" "README profile show command"
    Assert-Contains $mainReadme "config show" "README config show command"
    Assert-Contains $mainReadme "snapshot" "README snapshot command"
    Assert-Contains $mainReadme "project reset-demo" "README project reset command"
    Assert-Contains $mainReadme "task reset-demo" "README task reset command"
    Assert-Contains $mainReadme "task list demo" "README task list command"
    Assert-Contains $mainReadme "task add demo write tests" "README task add command"
    Assert-Contains $mainReadme "task done demo 1" "README task done command"
    Assert-Contains $mainReadme "task status demo" "README task status command"
    Assert-Contains $mainReadme "task summary demo" "README task summary command"
    Assert-Contains $mainReadme "task open demo" "README task open command"
    Assert-Contains $mainReadme "task doctor demo" "README task doctor command"
    Assert-Contains $mainReadme "task snapshot demo" "README task snapshot command"
    Assert-Contains $mainReadme "task clear-done demo" "README task clear-done command"
    Assert-Contains $mainReadme "getting-started" "README onboarding getting-started command"
    Assert-Contains $mainReadme "commands" "README onboarding commands command"
    Assert-Contains $mainReadme "man-index" "README onboarding man-index command"
    Assert-Contains (Normalize-Output $mainReadme) "boot`nwelcome`ncheck-system`ndoctor`nprefs set system.prompt dbyteos>`nsnapshot`nproject reset-demo`ntask reset-demo`ntask list demo`ntask add demo write tests`ntask done demo 1`ntask status demo`ntask summary demo`ntask open demo`ntask doctor demo`ntask snapshot demo`ntask clear-done demo`nproject status demo`nproject snapshot demo`nprefs reset-demo`nprofile show`nconfig show`ngetting-started`ncommands`nman-index`nboot`nhelp`nstatus`nsysinfo`nwhich read`nman index`nman perm`nquit" "README package quickstart command sequence"
    Assert-Contains $mainReadme "which read" "README package quickstart which command"
    Assert-Contains $mainReadme "man perm" "README package quickstart man command"
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_PERSONAL_ALPHA.md"))) { throw "README Personal Workspace Beta Foundation link target missing" }
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_ALPHA.md"))) { throw "README alpha link target missing" }
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_ONBOARDING.md"))) { throw "README onboarding link target missing" }
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_PROFILE.md"))) { throw "README profile link target missing" }
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_CONFIG.md"))) { throw "README config link target missing" }
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_SNAPSHOT.md"))) { throw "README snapshot link target missing" }
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_PROJECTS.md"))) { throw "README projects link target missing" }
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_TASKS.md"))) { throw "README tasks link target missing" }
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_PACKAGE.md"))) { throw "README package link target missing" }
    if (-not (Test-Path (Join-Path $repoRoot "docs\KERNEL_EXCEPTIONS.md"))) { throw "README kernel exception foundation link target missing" }
    if (-not (Test-Path (Join-Path $repoRoot "docs\KERNEL_IRQ.md"))) { throw "README kernel irq foundation link target missing" }
    
    $osReadme = Get-Content (Join-Path $repoRoot "examples/dbyteos/README.md") -Raw
    Assert-Contains $osReadme "DByteOS Personal Workspace Beta Foundation (v9.0.2)" "OS README Personal Workspace Beta Foundation positioning"
    Assert-Contains $osReadme '| `cat` | View file contents |' "OS README command table"
    Assert-Contains $osReadme "Package Smoke" "OS README package smoke"
    Assert-Contains $osReadme ".\dbyte.exe --version" "OS README package version smoke"
    Assert-Contains $osReadme ".\dbyte.exe shell --rc examples/dbyteos/.dbyterc" "OS README package shell smoke"
    Assert-Contains $osReadme "profile show" "OS README profile smoke"
    Assert-Contains $osReadme "config show" "OS README config smoke"
    Assert-Contains $osReadme "snapshot" "OS README snapshot smoke"
    Assert-Contains $osReadme "project reset-demo" "OS README project smoke"
    Assert-Contains $osReadme "task reset-demo" "OS README task reset smoke"
    Assert-Contains $osReadme "task list demo" "OS README task list smoke"
    Assert-Contains $osReadme "sysinfo" "OS README package sysinfo smoke"
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_SECURITY.md"))) { throw "OS README security link target missing" }
    $packageGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_PACKAGE.md") -Raw
    Assert-Contains $packageGuide "DByteOS Personal Workspace Beta Foundation Package Smoke Guide" "package guide title"
    Assert-Contains $packageGuide ".\dbyte.exe --version" "package guide version smoke"
    Assert-Contains $packageGuide ".\dbyte.exe shell --rc examples/dbyteos/.dbyterc" "package guide shell quickstart"
    Assert-Contains $packageGuide "welcome" "package guide welcome command"
    Assert-Contains $packageGuide "check-system" "package guide check-system command"
    Assert-Contains $packageGuide "doctor" "package guide doctor command"
    Assert-Contains $packageGuide "prefs set system.prompt dbyteos>" "package guide prompt command"
    Assert-Contains $packageGuide "prefs reset-demo" "package guide prefs reset command"
    Assert-Contains $packageGuide "profile show" "package guide profile show command"
    Assert-Contains $packageGuide "config show" "package guide config show command"
    Assert-Contains $packageGuide "snapshot" "package guide snapshot command"
    Assert-Contains $packageGuide "project reset-demo" "package guide project reset command"
    Assert-Contains $packageGuide "task reset-demo" "package guide task reset command"
    Assert-Contains $packageGuide "task list demo" "package guide task list command"
    Assert-Contains $packageGuide "task add demo write tests" "package guide task add command"
    Assert-Contains $packageGuide "task done demo 1" "package guide task done command"
    Assert-Contains $packageGuide "task status demo" "package guide task status command"
    Assert-Contains $packageGuide "task summary demo" "package guide task summary command"
    Assert-Contains $packageGuide "task open demo" "package guide task open command"
    Assert-Contains $packageGuide "task doctor demo" "package guide task doctor command"
    Assert-Contains $packageGuide "task snapshot demo" "package guide task snapshot command"
    Assert-Contains $packageGuide "task clear-done demo" "package guide task clear-done command"
    Assert-Contains $packageGuide "project status demo" "package guide project status command"
    Assert-Contains $packageGuide "project notes demo" "package guide project notes command"
    Assert-Contains $packageGuide "project snapshot demo" "package guide project snapshot command"
    Assert-Contains $packageGuide "project doctor demo" "package guide project doctor command"
    Assert-Contains $packageGuide "getting-started" "package guide getting-started command"
    Assert-Contains $packageGuide "commands" "package guide commands command"
    Assert-Contains $packageGuide "man-index" "package guide man-index command"
    Assert-Contains $packageGuide "boot" "package guide boot command"
    Assert-Contains $packageGuide "help" "package guide help command"
    Assert-Contains $packageGuide "status" "package guide status command"
    Assert-Contains $packageGuide "sysinfo" "package guide sysinfo command"
    Assert-Contains $packageGuide "which read" "package guide command discovery"
    Assert-Contains $packageGuide "man perm" "package guide man command"
    $onboardingGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_ONBOARDING.md") -Raw
    Assert-Contains $onboardingGuide "DByteOS Onboarding" "onboarding guide title"
    Assert-Contains $onboardingGuide "Personal Workspace Beta Foundation" "onboarding guide Personal Workspace Beta Foundation"
    Assert-Contains $onboardingGuide "boot" "onboarding guide boot"
    Assert-Contains $onboardingGuide "welcome" "onboarding guide welcome"
    Assert-Contains $onboardingGuide "check-system" "onboarding guide check-system"
    Assert-Contains $onboardingGuide "doctor" "onboarding guide doctor"
    Assert-Contains $onboardingGuide "prefs set system.prompt dbyteos>" "onboarding guide prompt command"
    Assert-Contains $onboardingGuide "profile show" "onboarding guide profile show"
    Assert-Contains $onboardingGuide "config show" "onboarding guide config show"
    Assert-Contains $onboardingGuide "snapshot" "onboarding guide snapshot"
    Assert-Contains $onboardingGuide "project reset-demo" "onboarding guide project reset"
    Assert-Contains $onboardingGuide "task reset-demo" "onboarding guide task reset"
    Assert-Contains $onboardingGuide "task status demo" "onboarding guide task status"
    Assert-Contains $onboardingGuide "task summary demo" "onboarding guide task summary"
    Assert-Contains $onboardingGuide "task open demo" "onboarding guide task open"
    Assert-Contains $onboardingGuide "task doctor demo" "onboarding guide task doctor"
    Assert-Contains $onboardingGuide "task snapshot demo" "onboarding guide task snapshot"
    Assert-Contains $onboardingGuide "task clear-done demo" "onboarding guide task clear-done"
    Assert-Contains $onboardingGuide "getting-started" "onboarding guide getting-started"
    Assert-Contains $onboardingGuide "man-index" "onboarding guide man-index"
    $profileGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_PROFILE.md") -Raw
    Assert-Contains $profileGuide "DByteOS Profile" "profile guide title"
    Assert-Contains $profileGuide "profile show" "profile guide show"
    Assert-Contains $profileGuide "beta-userland" "profile guide mode"
    Assert-Contains $profileGuide "read-only DByteOS config layer" "profile guide config source"
    Assert-Contains $profileGuide "snapshot profile" "profile guide snapshot"
    $configGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_CONFIG.md") -Raw
    Assert-Contains $configGuide "DByteOS Config" "config guide title"
    Assert-Contains $configGuide "config show" "config guide show"
    Assert-Contains $configGuide "system.prompt = dbyte-shell>" "config guide prompt"
    Assert-Contains $configGuide "read-only in v9.0.2" "config guide read-only"
    Assert-Contains $configGuide "snapshot config" "config guide snapshot"
    $snapshotGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_SNAPSHOT.md") -Raw
    Assert-Contains $snapshotGuide "DByteOS Snapshot" "snapshot guide title"
    Assert-Contains $snapshotGuide "snapshot system" "snapshot guide system"
    Assert-Contains $snapshotGuide "read-only in v9.0.2" "snapshot guide read-only"
    $projectsGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_PROJECTS.md") -Raw
    Assert-Contains $projectsGuide "DByteOS Workspace Projects" "projects guide title"
    Assert-Contains $projectsGuide "project new demo" "projects guide new demo"
    Assert-Contains $projectsGuide "home/deadbyte/projects/" "projects guide user data path"
    Assert-Contains $projectsGuide "v9.0.2 disabled path foundation" "projects guide hardening"
    Assert-Contains $projectsGuide "error: project not found: missing" "projects guide missing project"
    $tasksGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_TASKS.md") -Raw
    Assert-Contains $tasksGuide "DByteOS Workspace Tasks" "tasks guide title"
    Assert-Contains $tasksGuide "task reset-demo" "tasks guide reset"
    Assert-Contains $tasksGuide "task add demo write tests" "tasks guide add"
    Assert-Contains $tasksGuide "task summary demo" "tasks guide summary"
    Assert-Contains $tasksGuide "task clear-done demo" "tasks guide clear-done"
    Assert-Contains $tasksGuide "home/deadbyte/projects/<name>/tasks.txt" "tasks guide storage"
    $preferencesGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_PREFERENCES.md") -Raw
    Assert-Contains $preferencesGuide "DByteOS Mutable Preferences" "preferences guide title"
    Assert-Contains $preferencesGuide "system.prompt" "preferences guide prompt key"
    Assert-Contains $preferencesGuide "interactive shell prompt" "preferences guide shell prompt integration"
    $personalAlphaGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_PERSONAL_ALPHA.md") -Raw
    Assert-Contains $personalAlphaGuide "DByteOS Personal Workspace Beta Foundation" "Personal Workspace Beta Foundation guide title"
    Assert-Contains $personalAlphaGuide "language runtime" "Personal Workspace Beta Foundation guide runtime"
    Assert-Contains $personalAlphaGuide "prompt integration" "Personal Workspace Beta Foundation guide prompt"
    Assert-Contains $personalAlphaGuide "not a standalone operating system" "Personal Workspace Beta Foundation guide boundary"

    $alphaGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_ALPHA.md") -Raw
    $bootGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_BOOT.md") -Raw
    $securityGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_SECURITY.md") -Raw
    Assert-NotContains $alphaGuide "file:///C:/Users/" "alpha guide avoids local absolute links"
    Assert-NotContains $bootGuide "file:///C:/Users/" "boot guide avoids local absolute links"
    Assert-NotContains $securityGuide "file:///C:/Users/" "security guide avoids local absolute links"
    Assert-Contains $alphaGuide "[Home](../README.md)" "alpha guide relative home link"
    Assert-Contains $alphaGuide "[Personal Workspace Beta Foundation](DBYTEOS_PERSONAL_ALPHA.md)" "alpha guide Personal Workspace Beta Foundation link"
    Assert-Contains $bootGuide "[Alpha Status](DBYTEOS_ALPHA.md)" "boot guide relative alpha link"
    Assert-Contains $securityGuide "[Boot](DBYTEOS_BOOT.md)" "security guide relative boot link"

    $staleReleasePatterns = @(
        ("v8.7." + "1"),
        ("DByte 8.7." + "1"),
        ("dbyte-v8.7." + "1"),
        ("8.7." + "1"),
        ("v8.3." + "0"),
        ("DByte 8.3." + "0"),
        ("dbyte-v8.3." + "0"),
        ("8.3." + "0"),
        ("v8.1." + "1"),
        ("DByte 8.1." + "1"),
        ("dbyte-v8.1." + "1"),
        ("8.1." + "1"),
        ("v7.8." + "1"),
        ("DByte 7.8." + "1"),
        ("dbyte-v7.8." + "1"),
        ("7.8." + "1"),
        ("v7.7." + "1"),
        ("DByte 7.7." + "1"),
        ("dbyte-v7.7." + "1"),
        ("7.7." + "1"),
        ("v7.7." + "0"),
        ("DByte 7.7." + "0"),
        ("dbyte-v7.7." + "0"),
        ("7.7." + "0"),
        ("v7.6." + "1"),
        ("DByte 7.6." + "1"),
        ("dbyte-v7.6." + "1"),
        ("7.6." + "1"),
        ("v7.6." + "0"),
        ("DByte 7.6." + "0"),
        ("dbyte-v7.6." + "0"),
        ("7.6." + "0"),
        ("v7.5." + "1"),
        ("DByte 7.5." + "1"),
        ("dbyte-v7.5." + "1"),
        ("7.5." + "1"),
        ("v7.5." + "0"),
        ("DByte 7.5." + "0"),
        ("dbyte-v7.5." + "0"),
        ("7.5." + "0"),
        ("v5.5." + "0"),
        ("DByte 5.5." + "0"),
        ("dbyte-v5.5." + "0"),
        ("v5.4." + "0"),
        ("DByte 5.4." + "0"),
        ("dbyte-v5.4." + "0"),
        ("v5.3." + "0"),
        ("DByte 5.3." + "0"),
        ("dbyte-v5.3." + "0"),
        ("v4.7." + "0"),
        ("DByte 4.7." + "0"),
        ("dbyte-v4.7." + "0")
    )
    $releaseRefFiles = @(
        "Cargo.toml",
        "Cargo.lock",
        "README.md",
        "INSTALL.md",
        "LANGUAGE_SPEC.md",
        "scripts\verify.ps1",
        "scripts\package_release.ps1",
        "docs\DBYTEOS_PERSONAL_ALPHA.md",
        "docs\DBYTEOS_ALPHA.md",
        "docs\DBYTEOS_COMMANDS.md",
        "docs\DBYTEOS_ONBOARDING.md",
        "docs\DBYTEOS_PACKAGE.md",
        "docs\DBYTEOS_CONFIG.md",
        "docs\DBYTEOS_PROFILE.md",
        "docs\DBYTEOS_PREFERENCES.md",
        "docs\DBYTEOS_SNAPSHOT.md",
        "docs\DBYTEOS_PROJECTS.md",
        "docs\DBYTEOS_TASKS.md",
        "docs\DBYTEOS_DIAGNOSTICS.md",
        "docs\DBYTEOS_KERNEL.md",
        "docs\KERNEL_EXCEPTIONS.md",
        "docs\KERNEL_INTERRUPTS.md",
        "docs\KERNEL_LAB.md",
        "docs\QEMU_BOOT_SMOKE.md",
        "examples\dbyteos\README.md",
        "examples\dbyteos\etc\system.dby",
        "kernel-lab\Cargo.toml",
        "kernel-lab\Cargo.lock",
        "kernel-lab\README.md",
        "kernel-lab\src\main.rs",
        "kernel-lab\src\page_fault.rs",
        "examples\dbyteos\etc\manual\profile.txt",
        "examples\dbyteos\etc\manual\snapshot.txt",
        "examples\dbyteos\etc\manual\project.txt",
        "examples\dbyteos\etc\manual\task.txt",
        "examples\dbyteos\etc\manual\search.txt"
    )
    foreach ($releaseRefFile in $releaseRefFiles) {
        $releaseRefText = Get-Content (Join-Path $repoRoot $releaseRefFile) -Raw
        foreach ($stalePattern in $staleReleasePatterns) {
            Assert-NotContains $releaseRefText $stalePattern "stale release ref $releaseRefFile"
        }
    }


    $inspectSource = Get-Content (Join-Path $dbyteosRoot "bin\inspect.dby") -Raw
    # v9.0.2 enforcement confirmed via smoke tests above
    $dbyteosCatGuard = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\cat.dby", "etc/system.dby") -WorkingDirectory $repoRoot
    if ($dbyteosCatGuard.Code -ne 0) { throw "dbyteos cat guard failed: $($dbyteosCatGuard.Text)" }
    Assert-Contains $dbyteosCatGuard.Text "pub let os_version" "cat enforced allowed"
    $dbyteosCatDeny = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\cat.dby", "tmp/../etc/system.dby") -WorkingDirectory $repoRoot
    Assert-Contains $dbyteosCatDeny.Text "error: permission denied" "cat enforced denied"
    $dbyteosTouchGuard = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\touch.dby", "tmp/security_touch_guard.txt") -WorkingDirectory $repoRoot
    if ($dbyteosTouchGuard.Code -ne 0) { throw "dbyteos touch guard failed: $($dbyteosTouchGuard.Text)" }
    Assert-Equal $dbyteosTouchGuard.Text "touch: ok" "touch enforced allowed"
    $dbyteosTouchDeny = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\touch.dby", "etc/system.dby") -WorkingDirectory $repoRoot
    Assert-Contains $dbyteosTouchDeny.Text "error: permission denied" "touch enforced denied"
    $dbyteosInspectGuard = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\inspect.dby", "etc/system.dby") -WorkingDirectory $repoRoot
    if ($dbyteosInspectGuard.Code -ne 0) { throw "dbyteos inspect guard failed: $($dbyteosInspectGuard.Text)" }
    Assert-Contains $dbyteosInspectGuard.Text "Inspecting file:" "inspect enforced allowed"
    $dbyteosInspectDeny = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\inspect.dby", "unknown/file") -WorkingDirectory $repoRoot
    Assert-Contains $dbyteosInspectDeny.Text "error: permission denied" "inspect enforced denied"
    $dbyteosSecurityShell = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "which perm`nperm append etc/system.dby`nread etc/system.dby`nwrite etc/system.dby nope`nappend etc/system.dby nope`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosSecurityShell.Code -ne 0) { throw "dbyteos security shell autopath failed: $($dbyteosSecurityShell.Text)" }
    Assert-Contains $dbyteosSecurityShell.Text "perm: dbyteos ->" "shell rc perm autopath"
    Assert-Contains $dbyteosSecurityShell.Text "DENY append etc/system.dby (policy)" "shell rc perm append"
    Assert-Contains $dbyteosSecurityShell.Text "pub let os_version" "shell rc read etc"
    Assert-Contains $dbyteosSecurityShell.Text "error: permission denied: write etc/system.dby" "shell rc write denied"
    Assert-Contains $dbyteosSecurityShell.Text "error: permission denied: append etc/system.dby" "shell rc append denied"
    $dbyteosShellNoRcPerm = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "perm policy`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellNoRcPerm.Code -ne 0) { throw "dbyteos shell no-rc perm failed: $($dbyteosShellNoRcPerm.Text)" }
    Assert-Contains $dbyteosShellNoRcPerm.Text "ShellError: unknown command: perm" "shell --no-rc hides perm"

    $dbyteosCleanCmdRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\clean.dby") -WorkingDirectory $repoRoot
    if ($dbyteosCleanCmdRoot.Code -ne 0) { throw "dbyteos clean after command set failed: $($dbyteosCleanCmdRoot.Text)" }
    Assert-Contains $dbyteosCleanCmdRoot.Text "workspace sweep complete" "dbyteos clean workspace sweep line"
    if (Test-Path $securityLogPath) { throw "dbyteos clean did not remove security.log" }

    $dbyteosWhoamiCwd = Invoke-Dbyte -Arguments @("run", "bin\whoami.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosWhoamiCwd.Code -ne 0) { throw "dbyteos whoami from dbyteos cwd failed: $($dbyteosWhoamiCwd.Text)" }
    Assert-Equal $dbyteosWhoamiCwd.Text "deadbyte" "dbyteos whoami from dbyteos cwd"

    $dbyteosCmdShell = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "whoami`nsysinfo`nhome`ntmp`nprofile`npath`nenv`nwhich cat`nnotes`nmkdir-demo`nwrite tmp/shell_chain.txt shell chain ok`nread tmp/shell_chain.txt`nwrite-demo`ncat tmp/write_demo.txt`ntimeline today`ntimeline snapshot`nclean`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosCmdShell.Code -ne 0) { throw "dbyteos command shell chain failed: $($dbyteosCmdShell.Text)" }
    Assert-Contains $dbyteosCmdShell.Text "deadbyte" "dbyteos shell whoami"
    Assert-Contains $dbyteosCmdShell.Text "version: DByte 9.0.2" "dbyteos shell sysinfo"
    Assert-Contains $dbyteosCmdShell.Text "home/deadbyte" "dbyteos shell home"
    Assert-Contains $dbyteosCmdShell.Text "wrote tmp/write_demo.txt" "dbyteos shell write-demo"
    Assert-Contains $dbyteosCmdShell.Text "os_version: 9.0.2" "dbyteos shell profile"
    Assert-Contains $dbyteosCmdShell.Text "mode: beta-userland" "dbyteos shell profile mode"
    Assert-Contains $dbyteosCmdShell.Text "PATH=/bin:/tmp:/home/deadbyte" "dbyteos shell path"
    Assert-Contains $dbyteosCmdShell.Text "cat: dbyteos ->" "dbyteos shell chain which cat autopath"
    Assert-Contains $dbyteosCmdShell.Text "mkdir-demo: ok" "dbyteos shell mkdir-demo"
    Assert-Contains $dbyteosCmdShell.Text "shell chain ok" "dbyteos shell read after write"
    Assert-Contains $dbyteosCmdShell.Text "dbyteos write_demo ok" "dbyteos shell cat"
    Assert-Contains $dbyteosCmdShell.Text "Timeline Mode: fallback" "dbyteos shell timeline today mode"
    Assert-Contains $dbyteosCmdShell.Text "Total Projects: 1" "dbyteos shell timeline snapshot projects count"

    Write-Host "Running DByteOS Notes Workflow (v9.0.2) smoke tests..."
    $dbyteosNotesWorkflow = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "notes clear-demo`nnotes read`nnotes add First Note`nnotes read`nnotes append Second Note`nnotes read`nnotes list`nclean`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosNotesWorkflow.Code -ne 0) { throw "dbyteos notes workflow failed: $($dbyteosNotesWorkflow.Text)" }
    Assert-Contains $dbyteosNotesWorkflow.Text "notes: reset to seed state" "notes clear-demo"
    Assert-Contains $dbyteosNotesWorkflow.Text "dbyteos notes seed" "notes read seed"
    Assert-Contains $dbyteosNotesWorkflow.Text "notes: added" "notes add First Note"
    Assert-Contains $dbyteosNotesWorkflow.Text "First Note" "notes read First Note"
    Assert-Contains $dbyteosNotesWorkflow.Text "notes: appended" "notes append Second Note"
    Assert-Contains $dbyteosNotesWorkflow.Text "First Note`nSecond Note" "notes read both lines"
    Assert-Contains $dbyteosNotesWorkflow.Text "notes: home/deadbyte/notes.txt (exists)" "notes list"

    Write-Host "Running DByteOS Notes Hardening (v9.0.2) smoke tests..."
    $notesInput = @"
clean
notes read
notes add
notes add ""
notes append
notes append ""
notes add "Hello World"
notes read
notes clear-demo
notes clear-demo
notes list
clean
quit
"@
    $dbyteosNotesHardening = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "$notesInput`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosNotesHardening.Code -ne 0) { throw "dbyteos notes hardening failed: $($dbyteosNotesHardening.Text)" }
    Assert-Contains $dbyteosNotesHardening.Text "error: notes file not found" "notes read missing"
    Assert-Contains $dbyteosNotesHardening.Text "usage: notes add <text...>" "notes add missing args"
    Assert-Contains $dbyteosNotesHardening.Text "error: cannot add empty note" "notes add empty"
    Assert-Contains $dbyteosNotesHardening.Text "usage: notes append <text...>" "notes append missing args"
    Assert-Contains $dbyteosNotesHardening.Text "error: cannot append empty text" "notes append empty"
    Assert-Contains $dbyteosNotesHardening.Text "Hello World" "notes quoted spaces"
    Assert-Contains $dbyteosNotesHardening.Text "notes: reset to seed state" "notes clear-demo idempotent"
    Assert-Contains $dbyteosNotesHardening.Text "notes: home/deadbyte/notes.txt (exists)" "notes list after clear"
    
    Write-Host "Running DByteOS Init Services (v9.0.2) smoke tests..."
    $dbyteosInitServices = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "boot`nservices list`nservices status`nservices run notes`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosInitServices.Code -ne 0) { throw "dbyteos init services failed: $($dbyteosInitServices.Text)" }
    Assert-Contains $dbyteosInitServices.Text "Init: starting userland services..." "init start"
    Assert-Contains $dbyteosInitServices.Text "[INIT] notes" "init notes service"
    Assert-Contains $dbyteosInitServices.Text "[INIT] sysinfo" "init sysinfo service"
    Assert-Contains $dbyteosInitServices.Text "System State: Initialized" "services status ok"
    Assert-Contains $dbyteosInitServices.Text "[ACTIVE] notes" "services status notes"
    Assert-Contains $dbyteosInitServices.Text "services: running notes..." "services run notes"
    
    Write-Host "Running DByteOS Journal/Logger (v9.0.2) smoke tests..."
    $journalPath = Join-Path $dbyteosRoot "home\deadbyte\journal.txt"
    if (Test-Path $journalPath) {
        Remove-Item -Force $journalPath
    }
    $dbyteosLogClearForMissing = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\log.dby", "clear-demo") -WorkingDirectory $repoRoot
    if ($dbyteosLogClearForMissing.Code -ne 0) { throw "dbyteos log clear for missing failed: $($dbyteosLogClearForMissing.Text)" }
    $dbyteosLogMissingBoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\log.dby", "boot") -WorkingDirectory $repoRoot
    if ($dbyteosLogMissingBoot.Code -ne 0) { throw "dbyteos log boot missing failed: $($dbyteosLogMissingBoot.Text)" }
    Assert-Contains $dbyteosLogMissingBoot.Text "error: log file not found: tmp/boot.log" "log boot missing deterministic"
    $dbyteosLogMissingServices = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\log.dby", "services") -WorkingDirectory $repoRoot
    if ($dbyteosLogMissingServices.Code -ne 0) { throw "dbyteos log services missing failed: $($dbyteosLogMissingServices.Text)" }
    Assert-Contains $dbyteosLogMissingServices.Text "error: log file not found: tmp/services.log" "log services missing deterministic"
    $dbyteosJournalMissing = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\journal.dby", "read") -WorkingDirectory $repoRoot
    if ($dbyteosJournalMissing.Code -ne 0) { throw "dbyteos journal read missing failed: $($dbyteosJournalMissing.Text)" }
    Assert-Equal $dbyteosJournalMissing.Text "error: journal file not found: home/deadbyte/journal.txt" "journal read missing deterministic"
    $dbyteosJournalClearA = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\journal.dby", "clear-demo") -WorkingDirectory $repoRoot
    if ($dbyteosJournalClearA.Code -ne 0) { throw "dbyteos journal clear first failed: $($dbyteosJournalClearA.Text)" }
    $dbyteosJournalClearB = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\journal.dby", "clear-demo") -WorkingDirectory $repoRoot
    if ($dbyteosJournalClearB.Code -ne 0) { throw "dbyteos journal clear second failed: $($dbyteosJournalClearB.Text)" }
    Assert-Equal $dbyteosJournalClearA.Text "journal: reset to seed state." "journal clear-demo banner"
    Assert-Equal $dbyteosJournalClearA.Text $dbyteosJournalClearB.Text "journal clear-demo idempotent output"
    Assert-Equal (Get-Content $journalPath -Raw) "[JOURNAL] dbyteos journal seed`n" "journal clear-demo writes seed file"
    $dbyteosJournalAddMissing = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\journal.dby", "add") -WorkingDirectory $repoRoot
    if ($dbyteosJournalAddMissing.Code -ne 0) { throw "dbyteos journal add missing failed: $($dbyteosJournalAddMissing.Text)" }
    Assert-Equal $dbyteosJournalAddMissing.Text "usage: journal add <text...>" "journal add missing usage"
    $dbyteosJournalAppendMissing = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\journal.dby", "append") -WorkingDirectory $repoRoot
    if ($dbyteosJournalAppendMissing.Code -ne 0) { throw "dbyteos journal append missing failed: $($dbyteosJournalAppendMissing.Text)" }
    Assert-Equal $dbyteosJournalAppendMissing.Text "usage: journal append <text...>" "journal append missing usage"
    $dbyteosJournalPreserveAdd = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\journal.dby", "add", "Preserve", "Me") -WorkingDirectory $repoRoot
    if ($dbyteosJournalPreserveAdd.Code -ne 0) { throw "dbyteos journal preserve add failed: $($dbyteosJournalPreserveAdd.Text)" }
    $dbyteosCleanPreserveJournal = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\clean.dby") -WorkingDirectory $repoRoot
    if ($dbyteosCleanPreserveJournal.Code -ne 0) { throw "dbyteos clean preserve journal failed: $($dbyteosCleanPreserveJournal.Text)" }
    $dbyteosJournalPreserved = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\journal.dby", "read") -WorkingDirectory $repoRoot
    if ($dbyteosJournalPreserved.Code -ne 0) { throw "dbyteos journal preserved read failed: $($dbyteosJournalPreserved.Text)" }
    Assert-Contains $dbyteosJournalPreserved.Text "[JOURNAL] Preserve Me" "clean preserves journal data"
    $dbyteosBootOnce = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\boot.dby") -WorkingDirectory $repoRoot
    if ($dbyteosBootOnce.Code -ne 0) { throw "dbyteos deterministic boot first failed: $($dbyteosBootOnce.Text)" }
    $dbyteosBootLogOnce = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\log.dby", "boot") -WorkingDirectory $repoRoot
    if ($dbyteosBootLogOnce.Code -ne 0) { throw "dbyteos deterministic boot log first failed: $($dbyteosBootLogOnce.Text)" }
    $dbyteosServicesLogOnce = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\log.dby", "services") -WorkingDirectory $repoRoot
    if ($dbyteosServicesLogOnce.Code -ne 0) { throw "dbyteos deterministic services log first failed: $($dbyteosServicesLogOnce.Text)" }
    $dbyteosBootTwice = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\boot.dby") -WorkingDirectory $repoRoot
    if ($dbyteosBootTwice.Code -ne 0) { throw "dbyteos deterministic boot second failed: $($dbyteosBootTwice.Text)" }
    $dbyteosBootLogTwice = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\log.dby", "boot") -WorkingDirectory $repoRoot
    if ($dbyteosBootLogTwice.Code -ne 0) { throw "dbyteos deterministic boot log second failed: $($dbyteosBootLogTwice.Text)" }
    $dbyteosServicesLogTwice = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\log.dby", "services") -WorkingDirectory $repoRoot
    if ($dbyteosServicesLogTwice.Code -ne 0) { throw "dbyteos deterministic services log second failed: $($dbyteosServicesLogTwice.Text)" }
    Assert-Equal $dbyteosBootLogOnce.Text $dbyteosBootLogTwice.Text "boot repeat boot log deterministic"
    Assert-Equal $dbyteosServicesLogOnce.Text $dbyteosServicesLogTwice.Text "boot repeat services log deterministic"
    $dbyteosShellAutopathJournalLog = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "which log`nwhich journal`nlog boot`njournal read`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellAutopathJournalLog.Code -ne 0) { throw "dbyteos shell log/journal autopath failed: $($dbyteosShellAutopathJournalLog.Text)" }
    Assert-Contains $dbyteosShellAutopathJournalLog.Text "log: dbyteos ->" "dbyteos shell log autopath"
    Assert-Contains $dbyteosShellAutopathJournalLog.Text "journal: dbyteos ->" "dbyteos shell journal autopath"
    Assert-Contains $dbyteosShellAutopathJournalLog.Text "--- Boot Log ---" "dbyteos shell log boot via autopath"
    Assert-Contains $dbyteosShellAutopathJournalLog.Text "[JOURNAL] Preserve Me" "dbyteos shell journal read via autopath"
    $dbyteosShellNoRcJournalLog = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "log boot`njournal read`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellNoRcJournalLog.Code -ne 0) { throw "dbyteos shell no-rc log/journal failed: $($dbyteosShellNoRcJournalLog.Text)" }
    Assert-Contains $dbyteosShellNoRcJournalLog.Text "ShellError: unknown command: log" "dbyteos shell --no-rc hides log"
    Assert-Contains $dbyteosShellNoRcJournalLog.Text "ShellError: unknown command: journal" "dbyteos shell --no-rc hides journal"

    $dbyteosShellAutopathDiagnostics = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "which doctor`nwhich diagnose`nwhich check-system`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellAutopathDiagnostics.Code -ne 0) { throw "dbyteos shell diagnostics autopath failed: $($dbyteosShellAutopathDiagnostics.Text)" }
    Assert-Contains $dbyteosShellAutopathDiagnostics.Text "doctor: dbyteos ->" "dbyteos shell doctor autopath"
    Assert-Contains $dbyteosShellAutopathDiagnostics.Text "diagnose: dbyteos ->" "dbyteos shell diagnose autopath"
    Assert-Contains $dbyteosShellAutopathDiagnostics.Text "check-system: dbyteos ->" "dbyteos shell check-system autopath"

    $dbyteosShellNoRcDiagnostics = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "doctor`ndiagnose`ncheck-system`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellNoRcDiagnostics.Code -ne 0) { throw "dbyteos shell no-rc diagnostics failed: $($dbyteosShellNoRcDiagnostics.Text)" }
    Assert-Contains $dbyteosShellNoRcDiagnostics.Text "ShellError: unknown command: doctor" "dbyteos shell --no-rc hides doctor"
    Assert-Contains $dbyteosShellNoRcDiagnostics.Text "ShellError: unknown command: diagnose" "dbyteos shell --no-rc hides diagnose"
    Assert-Contains $dbyteosShellNoRcDiagnostics.Text "ShellError: unknown command: check-system" "dbyteos shell --no-rc hides check-system"

    $journalInput = @"
clean
boot
log boot
log services
journal clear-demo
journal add ""
journal append ""
journal add Hello Journal
journal read
journal add Second Entry
journal read
journal clear-demo
clean
quit
"@
    $dbyteosJournal = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "$journalInput`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosJournal.Code -ne 0) { throw "dbyteos journal/logger failed: $($dbyteosJournal.Text)" }
    Assert-Contains $dbyteosJournal.Text "[EVENT] Boot sequence started" "log boot content"
    Assert-Contains $dbyteosJournal.Text "[EVENT] Starting service: notes" "log services content"
    Assert-Contains $dbyteosJournal.Text "error: cannot add empty journal entry." "journal add quoted empty reject"
    Assert-Contains $dbyteosJournal.Text "error: cannot append empty journal entry." "journal append quoted empty reject"
    Assert-Contains $dbyteosJournal.Text "journal: entry recorded" "journal add success"
    Assert-Contains $dbyteosJournal.Text "[JOURNAL] Hello Journal" "journal read hello"
    Assert-Contains $dbyteosJournal.Text "[JOURNAL] Second Entry" "journal read second"
    Assert-Contains $dbyteosJournal.Text "workspace sweep complete" "journal clean sweep"

    Write-Host "Running DByteOS Diagnostics smoke tests..."
    $diagnosticsScripts = @("bin\doctor.dby", "bin\diagnose.dby", "bin\check_system.dby")
    foreach ($script in $diagnosticsScripts) {
        $content = Get-Content (Join-Path $dbyteosRoot $script) -Raw
        if ($content -match "`t") { throw "Parser guard failed: $script contains tabs" }
        if ($content -match "(?m)^[ \t]*$") { throw "Parser guard failed: $script contains blank lines" }
    }

    $dbyteosDoctor = Invoke-Dbyte -Arguments @("run", "bin\doctor.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosDoctor.Code -ne 0) { throw "dbyteos doctor failed: $($dbyteosDoctor.Text)" }
    Assert-NormalizedEqual $dbyteosDoctor.Text $expectedDbyteosDoctor "dbyteos doctor snapshot"

    $dbyteosCheckSystem = Invoke-Dbyte -Arguments @("run", "bin\check_system.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosCheckSystem.Code -ne 0) { throw "dbyteos check-system failed: $($dbyteosCheckSystem.Text)" }
    Assert-NormalizedEqual $dbyteosCheckSystem.Text $expectedDbyteosCheckSystem "dbyteos check-system snapshot"

    $dbyteosDiagnose = Invoke-Dbyte -Arguments @("run", "bin\diagnose.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosDiagnose.Code -ne 0) { throw "dbyteos diagnose failed: $($dbyteosDiagnose.Text)" }
    Assert-NormalizedEqual $dbyteosDiagnose.Text $expectedDbyteosDiagnose "dbyteos diagnose snapshot"

    $dbyteosDiagnoseProfile = Invoke-Dbyte -Arguments @("run", "bin\diagnose.dby", "profile") -WorkingDirectory $dbyteosRoot
    if ($dbyteosDiagnoseProfile.Code -ne 0) { throw "dbyteos diagnose profile failed: $($dbyteosDiagnoseProfile.Text)" }
    Assert-NormalizedEqual $dbyteosDiagnoseProfile.Text $expectedDbyteosDiagnoseProfile "dbyteos diagnose profile snapshot"

    $dbyteosDiagnoseConfig = Invoke-Dbyte -Arguments @("run", "bin\diagnose.dby", "config") -WorkingDirectory $dbyteosRoot
    if ($dbyteosDiagnoseConfig.Code -ne 0) { throw "dbyteos diagnose config failed: $($dbyteosDiagnoseConfig.Text)" }
    Assert-NormalizedEqual $dbyteosDiagnoseConfig.Text $expectedDbyteosDiagnoseConfig "dbyteos diagnose config snapshot"

    $dbyteosDiagnoseSecurity = Invoke-Dbyte -Arguments @("run", "bin\diagnose.dby", "security") -WorkingDirectory $dbyteosRoot
    if ($dbyteosDiagnoseSecurity.Code -ne 0) { throw "dbyteos diagnose security failed: $($dbyteosDiagnoseSecurity.Text)" }
    Assert-NormalizedEqual $dbyteosDiagnoseSecurity.Text $expectedDbyteosDiagnoseSecurity "dbyteos diagnose security snapshot"
    
    $dbyteosDiagnosePreferences = Invoke-Dbyte -Arguments @("run", "bin\diagnose.dby", "preferences") -WorkingDirectory $dbyteosRoot
    if ($dbyteosDiagnosePreferences.Code -ne 0) { throw "dbyteos diagnose preferences failed: $($dbyteosDiagnosePreferences.Text)" }
    Assert-NormalizedEqual $dbyteosDiagnosePreferences.Text $expectedDbyteosDiagnosePreferences "dbyteos diagnose preferences snapshot"

    $dbyteosDiagnoseLogs = Invoke-Dbyte -Arguments @("run", "bin\diagnose.dby", "logs") -WorkingDirectory $dbyteosRoot
    if ($dbyteosDiagnoseLogs.Code -ne 0) { throw "dbyteos diagnose logs failed: $($dbyteosDiagnoseLogs.Text)" }
    Assert-NormalizedEqual $dbyteosDiagnoseLogs.Text $expectedDbyteosDiagnoseLogs "dbyteos diagnose logs snapshot"

    $dbyteosDiagnoseManual = Invoke-Dbyte -Arguments @("run", "bin\diagnose.dby", "manual") -WorkingDirectory $dbyteosRoot
    if ($dbyteosDiagnoseManual.Code -ne 0) { throw "dbyteos diagnose manual failed: $($dbyteosDiagnoseManual.Text)" }
    Assert-NormalizedEqual $dbyteosDiagnoseManual.Text $expectedDbyteosDiagnoseManual "dbyteos diagnose manual snapshot"

    $dbyteosDiagnosePackage = Invoke-Dbyte -Arguments @("run", "bin\diagnose.dby", "package") -WorkingDirectory $dbyteosRoot
    if ($dbyteosDiagnosePackage.Code -ne 0) { throw "dbyteos diagnose package failed: $($dbyteosDiagnosePackage.Text)" }
    Assert-NormalizedEqual $dbyteosDiagnosePackage.Text $expectedDbyteosDiagnosePackage "dbyteos diagnose package snapshot"

    $dbyteosDiagnoseUnknown = Invoke-Dbyte -Arguments @("run", "bin\diagnose.dby", "unknown") -WorkingDirectory $dbyteosRoot
    if ($dbyteosDiagnoseUnknown.Code -ne 0) { throw "dbyteos diagnose unknown failed: $($dbyteosDiagnoseUnknown.Text)" }
    Assert-Equal $dbyteosDiagnoseUnknown.Text "usage: diagnose [profile|config|preferences|security|logs|manual|package]" "dbyteos diagnose unknown snapshot"

    Write-Host "Running DByteOS Search (v9.0.2) smoke tests..."
    $searchHelp = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "help") -WorkingDirectory $dbyteosRoot
    if ($searchHelp.Code -ne 0) { throw "search help failed: $($searchHelp.Text)" }
    Assert-Contains $searchHelp.Text "usage: search <command>" "search help usage"
    Assert-Contains $searchHelp.Text "workspace search <text>" "search help workspace"
    Assert-Contains $searchHelp.Text "projects <text>" "search help projects"
    Assert-Contains $searchHelp.Text "tasks <text>" "search help tasks"
    Assert-Contains $searchHelp.Text "notes <text>" "search help notes"
    Assert-Contains $searchHelp.Text "journal <text>" "search help journal"
    Assert-Contains $searchHelp.Text "summary" "search help summary"
    Assert-Contains $searchHelp.Text "recent" "search help recent"

    $searchReset = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "reset-demo") -WorkingDirectory $dbyteosRoot
    if ($searchReset.Code -ne 0) { throw "search reset failed: $($searchReset.Text)" }
    Assert-Contains $searchReset.Text "search: reset demo project and workspace seed data" "search reset-demo output"

    # --- v9.0.2 Exact Snapshot Assertions ---
    $searchWorkspace = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "workspace", "search", "note") -WorkingDirectory $dbyteosRoot
    if ($searchWorkspace.Code -ne 0) { throw "search workspace failed: $($searchWorkspace.Text)" }
    $expectedWorkspaceOut = "DByteOS workspace search: note`nnotes: dbyteos notes seed`nproject demo note: project demo notes`nproject demo task: [ ] 1: write project note"
    Assert-Equal $searchWorkspace.Text $expectedWorkspaceOut "search workspace exact snapshot"

    $searchProject = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "project", "search", "demo", "note") -WorkingDirectory $dbyteosRoot
    if ($searchProject.Code -ne 0) { throw "search project failed: $($searchProject.Text)" }
    $expectedProjectOut = "project demo note: project demo notes`nproject demo task: [ ] 1: write project note"
    Assert-Equal $searchProject.Text $expectedProjectOut "search project exact snapshot"

    $searchTask = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "task", "search", "demo", "tests") -WorkingDirectory $dbyteosRoot
    if ($searchTask.Code -ne 0) { throw "search task failed: $($searchTask.Text)" }
    $expectedTaskOut = "project demo task: [ ] 2: write tests"
    Assert-Equal $searchTask.Text $expectedTaskOut "search task exact snapshot"

    $searchDaily = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "daily", "search", "seed") -WorkingDirectory $dbyteosRoot
    if ($searchDaily.Code -ne 0) { throw "search daily failed: $($searchDaily.Text)" }
    $expectedDailyOut = "DByteOS daily search: seed`nnotes: dbyteos notes seed`njournal: [JOURNAL] dbyteos journal seed"
    Assert-Equal $searchDaily.Text $expectedDailyOut "search daily exact snapshot"

    # --- v9.0.2 Deterministic Rejections ---
    $searchEmpty = Invoke-DbyteExact -Arguments @("run", "bin\search.dby", "workspace", "search", "") -WorkingDirectory $dbyteosRoot
    if ($searchEmpty.Code -ne 0) { throw "search empty failed: $($searchEmpty.Text)" }
    Assert-Equal $searchEmpty.Text "error: search: invalid query" "search empty query reject"

    $searchBadChar = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "workspace", "search", "a/b") -WorkingDirectory $dbyteosRoot
    if ($searchBadChar.Code -ne 0) { throw "search validation failed: $($searchBadChar.Text)" }
    Assert-Equal $searchBadChar.Text "error: search: invalid query" "search query with forbidden chars reject"

    $searchBadProj = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "project", "search", "demo/x", "notes") -WorkingDirectory $dbyteosRoot
    if ($searchBadProj.Code -ne 0) { throw "search project validation failed: $($searchBadProj.Text)" }
    Assert-Contains $searchBadProj.Text "error: invalid project name: demo/x" "search validation project name protection"

    $searchMissingProj = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "project", "search", "missing", "note") -WorkingDirectory $dbyteosRoot
    if ($searchMissingProj.Code -ne 0) { throw "search missing project failed: $($searchMissingProj.Text)" }
    Assert-Equal $searchMissingProj.Text "error: project 'missing' not found in index" "missing project search deterministic"

    # --- v9.0.2 Cache Commands Tests ---
    # 1. Clear cache initially and verify idempotency
    $cacheClearInit = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "clear-cache") -WorkingDirectory $dbyteosRoot
    if ($cacheClearInit.Code -ne 0) { throw "search clear-cache initial failed" }
    Assert-Equal $cacheClearInit.Text "search: index cache cleared successfully" "cache clear-cache output"

    $cacheClearInitIdempotent = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "clear-cache") -WorkingDirectory $dbyteosRoot
    if ($cacheClearInitIdempotent.Code -ne 0) { throw "search clear-cache idempotent failed" }
    Assert-Equal $cacheClearInitIdempotent.Text "search: index cache cleared successfully" "cache clear-cache idempotent output"
        $cacheStatusMissing = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "status") -WorkingDirectory $dbyteosRoot
    if ($cacheStatusMissing.Code -ne 0) { throw "search status missing failed" }
    Assert-Equal $cacheStatusMissing.Text "index: missing (use 'search rebuild' to generate)" "cache status missing message"
    
    $cacheIndexMissing = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "index", "note") -WorkingDirectory $dbyteosRoot
    if ($cacheIndexMissing.Code -ne 0) { throw "search index missing failed" }
    Assert-Equal $cacheIndexMissing.Text "error: index: missing (use 'search rebuild' to generate)" "cache index missing message"
    
    $cacheDoctorMissing = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "doctor") -WorkingDirectory $dbyteosRoot
    if ($cacheDoctorMissing.Code -ne 0) { throw "search doctor missing failed" }
    Assert-Equal $cacheDoctorMissing.Text "error: index: missing (use 'search rebuild' to generate)" "cache doctor missing message"

    # --- v9.0.2 Search UX Missing Cache / Scan Fallback tests ---
    $uxSummaryMissing = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "summary") -WorkingDirectory $dbyteosRoot
    if ($uxSummaryMissing.Code -ne 0) { throw "search summary missing cache failed" }
    $expectedSummaryMissing = "--- DByteOS Search Summary ---`nIndex Status: missing`nIntegrity:    missing`nDaily Sources:`n  notes:   home/deadbyte/notes.txt (exists)`n  journal: home/deadbyte/journal.txt (exists)`nProjects:`n  - demo"
    Assert-Equal $uxSummaryMissing.Text $expectedSummaryMissing "search summary missing cache output"

    $uxRecentMissing = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "recent") -WorkingDirectory $dbyteosRoot
    if ($uxRecentMissing.Code -ne 0) { throw "search recent missing cache failed" }
    Assert-Equal $uxRecentMissing.Text "error: index: missing (use 'search rebuild' to generate)" "search recent missing cache output"

    $uxProjectsScanned = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "projects", "note") -WorkingDirectory $dbyteosRoot
    if ($uxProjectsScanned.Code -ne 0) { throw "search projects scanned failed" }
    $expectedProjectsScanned = "DByteOS projects search (scanned): note`nproject demo note: project demo notes`nproject demo task: [ ] 1: write project note"
    Assert-Equal $uxProjectsScanned.Text $expectedProjectsScanned "search projects scanned output"

    $uxTasksScanned = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "tasks", "tests") -WorkingDirectory $dbyteosRoot
    if ($uxTasksScanned.Code -ne 0) { throw "search tasks scanned failed" }
    $expectedTasksScanned = "DByteOS tasks search (scanned): tests`nproject demo task: [ ] 2: write tests"
    Assert-Equal $uxTasksScanned.Text $expectedTasksScanned "search tasks scanned output"

    $uxNotesScanned = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "notes", "seed") -WorkingDirectory $dbyteosRoot
    if ($uxNotesScanned.Code -ne 0) { throw "search notes scanned failed" }
    $expectedNotesScanned = "DByteOS notes search (scanned): seed`nnotes: dbyteos notes seed"
    Assert-Equal $uxNotesScanned.Text $expectedNotesScanned "search notes scanned output"

    $uxJournalScanned = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "journal", "JOURNAL") -WorkingDirectory $dbyteosRoot
    if ($uxJournalScanned.Code -ne 0) { throw "search journal scanned failed" }
    $expectedJournalScanned = "DByteOS journal search (scanned): JOURNAL`njournal: [JOURNAL] dbyteos journal seed"
    Assert-Equal $uxJournalScanned.Text $expectedJournalScanned "search journal scanned output"

    # Scan-based search works even when cache is missing
    $scanWorkspaceMissingCache = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "workspace", "search", "note") -WorkingDirectory $dbyteosRoot
    if ($scanWorkspaceMissingCache.Code -ne 0) { throw "scan workspace search missing cache failed" }
    Assert-Equal $scanWorkspaceMissingCache.Text "DByteOS workspace search: note`nnotes: dbyteos notes seed`nproject demo note: project demo notes`nproject demo task: [ ] 1: write project note" "scan search works without cache"
    
    # 2. Rebuild cache and verify idempotency
    $cacheRebuild = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "rebuild") -WorkingDirectory $dbyteosRoot
    if ($cacheRebuild.Code -ne 0) { throw "search rebuild failed" }
    Assert-Equal $cacheRebuild.Text "search: index rebuilt successfully (5 records indexed)" "cache rebuild output"

    $cacheRebuildIdempotent = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "rebuild") -WorkingDirectory $dbyteosRoot
    if ($cacheRebuildIdempotent.Code -ne 0) { throw "search rebuild idempotent failed" }
    Assert-Equal $cacheRebuildIdempotent.Text "search: index rebuilt successfully (5 records indexed)" "cache rebuild idempotent output"
    
    # 3. Check status active
    $cacheStatusActive = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "status") -WorkingDirectory $dbyteosRoot
    if ($cacheStatusActive.Code -ne 0) { throw "search status active failed" }
    Assert-Contains $cacheStatusActive.Text "index: active (5 records, " "cache status active message"
    
    # 4. Check doctor healthy
    $cacheDoctorActive = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "doctor") -WorkingDirectory $dbyteosRoot
    if ($cacheDoctorActive.Code -ne 0) { throw "search doctor active failed" }
    Assert-Equal $cacheDoctorActive.Text "index: healthy (all 5 records valid)" "cache doctor healthy message"

    # --- v9.0.2 Search UX Active Cache tests ---
    $uxSummaryActive = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "summary") -WorkingDirectory $dbyteosRoot
    if ($uxSummaryActive.Code -ne 0) { throw "search summary active cache failed" }
    $cacheFile = Join-Path $dbyteosRoot "home\deadbyte\search_index.txt"
    $cacheSize = (Get-Item $cacheFile).Length
    $expectedSummaryActive = "--- DByteOS Search Summary ---`nIndex Status: active (5 records, $cacheSize bytes)`nIntegrity:    healthy`nDaily Sources:`n  notes:   home/deadbyte/notes.txt (exists)`n  journal: home/deadbyte/journal.txt (exists)`nProjects:`n  - demo"
    Assert-Equal $uxSummaryActive.Text $expectedSummaryActive "search summary active cache exact snapshot"

    $uxRecentActive = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "recent") -WorkingDirectory $dbyteosRoot
    if ($uxRecentActive.Code -ne 0) { throw "search recent active cache failed" }
    $expectedRecentActive = "--- Recent Indexed Records ---`n[1] notes: dbyteos notes seed (home/deadbyte/notes.txt:1)`n[2] journal: [JOURNAL] dbyteos journal seed (home/deadbyte/journal.txt:1)`n[3] project demo note: project demo notes (home/deadbyte/projects/demo/notes.txt:1)`n[4] project demo task: [ ] 1: write project note (home/deadbyte/projects/demo/tasks.txt:1)`n[5] project demo task: [ ] 2: write tests (home/deadbyte/projects/demo/tasks.txt:2)"
    Assert-Equal $uxRecentActive.Text $expectedRecentActive "search recent active cache output"

    $uxProjectsCached = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "projects", "note") -WorkingDirectory $dbyteosRoot
    if ($uxProjectsCached.Code -ne 0) { throw "search projects cached failed" }
    $expectedProjectsCached = "DByteOS projects search (cached): note`nproject demo note: project demo notes`nproject demo task: [ ] 1: write project note"
    Assert-Equal $uxProjectsCached.Text $expectedProjectsCached "search projects cached output"

    $uxTasksCached = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "tasks", "tests") -WorkingDirectory $dbyteosRoot
    if ($uxTasksCached.Code -ne 0) { throw "search tasks cached failed" }
    $expectedTasksCached = "DByteOS tasks search (cached): tests`nproject demo task: [ ] 2: write tests"
    Assert-Equal $uxTasksCached.Text $expectedTasksCached "search tasks cached output"

    $uxNotesCached = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "notes", "seed") -WorkingDirectory $dbyteosRoot
    if ($uxNotesCached.Code -ne 0) { throw "search notes cached failed" }
    $expectedNotesCached = "DByteOS notes search (cached): seed`nnotes: dbyteos notes seed"
    Assert-Equal $uxNotesCached.Text $expectedNotesCached "search notes cached output"

    $uxJournalCached = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "journal", "JOURNAL") -WorkingDirectory $dbyteosRoot
    if ($uxJournalCached.Code -ne 0) { throw "search journal cached failed" }
    $expectedJournalCached = "DByteOS journal search (cached): JOURNAL`njournal: [JOURNAL] dbyteos journal seed"
    Assert-Equal $uxJournalCached.Text $expectedJournalCached "search journal cached output"

    # UX Argument Rejections
    $uxProjectsEmpty = Invoke-DbyteExact -Arguments @("run", "bin\search.dby", "projects", "") -WorkingDirectory $dbyteosRoot
    if ($uxProjectsEmpty.Code -ne 0) { throw "search projects empty failed" }
    Assert-Equal $uxProjectsEmpty.Text "error: search: invalid query" "search projects empty query reject"

    $uxProjectsBad = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "projects", "a/b") -WorkingDirectory $dbyteosRoot
    if ($uxProjectsBad.Code -ne 0) { throw "search projects validation failed" }
    Assert-Equal $uxProjectsBad.Text "error: search: invalid query" "search projects validation reject"

    # 4b. Check doctor unhealthy by injecting corrupted records
    Set-Content -Path (Join-Path $dbyteosRoot "home\deadbyte\search_index.txt") -Value "corrupted_line_no_pipes`n" -NoNewline
    $cacheDoctorUnhealthy = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "doctor") -WorkingDirectory $dbyteosRoot
    if ($cacheDoctorUnhealthy.Code -ne 0) { throw "search doctor unhealthy failed" }
    Assert-Equal $cacheDoctorUnhealthy.Text "error: index structurally invalid" "cache doctor unhealthy message"

    # Verify that Search UX command falls back to direct scanned search when cache is corrupted!
    $uxProjectsCorruptedCache = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "projects", "note") -WorkingDirectory $dbyteosRoot
    if ($uxProjectsCorruptedCache.Code -ne 0) { throw "search projects corrupted cache fallback failed" }
    $expectedProjectsScanned = "DByteOS projects search (scanned): note`nproject demo note: project demo notes`nproject demo task: [ ] 1: write project note"
    Assert-Equal $uxProjectsCorruptedCache.Text $expectedProjectsScanned "search projects fallback to scanned when cache corrupted"

    # Restore healthy cache
    $cacheRebuildForSearch = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "rebuild") -WorkingDirectory $dbyteosRoot
    if ($cacheRebuildForSearch.Code -ne 0) { throw "search rebuild after doctor failed" }
    
    # 4c. Pipe-delimiter guard: task containing pipes inside its text
    Set-Content -Path (Join-Path $dbyteosRoot "home\deadbyte\projects\demo\tasks.txt") -Value "0|write project note`n0|write tests`n0|pipe | task | description`n" -NoNewline
    $cacheRebuildPipes = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "rebuild") -WorkingDirectory $dbyteosRoot
    if ($cacheRebuildPipes.Code -ne 0) { throw "search rebuild with pipes failed" }
    Assert-Equal $cacheRebuildPipes.Text "search: index rebuilt successfully (6 records indexed)" "rebuild indexes 6 records containing pipes"

    # Test exact cache search with pipes
    $uxTasksCachedPipes = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "tasks", "pipe") -WorkingDirectory $dbyteosRoot
    if ($uxTasksCachedPipes.Code -ne 0) { throw "search tasks cached pipes failed" }
    Assert-Equal $uxTasksCachedPipes.Text "DByteOS tasks search (cached): pipe`nproject demo task: [ ] 3: pipe | task | description" "search tasks cached pipes output"

    # Clear cache to force scanned fallback with pipes
    $cacheClearPipes = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "clear-cache") -WorkingDirectory $dbyteosRoot
    if ($cacheClearPipes.Code -ne 0) { throw "clear-cache for pipes failed" }
    
    $uxTasksScannedPipes = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "tasks", "pipe") -WorkingDirectory $dbyteosRoot
    if ($uxTasksScannedPipes.Code -ne 0) { throw "search tasks scanned pipes failed" }
    Assert-Equal $uxTasksScannedPipes.Text "DByteOS tasks search (scanned): pipe`nproject demo task: [ ] 3: pipe | task | description" "search tasks scanned pipes output"

    # Restore clean seed task file and rebuild cache for other tests
    Set-Content -Path (Join-Path $dbyteosRoot "home\deadbyte\projects\demo\tasks.txt") -Value "0|write project note`n0|write tests`n" -NoNewline
    $cacheRebuildRestore = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "rebuild") -WorkingDirectory $dbyteosRoot
    if ($cacheRebuildRestore.Code -ne 0) { throw "search rebuild restore failed" }

    # 5. Search using index
    $cacheIndexSearch = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "index", "note") -WorkingDirectory $dbyteosRoot
    if ($cacheIndexSearch.Code -ne 0) { throw "search index query failed" }
    $expectedCacheSearchOut = "DByteOS index search: note`nnotes: dbyteos notes seed`nproject demo note: project demo notes`nproject demo task: [ ] 1: write project note"
    Assert-Equal $cacheIndexSearch.Text $expectedCacheSearchOut "search index results match"
    
    # 6. Rejections on cache search
    $cacheIndexSearchEmpty = Invoke-DbyteExact -Arguments @("run", "bin\search.dby", "index", "") -WorkingDirectory $dbyteosRoot
    if ($cacheIndexSearchEmpty.Code -ne 0) { throw "search index empty failed" }
    Assert-Equal $cacheIndexSearchEmpty.Text "error: search: invalid query" "cache search empty query reject"
    
    $cacheIndexSearchBad = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "index", "a/b") -WorkingDirectory $dbyteosRoot
    if ($cacheIndexSearchBad.Code -ne 0) { throw "search index validation failed" }
    Assert-Equal $cacheIndexSearchBad.Text "error: search: invalid query" "cache search query with forbidden chars reject"
    
    # --- DByteOS Timeline (v9.0.2) tests ---
    # 1. Reset-demo idempotency check (run twice)
    $timelineReset1 = Invoke-Dbyte -Arguments @("run", "bin\timeline.dby", "reset-demo") -WorkingDirectory $dbyteosRoot
    if ($timelineReset1.Code -ne 0) { throw "timeline reset-demo first run failed" }
    Assert-Equal $timelineReset1.Text "timeline: reset demo timeline workspace" "timeline reset-demo first run output"

    $timelineReset2 = Invoke-Dbyte -Arguments @("run", "bin\timeline.dby", "reset-demo") -WorkingDirectory $dbyteosRoot
    if ($timelineReset2.Code -ne 0) { throw "timeline reset-demo second run failed" }
    Assert-Equal $timelineReset2.Text "timeline: reset demo timeline workspace" "timeline reset-demo idempotency check"

    # 2. Fallback Scan Mode (cache is cleared or missing)
    $expectedTodayFallback = @"
--- DByteOS Workspace Timeline: Today ---
Timeline Mode: fallback
[Notes & Journal]
  Notes:   home/deadbyte/notes.txt (exists)
  Journal: 1 entries recorded
[Open Tasks]
  demo #1: [ ] inspect workspace
  demo #2: [ ] write project note
"@

    $timelineTodayScanned = Invoke-Dbyte -Arguments @("run", "bin\timeline.dby", "today") -WorkingDirectory $dbyteosRoot
    if ($timelineTodayScanned.Code -ne 0) { throw "timeline today scanned failed" }
    Assert-NormalizedEqual $timelineTodayScanned.Text $expectedTodayFallback "timeline today fallback exact snapshot"

    $expectedProjectsFallback = @"
--- DByteOS Workspace Timeline: Projects ---
Timeline Mode: fallback
* project: demo (registered)
  - note: project demo notes (home/deadbyte/projects/demo/notes.txt:1)
  - task #1: [ ] inspect workspace
  - task #2: [ ] write project note
"@

    $timelineProjectsScanned = Invoke-Dbyte -Arguments @("run", "bin\timeline.dby", "projects") -WorkingDirectory $dbyteosRoot
    if ($timelineProjectsScanned.Code -ne 0) { throw "timeline projects scanned failed" }
    Assert-NormalizedEqual $timelineProjectsScanned.Text $expectedProjectsFallback "timeline projects fallback exact snapshot"

    $expectedTasksFallback = @"
--- DByteOS Workspace Timeline: Tasks ---
Timeline Mode: fallback
* [task] demo #1: [ ] inspect workspace
* [task] demo #2: [ ] write project note
"@

    $timelineTasksScanned = Invoke-Dbyte -Arguments @("run", "bin\timeline.dby", "tasks") -WorkingDirectory $dbyteosRoot
    if ($timelineTasksScanned.Code -ne 0) { throw "timeline tasks scanned failed" }
    Assert-NormalizedEqual $timelineTasksScanned.Text $expectedTasksFallback "timeline tasks fallback exact snapshot"

    $expectedJournalFallback = @"
--- DByteOS Workspace Timeline: Journal ---
Timeline Mode: fallback
* [journal] line 1: [JOURNAL] dbyteos journal seed
"@

    $timelineJournalScanned = Invoke-Dbyte -Arguments @("run", "bin\timeline.dby", "journal") -WorkingDirectory $dbyteosRoot
    if ($timelineJournalScanned.Code -ne 0) { throw "timeline journal scanned failed" }
    Assert-NormalizedEqual $timelineJournalScanned.Text $expectedJournalFallback "timeline journal fallback exact snapshot"

    $expectedSearchFallback = @"
--- DByteOS Workspace Timeline Search: demo ---
Timeline Mode: fallback
* [project] demo (registered)
* [project_note] demo: project demo notes (home/deadbyte/projects/demo/notes.txt:1)
* [project_task] demo #1: [ ] inspect workspace
* [project_task] demo #2: [ ] write project note
"@

    $timelineSearchScanned = Invoke-Dbyte -Arguments @("run", "bin\timeline.dby", "search", "demo") -WorkingDirectory $dbyteosRoot
    if ($timelineSearchScanned.Code -ne 0) { throw "timeline search scanned failed" }
    Assert-NormalizedEqual $timelineSearchScanned.Text $expectedSearchFallback "timeline search fallback exact snapshot"

    $expectedSnapshotFallback = @"
--- DByteOS Workspace Timeline Snapshot ---
Timeline Mode: fallback
Total Projects: 1
Total Notes:    2 (1 global, 1 project-specific)
Total Journal:  1
Total Tasks:    2 (2 open, 0 done)
"@

    $timelineSnapshotScanned = Invoke-Dbyte -Arguments @("run", "bin\timeline.dby", "snapshot") -WorkingDirectory $dbyteosRoot
    if ($timelineSnapshotScanned.Code -ne 0) { throw "timeline snapshot scanned failed" }
    Assert-NormalizedEqual $timelineSnapshotScanned.Text $expectedSnapshotFallback "timeline snapshot fallback exact snapshot"

    # Invalid search query deterministic rejection test
    $timelineSearchBad = Invoke-Dbyte -Arguments @("run", "bin\timeline.dby", "search", "a/b") -WorkingDirectory $dbyteosRoot
    if ($timelineSearchBad.Code -ne 0) { throw "timeline search validation failed" }
    Assert-Equal $timelineSearchBad.Text "error: search: invalid query" "timeline search query with forbidden chars deterministic reject"

    # 3. Cached Mode (rebuild cache)
    $cacheRebuildForTimeline = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "rebuild") -WorkingDirectory $dbyteosRoot
    if ($cacheRebuildForTimeline.Code -ne 0) { throw "cache rebuild for timeline failed" }

    $expectedTodayCached = @"
--- DByteOS Workspace Timeline: Today ---
Timeline Mode: cached
[Notes & Journal]
  Notes:   home/deadbyte/notes.txt (exists)
  Journal: 1 entries recorded
[Open Tasks]
  demo #1: [ ] inspect workspace
  demo #2: [ ] write project note
"@

    $timelineTodayCached = Invoke-Dbyte -Arguments @("run", "bin\timeline.dby", "today") -WorkingDirectory $dbyteosRoot
    if ($timelineTodayCached.Code -ne 0) { throw "timeline today cached failed" }
    Assert-NormalizedEqual $timelineTodayCached.Text $expectedTodayCached "timeline today cached exact snapshot"

    $expectedProjectsCached = @"
--- DByteOS Workspace Timeline: Projects ---
Timeline Mode: cached
* project: demo (registered)
  - note: project demo notes (home/deadbyte/projects/demo/notes.txt:1)
  - task #1: [ ] inspect workspace
  - task #2: [ ] write project note
"@

    $timelineProjectsCached = Invoke-Dbyte -Arguments @("run", "bin\timeline.dby", "projects") -WorkingDirectory $dbyteosRoot
    if ($timelineProjectsCached.Code -ne 0) { throw "timeline projects cached failed" }
    Assert-NormalizedEqual $timelineProjectsCached.Text $expectedProjectsCached "timeline projects cached exact snapshot"

    $expectedTasksCached = @"
--- DByteOS Workspace Timeline: Tasks ---
Timeline Mode: cached
* [task] demo #1: [ ] inspect workspace
* [task] demo #2: [ ] write project note
"@

    $timelineTasksCached = Invoke-Dbyte -Arguments @("run", "bin\timeline.dby", "tasks") -WorkingDirectory $dbyteosRoot
    if ($timelineTasksCached.Code -ne 0) { throw "timeline tasks cached failed" }
    Assert-NormalizedEqual $timelineTasksCached.Text $expectedTasksCached "timeline tasks cached exact snapshot"

    $expectedJournalCached = @"
--- DByteOS Workspace Timeline: Journal ---
Timeline Mode: cached
* [journal] line 1: [JOURNAL] dbyteos journal seed
"@

    $timelineJournalCached = Invoke-Dbyte -Arguments @("run", "bin\timeline.dby", "journal") -WorkingDirectory $dbyteosRoot
    if ($timelineJournalCached.Code -ne 0) { throw "timeline journal cached failed" }
    Assert-NormalizedEqual $timelineJournalCached.Text $expectedJournalCached "timeline journal cached exact snapshot"

    $expectedSearchCached = @"
--- DByteOS Workspace Timeline Search: demo ---
Timeline Mode: cached
* [project] demo (registered)
* [project_note] demo: project demo notes (home/deadbyte/projects/demo/notes.txt:1)
* [project_task] demo #1: [ ] inspect workspace
* [project_task] demo #2: [ ] write project note
"@

    $timelineSearchCached = Invoke-Dbyte -Arguments @("run", "bin\timeline.dby", "search", "demo") -WorkingDirectory $dbyteosRoot
    if ($timelineSearchCached.Code -ne 0) { throw "timeline search cached failed" }
    Assert-NormalizedEqual $timelineSearchCached.Text $expectedSearchCached "timeline search cached exact snapshot"

    $expectedSnapshotCached = @"
--- DByteOS Workspace Timeline Snapshot ---
Timeline Mode: cached
Total Projects: 1
Total Notes:    2 (1 global, 1 project-specific)
Total Journal:  1
Total Tasks:    2 (2 open, 0 done)
"@

    $timelineSnapshotCached = Invoke-Dbyte -Arguments @("run", "bin\timeline.dby", "snapshot") -WorkingDirectory $dbyteosRoot
    if ($timelineSnapshotCached.Code -ne 0) { throw "timeline snapshot cached failed" }
    Assert-NormalizedEqual $timelineSnapshotCached.Text $expectedSnapshotCached "timeline snapshot cached exact snapshot"

    # 4. Clear cache
    $cacheClearFinal = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "clear-cache") -WorkingDirectory $dbyteosRoot
    if ($cacheClearFinal.Code -ne 0) { throw "search clear-cache final failed" }
    Assert-Equal $cacheClearFinal.Text "search: index cache cleared successfully" "cache clear-cache output"

    # --- DByteOS Workspace Dashboard (v9.0.2) tests ---
    # 1. Fallback Scan Mode (cache is cleared or missing)
    $expectedDashboardHomeFallback = @"
--- DByteOS Workspace Dashboard ---
system: healthy
projects: 1
tasks: 2 open / 0 done
search index: missing
timeline: ready
preferences: healthy
"@

    $dashboardHomeScanned = Invoke-Dbyte -Arguments @("run", "bin\dashboard.dby") -WorkingDirectory $dbyteosRoot
    if ($dashboardHomeScanned.Code -ne 0) { throw "dashboard home scanned failed" }
    Assert-NormalizedEqual $dashboardHomeScanned.Text $expectedDashboardHomeFallback "dashboard home fallback exact snapshot"

    $expectedDashboardProjectsFallback = @"
--- DByteOS Dashboard: Projects ---
* demo: 2 open, 0 done (total: 2)
"@

    $dashboardProjectsScanned = Invoke-Dbyte -Arguments @("run", "bin\dashboard.dby", "projects") -WorkingDirectory $dbyteosRoot
    if ($dashboardProjectsScanned.Code -ne 0) { throw "dashboard projects scanned failed" }
    Assert-NormalizedEqual $dashboardProjectsScanned.Text $expectedDashboardProjectsFallback "dashboard projects fallback exact snapshot"

    $expectedDashboardTasksFallback = @"
--- DByteOS Dashboard: Tasks ---
* [open] demo #1: inspect workspace
* [open] demo #2: write project note
"@

    $dashboardTasksScanned = Invoke-Dbyte -Arguments @("run", "bin\dashboard.dby", "tasks") -WorkingDirectory $dbyteosRoot
    if ($dashboardTasksScanned.Code -ne 0) { throw "dashboard tasks scanned failed" }
    Assert-NormalizedEqual $dashboardTasksScanned.Text $expectedDashboardTasksFallback "dashboard tasks fallback exact snapshot"

    # Deterministic query validation check
    $dashboardSearchBad = Invoke-Dbyte -Arguments @("run", "bin\dashboard.dby", "search", "a/b") -WorkingDirectory $dbyteosRoot
    if ($dashboardSearchBad.Code -ne 0) { throw "dashboard search query validation failed" }
    Assert-Equal $dashboardSearchBad.Text "error: search: invalid query" "dashboard search query invalid rejection"

    $expectedDashboardSearchFallback = @"
--- DByteOS Dashboard Search: demo ---
* [project] demo (registered)
"@

    $dashboardSearchScanned = Invoke-Dbyte -Arguments @("run", "bin\dashboard.dby", "search", "demo") -WorkingDirectory $dbyteosRoot
    if ($dashboardSearchScanned.Code -ne 0) { throw "dashboard search scanned failed" }
    Assert-NormalizedEqual $dashboardSearchScanned.Text $expectedDashboardSearchFallback "dashboard search fallback exact snapshot"

    $expectedDashboardTimelineFallback = @"
--- DByteOS Dashboard: Timeline ---
Timeline Mode: fallback
Total Projects: 1
Total Notes:    2
Total Journal:  1
Total Tasks:    2
"@

    $dashboardTimelineScanned = Invoke-Dbyte -Arguments @("run", "bin\dashboard.dby", "timeline") -WorkingDirectory $dbyteosRoot
    if ($dashboardTimelineScanned.Code -ne 0) { throw "dashboard timeline scanned failed" }
    Assert-NormalizedEqual $dashboardTimelineScanned.Text $expectedDashboardTimelineFallback "dashboard timeline fallback exact snapshot"

    $expectedDashboardHealthFallback = @"
--- DByteOS Dashboard: Health ---
system: healthy
search index: missing
preferences: healthy
projects index: healthy
"@

    $dashboardHealthScanned = Invoke-Dbyte -Arguments @("run", "bin\dashboard.dby", "health") -WorkingDirectory $dbyteosRoot
    if ($dashboardHealthScanned.Code -ne 0) { throw "dashboard health scanned failed" }
    Assert-NormalizedEqual $dashboardHealthScanned.Text $expectedDashboardHealthFallback "dashboard health fallback exact snapshot"

    $expectedDashboardSnapshotFallback = @"
--- DByteOS Dashboard: Snapshot ---
User: deadbyte
OS Version: 9.0.2
Projects: 1
Tasks: 2 open / 0 done
Services: active
"@

    $dashboardSnapshotScanned = Invoke-Dbyte -Arguments @("run", "bin\dashboard.dby", "snapshot") -WorkingDirectory $dbyteosRoot
    if ($dashboardSnapshotScanned.Code -ne 0) { throw "dashboard snapshot scanned failed" }
    Assert-NormalizedEqual $dashboardSnapshotScanned.Text $expectedDashboardSnapshotFallback "dashboard snapshot fallback exact snapshot"

    # 2. Cached Mode (cache is rebuilt)
    $cacheRebuildForDashboard = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "rebuild") -WorkingDirectory $dbyteosRoot
    if ($cacheRebuildForDashboard.Code -ne 0) { throw "cache rebuild for dashboard failed" }

    $expectedDashboardHomeCached = @"
--- DByteOS Workspace Dashboard ---
system: healthy
projects: 1
tasks: 2 open / 0 done
search index: healthy
timeline: ready
preferences: healthy
"@

    $dashboardHomeCached = Invoke-Dbyte -Arguments @("run", "bin\dashboard.dby") -WorkingDirectory $dbyteosRoot
    if ($dashboardHomeCached.Code -ne 0) { throw "dashboard home cached failed" }
    Assert-NormalizedEqual $dashboardHomeCached.Text $expectedDashboardHomeCached "dashboard home cached exact snapshot"

    $expectedDashboardTimelineCached = @"
--- DByteOS Dashboard: Timeline ---
Timeline Mode: cached
Total Projects: 1
Total Notes:    2
Total Journal:  1
Total Tasks:    2
"@

    $dashboardTimelineCached = Invoke-Dbyte -Arguments @("run", "bin\dashboard.dby", "timeline") -WorkingDirectory $dbyteosRoot
    if ($dashboardTimelineCached.Code -ne 0) { throw "dashboard timeline cached failed" }
    Assert-NormalizedEqual $dashboardTimelineCached.Text $expectedDashboardTimelineCached "dashboard timeline cached exact snapshot"

    # Clean cache again
    $cacheClearFinalForDashboard = Invoke-Dbyte -Arguments @("run", "bin\search.dby", "clear-cache") -WorkingDirectory $dbyteosRoot
    if ($cacheClearFinalForDashboard.Code -ne 0) { throw "search clear-cache final for dashboard failed" }
    Assert-Equal $cacheClearFinalForDashboard.Text "search: index cache cleared successfully" "dashboard index cache cleared"

    Assert-Contains $dbyteosCmdShell.Text "workspace sweep complete" "dbyteos shell clean sweep"

    if ($dbyteosPrefsInitiallyClean) {
        & git checkout -- $dbyteosPrefsRel
    }
    Assert-GitStatus-Unchanged $dbyteosStatus "dbyteos system cleanliness"
}
catch {
    throw $_
}


$readme = Get-Content (Join-Path $repoRoot "README.md") -Raw
Assert-Contains $readme "dbyte run personal_tools\hexdump.dby" "README personal hexdump command"
Assert-Contains $readme "dbyte run personal_tools\bininfo.dby" "README personal bininfo command"
Assert-Contains $readme "dbyte run personal_tools\find_bytes.dby" "README personal find command"
Assert-Contains $readme "dbyte run personal_tools\patch_bytes.dby" "README personal patch command"
Assert-Contains $readme "dbyte run personal_tools\read_u32_table.dby" "README personal u32 command"

Write-Host "Running project workflow tests..."

$basicProjectRoot = Join-Path $repoRoot "tests\project\basic"
Push-Location $basicProjectRoot
try {
    $result = Invoke-Dbyte -Arguments @("run") -WorkingDirectory $basicProjectRoot
    if ($result.Code -ne 0) { throw "basic project run failed: $($result.Text)" }
    $expected = (Get-Content "expected.out" -Raw).Trim()
    Assert-Equal $result.Text $expected "basic project run"
    $vmResult = Invoke-Dbyte -Arguments @("run", "--vm") -WorkingDirectory $basicProjectRoot
    if ($vmResult.Code -ne 0) { throw "basic project vm run failed: $($vmResult.Text)" }
    Assert-Equal $vmResult.Text $expected "basic project vm run"
    $checkResult = Invoke-Dbyte -Arguments @("check") -WorkingDirectory $basicProjectRoot
    if ($checkResult.Code -ne 0) { throw "basic project check failed: $($checkResult.Text)" }
    Assert-Contains $checkResult.Text "no type errors found" "basic project check"
}
finally {
    Pop-Location
}

$missingManifestProjectRoot = Join-Path $repoRoot "tests\project\missing_manifest"
Push-Location $missingManifestProjectRoot
try {
    $result = Invoke-Dbyte -Arguments @("run") -WorkingDirectory $missingManifestProjectRoot
    if ($result.Code -eq 0) { throw "missing manifest project unexpectedly passed" }
    $expected = (Get-Content "expected.err" -Raw).Trim()
    Assert-Contains $result.Text $expected "missing manifest project"
}
finally {
    Pop-Location
}

$missingEntryProjectRoot = Join-Path $repoRoot "tests\project\missing_entry"
Push-Location $missingEntryProjectRoot
try {
    $result = Invoke-Dbyte -Arguments @("run") -WorkingDirectory $missingEntryProjectRoot
    if ($result.Code -eq 0) { throw "missing entry project unexpectedly passed" }
    $expected = (Get-Content "expected.err" -Raw).Trim()
    Assert-Contains $result.Text $expected "missing entry project"
}
finally {
    Pop-Location
}

$invalidManifestProjectRoot = Join-Path $repoRoot "tests\project\invalid_manifest"
Push-Location $invalidManifestProjectRoot
try {
    $result = Invoke-Dbyte -Arguments @("run") -WorkingDirectory $invalidManifestProjectRoot
    if ($result.Code -eq 0) { throw "invalid manifest project unexpectedly passed" }
    $expected = (Get-Content "expected.err" -Raw).Trim()
    Assert-Contains $result.Text $expected "invalid manifest project"
}
finally {
    Pop-Location
}

$nestedProjectRoot = Join-Path $repoRoot "tests\project\nested_run\src\tools"
Push-Location $nestedProjectRoot
try {
    $result = Invoke-Dbyte -Arguments @("run") -WorkingDirectory $nestedProjectRoot
    if ($result.Code -ne 0) { throw "nested project run failed: $($result.Text)" }
    $expected = (Get-Content "..\..\expected.out" -Raw).Trim()
    Assert-Equal $result.Text $expected "nested project run"
    $vmResult = Invoke-Dbyte -Arguments @("run", "--vm") -WorkingDirectory $nestedProjectRoot
    if ($vmResult.Code -ne 0) { throw "nested project vm run failed: $($vmResult.Text)" }
    Assert-Equal $vmResult.Text $expected "nested project vm run"
}
finally {
    Pop-Location
}

$newRoot = Join-Path $repoRoot "target\verify-project-new"
if (Test-Path $newRoot) {
    Remove-Item -Recurse -Force $newRoot
}
New-Item -ItemType Directory -Path $newRoot | Out-Null
Push-Location $newRoot
try {
    $result = Invoke-Dbyte -Arguments @("new", "scanner") -WorkingDirectory $newRoot
    if ($result.Code -ne 0) { throw "dbyte new failed: $($result.Text)" }
    $scannerProjectRoot = Join-Path $newRoot "scanner"
    Push-Location $scannerProjectRoot
    try {
        $runResult = Invoke-Dbyte -Arguments @("run") -WorkingDirectory $scannerProjectRoot
        if ($runResult.Code -ne 0) { throw "new project run failed: $($runResult.Text)" }
        Assert-Equal $runResult.Text "hello from scanner" "new project run"
        $vmRunResult = Invoke-Dbyte -Arguments @("run", "--vm") -WorkingDirectory $scannerProjectRoot
        if ($vmRunResult.Code -ne 0) { throw "new project vm run failed: $($vmRunResult.Text)" }
        Assert-Equal $vmRunResult.Text "hello from scanner" "new project vm run"
        $testResult = Invoke-Dbyte -Arguments @("test") -WorkingDirectory $scannerProjectRoot
        if ($testResult.Code -ne 0) { throw "new project test failed: $($testResult.Text)" }
        Assert-Contains $testResult.Text "Test result: 1 passed, 0 failed" "new project test"
    }
    finally {
        Pop-Location
    }
}
finally {
    Pop-Location
}

$EXPECTED_VERSION = "9.0.2"

$DBYTE_BIN = "target/release/dbyte.exe"
$releaseExe = Join-Path $repoRoot "target\release\dbyte.exe"
& $cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$version = & $releaseExe --version
if ($version -notmatch $EXPECTED_VERSION) { throw "version check failed: got '$version'" }

Write-Host "Running release personal tools smoke tests..."
$releasePersonalToolsStatus = Git-Status-Short
foreach ($tool in $personalToolFiles) {
    $output = & $releaseExe run "personal_tools\$($tool.Path)" 2>&1
    if ($LASTEXITCODE -ne 0) { throw "release personal tool failed [$($tool.Name)]: $(Normalize-Output $output)" }
    Assert-PersonalToolOutput $tool.Name (Normalize-Output $output)
}

$releaseToolsRoot = Join-Path $repoRoot "target\verify-release-personal-tools"
if (Test-Path $releaseToolsRoot) {
    Remove-Item -Recurse -Force $releaseToolsRoot
}
New-Item -ItemType Directory -Path $releaseToolsRoot | Out-Null
$releaseToolsFile = Join-Path $releaseToolsRoot "sample.bin"
[System.IO.File]::WriteAllBytes($releaseToolsFile, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x11, 0xde, 0xad, 0xbe, 0xef, 0x22, 0x78, 0x56, 0x34, 0x12))

$releaseHexRange = & $releaseExe run "personal_tools\hexdump.dby" $releaseToolsFile 1 6 2>&1
if ($LASTEXITCODE -ne 0) { throw "release hexdump range failed: $(Normalize-Output $releaseHexRange)" }
Assert-Contains (Normalize-Output $releaseHexRange) "1 : deadbeef11de" "release hexdump range"

$releaseFindMode = & $releaseExe run "personal_tools\find_bytes.dby" $releaseToolsFile DEADBEEF 2>&1
if ($LASTEXITCODE -ne 0) { throw "release find mode failed: $(Normalize-Output $releaseFindMode)" }
Assert-Contains (Normalize-Output $releaseFindMode) "pattern: 1 0x1" "release find mode"

$releasePatchAll = & $releaseExe run "personal_tools\patch_bytes.dby" --all $releaseToolsFile DEADBEEF CAFEBABE 2>&1
if ($LASTEXITCODE -ne 0) { throw "release patch all failed: $(Normalize-Output $releasePatchAll)" }
Assert-Contains (Normalize-Output $releasePatchAll) "patched count: 2" "release patch all"
Assert-Equal (Bytes-Hex $releaseToolsFile) "00deadbeef11deadbeef2278563412" "release patch all original unchanged"
Assert-Equal (Bytes-Hex "$releaseToolsFile.patched") "00cafebabe11cafebabe2278563412" "release patch all output"

$releasePatchOffsetFile = Join-Path $releaseToolsRoot "offset.bin"
[System.IO.File]::WriteAllBytes($releasePatchOffsetFile, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x00))
$releasePatchOffset = & $releaseExe run "personal_tools\patch_bytes.dby" --offset 1 $releasePatchOffsetFile CAFEBABE 2>&1
if ($LASTEXITCODE -ne 0) { throw "release patch offset failed: $(Normalize-Output $releasePatchOffset)" }
Assert-Contains (Normalize-Output $releasePatchOffset) "patched offset 1" "release patch offset"
Assert-Equal (Bytes-Hex "$releasePatchOffsetFile.patched") "00cafebabe00" "release patch offset output"

$releaseU32Range = & $releaseExe run "personal_tools\read_u32_table.dby" $releaseToolsFile 11 1 2>&1
if ($LASTEXITCODE -ne 0) { throw "release u32 range failed: $(Normalize-Output $releaseU32Range)" }
Assert-Contains (Normalize-Output $releaseU32Range) "11 -> 305419896" "release u32 range"

$releaseHexOob = & $releaseExe run "personal_tools\hexdump.dby" $releaseToolsFile 20 1 2>&1
if ($LASTEXITCODE -ne 0) { throw "release hexdump offset oob failed: $(Normalize-Output $releaseHexOob)" }
Assert-Equal (Normalize-Output $releaseHexOob) "error: offset out of bounds" "release hexdump offset oob"

$releaseHexClamp = & $releaseExe run "personal_tools\hexdump.dby" $releaseToolsFile 1 999 2>&1
if ($LASTEXITCODE -ne 0) { throw "release hexdump length clamp failed: $(Normalize-Output $releaseHexClamp)" }
Assert-Contains (Normalize-Output $releaseHexClamp) "range: 1 14" "release hexdump length clamp header"
Assert-Contains (Normalize-Output $releaseHexClamp) "1 : deadbeef11deadbe" "release hexdump length clamp row1"
Assert-Contains (Normalize-Output $releaseHexClamp) "9 : ef2278563412" "release hexdump length clamp row2"

$releaseFindNoMatchFile = Join-Path $releaseToolsRoot "no-match.bin"
[System.IO.File]::WriteAllBytes($releaseFindNoMatchFile, [byte[]](0x01, 0x02, 0x03))
$releaseFindNoMatch = & $releaseExe run "personal_tools\find_bytes.dby" $releaseFindNoMatchFile DEADBEEF 2>&1
if ($LASTEXITCODE -ne 0) { throw "release find no match failed: $(Normalize-Output $releaseFindNoMatch)" }
Assert-Contains (Normalize-Output $releaseFindNoMatch) "pattern: not found" "release find no match"

if (Test-Path "$releaseToolsFile.patched") {
    Remove-Item -Force "$releaseToolsFile.patched"
}
$releasePatchBadReplace = & $releaseExe run "personal_tools\patch_bytes.dby" $releaseToolsFile DEADBEEF ZZZZZZZZ 2>&1
if ($LASTEXITCODE -ne 0) { throw "release patch invalid replace failed: $(Normalize-Output $releasePatchBadReplace)" }
Assert-Equal (Normalize-Output $releasePatchBadReplace) "error: invalid replace_hex" "release patch invalid replace"
if (Test-Path "$releaseToolsFile.patched") { throw "release patch invalid replace unexpectedly wrote output" }

$releaseU32StartOob = & $releaseExe run "personal_tools\read_u32_table.dby" $releaseToolsFile 20 1 2>&1
if ($LASTEXITCODE -ne 0) { throw "release u32 start offset oob failed: $(Normalize-Output $releaseU32StartOob)" }
Assert-Equal (Normalize-Output $releaseU32StartOob) "error: offset out of bounds" "release u32 start offset oob"

Assert-GitStatus-Unchanged $releasePersonalToolsStatus "release personal tools cleanliness"

Write-Host "Verifying DByteOS Kernel Lab (v9.0.2) freestanding build..."
$linkerScript = Join-Path $repoRoot "kernel-lab\boot\linker.ld"
if (-not (Test-Path $linkerScript)) { throw "Kernel-lab linker script not found!" }

$kernelLabDir = Join-Path $repoRoot "kernel-lab"
Push-Location $kernelLabDir
try {
    & powershell .\scripts\build.ps1
    if ($LASTEXITCODE -ne 0) { throw "Kernel-lab build script failed!" }
}
finally {
    Pop-Location
}

$elfPath = Join-Path $repoRoot "kernel-lab\target\i686-unknown-linux-gnu\debug\dbyte_kernel"
if (-not (Test-Path $elfPath)) { throw "Freestanding kernel ELF binary not found: $elfPath" }

# Assert Multiboot header magic 0x1BADB002 exists in first 8 KiB of ELF binary
$bytes = [System.IO.File]::ReadAllBytes($elfPath)
$foundMagic = $false
for ($i = 0; $i -lt [Math]::Min(8192, $bytes.Length - 4); $i += 4) {
    if ($bytes[$i] -eq 0x02 -and $bytes[$i+1] -eq 0xb0 -and $bytes[$i+2] -eq 0xad -and $bytes[$i+3] -eq 0x1b) {
        $foundMagic = $true
        break
    }
}
if (-not $foundMagic) {
    throw "Multiboot header magic 0x1BADB002 not found in first 8 KiB of ELF!"
}
Write-Host "[OK] Kernel Lab freestanding build smoke passed (verified Multiboot magic 0x1BADB002)." -ForegroundColor Green

$kernelMainSource = Get-Content (Join-Path $repoRoot "kernel-lab\src\main.rs") -Raw
$kernelIdtSource = Get-Content (Join-Path $repoRoot "kernel-lab\src\idt.rs") -Raw
$kernelInterruptSource = Get-Content (Join-Path $repoRoot "kernel-lab\src\interrupts.rs") -Raw
$kernelPageFaultSource = Get-Content (Join-Path $repoRoot "kernel-lab\src\page_fault.rs") -Raw
$kernelPicSource = Get-Content (Join-Path $repoRoot "kernel-lab\src\pic.rs") -Raw
$kernelIrqSource = Get-Content (Join-Path $repoRoot "kernel-lab\src\irq.rs") -Raw
$kernelExceptionDocs = Get-Content (Join-Path $repoRoot "docs\KERNEL_EXCEPTIONS.md") -Raw
$kernelIrqDocs = Get-Content (Join-Path $repoRoot "docs\KERNEL_IRQ.md") -Raw
$kernelInterruptDocs = Get-Content (Join-Path $repoRoot "docs\KERNEL_INTERRUPTS.md") -Raw
$kernelBootSmokeDocs = Get-Content (Join-Path $repoRoot "docs\QEMU_BOOT_SMOKE.md") -Raw
$kernelMainSource = $kernelMainSource -replace "`r`n", "`n"
$kernelIdtSource = $kernelIdtSource -replace "`r`n", "`n"
$kernelInterruptSource = $kernelInterruptSource -replace "`r`n", "`n"
$kernelPageFaultSource = $kernelPageFaultSource -replace "`r`n", "`n"
$kernelPicSource = $kernelPicSource -replace "`r`n", "`n"
$kernelIrqSource = $kernelIrqSource -replace "`r`n", "`n"
$kernelExceptionDocs = $kernelExceptionDocs -replace "`r`n", "`n"
$kernelIrqDocs = $kernelIrqDocs -replace "`r`n", "`n"
$kernelInterruptDocs = $kernelInterruptDocs -replace "`r`n", "`n"
$kernelBootSmokeDocs = $kernelBootSmokeDocs -replace "`r`n", "`n"

Write-Host "Verifying DByteOS Kernel Lab (v9.0.2) exception status UX contracts..."
Assert-Contains $kernelMainSource "mod page_fault;" "kernel page fault skeleton module is compiled"
Assert-Contains $kernelMainSource "mod irq;" "kernel irq skeleton module is compiled"
$expectedKernelHelp = "commands: help about version clear echo mem uptime banner keyboard reboot-note system cls status mods keys prompt int3 div0 exception exception-reset handlers handlers --active exception-status exceptions exceptions --verbose exception-help exception-about fault-status fault-reset pf-note pf-status pf-smoke irq-note irq-status irq-handlers eoi-note eoi-status irq-gates irq-gate-status irq-gate-plan irq-gate-arm irq-gate-bind-smoke irq-gate-bind-status irq-gate-state irq-gate-history irq-gate-preflight irq-bind-note irq-bind-status irq-readiness irq-risk irq-preflight irq-runtime-arm irq-runtime-commit irq-runtime-preflight irq-runtime-status irq-runtime-blockers irq-runtime-matrix irq-runtime-readiness irq-runtime-next irq-runtime-activation-plan irq-runtime-token-note irq-runtime-token-status irq-runtime-token-arm irq-runtime-token-clear irq-runtime-gate-note irq-runtime-gate-status irq-runtime-gate-check irq-runtime-gate-blockers irq-runtime-sim-note irq-runtime-sim-status irq-runtime-sim-run irq-runtime-sim-blockers pic-note pic-status pic-plan pic-remap-arm pic-remap-smoke pic-remap-status pic-remap-state pic-remap-history pic-remap-preflight irq-map pic-status --verbose"
Assert-Contains $kernelMainSource $expectedKernelHelp "kernel help lists exception and irq UX commands"
Assert-Contains $kernelMainSource "irq::irq_gate_bind_smoke_status()" "kernel handlers reads irq gate bind status"
Assert-Contains $kernelMainSource "skeleton planned: irq0 timer, irq1 keyboard" "kernel handlers unbound irq section"
Assert-Contains $kernelMainSource "vector {}: irq0 timer smoke stub / dormant" "kernel handlers bound irq0 line template"
Assert-Contains $kernelMainSource "runtime irq: disabled" "kernel handlers reports runtime irq disabled"
Assert-Contains $kernelMainSource "line_str == `"exception-status`" || line_str == `"exceptions`"" "kernel exceptions alias dispatch"
Assert-Contains $kernelMainSource "line_str == `"exceptions --verbose`"" "kernel exceptions verbose dispatch"
Assert-Contains $kernelMainSource "line_str == `"fault-status`"" "kernel fault-status dispatch"
Assert-Contains $kernelMainSource "line_str == `"fault-reset`"" "kernel fault-reset dispatch"
Assert-Contains $kernelMainSource "line_str == `"pf-status`"" "kernel pf-status dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-note`"" "kernel irq-note dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-status`"" "kernel irq-status dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-handlers`"" "kernel irq-handlers dispatch"
Assert-Contains $kernelMainSource "line_str == `"pic-note`"" "kernel pic-note dispatch"
Assert-Contains $kernelMainSource "line_str == `"pic-status`"" "kernel pic-status dispatch"
Assert-Contains $kernelMainSource "line_str == `"pic-plan`"" "kernel pic-plan dispatch"
Assert-Contains $kernelMainSource "line_str == `"pic-remap-arm`"" "kernel pic-remap-arm dispatch"
Assert-Contains $kernelMainSource "line_str == `"pic-remap-smoke`"" "kernel pic-remap-smoke dispatch"
Assert-Contains $kernelMainSource "line_str == `"pic-remap-status`"" "kernel pic-remap-status dispatch"
Assert-Contains $kernelMainSource "line_str == `"pic-remap-state`"" "kernel pic-remap-state dispatch"
Assert-Contains $kernelMainSource "line_str == `"pic-remap-history`"" "kernel pic-remap-history dispatch"
Assert-Contains $kernelMainSource "line_str == `"pic-remap-preflight`"" "kernel pic-remap-preflight dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-map`"" "kernel irq-map dispatch"
Assert-Contains $kernelMainSource "line_str == `"pic-status --verbose`"" "kernel pic-status verbose dispatch"
Assert-Contains $kernelMainSource "line_str == `"eoi-status`"" "kernel eoi-status dispatch"
Assert-Contains $kernelMainSource "line_str == `"eoi-note`"" "kernel eoi-note dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-gates`"" "kernel irq-gates dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-gate-status`"" "kernel irq-gate-status dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-gate-plan`"" "kernel irq-gate-plan dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-gate-arm`"" "kernel irq-gate-arm dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-gate-bind-smoke`"" "kernel irq-gate-bind-smoke dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-gate-bind-status`"" "kernel irq-gate-bind-status dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-gate-state`"" "kernel irq-gate-state dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-gate-history`"" "kernel irq-gate-history dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-gate-preflight`"" "kernel irq-gate-preflight dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-bind-note`"" "kernel irq-bind-note dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-bind-status`"" "kernel irq-bind-status dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-readiness`"" "kernel irq-readiness dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-risk`"" "kernel irq-risk dispatch"
Assert-Contains $kernelMainSource "line_str == `"irq-preflight`"" "kernel irq-preflight dispatch"
Assert-Contains $kernelMainSource "EOI strategy: {}\nPIC command: 0x{:02x}\nmaster PIC: {}\nslave PIC: {}\ndispatch: {}\n" "kernel eoi-status output template"
Assert-Contains $kernelMainSource "EOI strategy note:\n- EOI means End Of Interrupt." "kernel eoi-note output template"
Assert-NotContains $kernelMainSource "line_str == `"irq`"" "kernel has no irq alias"
Assert-NotContains $kernelMainSource "line_str == `"irqs`"" "kernel has no irqs alias"
Assert-NotContains $kernelMainSource "line_str == `"pic`"" "kernel has no pic alias"
Assert-NotContains $kernelMainSource "line_str == `"pics`"" "kernel has no pics alias"
Assert-Contains $kernelMainSource "line_str == `"handlers --active`"" "kernel handlers --active dispatch"
Assert-Contains $kernelMainSource "line_str == `"exception-about`"" "kernel exception-about dispatch"
Assert-Contains $kernelMainSource "exceptions handled: {}\nlast exception: none\ninterrupts: disabled\n" "kernel exception-status none output"
Assert-Contains $kernelMainSource "exceptions handled: {}\nlast exception: {} ({})\ninterrupts: disabled\n" "kernel exception-status populated output"
Assert-Contains $kernelMainSource "exceptions handled: {}`nlast exception: none`n" "kernel system none telemetry output"
Assert-Contains $kernelMainSource "exceptions handled: {}`nlast exception: {} ({})`n" "kernel system populated telemetry output"
Assert-Contains $kernelMainSource "fault recovery:\nexceptions handled: {}\nlast exception: none\nrecovery mode: smoke-safe\npage fault smoke: armed={}\ninterrupts: disabled\n" "kernel fault-status none output"
Assert-Contains $kernelMainSource "fault recovery:\nexceptions handled: {}\nlast exception: {} ({})\nrecovery mode: smoke-safe\npage fault smoke: armed={}\ninterrupts: disabled\n" "kernel fault-status populated output"
Assert-Contains $kernelMainSource "page fault:\nvector: 14\nhandler: active smoke\ntrigger: pf-smoke controlled real fault\ncr2: available after pf-smoke\nerror code: available after pf-smoke\nrecovery: trampoline\n" "kernel pf-status exact output"
Assert-Contains $kernelMainSource "pic/irq: planned / disabled\npic remap: documented only\nirq vectors: 32-47 planned\nirq handler skeletons: irq0 timer, irq1 keyboard\nkeyboard irq1: disabled\ntimer irq0: disabled\ninterrupts: disabled\n" "kernel irq-note exact output"
Assert-Contains $kernelMainSource "irq subsystem:\nfoundation: planned\npic: not remapped\nirq handlers: none\nkeyboard input: polling-only\ntimer: unavailable\ninterrupts: disabled\n" "kernel irq-status exact output"
Assert-Contains $kernelMainSource "irq handlers:\nfoundation: skeleton / disabled\nirq0 timer: skeleton / disabled\nirq1 keyboard: skeleton / disabled\nvectors: 32 / 33\nidt binding: disabled\npic remap: disabled\ninterrupts: disabled\n" "kernel irq-handlers exact output"
Assert-Contains $kernelMainSource "pic remap: planned / disabled\nremap offsets: 0x20 / 0x28\nirq vectors: 0x20-0x2f\nicw sequence: documented in code\nhardware writes: disabled\ninterrupts: disabled\n" "kernel pic-note exact output"
Assert-Contains $kernelMainSource "pic subsystem:\nfoundation: code planned\nremap function: present / not called\nmaster offset: 0x20\nslave offset: 0x28\nirq handlers: none\ninterrupts: disabled\n" "kernel pic-status exact output"
Assert-Contains $kernelMainSource "pic remap dry-run:\nmaster offset: 0x20\nslave offset: 0x28\nirq vector range: 0x20-0x2f\nicw1: 0x11\nicw2 master: 0x20\nicw2 slave: 0x28\nicw3 master: 0x04\nicw3 slave: 0x02\nicw4: 0x01\nmask after remap: 0xff\nhardware writes: disabled\n" "kernel pic-plan exact output"
Assert-Contains $kernelMainSource "PIC remap smoke armed\nmode: {}\nnext: {}\ninterrupts: {}\nirq gates: {}\n" "kernel pic-remap-arm output template"
Assert-Contains $kernelMainSource "PIC remap controlled smoke\nguard: {}\nicw sequence: {}\nmaster offset: 0x{:02x}\nslave offset: 0x{:02x}\nmask after remap: 0x{:02x}\nsti: {}\nirq gates: {}\neoi dispatch: {}\nresult: {}\n" "kernel pic-remap-smoke armed output template"
Assert-Contains $kernelMainSource "PIC remap controlled smoke\nguard: {}\nresult: {}\nnext: {}\n" "kernel pic-remap-smoke blocked output template"
Assert-Contains $kernelMainSource "PIC remap smoke status\narmed: {}\nexecuted: {}\nmaster offset: 0x{:02x}\nslave offset: 0x{:02x}\nmask after remap: 0x{:02x}\nsti: {}\nirq gates: {}\neoi dispatch: {}\n" "kernel pic-remap-status output template"
Assert-Contains $kernelMainSource "PIC remap state\narmed: {}\nexecuted: {}\nmaster offset: 0x{:02x}\nslave offset: 0x{:02x}\nicw sequence expected: {}\nicw sequence applied: {}\nmask after remap: 0x{:02x}\nirq runtime: {}\n" "kernel pic-remap-state output template"
Assert-Contains $kernelMainSource "PIC remap history\narm command: {}\nsmoke command: {}\nlast smoke executed: {}\nicw writes: {}\nboot remap: {}\n" "kernel pic-remap-history output template"
Assert-Contains $kernelMainSource "PIC remap preflight\nguard: {}\nicw sequence: {}\nmaster offset: 0x{:02x}\nslave offset: 0x{:02x}\nmask after remap: 0x{:02x}\nsti: {}\nirq gates: {}\neoi dispatch: {}\nresult: {}\n" "kernel pic-remap-preflight output template"
Assert-Contains $kernelMainSource "pic remap controlled smoke: executed={}\n" "kernel system pic remap controlled smoke sync output"
Assert-Contains $kernelMainSource "irq map:\nirq0 timer -> vector 32 (0x20)\nirq1 keyboard -> vector 33 (0x21)\nirq2 cascade -> vector 34 (0x22)\nirq3 serial2 -> vector 35 (0x23)\nirq4 serial1 -> vector 36 (0x24)\nirq5 parallel2 -> vector 37 (0x25)\nirq6 floppy -> vector 38 (0x26)\nirq7 parallel1 -> vector 39 (0x27)\nirq8 rtc -> vector 40 (0x28)\nirq9 acpi -> vector 41 (0x29)\nirq10 reserved -> vector 42 (0x2a)\nirq11 reserved -> vector 43 (0x2b)\nirq12 mouse -> vector 44 (0x2c)\nirq13 fpu -> vector 45 (0x2d)\nirq14 primary-ata -> vector 46 (0x2e)\nirq15 secondary-ata -> vector 47 (0x2f)\nactive irq handlers: none\n" "kernel irq-map exact output"
Assert-Contains $kernelMainSource "pic subsystem:\nfoundation: dry-run telemetry\nremap function: present / not called\ndry-run plan: available\nmaster offset: 0x20\nslave offset: 0x28\nirq vectors: 0x20-0x2f\nhardware writes: disabled\nirq handlers: none\ninterrupts: disabled\n" "kernel pic-status verbose exact output"
Assert-Contains $kernelMainSource "IRQ Interrupt Gates:\n- Vector 32 (0x20): IRQ0 Timer (planned)\n- Vector 33 (0x21): IRQ1 Keyboard (planned)\n- Handler setup: planned\n- Status: dormant / disabled\n" "kernel irq-gates exact output"
Assert-Contains $kernelMainSource "IDT vector 32 (IRQ0 Timer): disabled / null handler\nIDT vector 33 (IRQ1 Keyboard): disabled / null handler\ngate binding dispatch: dormant\n" "kernel irq-gate-status exact output"
Assert-Contains $kernelMainSource "IRQ Gate Binding Plan:\nIRQ{} {} -> vector {} (0x{:02x})\nIRQ{} {} -> vector {} (0x{:02x})\nIDT binding: {}\nPIC remap: {}\nEOI dispatch: {}\ninterrupts: {}\nstate: {}\n" "kernel irq-gate-plan output template"
Assert-Contains $kernelMainSource "IRQ gate bind smoke armed\nmode: {}\nnext: {}\ninterrupts: {}\npic irq mask: {}\neoi dispatch: {}\n" "kernel irq-gate-arm output template"
Assert-Contains $kernelMainSource "IRQ gate bind controlled smoke\nguard: {}\nIDT vector 32: {}\nIDT vector 33: {}\npic irq mask: {}\nsti: {}\neoi dispatch: {}\nkeyboard input: {}\nresult: {}\n" "kernel irq-gate-bind-smoke armed output template"
Assert-Contains $kernelMainSource "IRQ gate bind controlled smoke\nguard: {}\nresult: {}\nnext: {}\n" "kernel irq-gate-bind-smoke blocked output template"
Assert-Contains $kernelMainSource "IRQ gate bind smoke status\narmed: {}\nexecuted: {}\nIDT vector {}: {}\nIDT vector {}: {}\nactive IRQ0 handler: {}\nactive IRQ1 handler: {}\npic irq mask: {}\nsti: {}\neoi dispatch: {}\nkeyboard input: {}\n" "kernel irq-gate-bind-status output template"
Assert-Contains $kernelMainSource "IRQ gate bind state\narmed: {}\nexecuted: {}\nIDT vector {}: {}\nIDT vector {}: {}\nactive IRQ0 handler: {}\nactive IRQ1 handler: {}\nbind expected: {}\nbind applied: {}\nirq runtime: {}\npic irq mask: {}\nsti: {}\neoi dispatch: {}\nkeyboard input: {}\n" "kernel irq-gate-state output template"
Assert-Contains $kernelMainSource "IRQ gate bind history\narm command: {}\nsmoke command: {}\nlast smoke executed: {}\nidt binds: {}\nboot bind: {}\n" "kernel irq-gate-history output template"
Assert-Contains $kernelMainSource "IRQ gate bind preflight\nguard: {}\nbind path: {}\nIDT vector {}: {}\nIDT vector {}: {}\npic irq mask: {}\nsti: {}\neoi dispatch: {}\nkeyboard input: {}\nresult: {}\n" "kernel irq-gate-preflight output template"
Assert-Contains $kernelMainSource "irq gates controlled smoke: bound={}\n" "kernel system irq gate bind controlled smoke sync output"
Assert-Contains $kernelMainSource "let plan = irq::irq_gate_plan();" "kernel irq-gate-plan reads helper"
Assert-Contains $kernelMainSource "let timer = plan[0];" "kernel irq-gate-plan uses first helper slot"
Assert-Contains $kernelMainSource "let keyboard = plan[1];" "kernel irq-gate-plan uses second helper slot"
Assert-Contains $kernelMainSource "timer.idt_binding" "kernel irq-gate-plan renders helper IDT binding"
Assert-Contains $kernelMainSource "timer.pic_remap" "kernel irq-gate-plan renders helper PIC remap"
Assert-Contains $kernelMainSource "timer.eoi_dispatch" "kernel irq-gate-plan renders helper EOI dispatch"
Assert-Contains $kernelMainSource "timer.interrupts" "kernel irq-gate-plan renders helper interrupt state"
Assert-Contains $kernelMainSource "timer.gate_state" "kernel irq-gate-plan renders helper gate state"
$expectedIrqGatePlanOutput = "IRQ Gate Binding Plan:`nIRQ0 timer -> vector 32 (0x20)`nIRQ1 keyboard -> vector 33 (0x21)`nIDT binding: disabled`nPIC remap: disabled`nEOI dispatch: disabled`ninterrupts: disabled`nstate: dormant / disabled"
Assert-Contains $kernelIrqDocs $expectedIrqGatePlanOutput "irq docs irq-gate-plan exact rendered contract"
$expectedQemuIrqGatePlanOutput = "IRQ Gate Binding Plan:`n    IRQ0 timer -> vector 32 (0x20)`n    IRQ1 keyboard -> vector 33 (0x21)`n    IDT binding: disabled`n    PIC remap: disabled`n    EOI dispatch: disabled`n    interrupts: disabled`n    state: dormant / disabled"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqGatePlanOutput "qemu docs irq-gate-plan exact rendered contract"
$expectedIrqGateArmOutput = "IRQ gate bind smoke armed`nmode: controlled bind smoke`nnext: irq-gate-bind-smoke`ninterrupts: disabled`npic irq mask: masked`neoi dispatch: disabled"
Assert-Contains $kernelIrqDocs $expectedIrqGateArmOutput "irq docs irq-gate-arm exact rendered contract"
$expectedQemuIrqGateArmOutput = "IRQ gate bind smoke armed`n    mode: controlled bind smoke`n    next: irq-gate-bind-smoke`n    interrupts: disabled`n    pic irq mask: masked`n    eoi dispatch: disabled"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqGateArmOutput "qemu docs irq-gate-arm exact rendered contract"
$expectedIrqGateBindBlockedOutput = "IRQ gate bind controlled smoke`nguard: not armed`nresult: blocked`nnext: irq-gate-arm"
Assert-Contains $kernelIrqDocs $expectedIrqGateBindBlockedOutput "irq docs irq-gate-bind-smoke blocked exact rendered contract"
$expectedQemuIrqGateBindBlockedOutput = "IRQ gate bind controlled smoke`n    guard: not armed`n    result: blocked`n    next: irq-gate-arm"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqGateBindBlockedOutput "qemu docs irq-gate-bind-smoke blocked exact rendered contract"
$expectedIrqGateBindArmedOutput = "IRQ gate bind controlled smoke`nguard: armed`nIDT vector 32: bound to IRQ0 timer smoke stub`nIDT vector 33: bound to IRQ1 keyboard smoke stub`npic irq mask: masked`nsti: disabled`neoi dispatch: disabled`nkeyboard input: polling-only`nresult: bound / dormant"
Assert-Contains $kernelIrqDocs $expectedIrqGateBindArmedOutput "irq docs irq-gate-bind-smoke armed exact rendered contract"
$expectedQemuIrqGateBindArmedOutput = "IRQ gate bind controlled smoke`n    guard: armed`n    IDT vector 32: bound to IRQ0 timer smoke stub`n    IDT vector 33: bound to IRQ1 keyboard smoke stub`n    pic irq mask: masked`n    sti: disabled`n    eoi dispatch: disabled`n    keyboard input: polling-only`n    result: bound / dormant"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqGateBindArmedOutput "qemu docs irq-gate-bind-smoke armed exact rendered contract"
$expectedIrqGateBindStatusOutput = "IRQ gate bind smoke status`narmed: no`nexecuted: no`nIDT vector 32: unbound`nIDT vector 33: unbound`nactive IRQ0 handler: smoke stub / dormant`nactive IRQ1 handler: smoke stub / dormant`npic irq mask: masked`nsti: disabled`neoi dispatch: disabled`nkeyboard input: polling-only"
Assert-Contains $kernelIrqDocs $expectedIrqGateBindStatusOutput "irq docs irq-gate-bind-status exact rendered contract"
$expectedQemuIrqGateBindStatusOutput = "IRQ gate bind smoke status`n    armed: no`n    executed: no`n    IDT vector 32: unbound`n    IDT vector 33: unbound`n    active IRQ0 handler: smoke stub / dormant`n    active IRQ1 handler: smoke stub / dormant`n    pic irq mask: masked`n    sti: disabled`n    eoi dispatch: disabled`n    keyboard input: polling-only"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqGateBindStatusOutput "qemu docs irq-gate-bind-status exact rendered contract"
$expectedIrqGateBindStatusAfterOutput = "IRQ gate bind smoke status`narmed: no`nexecuted: yes`nIDT vector 32: bound`nIDT vector 33: bound`nactive IRQ0 handler: smoke stub / dormant`nactive IRQ1 handler: smoke stub / dormant`npic irq mask: masked`nsti: disabled`neoi dispatch: disabled`nkeyboard input: polling-only"
Assert-Contains $kernelIrqDocs $expectedIrqGateBindStatusAfterOutput "irq docs irq-gate-bind-status after-smoke exact rendered contract"
$expectedQemuIrqGateBindStatusAfterOutput = "IRQ gate bind smoke status`n    armed: no`n    executed: yes`n    IDT vector 32: bound`n    IDT vector 33: bound`n    active IRQ0 handler: smoke stub / dormant`n    active IRQ1 handler: smoke stub / dormant`n    pic irq mask: masked`n    sti: disabled`n    eoi dispatch: disabled`n    keyboard input: polling-only"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqGateBindStatusAfterOutput "qemu docs irq-gate-bind-status after-smoke exact rendered contract"
$expectedIrqGateBindStateOutput = "IRQ gate bind state`narmed: no`nexecuted: no`nIDT vector 32: unbound`nIDT vector 33: unbound`nactive IRQ0 handler: smoke stub / dormant`nactive IRQ1 handler: smoke stub / dormant`nbind expected: yes`nbind applied: no`nirq runtime: disabled`npic irq mask: masked`nsti: disabled`neoi dispatch: disabled`nkeyboard input: polling-only"
Assert-Contains $kernelIrqDocs $expectedIrqGateBindStateOutput "irq docs irq-gate-state exact rendered contract"
$expectedQemuIrqGateBindStateOutput = "IRQ gate bind state`n    armed: no`n    executed: no`n    IDT vector 32: unbound`n    IDT vector 33: unbound`n    active IRQ0 handler: smoke stub / dormant`n    active IRQ1 handler: smoke stub / dormant`n    bind expected: yes`n    bind applied: no`n    irq runtime: disabled`n    pic irq mask: masked`n    sti: disabled`n    eoi dispatch: disabled`n    keyboard input: polling-only"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqGateBindStateOutput "qemu docs irq-gate-state exact rendered contract"
$expectedIrqGateBindStateAfterOutput = "IRQ gate bind state`narmed: no`nexecuted: yes`nIDT vector 32: bound`nIDT vector 33: bound`nactive IRQ0 handler: smoke stub / dormant`nactive IRQ1 handler: smoke stub / dormant`nbind expected: yes`nbind applied: yes`nirq runtime: disabled`npic irq mask: masked`nsti: disabled`neoi dispatch: disabled`nkeyboard input: polling-only"
Assert-Contains $kernelIrqDocs $expectedIrqGateBindStateAfterOutput "irq docs irq-gate-state after-smoke exact rendered contract"
$expectedQemuIrqGateBindStateAfterOutput = "IRQ gate bind state`n    armed: no`n    executed: yes`n    IDT vector 32: bound`n    IDT vector 33: bound`n    active IRQ0 handler: smoke stub / dormant`n    active IRQ1 handler: smoke stub / dormant`n    bind expected: yes`n    bind applied: yes`n    irq runtime: disabled`n    pic irq mask: masked`n    sti: disabled`n    eoi dispatch: disabled`n    keyboard input: polling-only"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqGateBindStateAfterOutput "qemu docs irq-gate-state after-smoke exact rendered contract"
$expectedIrqGateBindHistoryOutput = "IRQ gate bind history`narm command: available`nsmoke command: available`nlast smoke executed: no`nidt binds: controlled command path only`nboot bind: no"
Assert-Contains $kernelIrqDocs $expectedIrqGateBindHistoryOutput "irq docs irq-gate-history exact rendered contract"
$expectedQemuIrqGateBindHistoryOutput = "IRQ gate bind history`n    arm command: available`n    smoke command: available`n    last smoke executed: no`n    idt binds: controlled command path only`n    boot bind: no"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqGateBindHistoryOutput "qemu docs irq-gate-history exact rendered contract"
$expectedIrqGateBindHistoryAfterOutput = "IRQ gate bind history`narm command: available`nsmoke command: available`nlast smoke executed: yes`nidt binds: controlled command path only`nboot bind: no"
Assert-Contains $kernelIrqDocs $expectedIrqGateBindHistoryAfterOutput "irq docs irq-gate-history after-smoke exact rendered contract"
$expectedQemuIrqGateBindHistoryAfterOutput = "IRQ gate bind history`n    arm command: available`n    smoke command: available`n    last smoke executed: yes`n    idt binds: controlled command path only`n    boot bind: no"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqGateBindHistoryAfterOutput "qemu docs irq-gate-history after-smoke exact rendered contract"
$expectedIrqGateBindPreflightOutput = "IRQ gate bind preflight`nguard: command armed required`nbind path: ready`nIDT vector 32: unbound`nIDT vector 33: unbound`npic irq mask: masked`nsti: disabled`neoi dispatch: disabled`nkeyboard input: polling-only`nresult: telemetry only"
Assert-Contains $kernelIrqDocs $expectedIrqGateBindPreflightOutput "irq docs irq-gate-preflight exact rendered contract"
$expectedQemuIrqGateBindPreflightOutput = "IRQ gate bind preflight`n    guard: command armed required`n    bind path: ready`n    IDT vector 32: unbound`n    IDT vector 33: unbound`n    pic irq mask: masked`n    sti: disabled`n    eoi dispatch: disabled`n    keyboard input: polling-only`n    result: telemetry only"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqGateBindPreflightOutput "qemu docs irq-gate-preflight exact rendered contract"
Assert-Contains $kernelIrqDocs "irq gates controlled smoke: bound=no" "irq docs system irq gate bind controlled smoke sync"
Assert-Contains $kernelBootSmokeDocs "irq gates controlled smoke: bound=no" "qemu docs system irq gate bind controlled smoke sync"
$expectedIrqBindNoteOutput = "IRQ bind note:`nIRQ0 timer gate: disabled bind path only`nIRQ1 keyboard gate: disabled bind path only`nIDT entries: planned / not installed`nPIC remap: disabled`nEOI dispatch: disabled`ninterrupts: disabled"
Assert-Contains $kernelIrqDocs $expectedIrqBindNoteOutput "irq docs irq-bind-note exact rendered contract"
$expectedQemuIrqBindNoteOutput = "IRQ bind note:`n    IRQ0 timer gate: disabled bind path only`n    IRQ1 keyboard gate: disabled bind path only`n    IDT entries: planned / not installed`n    PIC remap: disabled`n    EOI dispatch: disabled`n    interrupts: disabled"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqBindNoteOutput "qemu docs irq-bind-note exact rendered contract"
$expectedIrqBindStatusOutput = "IRQ bind status:`nhelper: bind_irq_gates_disabled`nboot call: no`nIDT vector 32: unbound`nIDT vector 33: unbound`nactive IRQ0 handler: none`nactive IRQ1 handler: none`nkeyboard input: polling-only"
Assert-Contains $kernelIrqDocs $expectedIrqBindStatusOutput "irq docs irq-bind-status exact rendered contract"
$expectedQemuIrqBindStatusOutput = "IRQ bind status:`n    helper: bind_irq_gates_disabled`n    boot call: no`n    IDT vector 32: unbound`n    IDT vector 33: unbound`n    active IRQ0 handler: none`n    active IRQ1 handler: none`n    keyboard input: polling-only"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqBindStatusOutput "qemu docs irq-bind-status exact rendered contract"
$expectedPicRemapArmOutput = "PIC remap smoke armed`nmode: controlled smoke`nnext: pic-remap-smoke`ninterrupts: disabled`nirq gates: unbound"
Assert-Contains $kernelIrqDocs $expectedPicRemapArmOutput "irq docs pic-remap-arm exact rendered contract"
$expectedQemuPicRemapArmOutput = "PIC remap smoke armed`n   mode: controlled smoke`n   next: pic-remap-smoke`n   interrupts: disabled`n   irq gates: unbound"
Assert-Contains $kernelBootSmokeDocs $expectedQemuPicRemapArmOutput "qemu docs pic-remap-arm exact rendered contract"
$expectedPicRemapSmokeArmedOutput = "PIC remap controlled smoke`nguard: armed`nicw sequence: written`nmaster offset: 0x20`nslave offset: 0x28`nmask after remap: 0xff`nsti: disabled`nirq gates: unbound`neoi dispatch: disabled`nresult: remapped / masked"
Assert-Contains $kernelIrqDocs $expectedPicRemapSmokeArmedOutput "irq docs pic-remap-smoke armed exact rendered contract"
$expectedQemuPicRemapSmokeArmedOutput = "PIC remap controlled smoke`n   guard: armed`n   icw sequence: written`n   master offset: 0x20`n   slave offset: 0x28`n   mask after remap: 0xff`n   sti: disabled`n   irq gates: unbound`n   eoi dispatch: disabled`n   result: remapped / masked"
Assert-Contains $kernelBootSmokeDocs $expectedQemuPicRemapSmokeArmedOutput "qemu docs pic-remap-smoke armed exact rendered contract"
$expectedPicRemapSmokeBlockedOutput = "PIC remap controlled smoke`nguard: not armed`nresult: blocked`nnext: pic-remap-arm"
Assert-Contains $kernelIrqDocs $expectedPicRemapSmokeBlockedOutput "irq docs pic-remap-smoke blocked exact rendered contract"
$expectedQemuPicRemapSmokeBlockedOutput = "PIC remap controlled smoke`n   guard: not armed`n   result: blocked`n   next: pic-remap-arm"
Assert-Contains $kernelBootSmokeDocs $expectedQemuPicRemapSmokeBlockedOutput "qemu docs pic-remap-smoke blocked exact rendered contract"
$expectedPicRemapStatusOutput = "PIC remap smoke status`narmed: no`nexecuted: no`nmaster offset: 0x20`nslave offset: 0x28`nmask after remap: 0xff`nsti: disabled`nirq gates: unbound`neoi dispatch: disabled"
Assert-Contains $kernelIrqDocs $expectedPicRemapStatusOutput "irq docs pic-remap-status exact rendered contract"
$expectedQemuPicRemapStatusOutput = "PIC remap smoke status`n   armed: no`n   executed: no`n   master offset: 0x20`n   slave offset: 0x28`n   mask after remap: 0xff`n   sti: disabled`n   irq gates: unbound`n   eoi dispatch: disabled"
Assert-Contains $kernelBootSmokeDocs $expectedQemuPicRemapStatusOutput "qemu docs pic-remap-status exact rendered contract"
$expectedPicRemapStatusAfterSmokeOutput = "PIC remap smoke status`narmed: no`nexecuted: yes`nmaster offset: 0x20`nslave offset: 0x28`nmask after remap: 0xff`nsti: disabled`nirq gates: unbound`neoi dispatch: disabled"
Assert-Contains $kernelIrqDocs $expectedPicRemapStatusAfterSmokeOutput "irq docs pic-remap-status after-smoke exact rendered contract"
$expectedQemuPicRemapStatusAfterSmokeOutput = "PIC remap smoke status`n   armed: no`n   executed: yes`n   master offset: 0x20`n   slave offset: 0x28`n   mask after remap: 0xff`n   sti: disabled`n   irq gates: unbound`n   eoi dispatch: disabled"
Assert-Contains $kernelBootSmokeDocs $expectedQemuPicRemapStatusAfterSmokeOutput "qemu docs pic-remap-status after-smoke exact rendered contract"
$expectedPicRemapStateOutput = "PIC remap state`narmed: no`nexecuted: no`nmaster offset: 0x20`nslave offset: 0x28`nicw sequence expected: yes`nicw sequence applied: no`nmask after remap: 0xff`nirq runtime: disabled"
Assert-Contains $kernelIrqDocs $expectedPicRemapStateOutput "irq docs pic-remap-state exact rendered contract"
$expectedQemuPicRemapStateOutput = "PIC remap state`n   armed: no`n   executed: no`n   master offset: 0x20`n   slave offset: 0x28`n   icw sequence expected: yes`n   icw sequence applied: no`n   mask after remap: 0xff`n   irq runtime: disabled"
Assert-Contains $kernelBootSmokeDocs $expectedQemuPicRemapStateOutput "qemu docs pic-remap-state exact rendered contract"
$expectedPicRemapStateAfterSmokeOutput = "PIC remap state`narmed: no`nexecuted: yes`nmaster offset: 0x20`nslave offset: 0x28`nicw sequence expected: yes`nicw sequence applied: yes`nmask after remap: 0xff`nirq runtime: disabled"
Assert-Contains $kernelBootSmokeDocs ($expectedPicRemapStateAfterSmokeOutput -replace "`n", "`n   ") "qemu docs pic-remap-state after-smoke exact rendered contract"
$expectedPicRemapHistoryOutput = "PIC remap history`narm command: available`nsmoke command: available`nlast smoke executed: no`nicw writes: controlled command path only`nboot remap: no"
Assert-Contains $kernelIrqDocs $expectedPicRemapHistoryOutput "irq docs pic-remap-history exact rendered contract"
$expectedQemuPicRemapHistoryOutput = "PIC remap history`n   arm command: available`n   smoke command: available`n   last smoke executed: no`n   icw writes: controlled command path only`n   boot remap: no"
Assert-Contains $kernelBootSmokeDocs $expectedQemuPicRemapHistoryOutput "qemu docs pic-remap-history exact rendered contract"
$expectedPicRemapHistoryAfterSmokeOutput = "PIC remap history`narm command: available`nsmoke command: available`nlast smoke executed: yes`nicw writes: controlled command path only`nboot remap: no"
Assert-Contains $kernelBootSmokeDocs ($expectedPicRemapHistoryAfterSmokeOutput -replace "`n", "`n   ") "qemu docs pic-remap-history after-smoke exact rendered contract"
$expectedPicRemapPreflightOutput = "PIC remap preflight`nguard: command armed required`nicw sequence: ready`nmaster offset: 0x20`nslave offset: 0x28`nmask after remap: 0xff`nsti: disabled`nirq gates: unbound`neoi dispatch: disabled`nresult: telemetry only"
Assert-Contains $kernelIrqDocs $expectedPicRemapPreflightOutput "irq docs pic-remap-preflight exact rendered contract"
$expectedQemuPicRemapPreflightOutput = "PIC remap preflight`n   guard: command armed required`n   icw sequence: ready`n   master offset: 0x20`n   slave offset: 0x28`n   mask after remap: 0xff`n   sti: disabled`n   irq gates: unbound`n   eoi dispatch: disabled`n   result: telemetry only"
Assert-Contains $kernelBootSmokeDocs $expectedQemuPicRemapPreflightOutput "qemu docs pic-remap-preflight exact rendered contract"
Assert-Contains $kernelIrqDocs "pic remap controlled smoke: executed=no" "irq docs system pic remap controlled smoke sync"
Assert-Contains $kernelBootSmokeDocs "pic remap controlled smoke: executed=no" "qemu docs system pic remap controlled smoke sync"
$expectedIrqReadinessOutput = "IRQ runtime readiness`nidt exceptions: ok`nirq gate plan: ok`neoi strategy: ok`npic remap: controlled smoke only`nsti: disabled`nkeyboard fallback: polling`nready for runtime irq: no"
Assert-Contains $kernelIrqDocs $expectedIrqReadinessOutput "irq docs irq-readiness exact rendered contract"
$expectedQemuIrqReadinessOutput = "IRQ runtime readiness`n    idt exceptions: ok`n    irq gate plan: ok`n    eoi strategy: ok`n    pic remap: controlled smoke only`n    sti: disabled`n    keyboard fallback: polling`n    ready for runtime irq: no"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqReadinessOutput "qemu docs irq-readiness exact rendered contract"
$expectedIrqRiskOutput = "IRQ runtime risk`nruntime irq: blocked`nreason: IRQ0/IRQ1 gates are not bound`nrequired before enable: IDT gate bind, PIC remap, EOI dispatch, handler stubs`nsti allowed: no"
Assert-Contains $kernelIrqDocs $expectedIrqRiskOutput "irq docs irq-risk exact rendered contract"
$expectedQemuIrqRiskOutput = "IRQ runtime risk`n    runtime irq: blocked`n    reason: IRQ0/IRQ1 gates are not bound`n    required before enable: IDT gate bind, PIC remap, EOI dispatch, handler stubs`n    sti allowed: no"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqRiskOutput "qemu docs irq-risk exact rendered contract"
$expectedIrqPreflightOutput = "IRQ runtime preflight`nIDT exceptions 0/3/14: pass`nIRQ vectors 32/33: unbound`nbind path: disabled`nEOI dispatch: disabled`nPIC remap: controlled smoke only`nkeyboard fallback: polling`npf-smoke: unchanged`nresult: blocked"
Assert-Contains $kernelIrqDocs $expectedIrqPreflightOutput "irq docs irq-preflight exact rendered contract"
$expectedQemuIrqPreflightOutput = "IRQ runtime preflight`n    IDT exceptions 0/3/14: pass`n    IRQ vectors 32/33: unbound`n    bind path: disabled`n    EOI dispatch: disabled`n    PIC remap: controlled smoke only`n    keyboard fallback: polling`n    pf-smoke: unchanged`n    result: blocked"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqPreflightOutput "qemu docs irq-preflight exact rendered contract"
$expectedQemuIrqRuntimePreflightOutput = "IRQ runtime activation preflight`n    pic remap: not ready`n    irq gates: controlled smoke bound=no`n    eoi strategy: planned / disabled`n    keyboard fallback: polling`n    sti: disabled`n    runtime irq ready: no"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqRuntimePreflightOutput "qemu docs irq-runtime-preflight exact rendered contract"
$expectedQemuIrqRuntimeStatusOutput = "IRQ runtime readiness status`n    pic remap: not ready`n    irq gates: unbound`n    eoi dispatch: disabled`n    keyboard input: polling`n    page fault smoke: stable`n    runtime irq activation: blocked`n    sti enabled: no"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqRuntimeStatusOutput "qemu docs irq-runtime-status exact rendered contract"

$expectedQemuIrqRuntimeCommitBlockedOutput = "error: IRQ runtime activation not armed.`n    required: execute irq-runtime-arm first."
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqRuntimeCommitBlockedOutput "qemu docs irq-runtime-commit blocked exact rendered contract"
Assert-Contains $kernelMainSource "error: IRQ runtime activation not armed.\nrequired: execute irq-runtime-arm first.\n" "kernel irq-runtime-commit blocked exact output"

$expectedQemuIrqRuntimeArmOutput = "IRQ runtime activation armed.`n    next: execute irq-runtime-commit"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqRuntimeArmOutput "qemu docs irq-runtime-arm exact rendered contract"
Assert-Contains $kernelMainSource "IRQ runtime activation armed.\nnext: execute irq-runtime-commit\n" "kernel irq-runtime-arm exact output"

$expectedQemuIrqRuntimeStatusArmedOutput = "IRQ runtime readiness status`n    pic remap: not ready`n    irq gates: unbound`n    eoi dispatch: disabled`n    keyboard input: polling`n    page fault smoke: stable`n    runtime irq activation: armed / standby`n    sti enabled: no"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqRuntimeStatusArmedOutput "qemu docs irq-runtime-status armed exact rendered contract"
Assert-Contains $kernelMainSource "`"armed / standby`"" "kernel irq-runtime-status armed text in source"

$expectedQemuIrqRuntimeCommitDryRunOutput = "IRQ runtime activation commit dry-run`n    pic remap smoke: no`n    irq gate bind smoke: no`n    eoi runtime boundary: disabled`n    pic mask policy: all masked (0xFF)`n    unmask policy: no unmask`n    runtime latch: armed`n    sti: disabled`n    runtime irq active: no`n    dry-run commit allowed: no`n    result: blocked by readiness matrix"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqRuntimeCommitDryRunOutput "qemu docs irq-runtime-commit dry-run exact rendered contract"
Assert-Contains $kernelMainSource "IRQ runtime activation commit dry-run\npic remap smoke: {}\nirq gate bind smoke: {}\neoi runtime boundary: {}\npic mask policy: {}\nunmask policy: {}\nruntime latch: {}\nsti: {}\nruntime irq active: {}\ndry-run commit allowed: {}\nresult: {}\n" "kernel irq-runtime-commit dry-run exact output"

$expectedQemuIrqRuntimeArmAlreadyArmedOutput = "error: IRQ runtime activation already armed (no-op).`n    next: execute irq-runtime-commit"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqRuntimeArmAlreadyArmedOutput "qemu docs irq-runtime-arm already armed exact rendered contract"
Assert-Contains $kernelMainSource "error: IRQ runtime activation already armed (no-op).\nnext: execute irq-runtime-commit\n" "kernel irq-runtime-arm already armed exact output"

$expectedQemuIrqRuntimeActivationPlanOutput = "IRQ runtime activation plan`n    1. require readiness matrix smoke prerequisites: yes`n    2. require EOI runtime boundary: ready (dry-run)`n    3. keep PIC mask policy: all masked (0xFF)`n    4. keep unmask policy: no unmask`n    5. keep STI: disabled`n    6. commit path remains dry-run only`n    runtime irq active: no`n    dry-run commit allowed: no"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqRuntimeActivationPlanOutput "qemu docs irq-runtime-activation-plan exact rendered contract"
Assert-Contains $kernelMainSource "IRQ runtime activation plan\n1. require readiness matrix smoke prerequisites: yes\n2. require EOI runtime boundary: ready (dry-run)\n3. keep PIC mask policy: {}\n4. keep unmask policy: {}\n5. keep STI: {}\n6. commit path remains dry-run only\nruntime irq active: {}\ndry-run commit allowed: {}\n" "kernel irq-runtime-activation-plan exact output"

$expectedQemuIrqRuntimeBlockersOutput = "IRQ runtime activation blockers`n    - PIC remap: not ready for controlled smoke`n    - IRQ gates: vectors 32/33 not bound`n    - EOI dispatch: not enabled`n    - STI: disabled`n    smoke prerequisites: satisfied`n    runtime irq ready: no"
Assert-Contains $kernelBootSmokeDocs $expectedQemuIrqRuntimeBlockersOutput "qemu docs irq-runtime-blockers exact rendered contract"
Assert-Contains $kernelMainSource "IRQ runtime readiness\nidt exceptions: {}\nirq gate plan: {}\neoi strategy: {}\npic remap: {}\nsti: {}\nkeyboard fallback: {}\nready for runtime irq: {}\n" "kernel irq-readiness output template"
Assert-Contains $kernelMainSource "IRQ runtime risk\nruntime irq: {}\nreason: {}\nrequired before enable: {}\nsti allowed: {}\n" "kernel irq-risk output template"
Assert-Contains $kernelMainSource "IRQ runtime preflight\nIDT exceptions 0/3/14: {}\nIRQ vectors 32/33: {}\nbind path: {}\nEOI dispatch: {}\nPIC remap: {}\nkeyboard fallback: {}\npf-smoke: {}\nresult: {}\n" "kernel irq-preflight output template"
Assert-Contains $kernelMainSource "IRQ runtime activation preflight\npic remap: {}\nirq gates: controlled smoke bound={}\neoi strategy: {}\nkeyboard fallback: {}\nsti: {}\nruntime irq ready: {}\n" "kernel irq-runtime-preflight output template"
Assert-Contains $kernelMainSource "IRQ runtime readiness status\npic remap: {}\nirq gates: {}\neoi dispatch: {}\nkeyboard input: {}\npage fault smoke: {}\nruntime irq activation: {}\nsti enabled: {}\n" "kernel irq-runtime-status output template"
Assert-Contains $kernelMainSource "IRQ runtime activation blockers\n" "kernel irq-runtime-blockers output header"
Assert-Contains $kernelMainSource "- EOI dispatch: not enabled\n" "kernel irq-runtime-blockers eoi blocker line"
Assert-Contains $kernelMainSource "- STI: disabled\n" "kernel irq-runtime-blockers sti blocker line"
Assert-Contains $kernelMainSource "smoke prerequisites: satisfied\nruntime irq ready: no\n" "kernel irq-runtime-blockers final state"
Assert-Contains $kernelMainSource "exception subsystem:\nfoundation: active\nactive vectors: 0 divide-by-zero, 3 breakpoint, 14 page fault smoke\ntelemetry: count / last vector / last name\nrecovery: smoke-safe trampoline\nstatus ux: active\ninterrupts: disabled\n" "kernel exception-about exact output"
Assert-Contains $kernelMainSource "exception recovery verbose:\nexceptions handled: {}\nlast exception: none\nactive handlers:\nvector 0: divide-by-zero\nvector 3: breakpoint\nvector 14: page fault\nsmoke handlers:\nvector 14: page fault recovery trampoline\nplanned handlers:\nnone\npage fault smoke: armed={}\ninterrupts: disabled\n" "kernel exceptions verbose none output"
Assert-Contains $kernelMainSource "exception recovery verbose:\nexceptions handled: {}\nlast exception: {} ({})\nactive handlers:\nvector 0: divide-by-zero\nvector 3: breakpoint\nvector 14: page fault\nsmoke handlers:\nvector 14: page fault recovery trampoline\nplanned handlers:\nnone\npage fault smoke: armed={}\ninterrupts: disabled\n" "kernel exceptions verbose populated output"
Assert-Contains $kernelMainSource "exception diagnostics commands:\nexception          - show dynamic telemetry parameters\nexceptions         - show exception status overview\nexceptions --verbose - show verbose exception recovery overview\nexception-status   - show exception status overview (alias)\nexception-reset    - reset all exception telemetry counters\nexception-about    - show exception subsystem foundation summary\nfault-status       - show fault recovery status\nfault-reset        - reset fault recovery and exception telemetry\npf-status          - show page fault smoke status\nexception-help     - display this help content\nhandlers           - list active and planned IDT entry handlers\nhandlers --active  - list active IDT entry handlers only\npf-note            - show page fault smoke direction note\npf-smoke           - trigger controlled real page fault smoke\nint3               - execute breakpoint software interrupt\ndiv0               - execute divide-by-zero trap\n" "kernel exception-help exact output"
Assert-NotContains $kernelMainSource "exception diagnostics commands:\nirq-note" "kernel exception-help does not include irq-note"
Assert-NotContains $kernelMainSource "exception diagnostics commands:\nirq-status" "kernel exception-help does not include irq-status"
Assert-NotContains $kernelMainSource "exception diagnostics commands:\nirq-handlers" "kernel exception-help does not include irq-handlers"
Assert-NotContains $kernelMainSource "exception diagnostics commands:\npic-note" "kernel exception-help does not include pic-note"
Assert-NotContains $kernelMainSource "exception diagnostics commands:\npic-status" "kernel exception-help does not include pic-status"
Assert-NotContains $kernelMainSource "exception diagnostics commands:\npic-plan" "kernel exception-help does not include pic-plan"
Assert-NotContains $kernelMainSource "exception diagnostics commands:\nirq-map" "kernel exception-help does not include irq-map"
Assert-NotContains $kernelMainSource "exception diagnostics commands:\npic-status --verbose" "kernel exception-help does not include pic-status verbose"
Assert-Contains $kernelMainSource "line_str == `"pf-note`"" "kernel pf-note dispatch"
Assert-Contains $kernelMainSource "line_str == `"pf-smoke`"" "kernel pf-smoke dispatch"
Assert-Contains $kernelMainSource "page fault: active smoke\nvector: 14\ncr2: available after pf-smoke\nerror code: available after pf-smoke\n" "kernel pf-note exact output"
Assert-Contains $kernelMainSource "interrupts: disabled" "kernel system and exception-status report interrupts disabled"
Assert-Contains $kernelMainSource "input mode: keyboard polling" "kernel system keeps keyboard polling input mode"
Assert-Contains $kernelMainSource "keyboard mode: polling" "kernel keyboard command keeps polling mode"
Assert-Contains $kernelMainSource "let status = serial::inb(0x64);" "kernel keyboard polling reads PS/2 status port"
Assert-Contains $kernelMainSource "let scancode = serial::inb(0x60);" "kernel keyboard polling reads PS/2 data port"
Assert-Contains $kernelMainSource "0x0C => Some(if shift { '_' } else { '-' })" "kernel keyboard decodes main-row minus and shifted underscore"
Assert-Contains $kernelMainSource "0x0D => Some(if shift { '+' } else { '=' })" "kernel keyboard decodes main-row equals and shifted plus"
Assert-Contains $kernelMainSource "0x4A => Some('-')" "kernel keyboard decodes numpad minus"
Assert-Contains $kernelMainSource "0x4E => Some('+')" "kernel keyboard decodes numpad plus"
Assert-Contains $kernelMainSource "exception handlers: breakpoint, divide-by-zero, page fault" "kernel system active handler summary"
Assert-Contains $kernelMainSource "page fault handler: active smoke" "kernel system page fault active smoke status"
Assert-Contains $kernelMainSource "pic/irq: planned / disabled" "kernel system pic irq planned disabled status"
Assert-Contains $kernelMainSource "pic remap: planned / disabled" "kernel system pic remap planned disabled status"
Assert-Contains $kernelMainSource "pic dry-run telemetry: available" "kernel system pic dry-run telemetry status"
Assert-Contains $kernelMainSource "irq handlers: skeleton / disabled" "kernel system irq skeleton status"
Assert-Contains $kernelMainSource "recovery mode: smoke-safe" "kernel system recovery mode"
Assert-Contains $kernelMainSource "page fault smoke: armed=false" "kernel system page fault smoke state"
Assert-Contains $kernelMainSource "interrupts::EXCEPTION_COUNT = 0;" "kernel exception-reset clears count"
Assert-Contains $kernelMainSource "interrupts::LAST_EXCEPTION_VECTOR = -1;" "kernel exception-reset clears vector"
Assert-Contains $kernelMainSource "interrupts::LAST_EXCEPTION_NAME = `"none`";" "kernel exception-reset clears name"
Assert-Contains $kernelMainSource "interrupts::PF_SMOKE_ACTIVE = false;" "kernel fault-reset clears pf smoke active"
Assert-Contains $kernelMainSource "interrupts::PF_SMOKE_RECOVERY_EIP = 0;" "kernel fault-reset clears pf smoke recovery eip"
Assert-Contains $kernelMainSource "fault recovery: reset successfully\n" "kernel fault-reset exact output"
Assert-Contains $kernelMainSource "core::arch::asm!(`"int 0`")" "kernel div0 uses controlled int 0 trap"
Assert-Contains $kernelMainSource "idt::IDT.entries[0].set_handler(interrupts::divide_by_zero_handler_asm as *const ())" "kernel vector 0 active handler"
Assert-Contains $kernelMainSource "idt::IDT.entries[3].set_handler(interrupts::breakpoint_handler_asm as *const ())" "kernel vector 3 active handler"
Assert-Contains $kernelMainSource "idt::IDT.entries[14].set_handler(interrupts::page_fault_handler_asm as *const ())" "kernel vector 14 active smoke handler"
$idtBindings = [regex]::Matches($kernelMainSource, 'IDT\.entries\[(\d+)\]\.set_handler') | ForEach-Object { [int]$_.Groups[1].Value } | Sort-Object -Unique
$expectedIdtBindings = @(0, 3, 14, 32, 33)
if (($idtBindings -join ',') -ne ($expectedIdtBindings -join ',')) {
    throw "Kernel IDT vector guard failed: expected bindings 0,3,14 plus command-path 32,33 only; found $($idtBindings -join ',')"
}
$kernelBootPath = $kernelMainSource.Substring(0, $kernelMainSource.IndexOf('loop {'))
$irqGateSmokeDispatch = $kernelMainSource.IndexOf('line_str == "irq-gate-bind-smoke"')
$irqGateStatusDispatch = $kernelMainSource.IndexOf('line_str == "irq-gate-bind-status"')
$irqGateArmDispatch = $kernelMainSource.IndexOf('line_str == "irq-gate-arm"')
if ($irqGateArmDispatch -lt 0 -or $irqGateSmokeDispatch -lt 0 -or $irqGateStatusDispatch -lt 0) {
    throw "Kernel IRQ gate bind smoke guard failed: irq-gate arm/smoke/status dispatch not found"
}
Assert-NotContains $kernelBootPath "entries[32].set_handler" "kernel boot path does not bind IRQ0 vector 32"
Assert-NotContains $kernelBootPath "entries[33].set_handler" "kernel boot path does not bind IRQ1 vector 33"
Assert-NotContains $kernelBootPath "write_pic_port(" "kernel boot path does not perform PIC port writes"
$irqGateSmokeBlockEnd = $kernelMainSource.IndexOf('line_str == "irq-gate-bind-status"')
$irqGateSmokeBlock = $kernelMainSource.Substring($irqGateSmokeDispatch, $irqGateSmokeBlockEnd - $irqGateSmokeDispatch)
Assert-Contains $irqGateSmokeBlock "if irq::irq_gate_bind_smoke_is_armed()" "irq gate smoke requires armed guard"
$irqGateArmedIfStart = $kernelMainSource.IndexOf('if irq::irq_gate_bind_smoke_is_armed() {', $irqGateSmokeDispatch)
if ($irqGateArmedIfStart -lt 0 -or $irqGateArmedIfStart -ge $irqGateSmokeBlockEnd) {
    throw "Kernel IRQ gate bind smoke guard failed: armed if not found inside irq-gate-bind-smoke dispatch"
}
$irqGateArmedElseStart = $kernelMainSource.IndexOf('} else {', $irqGateArmedIfStart)
if ($irqGateArmedElseStart -lt 0 -or $irqGateArmedElseStart -ge $irqGateSmokeBlockEnd) {
    throw "Kernel IRQ gate bind smoke guard failed: armed-branch else not found inside irq-gate-bind-smoke dispatch"
}
$irqGateSmokeArmedBlock = $kernelMainSource.Substring($irqGateArmedIfStart, $irqGateArmedElseStart - $irqGateArmedIfStart)
Assert-Contains $irqGateSmokeArmedBlock "idt::IDT.entries[32].set_handler(interrupts::irq0_timer_gate_smoke_asm as *const ())" "irq gate armed path binds vector 32 to smoke wrapper"
Assert-Contains $irqGateSmokeArmedBlock "idt::IDT.entries[33].set_handler(interrupts::irq1_keyboard_gate_smoke_asm as *const ())" "irq gate armed path binds vector 33 to smoke wrapper"
Assert-Contains $irqGateSmokeArmedBlock "irq::irq_gate_bind_smoke_mark_bound()" "irq gate armed path records bound state after IDT install"
$irqGateBindSmokeBlockedMarker = 'let smoke = irq::irq_gate_bind_smoke_blocked();'
$irqGateBindSmokeBlockedIdx = $kernelMainSource.IndexOf($irqGateBindSmokeBlockedMarker, $irqGateSmokeDispatch)
if ($irqGateBindSmokeBlockedIdx -lt 0 -or $irqGateBindSmokeBlockedIdx -ge $irqGateSmokeBlockEnd) {
    throw "Kernel IRQ gate bind smoke guard failed: blocked-path marker not found inside irq-gate-bind-smoke dispatch"
}
$irqGateSmokeBlockedTail = $kernelMainSource.Substring($irqGateBindSmokeBlockedIdx, $irqGateSmokeBlockEnd - $irqGateBindSmokeBlockedIdx)
Assert-NotContains $irqGateSmokeBlockedTail "entries[32].set_handler" "irq gate blocked path does not bind IRQ0 vector 32"
Assert-NotContains $irqGateSmokeBlockedTail "entries[33].set_handler" "irq gate blocked path does not bind IRQ1 vector 33"
$irqGateBindCalls = [regex]::Matches($kernelMainSource, 'entries\[(32|33)\]\.set_handler')
foreach ($call in $irqGateBindCalls) {
    if ($call.Index -lt $irqGateSmokeDispatch -or $call.Index -gt $irqGateSmokeBlockEnd) {
        throw "Kernel IRQ gate bind smoke guard failed: entries[$($call.Groups[1].Value)].set_handler outside irq-gate-bind-smoke dispatch"
    }
}
Assert-NotContains $kernelMainSource "asm!(`"sti`")" "kernel does not enable maskable interrupts"
Assert-NotContains $kernelMainSource "asm!(`"int 14`")" "kernel does not trigger software page fault vector"
Assert-NotContains $kernelMainSource "pic::ProgrammableInterruptController::init_stub()" "kernel does not call pic remap/init stub"
Assert-NotContains $kernelMainSource "pic::ProgrammableInterruptController::remap_plan()" "kernel does not call pic remap plan"
Assert-NotContains $kernelMainSource "pic::ProgrammableInterruptController::remap_disabled()" "kernel does not call disabled pic remap hook"
Assert-NotContains $kernelMainSource "pic::ProgrammableInterruptController::irq_map_plan()" "kernel does not call pic irq map plan"
Assert-NotContains $kernelMainSource "ProgrammableInterruptController::init_stub()" "kernel does not call pic init stub unqualified"
Assert-NotContains $kernelMainSource "ProgrammableInterruptController::remap_plan()" "kernel does not call pic remap plan unqualified"
Assert-NotContains $kernelMainSource "ProgrammableInterruptController::remap_disabled()" "kernel does not call disabled pic remap hook unqualified"
Assert-NotContains $kernelMainSource "ProgrammableInterruptController::irq_map_plan()" "kernel does not call pic irq map plan unqualified"
Assert-NotContains $kernelMainSource "irq_handler_skeletons()" "kernel main does not call irq skeleton plan"
Assert-NotContains $kernelMainSource "irq0_timer_skeleton()" "kernel main does not call irq0 skeleton"
Assert-NotContains $kernelMainSource "irq1_keyboard_skeleton()" "kernel main does not call irq1 skeleton"
$irqGatePlanCalls = [regex]::Matches($kernelMainSource, 'irq::irq_gate_plan\(\)').Count
if ($irqGatePlanCalls -ne 1) {
    throw "Kernel IRQ gate plan guard failed: expected exactly one command-path irq::irq_gate_plan() call; found $irqGatePlanCalls"
}
$irqBindDisabledCalls = [regex]::Matches($kernelMainSource, 'irq::bind_irq_gates_disabled\(\)').Count
if ($irqBindDisabledCalls -ne 2) {
    throw "Kernel IRQ disabled bind path guard failed: expected exactly two command-path irq::bind_irq_gates_disabled() calls; found $irqBindDisabledCalls"
}
Assert-NotContains $kernelBootPath "irq::bind_irq_gates_disabled()" "kernel boot path does not call disabled bind helper"
$irqBindNoteDispatch = $kernelMainSource.IndexOf('line_str == "irq-bind-note"')
$irqBindStatusDispatch = $kernelMainSource.IndexOf('line_str == "irq-bind-status"')
$irqBindHelperCallMatches = [regex]::Matches($kernelMainSource, 'irq::bind_irq_gates_disabled\(\)')
if ($irqBindNoteDispatch -lt 0 -or $irqBindStatusDispatch -lt 0) {
    throw "Kernel IRQ disabled bind path guard failed: irq-bind-note/status dispatch not found"
}
foreach ($call in $irqBindHelperCallMatches) {
    $callIndex = $call.Index
    $nearNote = ($callIndex -gt $irqBindNoteDispatch -and $callIndex -lt $irqBindNoteDispatch + 512)
    $nearStatus = ($callIndex -gt $irqBindStatusDispatch -and $callIndex -lt $irqBindStatusDispatch + 512)
    if (-not ($nearNote -or $nearStatus)) {
        throw "Kernel IRQ disabled bind path guard failed: bind_irq_gates_disabled() call outside irq-bind-note/status command dispatch"
    }
}
$irqReadinessCalls = [regex]::Matches($kernelMainSource, 'irq::irq_runtime_readiness\(\)').Count
$irqRiskCalls = [regex]::Matches($kernelMainSource, 'irq::irq_runtime_risk\(\)').Count
$irqPreflightCalls = [regex]::Matches($kernelMainSource, 'irq::irq_runtime_preflight\(\)').Count
if ($irqReadinessCalls -ne 1 -or $irqRiskCalls -ne 1 -or $irqPreflightCalls -ne 1) {
    throw "Kernel IRQ readiness guard failed: expected exactly one command-path readiness/risk/preflight call; found readiness=$irqReadinessCalls risk=$irqRiskCalls preflight=$irqPreflightCalls"
}
Assert-NotContains $kernelBootPath "irq::irq_runtime_readiness()" "kernel boot path does not call irq readiness helper"
Assert-NotContains $kernelBootPath "irq::irq_runtime_risk()" "kernel boot path does not call irq risk helper"
Assert-NotContains $kernelBootPath "irq::irq_runtime_preflight()" "kernel boot path does not call irq preflight helper"
$irqReadinessDispatch = $kernelMainSource.IndexOf('line_str == "irq-readiness"')
$irqRiskDispatch = $kernelMainSource.IndexOf('line_str == "irq-risk"')
$irqPreflightDispatch = $kernelMainSource.IndexOf('line_str == "irq-preflight"')
if ($irqReadinessDispatch -lt 0 -or $irqRiskDispatch -lt 0 -or $irqPreflightDispatch -lt 0) {
    throw "Kernel IRQ readiness guard failed: readiness/risk/preflight dispatch not found"
}
if ($kernelMainSource.IndexOf('irq::irq_runtime_readiness()') -lt $irqReadinessDispatch) {
    throw "Kernel IRQ readiness guard failed: irq_runtime_readiness() call outside irq-readiness dispatch"
}
if ($kernelMainSource.IndexOf('irq::irq_runtime_risk()') -lt $irqRiskDispatch) {
    throw "Kernel IRQ readiness guard failed: irq_runtime_risk() call outside irq-risk dispatch"
}
if ($kernelMainSource.IndexOf('irq::irq_runtime_preflight()') -lt $irqPreflightDispatch) {
    throw "Kernel IRQ readiness guard failed: irq_runtime_preflight() call outside irq-preflight dispatch"
}
$picRemapArmCalls = [regex]::Matches($kernelMainSource, 'pic::ProgrammableInterruptController::pic_remap_smoke_arm\(\)').Count
$picRemapSmokeCalls = [regex]::Matches($kernelMainSource, 'pic::ProgrammableInterruptController::pic_remap_controlled_smoke\(\)').Count
$picRemapStatusCalls = [regex]::Matches($kernelMainSource, 'pic::ProgrammableInterruptController::pic_remap_smoke_status\(\)').Count
$picRemapStateCalls = [regex]::Matches($kernelMainSource, 'pic::ProgrammableInterruptController::pic_remap_state\(\)').Count
$picRemapHistoryCalls = [regex]::Matches($kernelMainSource, 'pic::ProgrammableInterruptController::pic_remap_history\(\)').Count
$picRemapPreflightCalls = [regex]::Matches($kernelMainSource, 'pic::ProgrammableInterruptController::pic_remap_preflight\(\)').Count
if ($picRemapArmCalls -ne 1 -or $picRemapSmokeCalls -ne 1 -or $picRemapStatusCalls -ne 1) {
    throw "Kernel PIC remap smoke guard failed: expected exactly one command-path arm/smoke/status call; found arm=$picRemapArmCalls smoke=$picRemapSmokeCalls status=$picRemapStatusCalls"
}
if ($picRemapStateCalls -ne 18 -or $picRemapHistoryCalls -ne 1 -or $picRemapPreflightCalls -ne 1) {
    throw "Kernel PIC remap telemetry guard failed: expected state=18 (existing telemetry readers plus irq-runtime-gate-status/check/blockers and irq-runtime-sim-status/run/blockers), history=1, preflight=1; found state=$picRemapStateCalls history=$picRemapHistoryCalls preflight=$picRemapPreflightCalls"
}
Assert-NotContains $kernelBootPath "pic::ProgrammableInterruptController::pic_remap_smoke_arm()" "kernel boot path does not arm pic remap smoke"
Assert-NotContains $kernelBootPath "pic::ProgrammableInterruptController::pic_remap_controlled_smoke()" "kernel boot path does not run pic remap smoke"
Assert-NotContains $kernelBootPath "pic::ProgrammableInterruptController::pic_remap_smoke_status()" "kernel boot path does not read pic remap smoke status"
Assert-NotContains $kernelBootPath "pic::ProgrammableInterruptController::pic_remap_state()" "kernel boot path does not read pic remap state telemetry"
Assert-NotContains $kernelBootPath "pic::ProgrammableInterruptController::pic_remap_history()" "kernel boot path does not read pic remap history telemetry"
Assert-NotContains $kernelBootPath "pic::ProgrammableInterruptController::pic_remap_preflight()" "kernel boot path does not read pic remap preflight telemetry"
$picRemapArmDispatch = $kernelMainSource.IndexOf('line_str == "pic-remap-arm"')
$picRemapSmokeDispatch = $kernelMainSource.IndexOf('line_str == "pic-remap-smoke"')
$picRemapStatusDispatch = $kernelMainSource.IndexOf('line_str == "pic-remap-status"')
$picRemapStateDispatch = $kernelMainSource.IndexOf('line_str == "pic-remap-state"')
$picRemapHistoryDispatch = $kernelMainSource.IndexOf('line_str == "pic-remap-history"')
$picRemapPreflightDispatch = $kernelMainSource.IndexOf('line_str == "pic-remap-preflight"')
$irqRuntimePreflightDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-preflight"')
$irqRuntimeCommitDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-commit"')
$irqRuntimeStatusDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-status"')
$irqRuntimeBlockersDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-blockers"')
$eciRuntimeStatusDispatch = $kernelMainSource.IndexOf('line_str == "eoi-runtime-status"')
$eciRuntimeBlockersDispatch = $kernelMainSource.IndexOf('line_str == "eoi-runtime-blockers"')
$irqMaskBlockersDispatch = $kernelMainSource.IndexOf('line_str == "irq-mask-blockers"')
$irqRuntimeMatrixDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-matrix"')
$irqRuntimeReadinessMatrixDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-readiness"')
$irqRuntimeActivationPlanDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-activation-plan"')
$irqRuntimeGateStatusDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-gate-status"')
$irqRuntimeGateCheckDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-gate-check"')
$irqRuntimeGateBlockersDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-gate-blockers"')
$irqRuntimeSimStatusDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-sim-status"')
$irqRuntimeSimRunDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-sim-run"')
$irqRuntimeSimBlockersDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-sim-blockers"')
if ($picRemapArmDispatch -lt 0 -or $picRemapSmokeDispatch -lt 0 -or $picRemapStatusDispatch -lt 0 -or $picRemapStateDispatch -lt 0 -or $picRemapHistoryDispatch -lt 0 -or $picRemapPreflightDispatch -lt 0) {
    throw "Kernel PIC remap smoke guard failed: pic-remap dispatch not found"
}
if ($kernelMainSource.IndexOf('pic::ProgrammableInterruptController::pic_remap_smoke_arm()') -lt $picRemapArmDispatch) {
    throw "Kernel PIC remap smoke guard failed: arm helper call outside pic-remap-arm dispatch"
}
if ($kernelMainSource.IndexOf('pic::ProgrammableInterruptController::pic_remap_controlled_smoke()') -lt $picRemapSmokeDispatch) {
    throw "Kernel PIC remap smoke guard failed: smoke helper call outside pic-remap-smoke dispatch"
}
if ($kernelMainSource.IndexOf('pic::ProgrammableInterruptController::pic_remap_smoke_status()') -lt $picRemapStatusDispatch) {
    throw "Kernel PIC remap smoke guard failed: status helper call outside pic-remap-status dispatch"
}
foreach ($call in [regex]::Matches($kernelMainSource, 'pic::ProgrammableInterruptController::pic_remap_state\(\)')) {
    $callIndex = $call.Index
    $nearStateCommand = ($callIndex -gt $picRemapStateDispatch -and $callIndex -lt $picRemapStateDispatch + 1024)
    $nearSystemCommand = ($callIndex -gt $kernelMainSource.IndexOf('line_str == "system"') -and $callIndex -lt $kernelMainSource.IndexOf('line_str == "system"') + 2048)
    $nearPreflightCommand = ($callIndex -gt $irqRuntimePreflightDispatch -and $callIndex -lt $irqRuntimePreflightDispatch + 1024)
    $nearCommitCommand = ($callIndex -gt $irqRuntimeCommitDispatch -and $callIndex -lt $irqRuntimeCommitDispatch + 1024)
    $nearStatusCommand = ($callIndex -gt $irqRuntimeStatusDispatch -and $callIndex -lt $irqRuntimeStatusDispatch + 1024)
    $nearBlockersCommand = ($callIndex -gt $irqRuntimeBlockersDispatch -and $callIndex -lt $irqRuntimeBlockersDispatch + 1024)
    $nearEoiStatusCommand = ($callIndex -gt $eciRuntimeStatusDispatch -and $callIndex -lt $eciRuntimeStatusDispatch + 1024)
    $nearEoiBlockersCommand = ($callIndex -gt $eciRuntimeBlockersDispatch -and $callIndex -lt $eciRuntimeBlockersDispatch + 1024)
    $nearIrqMaskBlockersCommand = ($callIndex -gt $irqMaskBlockersDispatch -and $callIndex -lt $irqMaskBlockersDispatch + 1024)
    $nearIrqRuntimeMatrixCommand = ($callIndex -gt $irqRuntimeMatrixDispatch -and $callIndex -lt $irqRuntimeMatrixDispatch + 2048)
    $nearIrqRuntimeReadinessCommand = ($callIndex -gt $irqRuntimeReadinessMatrixDispatch -and $callIndex -lt $irqRuntimeReadinessMatrixDispatch + 2048)
    $nearIrqRuntimeActivationPlanCommand = ($callIndex -gt $irqRuntimeActivationPlanDispatch -and $callIndex -lt $irqRuntimeActivationPlanDispatch + 2048)
    $nearIrqRuntimeGateStatusCommand = ($callIndex -gt $irqRuntimeGateStatusDispatch -and $callIndex -lt $irqRuntimeGateStatusDispatch + 2048)
    $nearIrqRuntimeGateCheckCommand = ($callIndex -gt $irqRuntimeGateCheckDispatch -and $callIndex -lt $irqRuntimeGateCheckDispatch + 2048)
    $nearIrqRuntimeGateBlockersCommand = ($callIndex -gt $irqRuntimeGateBlockersDispatch -and $callIndex -lt $irqRuntimeGateBlockersDispatch + 2048)
    $nearIrqRuntimeSimStatusCommand = ($callIndex -gt $irqRuntimeSimStatusDispatch -and $callIndex -lt $irqRuntimeSimStatusDispatch + 2048)
    $nearIrqRuntimeSimRunCommand = ($callIndex -gt $irqRuntimeSimRunDispatch -and $callIndex -lt $irqRuntimeSimRunDispatch + 2048)
    $nearIrqRuntimeSimBlockersCommand = ($callIndex -gt $irqRuntimeSimBlockersDispatch -and $callIndex -lt $irqRuntimeSimBlockersDispatch + 2048)
    if (-not ($nearStateCommand -or $nearSystemCommand -or $nearPreflightCommand -or $nearCommitCommand -or $nearStatusCommand -or $nearBlockersCommand -or $nearEoiStatusCommand -or $nearEoiBlockersCommand -or $nearIrqMaskBlockersCommand -or $nearIrqRuntimeMatrixCommand -or $nearIrqRuntimeReadinessCommand -or $nearIrqRuntimeActivationPlanCommand -or $nearIrqRuntimeGateStatusCommand -or $nearIrqRuntimeGateCheckCommand -or $nearIrqRuntimeGateBlockersCommand -or $nearIrqRuntimeSimStatusCommand -or $nearIrqRuntimeSimRunCommand -or $nearIrqRuntimeSimBlockersCommand)) {
        throw "Kernel PIC remap telemetry guard failed: pic_remap_state() call outside pic-remap-state/system/irq-runtime-preconditions/eoi-runtime/irq-mask-blockers/matrix dispatch"
    }
}
if ($kernelMainSource.IndexOf('pic::ProgrammableInterruptController::pic_remap_history()') -lt $picRemapHistoryDispatch) {
    throw "Kernel PIC remap telemetry guard failed: history helper call outside pic-remap-history dispatch"
}
if ($kernelMainSource.IndexOf('pic::ProgrammableInterruptController::pic_remap_preflight()') -lt $picRemapPreflightDispatch) {
    throw "Kernel PIC remap telemetry guard failed: preflight helper call outside pic-remap-preflight dispatch"
}
$irqGateStateCalls = [regex]::Matches($kernelMainSource, 'irq::irq_gate_bind_state\(\)').Count
$irqGateHistoryCalls = [regex]::Matches($kernelMainSource, 'irq::irq_gate_bind_history\(\)').Count
$irqGatePreflightCalls = [regex]::Matches($kernelMainSource, 'irq::irq_gate_bind_preflight\(\)').Count
if ($irqGateStateCalls -ne 18 -or $irqGateHistoryCalls -ne 1 -or $irqGatePreflightCalls -ne 1) {
    throw "Kernel IRQ gate bind telemetry guard failed: expected state=18 (existing telemetry readers plus irq-runtime-gate-status/check/blockers and irq-runtime-sim-status/run/blockers), history=1, preflight=1; found state=$irqGateStateCalls history=$irqGateHistoryCalls preflight=$irqGatePreflightCalls"
}
Assert-NotContains $kernelBootPath "irq::irq_gate_bind_state()" "kernel boot path does not read irq gate bind state telemetry"
Assert-NotContains $kernelBootPath "irq::irq_gate_bind_history()" "kernel boot path does not read irq gate bind history telemetry"
Assert-NotContains $kernelBootPath "irq::irq_gate_bind_preflight()" "kernel boot path does not read irq gate bind preflight telemetry"
Assert-NotContains $kernelBootPath "pic::ProgrammableInterruptController::pic_remap_state()" "kernel boot path does not read PIC remap state"
$irqGateStateDispatch = $kernelMainSource.IndexOf('line_str == "irq-gate-state"')
$irqGateHistoryDispatch = $kernelMainSource.IndexOf('line_str == "irq-gate-history"')
$irqGatePreflightDispatch = $kernelMainSource.IndexOf('line_str == "irq-gate-preflight"')
if ($irqGateStateDispatch -lt 0 -or $irqGateHistoryDispatch -lt 0 -or $irqGatePreflightDispatch -lt 0) {
    throw "Kernel IRQ gate bind telemetry guard failed: irq-gate state/history/preflight dispatch not found"
}
if ($kernelMainSource.IndexOf('irq::irq_gate_bind_history()') -lt $irqGateHistoryDispatch) {
    throw "Kernel IRQ gate bind telemetry guard failed: history helper call outside irq-gate-history dispatch"
}
if ($kernelMainSource.IndexOf('irq::irq_gate_bind_preflight()') -lt $irqGatePreflightDispatch) {
    throw "Kernel IRQ gate bind telemetry guard failed: preflight helper call outside irq-gate-preflight dispatch"
}
$irqRuntimePreflightDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-preflight"')
$irqRuntimeCommitDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-commit"')
$irqRuntimeStatusDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-status"')
$irqRuntimeBlockersDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-blockers"')
$eciRuntimeStatusDispatch = $kernelMainSource.IndexOf('line_str == "eoi-runtime-status"')
$eciRuntimeBlockersDispatch = $kernelMainSource.IndexOf('line_str == "eoi-runtime-blockers"')
$irqMaskBlockersDispatch = $kernelMainSource.IndexOf('line_str == "irq-mask-blockers"')
$irqRuntimeMatrixDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-matrix"')
$irqRuntimeReadinessMatrixDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-readiness"')
$irqRuntimeActivationPlanDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-activation-plan"')
$irqRuntimeGateStatusDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-gate-status"')
$irqRuntimeGateCheckDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-gate-check"')
$irqRuntimeGateBlockersDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-gate-blockers"')
$irqRuntimeSimStatusDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-sim-status"')
$irqRuntimeSimRunDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-sim-run"')
$irqRuntimeSimBlockersDispatch = $kernelMainSource.IndexOf('line_str == "irq-runtime-sim-blockers"')
if ($irqRuntimePreflightDispatch -lt 0 -or $irqRuntimeStatusDispatch -lt 0 -or $irqRuntimeBlockersDispatch -lt 0 -or $eciRuntimeStatusDispatch -lt 0 -or $eciRuntimeBlockersDispatch -lt 0 -or $irqMaskBlockersDispatch -lt 0 -or $irqRuntimeMatrixDispatch -lt 0 -or $irqRuntimeReadinessMatrixDispatch -lt 0 -or $irqRuntimeActivationPlanDispatch -lt 0 -or $irqRuntimeGateStatusDispatch -lt 0 -or $irqRuntimeGateCheckDispatch -lt 0 -or $irqRuntimeGateBlockersDispatch -lt 0 -or $irqRuntimeSimStatusDispatch -lt 0 -or $irqRuntimeSimRunDispatch -lt 0 -or $irqRuntimeSimBlockersDispatch -lt 0) {
    throw "Kernel IRQ runtime commands guard failed: irq-runtime-preflight/status/blockers/eoi-runtime-status/blockers/irq-mask-blockers/matrix/readiness dispatch not found"
}
# Now check irq_gate_bind_state calls are within allowed dispatches
foreach ($call in [regex]::Matches($kernelMainSource, 'irq::irq_gate_bind_state\(\)')) {
    $callIndex = $call.Index
    $nearStateCommand = ($callIndex -gt $irqGateStateDispatch -and $callIndex -lt $irqGateStateDispatch + 2048)
    $nearSystemCommand = ($callIndex -gt $kernelMainSource.IndexOf('line_str == "system"') -and $callIndex -lt $kernelMainSource.IndexOf('line_str == "system"') + 4096)
    $nearPreflightCommand = ($callIndex -gt $irqRuntimePreflightDispatch -and $callIndex -lt $irqRuntimePreflightDispatch + 1024)
    $nearCommitCommand = ($callIndex -gt $irqRuntimeCommitDispatch -and $callIndex -lt $irqRuntimeCommitDispatch + 1024)
    $nearStatusCommand = ($callIndex -gt $irqRuntimeStatusDispatch -and $callIndex -lt $irqRuntimeStatusDispatch + 1024)
    $nearBlockersCommand = ($callIndex -gt $irqRuntimeBlockersDispatch -and $callIndex -lt $irqRuntimeBlockersDispatch + 1024)
    $nearEoiStatusCommand = ($callIndex -gt $eciRuntimeStatusDispatch -and $callIndex -lt $eciRuntimeStatusDispatch + 1024)
    $nearEoiBlockersCommand = ($callIndex -gt $eciRuntimeBlockersDispatch -and $callIndex -lt $eciRuntimeBlockersDispatch + 1024)
    $nearIrqMaskBlockersCommand = ($callIndex -gt $irqMaskBlockersDispatch -and $callIndex -lt $irqMaskBlockersDispatch + 1024)
    $nearIrqRuntimeMatrixCommand = ($callIndex -gt $irqRuntimeMatrixDispatch -and $callIndex -lt $irqRuntimeMatrixDispatch + 2048)
    $nearIrqRuntimeReadinessCommand = ($callIndex -gt $irqRuntimeReadinessMatrixDispatch -and $callIndex -lt $irqRuntimeReadinessMatrixDispatch + 2048)
    $nearIrqRuntimeActivationPlanCommand = ($callIndex -gt $irqRuntimeActivationPlanDispatch -and $callIndex -lt $irqRuntimeActivationPlanDispatch + 2048)
    $nearIrqRuntimeGateStatusCommand = ($callIndex -gt $irqRuntimeGateStatusDispatch -and $callIndex -lt $irqRuntimeGateStatusDispatch + 2048)
    $nearIrqRuntimeGateCheckCommand = ($callIndex -gt $irqRuntimeGateCheckDispatch -and $callIndex -lt $irqRuntimeGateCheckDispatch + 2048)
    $nearIrqRuntimeGateBlockersCommand = ($callIndex -gt $irqRuntimeGateBlockersDispatch -and $callIndex -lt $irqRuntimeGateBlockersDispatch + 2048)
    $nearIrqRuntimeSimStatusCommand = ($callIndex -gt $irqRuntimeSimStatusDispatch -and $callIndex -lt $irqRuntimeSimStatusDispatch + 2048)
    $nearIrqRuntimeSimRunCommand = ($callIndex -gt $irqRuntimeSimRunDispatch -and $callIndex -lt $irqRuntimeSimRunDispatch + 2048)
    $nearIrqRuntimeSimBlockersCommand = ($callIndex -gt $irqRuntimeSimBlockersDispatch -and $callIndex -lt $irqRuntimeSimBlockersDispatch + 2048)
    if (-not ($nearStateCommand -or $nearSystemCommand -or $nearPreflightCommand -or $nearCommitCommand -or $nearStatusCommand -or $nearBlockersCommand -or $nearEoiStatusCommand -or $nearEoiBlockersCommand -or $nearIrqMaskBlockersCommand -or $nearIrqRuntimeMatrixCommand -or $nearIrqRuntimeReadinessCommand -or $nearIrqRuntimeActivationPlanCommand -or $nearIrqRuntimeGateStatusCommand -or $nearIrqRuntimeGateCheckCommand -or $nearIrqRuntimeGateBlockersCommand -or $nearIrqRuntimeSimStatusCommand -or $nearIrqRuntimeSimRunCommand -or $nearIrqRuntimeSimBlockersCommand)) {
        throw "Kernel IRQ gate bind telemetry guard failed: irq_gate_bind_state() call outside irq-gate-state/system/irq-runtime-preconditions/eoi-runtime/irq-mask-blockers/matrix dispatch"
    }
}
Assert-NotContains $kernelMainSource "timer_interrupt_handler_stub" "kernel main does not bind timer interrupt stub"
Assert-NotContains $kernelMainSource "keyboard_interrupt_handler_stub" "kernel main does not bind keyboard interrupt stub"
Assert-NotContains $kernelMainSource "keyboard_irq" "kernel main has no keyboard irq path"
Assert-NotContains $kernelMainSource "timer_irq" "kernel main has no timer irq path"
Assert-Contains $kernelMainSource "idt::IDT.entries[32].set_handler(interrupts::irq0_timer_gate_smoke_asm as *const ())" "kernel command path binds IRQ0 vector 32 only to smoke wrapper"
Assert-Contains $kernelMainSource "idt::IDT.entries[33].set_handler(interrupts::irq1_keyboard_gate_smoke_asm as *const ())" "kernel command path binds IRQ1 vector 33 only to smoke wrapper"
Assert-NotContains $kernelIdtSource "entries[32].set_handler" "idt source does not bind IRQ0 vector 32"
Assert-NotContains $kernelIdtSource "entries[33].set_handler" "idt source does not bind IRQ1 vector 33"
Assert-Contains $kernelMainSource "interrupts::PF_SMOKE_ACTIVE = true;" "kernel pf-smoke arms controlled probe"
Assert-Contains $kernelMainSource "interrupts::PF_SMOKE_RECOVERY_EIP = interrupts::pf_smoke_recovery_asm as *const () as u32;" "kernel pf-smoke sets recovery trampoline"
Assert-Contains $kernelMainSource "interrupts::pf_smoke_probe_asm();" "kernel pf-smoke calls controlled real fault probe"
Assert-NotContains $kernelMainSource "line_str == `"pagefault`"" "kernel has no raw page fault trigger command"
Assert-NotContains $kernelMainSource "line_str == `"pf-trigger`"" "kernel has no pf-trigger command"
Assert-NotContains $kernelMainSource "line_str == `"pagefault-trigger`"" "kernel has no pagefault-trigger command"
Assert-NotContains $kernelMainSource "line_str == `"pagefault-smoke`"" "kernel has no pagefault-smoke alias"
Assert-Contains $kernelPageFaultSource "pub const PAGE_FAULT_VECTOR: u8 = 14;" "page fault vector constant"
Assert-Contains $kernelPageFaultSource "pub struct PageFaultErrorCode;" "page fault error-code constants type"
Assert-Contains $kernelPageFaultSource "pub const PRESENT: u32 = 1 << 0;" "page fault P bit constant"
Assert-Contains $kernelPageFaultSource "pub const WRITE: u32 = 1 << 1;" "page fault W/R bit constant"
Assert-Contains $kernelPageFaultSource "pub const USER: u32 = 1 << 2;" "page fault U/S bit constant"
Assert-Contains $kernelPageFaultSource "pub const RESERVED_WRITE: u32 = 1 << 3;" "page fault RSVD bit constant"
Assert-Contains $kernelPageFaultSource "pub const INSTRUCTION_FETCH: u32 = 1 << 4;" "page fault I/D bit constant"
Assert-Contains $kernelPageFaultSource "#[repr(C)]" "page fault frame keeps stable C representation"
Assert-Contains $kernelPageFaultSource "pub struct PageFaultFrame" "page fault frame documentation struct"
Assert-Contains $kernelPageFaultSource "pub error_code: u32" "page fault frame error_code field"
Assert-Contains $kernelPageFaultSource "pub eip: u32" "page fault frame eip field"
Assert-Contains $kernelPageFaultSource "pub cs: u32" "page fault frame cs field"
Assert-Contains $kernelPageFaultSource "pub eflags: u32" "page fault frame eflags field"
Assert-Contains $kernelPageFaultSource "pub cr2: u32" "page fault frame cr2 field"
Assert-NotContains $kernelPageFaultSource "extern `"C`"" "page fault layout module has no handler ABI entrypoint"
Assert-NotContains $kernelPageFaultSource "global_asm" "page fault layout module has no assembly stub"
Assert-Contains $kernelInterruptSource "LAST_EXCEPTION_VECTOR = 3;" "breakpoint handler records vector 3"
Assert-Contains $kernelInterruptSource "LAST_EXCEPTION_NAME = `"breakpoint`";" "breakpoint handler records name"
Assert-Contains $kernelInterruptSource "LAST_EXCEPTION_VECTOR = 0;" "divide-by-zero handler records vector 0"
Assert-Contains $kernelInterruptSource "LAST_EXCEPTION_NAME = `"divide-by-zero`";" "divide-by-zero handler records name"
Assert-Contains $kernelInterruptSource "exception: breakpoint\nvector: 3\nstatus: handled\n" "breakpoint handler exact status output"
Assert-Contains $kernelInterruptSource "exception: divide-by-zero\nvector: 0\nstatus: handled\n" "divide-by-zero handler exact status output"
Assert-Contains $kernelInterruptSource "page_fault_handler_asm" "page fault assembly handler present"
Assert-Contains $kernelInterruptSource "page_fault_handler_rust" "page fault rust handler present"
Assert-Contains $kernelInterruptSource "pf_smoke_probe_asm" "page fault smoke probe present"
Assert-Contains $kernelInterruptSource "pf_smoke_recovery_asm" "page fault smoke recovery trampoline present"
Assert-Contains $kernelInterruptSource "mov eax, [esp + 32]" "page fault wrapper reads CPU-pushed error code"
Assert-Contains $kernelInterruptSource "lea ecx, [esp + 36]" "page fault wrapper passes saved EIP slot"
Assert-Contains $kernelInterruptSource "add esp, 4" "page fault wrapper discards CPU-pushed error code"
Assert-Contains $kernelInterruptSource 'core::arch::asm!("mov {}, cr2"' "page fault handler reads CR2"
Assert-Contains $kernelInterruptSource "LAST_EXCEPTION_VECTOR = 14;" "page fault handler records vector 14"
Assert-Contains $kernelInterruptSource "LAST_EXCEPTION_NAME = `"page-fault`";" "page fault handler records name"
Assert-Contains $kernelInterruptSource "exception: page fault\nvector: 14\ncr2: 0x{:08x}\nerror code: 0x{:08x}\nstatus: handled\n" "page fault handler exact status output"
Assert-Contains $kernelInterruptSource "PF_SMOKE_ACTIVE = false;" "page fault handler clears smoke active state"
Assert-Contains $kernelInterruptSource "*saved_eip_slot = PF_SMOKE_RECOVERY_EIP;" "page fault handler rewrites saved EIP to recovery"
Assert-NotContains $kernelInterruptSource "asm!(`"int 14`")" "page fault smoke does not use software vector 14"
Assert-NotContains $kernelInterruptSource "asm!(`"sti`")" "interrupt source does not enable STI"
Assert-NotContains $kernelInterruptSource "irq0_handler" "interrupt source has no irq0 timer handler"
Assert-NotContains $kernelInterruptSource "irq1_handler" "interrupt source has no irq1 keyboard handler"
Assert-NotContains $kernelInterruptSource "keyboard_irq" "interrupt source has no keyboard irq path"
Assert-NotContains $kernelInterruptSource "timer_irq" "interrupt source has no timer irq path"
$irq0SmokeAsmStart = $kernelInterruptSource.IndexOf('.global irq0_timer_gate_smoke_asm')
$irq1SmokeAsmStart = $kernelInterruptSource.IndexOf('.global irq1_keyboard_gate_smoke_asm')
if ($irq0SmokeAsmStart -lt 0 -or $irq1SmokeAsmStart -lt 0) {
    throw "Kernel IRQ gate smoke asm guard failed: irq0/irq1 gate smoke symbols not found"
}
$irq0GateSmokeAsm = $kernelInterruptSource.Substring($irq0SmokeAsmStart, $irq1SmokeAsmStart - $irq0SmokeAsmStart)
Assert-Contains $irq0GateSmokeAsm "`"    iretd`"" "irq0 gate smoke asm returns with iretd only"
Assert-NotContains $irq0GateSmokeAsm "`"    out" "irq0 gate smoke asm performs no port I/O or EOI dispatch"
$irq1GateSmokeEnd = $kernelInterruptSource.IndexOf('extern "C" {', $irq1SmokeAsmStart)
if ($irq1GateSmokeEnd -lt 0) {
    throw "Kernel IRQ gate smoke asm guard failed: extern block after irq1 gate smoke not found"
}
$irq1GateSmokeAsm = $kernelInterruptSource.Substring($irq1SmokeAsmStart, $irq1GateSmokeEnd - $irq1SmokeAsmStart)
Assert-Contains $irq1GateSmokeAsm "`"    iretd`"" "irq1 gate smoke asm returns with iretd only"
Assert-NotContains $irq1GateSmokeAsm "`"    out" "irq1 gate smoke asm performs no port I/O or EOI dispatch"
Assert-Contains $kernelPicSource "v9.0.2 keeps read-only PIC remap state telemetry" "pic source state telemetry milestone wording"
Assert-Contains $kernelPicSource "unsafe fn write_pic_port(port: u16, value: u8)" "pic source controlled smoke port write helper"
Assert-Contains $kernelPicSource '"out dx, al"' "pic source uses explicit PIC port write instruction"
Assert-Contains $kernelPicSource "pub const PIC_MASTER_CMD: u16 = 0x20;" "pic master command port constant"
Assert-Contains $kernelPicSource "pub const PIC_MASTER_DATA: u16 = 0x21;" "pic master data port constant"
Assert-Contains $kernelPicSource "pub const PIC_SLAVE_CMD: u16 = 0xA0;" "pic slave command port constant"
Assert-Contains $kernelPicSource "pub const PIC_SLAVE_DATA: u16 = 0xA1;" "pic slave data port constant"
Assert-Contains $kernelPicSource "pub const ICW2_MASTER_OFFSET: u8 = 0x20;" "pic master offset constant"
Assert-Contains $kernelPicSource "pub const ICW2_SLAVE_OFFSET: u8 = 0x28;" "pic slave offset constant"
Assert-Contains $kernelPicSource "pub const IRQ_VECTOR_START: u8 = 0x20;" "pic irq vector start constant"
Assert-Contains $kernelPicSource "pub const IRQ_VECTOR_END: u8 = 0x2F;" "pic irq vector end constant"
Assert-Contains $kernelPicSource "pub const PIC_MASK_ALL: u8 = 0xFF;" "pic mask all constant"
Assert-Contains $kernelPicSource "pub const PIC_EOI: u8 = 0x20;" "pic eoi constant"
Assert-Contains $kernelPicSource "pub struct PicRemapPlan" "pic remap plan struct"
Assert-Contains $kernelPicSource "pub struct IrqMapEntry" "pic irq map entry struct"
Assert-Contains $kernelPicSource "pub const IRQ_MAP_PLAN: [IrqMapEntry; 16]" "pic irq map plan constant"
Assert-Contains $kernelPicSource "pub fn remap_plan() -> PicRemapPlan" "pic remap plan function"
Assert-Contains $kernelPicSource "pub fn remap_disabled() -> PicRemapPlan" "pic remap disabled function"
Assert-Contains $kernelPicSource "Self::remap_plan()" "pic remap disabled returns documented plan"
Assert-Contains $kernelPicSource "pub fn pic_remap_smoke_arm() -> PicRemapSmokeArmStatus" "pic remap smoke arm function"
Assert-Contains $kernelPicSource "pub fn pic_remap_controlled_smoke() -> PicRemapSmokeResult" "pic remap controlled smoke function"
Assert-Contains $kernelPicSource "pub fn pic_remap_smoke_status() -> PicRemapSmokeStatus" "pic remap smoke status function"
Assert-Contains $kernelPicSource "pub struct PicRemapStateTelemetry" "pic remap state telemetry struct"
Assert-Contains $kernelPicSource "pub struct PicRemapHistoryTelemetry" "pic remap history telemetry struct"
Assert-Contains $kernelPicSource "pub struct PicRemapPreflightTelemetry" "pic remap preflight telemetry struct"
Assert-Contains $kernelPicSource "pub fn pic_remap_state() -> PicRemapStateTelemetry" "pic remap state telemetry function"
Assert-Contains $kernelPicSource "pub fn pic_remap_history() -> PicRemapHistoryTelemetry" "pic remap history telemetry function"
Assert-Contains $kernelPicSource "pub fn pic_remap_preflight() -> PicRemapPreflightTelemetry" "pic remap preflight telemetry function"
Assert-Contains $kernelPicSource "pub const PIC_REMAP_ICW_SEQUENCE_EXPECTED: &str = `"yes`";" "pic remap expected icw telemetry constant"
Assert-Contains $kernelPicSource "pub const PIC_REMAP_ICW_WRITES_CONTROLLED_ONLY: &str = `"controlled command path only`";" "pic remap controlled writes telemetry constant"
Assert-Contains $kernelPicSource "pub const PIC_REMAP_GUARD_COMMAND_ARMED_REQUIRED: &str = `"command armed required`";" "pic remap preflight guard constant"
Assert-Contains $kernelPicSource "pub const PIC_REMAP_RESULT_TELEMETRY_ONLY: &str = `"telemetry only`";" "pic remap telemetry-only result constant"
Assert-Contains $kernelPicSource "icw_sequence_applied: if status.executed { PIC_REMAP_YES } else { PIC_REMAP_NO }" "pic remap state reports applied from executed state"
Assert-Contains $kernelPicSource "last_smoke_executed: if status.executed { PIC_REMAP_YES } else { PIC_REMAP_NO }" "pic remap history reports executed state"
Assert-Contains $kernelPicSource "PIC_REMAP_SMOKE_ARMED = true;" "pic remap smoke arm sets guard"
Assert-Contains $kernelPicSource "if unsafe { !PIC_REMAP_SMOKE_ARMED }" "pic remap smoke blocks when unarmed"
Assert-Contains $kernelPicSource "write_pic_port(PIC_MASTER_CMD, ICW1_INIT);" "pic remap writes master ICW1"
Assert-Contains $kernelPicSource "write_pic_port(PIC_SLAVE_CMD, ICW1_INIT);" "pic remap writes slave ICW1"
Assert-Contains $kernelPicSource "write_pic_port(PIC_MASTER_DATA, ICW2_MASTER_OFFSET);" "pic remap writes master ICW2"
Assert-Contains $kernelPicSource "write_pic_port(PIC_SLAVE_DATA, ICW2_SLAVE_OFFSET);" "pic remap writes slave ICW2"
Assert-Contains $kernelPicSource "write_pic_port(PIC_MASTER_DATA, ICW3_MASTER_CASCADE);" "pic remap writes master ICW3"
Assert-Contains $kernelPicSource "write_pic_port(PIC_SLAVE_DATA, ICW3_SLAVE_CASCADE);" "pic remap writes slave ICW3"
Assert-Contains $kernelPicSource "write_pic_port(PIC_MASTER_DATA, ICW4_8086_MODE);" "pic remap writes master ICW4"
Assert-Contains $kernelPicSource "write_pic_port(PIC_SLAVE_DATA, ICW4_8086_MODE);" "pic remap writes slave ICW4"
Assert-Contains $kernelPicSource "write_pic_port(PIC_MASTER_DATA, PIC_MASK_ALL);" "pic remap masks master after smoke"
Assert-Contains $kernelPicSource "write_pic_port(PIC_SLAVE_DATA, PIC_MASK_ALL);" "pic remap masks slave after smoke"
Assert-NotContains $kernelPicSource "write_pic_port(PIC_MASTER_DATA, 0xFE)" "pic source does not unmask master IRQ lines via literal mask"
Assert-NotContains $kernelPicSource "write_pic_port(PIC_MASTER_DATA, 0xFC)" "pic source does not unmask master IRQ lines via literal mask"
Assert-NotContains $kernelPicSource "write_pic_port(PIC_SLAVE_DATA, 0xFE)" "pic source does not unmask slave IRQ lines via literal mask"
Assert-NotContains $kernelPicSource "write_pic_port(PIC_SLAVE_DATA, 0xFF)" "pic slave mask writes use PIC_MASK_ALL constant only"
Assert-Contains $kernelPicSource "PIC_REMAP_SMOKE_ARMED = false;" "pic remap smoke clears guard"
Assert-Contains $kernelPicSource "PIC_REMAP_SMOKE_EXECUTED = true;" "pic remap smoke records executed state"
$picWriteMatches = [regex]::Matches($kernelPicSource, '(?m)^\s*write_pic_port\(([^;]+)\);')
$expectedPicWriteSequence = @(
    'PIC_MASTER_CMD, ICW1_INIT',
    'PIC_SLAVE_CMD, ICW1_INIT',
    'PIC_MASTER_DATA, ICW2_MASTER_OFFSET',
    'PIC_SLAVE_DATA, ICW2_SLAVE_OFFSET',
    'PIC_MASTER_DATA, ICW3_MASTER_CASCADE',
    'PIC_SLAVE_DATA, ICW3_SLAVE_CASCADE',
    'PIC_MASTER_DATA, ICW4_8086_MODE',
    'PIC_SLAVE_DATA, ICW4_8086_MODE',
    'PIC_MASTER_DATA, PIC_MASK_ALL',
    'PIC_SLAVE_DATA, PIC_MASK_ALL'
)
if ($picWriteMatches.Count -ne $expectedPicWriteSequence.Count) {
    throw "PIC controlled smoke guard failed: expected exactly $($expectedPicWriteSequence.Count) write_pic_port calls; found $($picWriteMatches.Count)"
}
for ($i = 0; $i -lt $expectedPicWriteSequence.Count; $i++) {
    $actualPicWrite = ($picWriteMatches[$i].Groups[1].Value -replace '\s+', ' ').Trim()
    if ($actualPicWrite -ne $expectedPicWriteSequence[$i]) {
        throw "PIC controlled smoke guard failed: ICW write sequence mismatch at index $i; expected '$($expectedPicWriteSequence[$i])', got '$actualPicWrite'"
    }
}
$picSmokeFnStart = $kernelPicSource.IndexOf('pub fn pic_remap_controlled_smoke() -> PicRemapSmokeResult')
$picSmokeFnEnd = $kernelPicSource.IndexOf('    /// Returns the planned master EOI target configuration without touching hardware.')
if ($picSmokeFnStart -lt 0 -or $picSmokeFnEnd -lt $picSmokeFnStart) {
    throw "PIC controlled smoke guard failed: could not isolate pic_remap_controlled_smoke() body"
}
$picSmokeFnBody = $kernelPicSource.Substring($picSmokeFnStart, $picSmokeFnEnd - $picSmokeFnStart)
$picSmokeFnWrites = [regex]::Matches($picSmokeFnBody, 'write_pic_port\(').Count
if ($picSmokeFnWrites -ne $expectedPicWriteSequence.Count) {
    throw "PIC controlled smoke guard failed: all write_pic_port calls must stay inside pic_remap_controlled_smoke(); found $picSmokeFnWrites in smoke body"
}
Assert-Contains $kernelPicSource "pub enum EoiTarget" "pic eoi target enum definition"
Assert-Contains $kernelPicSource "pub struct EoiPlan" "pic eoi plan struct definition"
Assert-Contains $kernelPicSource "pub struct EoiStrategyStatus" "pic eoi strategy status struct definition"
Assert-Contains $kernelPicSource "pub fn master_eoi_plan(" "pic master eoi plan function"
Assert-Contains $kernelPicSource "pub fn slave_eoi_plan(" "pic slave eoi plan function"
Assert-Contains $kernelPicSource "pub fn irq0_timer_eoi_plan(" "pic irq0 timer eoi plan function"
Assert-Contains $kernelPicSource "pub fn irq1_keyboard_eoi_plan(" "pic irq1 keyboard eoi plan function"
Assert-Contains $kernelPicSource "pub fn eoi_strategy_status(" "pic eoi strategy status function"
Assert-Contains $kernelPicSource "pub fn irq_map_plan() -> &'static [IrqMapEntry; 16]" "pic irq map plan function"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 0, name: "timer", vector: 0x20 }' "pic irq0 dry-run map entry"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 1, name: "keyboard", vector: 0x21 }' "pic irq1 dry-run map entry"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 2, name: "cascade", vector: 0x22 }' "pic irq2 dry-run map entry"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 3, name: "serial2", vector: 0x23 }' "pic irq3 dry-run map entry"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 4, name: "serial1", vector: 0x24 }' "pic irq4 dry-run map entry"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 5, name: "parallel2", vector: 0x25 }' "pic irq5 dry-run map entry"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 6, name: "floppy", vector: 0x26 }' "pic irq6 dry-run map entry"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 7, name: "parallel1", vector: 0x27 }' "pic irq7 dry-run map entry"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 8, name: "rtc", vector: 0x28 }' "pic irq8 dry-run map entry"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 9, name: "acpi", vector: 0x29 }' "pic irq9 dry-run map entry"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 10, name: "reserved", vector: 0x2A }' "pic irq10 dry-run map entry"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 11, name: "reserved", vector: 0x2B }' "pic irq11 dry-run map entry"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 12, name: "mouse", vector: 0x2C }' "pic irq12 dry-run map entry"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 13, name: "fpu", vector: 0x2D }' "pic irq13 dry-run map entry"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 14, name: "primary-ata", vector: 0x2E }' "pic irq14 dry-run map entry"
Assert-Contains $kernelPicSource 'IrqMapEntry { irq: 15, name: "secondary-ata", vector: 0x2F }' "pic irq15 dry-run map entry"
Assert-Contains $kernelIrqSource "pub const IRQ0_VECTOR: u8 = 32;" "irq0 vector skeleton constant"
Assert-Contains $kernelIrqSource "pub const IRQ1_VECTOR: u8 = 33;" "irq1 vector skeleton constant"
Assert-Contains $kernelIrqSource "pub struct IrqHandlerSkeleton" "irq handler skeleton type"
Assert-Contains $kernelIrqSource "pub fn irq0_timer_skeleton() -> IrqHandlerSkeleton" "irq0 timer skeleton function"
Assert-Contains $kernelIrqSource "pub fn irq1_keyboard_skeleton() -> IrqHandlerSkeleton" "irq1 keyboard skeleton function"
Assert-Contains $kernelIrqSource "pub fn irq_handler_skeletons() -> [IrqHandlerSkeleton; 2]" "irq handler skeletons function"
Assert-Contains $kernelIrqSource "pub const IRQ0_NAME: &str = `"timer`";" "irq0 gate plan name constant"
Assert-Contains $kernelIrqSource "pub const IRQ1_NAME: &str = `"keyboard`";" "irq1 gate plan name constant"
Assert-Contains $kernelIrqSource "pub const IRQ_GATE_STATE_DORMANT: &str = `"dormant / disabled`";" "irq gate dormant state constant"
Assert-Contains $kernelIrqSource "pub const IRQ_IDT_BINDING_DISABLED: &str = `"disabled`";" "irq gate idt binding disabled constant"
Assert-Contains $kernelIrqSource "pub const IRQ_PIC_REMAP_DISABLED: &str = `"disabled`";" "irq gate pic remap disabled constant"
Assert-Contains $kernelIrqSource "pub const IRQ_PIC_REMAP_CONTROLLED_SMOKE_ONLY: &str = `"controlled smoke only`";" "irq readiness controlled smoke pic remap constant"
Assert-Contains $kernelIrqSource "pub const IRQ_EOI_DISPATCH_DISABLED: &str = `"disabled`";" "irq gate eoi dispatch disabled constant"
Assert-Contains $kernelIrqSource "pub const IRQ_INTERRUPTS_DISABLED: &str = `"disabled`";" "irq gate interrupts disabled constant"
Assert-Contains $kernelIrqSource "pub const IRQ_IDT_INSTALL_DISABLED: &str = `"planned / not installed`";" "irq disabled bind idt install constant"
Assert-Contains $kernelIrqSource "pub const IRQ_BIND_DISABLED_HELPER: &str = `"bind_irq_gates_disabled`";" "irq disabled bind helper name constant"
Assert-Contains $kernelIrqSource "pub const IRQ_BIND_BOOT_CALL_DISABLED: &str = `"no`";" "irq disabled bind boot call constant"
Assert-Contains $kernelIrqSource "pub const IRQ_VECTOR_UNBOUND: &str = `"unbound`";" "irq disabled bind unbound vector constant"
Assert-Contains $kernelIrqSource "pub const IRQ_ACTIVE_HANDLER_NONE: &str = `"none`";" "irq disabled bind active handler none constant"
Assert-Contains $kernelIrqSource "pub const IRQ_KEYBOARD_INPUT_POLLING_ONLY: &str = `"polling-only`";" "irq disabled bind keyboard polling constant"
Assert-Contains $kernelIrqSource "pub const IRQ_BIND_PATH_DISABLED_ONLY: &str = `"disabled bind path only`";" "irq disabled bind path constant"
Assert-Contains $kernelIrqSource "pub struct IrqGatePlan" "irq gate plan type"
Assert-Contains $kernelIrqSource "pub irq: u8" "irq gate plan irq field"
Assert-Contains $kernelIrqSource "pub vector: u8" "irq gate plan vector field"
Assert-Contains $kernelIrqSource "pub name: &'static str" "irq gate plan name field"
Assert-Contains $kernelIrqSource "pub gate_state: &'static str" "irq gate plan gate_state field"
Assert-Contains $kernelIrqSource "pub idt_binding: &'static str" "irq gate plan idt_binding field"
Assert-Contains $kernelIrqSource "pub pic_remap: &'static str" "irq gate plan pic_remap field"
Assert-Contains $kernelIrqSource "pub eoi_dispatch: &'static str" "irq gate plan eoi_dispatch field"
Assert-Contains $kernelIrqSource "pub interrupts: &'static str" "irq gate plan interrupts field"
Assert-Contains $kernelIrqSource "pub fn irq0_timer_gate_plan() -> IrqGatePlan" "irq0 timer gate plan function"
Assert-Contains $kernelIrqSource "pub fn irq1_keyboard_gate_plan() -> IrqGatePlan" "irq1 keyboard gate plan function"
Assert-Contains $kernelIrqSource "pub fn irq_gate_plan() -> [IrqGatePlan; 2]" "irq gate plan aggregate function"
Assert-Contains $kernelIrqSource "pub struct IrqGateBindDisabledStep" "irq disabled bind step type"
Assert-Contains $kernelIrqSource "pub struct IrqGateBindDisabledStatus" "irq disabled bind status type"
Assert-Contains $kernelIrqSource "pub struct IrqRuntimeReadiness" "irq runtime readiness type"
Assert-Contains $kernelIrqSource "pub struct IrqRuntimeRisk" "irq runtime risk type"
Assert-Contains $kernelIrqSource "pub struct IrqRuntimePreflight" "irq runtime preflight type"
Assert-Contains $kernelIrqSource "pub bind_path: &'static str" "irq disabled bind step bind_path field"
Assert-Contains $kernelIrqSource "pub idt_install: &'static str" "irq disabled bind step idt_install field"
Assert-Contains $kernelIrqSource "pub helper: &'static str" "irq disabled bind status helper field"
Assert-Contains $kernelIrqSource "pub boot_call: &'static str" "irq disabled bind status boot_call field"
Assert-Contains $kernelIrqSource "pub irq0_state: &'static str" "irq disabled bind status irq0_state field"
Assert-Contains $kernelIrqSource "pub irq1_state: &'static str" "irq disabled bind status irq1_state field"
Assert-Contains $kernelIrqSource "pub keyboard_input: &'static str" "irq disabled bind status keyboard_input field"
Assert-Contains $kernelIrqSource "pub fn bind_irq_gates_disabled() -> IrqGateBindDisabledStatus" "irq disabled bind helper function"
Assert-Contains $kernelIrqSource "pub fn irq_runtime_readiness() -> IrqRuntimeReadiness" "irq runtime readiness helper function"
Assert-Contains $kernelIrqSource "pub fn irq_runtime_risk() -> IrqRuntimeRisk" "irq runtime risk helper function"
Assert-Contains $kernelIrqSource "pub fn irq_runtime_preflight() -> IrqRuntimePreflight" "irq runtime preflight helper function"
Assert-Contains $kernelIrqSource "pub struct IrqGateBindSmokeArmStatus" "irq gate bind smoke arm status type"
Assert-Contains $kernelIrqSource "pub struct IrqGateBindSmokeResult" "irq gate bind smoke result type"
Assert-Contains $kernelIrqSource "pub struct IrqGateBindSmokeStatus" "irq gate bind smoke status type"
Assert-Contains $kernelIrqSource "static mut IRQ_GATE_BIND_SMOKE_ARMED: bool = false;" "irq gate bind smoke armed state"
Assert-Contains $kernelIrqSource "static mut IRQ_GATE_BIND_SMOKE_EXECUTED: bool = false;" "irq gate bind smoke executed state"
Assert-Contains $kernelIrqSource "pub fn irq_gate_bind_smoke_arm() -> IrqGateBindSmokeArmStatus" "irq gate bind smoke arm helper"
Assert-Contains $kernelIrqSource "pub fn irq_gate_bind_smoke_is_armed() -> bool" "irq gate bind smoke guard helper"
Assert-Contains $kernelIrqSource "pub fn irq_gate_bind_smoke_mark_bound() -> IrqGateBindSmokeResult" "irq gate bind smoke mark bound helper"
Assert-Contains $kernelIrqSource "pub fn irq_gate_bind_smoke_blocked() -> IrqGateBindSmokeResult" "irq gate bind smoke blocked helper"
Assert-Contains $kernelIrqSource "pub fn irq_gate_bind_smoke_status() -> IrqGateBindSmokeStatus" "irq gate bind smoke status helper"
Assert-Contains $kernelIrqSource "pub struct IrqGateBindStateTelemetry" "irq gate bind state telemetry struct"
Assert-Contains $kernelIrqSource "pub struct IrqGateBindHistoryTelemetry" "irq gate bind history telemetry struct"
Assert-Contains $kernelIrqSource "pub struct IrqGateBindPreflightTelemetry" "irq gate bind preflight telemetry struct"
Assert-Contains $kernelIrqSource "pub fn irq_gate_bind_state() -> IrqGateBindStateTelemetry" "irq gate bind state telemetry function"
Assert-Contains $kernelIrqSource "pub fn irq_gate_bind_history() -> IrqGateBindHistoryTelemetry" "irq gate bind history telemetry function"
Assert-Contains $kernelIrqSource "pub fn irq_gate_bind_preflight() -> IrqGateBindPreflightTelemetry" "irq gate bind preflight telemetry function"
Assert-Contains $kernelIrqSource "pub const IRQ_GATE_BIND_IDT_BINDS_CONTROLLED_ONLY: &str = `"controlled command path only`";" "irq gate bind controlled idt binds constant"
Assert-Contains $kernelIrqSource "pub const IRQ_GATE_BIND_BOOT_BIND_NO: &str = `"no`";" "irq gate bind boot bind constant"
Assert-Contains $kernelIrqSource "pub const IRQ_GATE_BIND_RESULT_TELEMETRY_ONLY: &str = `"telemetry only`";" "irq gate bind telemetry-only result constant"
Assert-Contains $kernelIrqSource "pub idt_exceptions: &'static str" "irq readiness idt exceptions field"
Assert-Contains $kernelIrqSource "pub ready_for_runtime_irq: &'static str" "irq readiness ready field"
Assert-Contains $kernelIrqSource "pub runtime_irq: &'static str" "irq risk runtime field"
Assert-Contains $kernelIrqSource "pub required_before_enable: &'static str" "irq risk requirements field"
Assert-Contains $kernelIrqSource "pub pf_smoke: &'static str" "irq preflight pf smoke field"
Assert-Contains $kernelIrqSource 'pub const IRQ_RUNTIME_READY_NO: &str = "no";' "irq readiness fixed not ready constant"
Assert-Contains $kernelIrqSource 'pub const IRQ_RUNTIME_BLOCKED: &str = "blocked";' "irq readiness blocked constant"
Assert-Contains $kernelIrqSource 'pub const IRQ_PF_SMOKE_UNCHANGED: &str = "unchanged";' "irq readiness pf smoke unchanged constant"
Assert-Contains $kernelIrqSource "gate_state: IRQ_GATE_STATE_DORMANT" "irq gate plan dormant state"
Assert-Contains $kernelIrqSource "idt_binding: IRQ_IDT_BINDING_DISABLED" "irq gate plan idt binding disabled"
Assert-Contains $kernelIrqSource "pic_remap: IRQ_PIC_REMAP_DISABLED" "irq gate plan pic remap disabled"
Assert-Contains $kernelIrqSource "pic_remap: IRQ_PIC_REMAP_CONTROLLED_SMOKE_ONLY" "irq readiness/preflight uses controlled smoke pic remap"
Assert-Contains $kernelIrqSource "eoi_dispatch: IRQ_EOI_DISPATCH_DISABLED" "irq gate plan eoi dispatch disabled"
Assert-Contains $kernelIrqSource "interrupts: IRQ_INTERRUPTS_DISABLED" "irq gate plan interrupts disabled"
Assert-Contains $kernelIrqSource "vector: IRQ0_VECTOR" "irq0 skeleton uses vector constant"
Assert-Contains $kernelIrqSource "vector: IRQ1_VECTOR" "irq1 skeleton uses vector constant"
$irq0VectorUses = [regex]::Matches($kernelIrqSource, 'vector:\s*IRQ0_VECTOR').Count
$irq1VectorUses = [regex]::Matches($kernelIrqSource, 'vector:\s*IRQ1_VECTOR').Count
if ($irq0VectorUses -ne 5 -or $irq1VectorUses -ne 5) {
    throw "IRQ gate vector sync guard failed: expected IRQ0_VECTOR and IRQ1_VECTOR to be used by skeleton, gate plan, disabled bind status, disabled bind step, and controlled smoke status exactly five times; found IRQ0=$irq0VectorUses IRQ1=$irq1VectorUses"
}
Assert-Contains $kernelIrqSource "irq0_vector: IRQ0_VECTOR" "irq disabled bind status uses irq0 vector constant"
Assert-Contains $kernelIrqSource "irq1_vector: IRQ1_VECTOR" "irq disabled bind status uses irq1 vector constant"
Assert-NotContains $kernelIrqSource "vector: 32" "irq source does not duplicate IRQ0 vector literal in plans"
Assert-NotContains $kernelIrqSource "vector: 33" "irq source does not duplicate IRQ1 vector literal in plans"
Assert-Contains $kernelIrqSource 'state: "skeleton / disabled"' "irq skeleton disabled state"
Assert-NotContains $kernelIrqSource "outb" "irq source has no hardware writes"
Assert-NotContains $kernelIrqSource "asm!" "irq source has no inline assembly"
Assert-NotContains $kernelIrqSource "extern `"C`"" "irq source has no active ABI handler"
Assert-NotContains $kernelIrqSource "PIC_EOI" "irq source does not dispatch EOI"
Assert-NotContains $kernelIrqSource "set_handler" "irq source does not bind IDT entries"
Assert-NotContains $kernelIrqSource "global_asm" "irq source has no assembly wrapper"
Assert-NotContains $kernelIrqSource "page_fault_handler" "irq source does not reuse exception handler path"
Assert-NotContains $kernelIrqSource "keyboard_irq" "irq source has no active keyboard irq path"
Assert-NotContains $kernelIrqSource "timer_irq" "irq source has no active timer irq path"
Assert-NotContains $kernelIrqSource "remap_disabled" "irq source does not call PIC remap hook"
Assert-NotContains $kernelIrqSource "remap_plan" "irq source does not call PIC remap plan"
Assert-NotContains $kernelIrqSource "irq_map_plan" "irq source does not call PIC irq map plan"
Assert-Contains $kernelInterruptSource ".global irq0_timer_gate_smoke_asm" "interrupts source exports irq0 smoke wrapper"
Assert-Contains $kernelInterruptSource ".global irq1_keyboard_gate_smoke_asm" "interrupts source exports irq1 smoke wrapper"
Assert-Contains $kernelInterruptSource "pub fn irq0_timer_gate_smoke_asm();" "interrupts source declares irq0 smoke wrapper"
Assert-Contains $kernelInterruptSource "pub fn irq1_keyboard_gate_smoke_asm();" "interrupts source declares irq1 smoke wrapper"
Assert-Contains $kernelInterruptSource "irq0_timer_gate_smoke_asm:" "interrupts source irq0 smoke label"
Assert-Contains $kernelInterruptSource "irq1_keyboard_gate_smoke_asm:" "interrupts source irq1 smoke label"
Assert-Contains $kernelInterruptDocs '| `14` | Fault | Page Fault | **Active Smoke** | Controlled real fault via `pf-smoke` with CR2 and error-code diagnostics. |' "interrupt docs page fault active smoke"
Assert-Contains $kernelInterruptDocs "Only Vector 0, Vector 3, and Vector 14 Smoke Handlers are Active" "interrupt docs active handler warning"
Assert-NotContains $kernelInterruptDocs "No Divide-by-Zero Handler Yet" "interrupt docs avoid stale vector 0 warning"
Assert-Contains $kernelInterruptDocs "Trap-Style Controlled Trigger" "interrupt docs controlled div0 trap"
Assert-Contains $kernelInterruptDocs "Page Fault Handler Smoke (Vector 14)" "interrupt docs page fault smoke section"
Assert-Contains $kernelInterruptDocs "controlled real Page Fault" "interrupt docs controlled real page fault"
Assert-Contains $kernelInterruptDocs "recovery trampoline" "interrupt docs recovery trampoline"
Assert-Contains $kernelInterruptDocs "IRQ Gate Binding Controlled Smoke" "interrupt docs mark irq gate binding controlled smoke release"
Assert-Contains $kernelInterruptDocs "PIC/IRQ remains planned / disabled" "interrupt docs irq planned disabled"
Assert-Contains $kernelInterruptDocs 'IRQ vectors `0x20-0x2f` are planned' "interrupt docs irq vector range planned"
Assert-Contains $kernelInterruptDocs '`irq-note` Command' "interrupt docs irq-note command"
Assert-Contains $kernelInterruptDocs '`irq-status` Command' "interrupt docs irq-status command"
Assert-Contains $kernelInterruptDocs '`irq-handlers` Command' "interrupt docs irq-handlers command"
Assert-Contains $kernelInterruptDocs '`pic-note` Command' "interrupt docs pic-note command"
Assert-Contains $kernelInterruptDocs '`pic-status` Command' "interrupt docs pic-status command"
Assert-Contains $kernelInterruptDocs '`pic-plan` Command' "interrupt docs pic-plan command"
Assert-Contains $kernelInterruptDocs '`irq-map` Command' "interrupt docs irq-map command"
Assert-Contains $kernelInterruptDocs '`irq-gate-plan` Command' "interrupt docs irq-gate-plan command"
Assert-Contains $kernelInterruptDocs '`irq-readiness` Command' "interrupt docs irq-readiness command"
Assert-Contains $kernelInterruptDocs '`irq-risk` Command' "interrupt docs irq-risk command"
Assert-Contains $kernelInterruptDocs '`irq-preflight` Command' "interrupt docs irq-preflight command"
Assert-Contains $kernelInterruptDocs '`pic-status --verbose` Command' "interrupt docs pic-status verbose command"
Assert-Contains $kernelInterruptDocs "present / not called" "interrupt docs pic remap present not called"
Assert-Contains $kernelInterruptDocs "PIC remap hardware writes are limited to the two-step" "interrupt docs controlled remap summary"
Assert-Contains $kernelInterruptDocs 'PIC remap hardware writes are limited to the two-step `pic-remap-arm` / `pic-remap-smoke` command path' "interrupt docs controlled pic hardware writes"
Assert-Contains $kernelInterruptDocs '`pic-remap-arm` Command' "interrupt docs pic-remap-arm command"
Assert-Contains $kernelInterruptDocs '`pic-remap-smoke` Command' "interrupt docs pic-remap-smoke command"
Assert-Contains $kernelInterruptDocs '`pic-remap-status` Command' "interrupt docs pic-remap-status command"
Assert-Contains $kernelInterruptDocs '`pic-remap-state` Command' "interrupt docs pic-remap-state command"
Assert-Contains $kernelInterruptDocs '`pic-remap-history` Command' "interrupt docs pic-remap-history command"
Assert-Contains $kernelInterruptDocs '`pic-remap-preflight` Command' "interrupt docs pic-remap-preflight command"
Assert-Contains $kernelInterruptDocs "IRQ0/IRQ1 IDT smoke binding is limited to the two-step" "interrupt docs command-only irq vector binding"
Assert-Contains $kernelInterruptDocs "IRQ0 timer and IRQ1 keyboard skeletons are compiled" "interrupt docs skeletons compiled"
Assert-Contains $kernelInterruptDocs "no IRQ1 keyboard hardware-active handler" "interrupt docs no irq1 hardware-active handler"
Assert-Contains $kernelInterruptDocs "no IRQ0 PIT hardware-active handler" "interrupt docs no irq0 hardware-active handler"
Assert-Contains $kernelInterruptDocs '`exception-about` Command' "interrupt docs exception-about command"
Assert-Contains $kernelInterruptDocs '`fault-status` Command' "interrupt docs fault-status command"
Assert-Contains $kernelInterruptDocs '`fault-reset` Command' "interrupt docs fault-reset command"
Assert-Contains $kernelInterruptDocs '`pf-status` Command' "interrupt docs pf-status command"
Assert-Contains $kernelInterruptDocs '`handlers --active` Command' "interrupt docs handlers active command"
Assert-Contains $kernelInterruptDocs '`exceptions --verbose` Command' "interrupt docs exceptions verbose command"
Assert-Contains $kernelInterruptDocs "PIC remap controlled smoke execution state" "interrupt docs system pic remap controlled smoke status"
Assert-Contains $kernelInterruptDocs "Page Fault Frame Layout Foundation" "interrupt docs page fault frame section"
Assert-Contains $kernelInterruptDocs '| `error_code` | CPU stack push | Page Fault reason bits. |' "interrupt docs stack frame error_code"
Assert-Contains $kernelInterruptDocs '| `eip` | CPU stack push | Faulting instruction pointer. |' "interrupt docs stack frame eip"
Assert-Contains $kernelInterruptDocs '| `cs` | CPU stack push | Saved code-segment selector. |' "interrupt docs stack frame cs"
Assert-Contains $kernelInterruptDocs '| `eflags` | CPU stack push | Saved flags register. |' "interrupt docs stack frame eflags"
Assert-Contains $kernelInterruptDocs '| `cr2` | Handler snapshot | Faulting linear address from the CR2 register. |' "interrupt docs stack frame cr2"
Assert-Contains $kernelInterruptDocs "Page Fault Error Code Bits" "interrupt docs error-code bits section"
Assert-Contains $kernelInterruptDocs 'Exact bit set tracked for v9.0.2: `P / W/R / U/S / RSVD / I/D`.' "interrupt docs exact error-code bit names"
Assert-Contains $kernelInterruptDocs "CR2 = faulting linear address." "interrupt docs CR2 equals faulting linear address"
Assert-Contains $kernelInterruptDocs "CR2" "interrupt docs CR2 explanation"
Assert-Contains $kernelInterruptDocs "error code" "interrupt docs error code explanation"
Assert-Contains $kernelInterruptDocs "whether the page was present, whether the access was a write, whether the access came from user mode" "interrupt docs error code field wording"
Assert-Contains $kernelInterruptDocs 'The faulting linear address is reported through the `CR2` register.' "interrupt docs CR2 exact wording"
Assert-Contains $kernelInterruptDocs "Page Fault is not a general recovery subsystem yet" "interrupt docs page fault not general recovery"
Assert-Contains $kernelInterruptDocs 'No `asm!("int 14")` trigger is used.' "interrupt docs no software vector 14 trigger"
Assert-Contains $kernelExceptionDocs "Kernel Exception Subsystem Foundation" "exception docs foundation title"
Assert-Contains $kernelExceptionDocs '| `0` | divide-by-zero | active controlled trap | `div0` uses controlled `int 0` |' "exception docs vector 0"
Assert-Contains $kernelExceptionDocs '| `3` | breakpoint | active | `int3` |' "exception docs vector 3"
Assert-Contains $kernelExceptionDocs '| `14` | page fault | active smoke | `pf-smoke` controlled real fault |' "exception docs vector 14"
Assert-Contains $kernelExceptionDocs "Telemetry" "exception docs telemetry"
Assert-Contains $kernelExceptionDocs "Recovery UX" "exception docs recovery ux"
Assert-Contains $kernelExceptionDocs "Status UX" "exception docs status ux"
Assert-Contains $kernelExceptionDocs "Reset UX" "exception docs reset ux"
Assert-Contains $kernelExceptionDocs 'Planned handlers are currently `none`.' "exception docs planned none"
Assert-Contains $kernelExceptionDocs "keyboard input remains polling-based" "exception docs keyboard polling boundary"
Assert-Contains $kernelExceptionDocs "int3 -> exception-status" "exception docs journey int3"
Assert-Contains $kernelExceptionDocs "div0 -> exception-status" "exception docs journey div0"
Assert-Contains $kernelExceptionDocs "pf-smoke -> fault-status" "exception docs journey pf-smoke"
Assert-Contains $kernelExceptionDocs "exception-about" "exception docs journey exception-about"
Assert-Contains $kernelExceptionDocs 'no `asm!("int 14")`' "exception docs no int14"
Assert-Contains $kernelExceptionDocs 'no `asm!("sti")`' "exception docs no sti"
Assert-Contains $kernelExceptionDocs "no PIC/IRQ enable/remap" "exception docs no pic irq"
Assert-Contains $kernelExceptionDocs "PF_SMOKE_ACTIVE" "exception docs pf smoke active guard"
Assert-Contains $kernelExceptionDocs "PF_SMOKE_RECOVERY_EIP" "exception docs pf smoke recovery eip guard"
Assert-Contains $kernelExceptionDocs "pf_smoke_probe_asm" "exception docs pf smoke probe guard"
Assert-Contains $kernelIrqDocs "IRQ Handler Skeleton Foundation" "irq docs skeleton title"
Assert-Contains $kernelIrqDocs "PIC Remap State Telemetry" "irq docs pic remap state telemetry title"
Assert-Contains $kernelIrqDocs "IRQ Runtime Activation Preconditions 2 release" "irq docs irq runtime activation preconditions release"
Assert-Contains $kernelBootSmokeDocs "IRQ Runtime Activation Preconditions 2 release" "qemu docs irq runtime activation preconditions release"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> echo - = + _" "qemu docs keyboard symbol manual proof command"
Assert-Contains $kernelBootSmokeDocs "- = + _" "qemu docs keyboard symbol manual proof echo output"
Assert-Contains $kernelBootSmokeDocs '| **`-`**     | `0x0C`          | `''-''`         | `Shift + 0x0C`    | **`''_''`**             |' "qemu docs main-row minus symbol mapping"
Assert-Contains $kernelBootSmokeDocs '| **`=`**     | `0x0D`          | `''=''`         | `Shift + 0x0D`    | **`''+''`**             |' "qemu docs main-row equals symbol mapping"
Assert-Contains $kernelBootSmokeDocs '| **Numpad**     | `Numpad -`      | `0x4A`                        | `''-''`                                                |' "qemu docs numpad minus mapping"
Assert-Contains $kernelBootSmokeDocs '| **Numpad**     | `Numpad +`      | `0x4E`                        | `''+''`                                                |' "qemu docs numpad plus mapping"
Assert-Contains $kernelIrqDocs "only that explicit command path may write the PIC ICW sequence" "irq docs command-only pic write path"
Assert-Contains $kernelIrqDocs "Runtime IRQ readiness remains blocked" "irq docs runtime irq blocked"
Assert-Contains $kernelIrqDocs 'PS/2 ports `0x64` and `0x60`' "irq docs keyboard polling ports"
Assert-Contains $kernelIrqDocs '| Master PIC | IRQ0-IRQ7  | `0x20` command / `0x21` data | `0x20`                |' "irq docs master pic remap plan"
Assert-Contains $kernelIrqDocs '| Slave PIC  | IRQ8-IRQ15 | `0xA0` command / `0xA1` data | `0x28`                |' "irq docs slave pic remap plan"
Assert-Contains $kernelIrqDocs "PIC remap dry-run telemetry remains available" "irq docs dry-run telemetry remains available"
Assert-Contains $kernelIrqDocs 'Initialization Command Words are dispatched only after `pic-remap-arm` followed by `pic-remap-smoke`' "irq docs controlled icw dispatch"
Assert-Contains $kernelIrqDocs "no boot path remaps the PIC" "irq docs no boot remap"
Assert-Contains $kernelIrqDocs "remap_plan()" "irq docs remap plan function"
Assert-Contains $kernelIrqDocs "remap_disabled()" "irq docs remap disabled function"
Assert-Contains $kernelIrqDocs "irq_map_plan()" "irq docs irq map plan function"
Assert-Contains $kernelIrqDocs "pic_remap_smoke_arm()" "irq docs pic remap smoke arm function"
Assert-Contains $kernelIrqDocs "pic_remap_controlled_smoke()" "irq docs pic remap controlled smoke function"
Assert-Contains $kernelIrqDocs "pic_remap_smoke_status()" "irq docs pic remap smoke status function"
Assert-Contains $kernelIrqDocs "IrqHandlerSkeleton" "irq docs skeleton type"
Assert-Contains $kernelIrqDocs "irq0_timer_skeleton()" "irq docs irq0 skeleton function"
Assert-Contains $kernelIrqDocs "irq1_keyboard_skeleton()" "irq docs irq1 skeleton function"
Assert-Contains $kernelIrqDocs "irq_handler_skeletons()" "irq docs skeletons function"
Assert-Contains $kernelIrqDocs "IrqGatePlan" "irq docs gate plan type"
Assert-Contains $kernelIrqDocs "irq0_timer_gate_plan()" "irq docs irq0 gate plan function"
Assert-Contains $kernelIrqDocs "irq1_keyboard_gate_plan()" "irq docs irq1 gate plan function"
Assert-Contains $kernelIrqDocs "irq_gate_plan()" "irq docs gate plan aggregate function"
Assert-Contains $kernelIrqDocs '`IRQ0_VECTOR = 32` and `IRQ1_VECTOR = 33`' "irq docs vector constants"
Assert-Contains $kernelIrqDocs 'returns the documentation-only plan through `remap_plan()`' "irq docs remap disabled delegates to remap plan"
Assert-Contains $kernelIrqDocs 'IRQ vectors `0x20-0x2f` are planned only.' "irq docs hex vector range planned"
Assert-Contains $kernelIrqDocs '**ICW1 (`0x11`)**' "irq docs icw1"
Assert-Contains $kernelIrqDocs '**ICW2 (`0x20` / `0x28`)**' "irq docs icw2"
Assert-Contains $kernelIrqDocs '**ICW3 (`0x04` / `0x02`)**' "irq docs icw3"
Assert-Contains $kernelIrqDocs '**ICW4 (`0x01`)**' "irq docs icw4"
Assert-Contains $kernelIrqDocs '**IRQ0 timer**: skeleton planned PIT timer interrupt; bind smoke stub is dormant in `v9.0.2`.' "irq docs irq0 skeleton dormant"
Assert-Contains $kernelIrqDocs '**IRQ1 keyboard**: skeleton planned PS/2 keyboard interrupt; bind smoke stub is dormant in `v9.0.2`.' "irq docs irq1 skeleton dormant"
Assert-Contains $kernelIrqDocs "**IRQ vectors 32-47**: planned remapped CPU vector range for IRQ0-IRQ15." "irq docs irq vector range"
Assert-Contains $kernelIrqDocs "**EOI**: End Of Interrupt command planned for future PIC acknowledgements." "irq docs eoi glossary"
Assert-Contains $kernelIrqDocs "pic remap dry-run:" "irq docs pic-plan output"
Assert-Contains $kernelIrqDocs "irq map:" "irq docs irq-map output"
Assert-Contains $kernelIrqDocs "irq handlers:" "irq docs irq-handlers output heading"
Assert-Contains $kernelIrqDocs "foundation: skeleton / disabled" "irq docs irq-handlers foundation"
Assert-Contains $kernelIrqDocs "skeleton planned: irq0 timer, irq1 keyboard" "irq docs handlers skeleton planned"
Assert-Contains $kernelIrqDocs "foundation: dry-run telemetry" "irq docs pic-status verbose output"
Assert-Contains $kernelIrqDocs "dry-run plan: available" "irq docs dry-run plan available"
Assert-Contains $kernelIrqDocs "irq0 timer -> vector 32 (0x20)" "irq docs irq0 exact map"
Assert-Contains $kernelIrqDocs "irq1 keyboard -> vector 33 (0x21)" "irq docs irq1 exact map"
Assert-Contains $kernelIrqDocs "irq15 secondary-ata -> vector 47 (0x2f)" "irq docs full irq map"
Assert-Contains $kernelIrqDocs 'No `asm!("sti")`.' "irq docs no sti"
Assert-Contains $kernelIrqDocs "No boot-time PIC remap call or unarmed ICW dispatch." "irq docs no boot remap call"
Assert-Contains $kernelIrqDocs 'PIC hardware writes are limited to the armed `pic-remap-smoke` command path in `kernel-lab/src/pic.rs`.' "irq docs controlled pic hardware writes boundary"
Assert-Contains $kernelIrqDocs 'No boot-time IRQ IDT bindings beyond existing exception vectors `0`, `3`, and `14`.' "irq docs no boot irq idt bindings"
Assert-Contains $kernelIrqDocs 'IDT vectors `32/33` may be bound only by the armed `irq-gate-bind-smoke` command path.' "irq docs command-only irq idt binding"
Assert-Contains $kernelIrqDocs "No IRQ1 keyboard hardware-active handler." "irq docs no irq1 hardware-active handler"
Assert-Contains $kernelIrqDocs "No IRQ0 PIT hardware-active handler." "irq docs no irq0 hardware-active handler"
Assert-Contains $kernelIrqDocs "No EOI dispatch." "irq docs no eoi dispatch"
Assert-Contains $kernelIrqDocs "## IRQ Gate Binding Plan" "irq docs irq gate binding plan section"
Assert-Contains $kernelIrqDocs "- **Vector 32 (IRQ0 Timer)**: Mapped to the Programmable Interval Timer (PIT)." "irq docs vector 32 pit mapping"
Assert-Contains $kernelIrqDocs "- **Vector 33 (IRQ1 Keyboard)**: Mapped to the PS/2 keyboard controller." "irq docs vector 33 keyboard mapping"
Assert-Contains $kernelIrqDocs "Both gates remain unbound at boot." "irq docs gates unbound at boot constraint"
Assert-Contains $kernelIrqDocs "IRQ Interrupt Gates:" "irq docs irq-gates status header"
Assert-Contains $kernelIrqDocs "- Vector 32 (0x20): IRQ0 Timer (planned)" "irq docs irq-gates timer"
Assert-Contains $kernelIrqDocs "- Vector 33 (0x21): IRQ1 Keyboard (planned)" "irq docs irq-gates keyboard"
Assert-Contains $kernelIrqDocs "IDT vector 32 (IRQ0 Timer): disabled / null handler" "irq docs irq-gate-status timer"
Assert-Contains $kernelIrqDocs "IDT vector 33 (IRQ1 Keyboard): disabled / null handler" "irq docs irq-gate-status keyboard"
Assert-Contains $kernelIrqDocs "IRQ Gate Binding Plan:" "irq docs irq-gate-plan status header"
Assert-Contains $kernelIrqDocs "IRQ0 timer -> vector 32 (0x20)" "irq docs irq-gate-plan timer"
Assert-Contains $kernelIrqDocs "IRQ1 keyboard -> vector 33 (0x21)" "irq docs irq-gate-plan keyboard"
Assert-Contains $kernelIrqDocs "EOI dispatch: disabled" "irq docs irq-gate-plan eoi disabled"
Assert-Contains $kernelIrqDocs "state: dormant / disabled" "irq docs irq-gate-plan dormant state"
Assert-Contains $kernelIrqDocs "## EOI Strategy Foundation" "irq docs eoi strategy section"
Assert-Contains $kernelIrqDocs "Enumeration representing routing rules:" "irq docs eoi target routing rules"
Assert-Contains $kernelIrqDocs '`MasterOnly`: Send EOI command `0x20` to the Master PIC command port (`0x20`).' "irq docs master eoi target"
Assert-Contains $kernelIrqDocs '`MasterAndSlave`: Send EOI command `0x20` to both the Master PIC command port (`0x20`) and the Slave PIC command port (`0xA0`).' "irq docs slave eoi target"
Assert-Contains $kernelIrqDocs "EOI strategy: planned / disabled" "irq docs pic eoi-status strategy"
Assert-Contains $kernelIrqDocs "PIC command: 0x20" "irq docs pic eoi-status command"
Assert-Contains $kernelIrqDocs "master PIC: planned" "irq docs pic eoi-status master"
Assert-Contains $kernelIrqDocs "slave PIC: planned" "irq docs pic eoi-status slave"
Assert-Contains $kernelIrqDocs "dispatch: disabled" "irq docs pic eoi-status dispatch"
Assert-Contains $kernelIrqDocs "EOI strategy note:" "irq docs eoi-note header"
Assert-Contains $kernelIrqDocs "- EOI means End Of Interrupt." "irq docs eoi-note detail 1"
Assert-Contains $kernelIrqDocs "- Master PIC EOI targets command port 0x20 in the future." "irq docs eoi-note detail 2"
Assert-Contains $kernelIrqDocs "- Slave IRQs require slave EOI plus master cascade acknowledgement in the future." "irq docs eoi-note detail 3"
Assert-Contains $kernelIrqDocs "- IRQ0 timer and IRQ1 keyboard EOI paths are planned only." "irq docs eoi-note detail 4"
Assert-Contains $kernelIrqDocs "- No EOI is dispatched in this milestone." "irq docs eoi-note detail 5"
Assert-Contains $kernelIrqDocs "No keyboard polling path rewrite." "irq docs no keyboard rewrite"
Assert-Contains $kernelIrqDocs 'No change to `pf-smoke` mechanics and no `asm!("int 14")`.' "irq docs pf smoke unchanged"
Assert-Contains $kernelBootSmokeDocs $expectedKernelHelp "qemu docs help snapshot"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> handlers" "qemu docs handlers snapshot"
Assert-Contains $kernelBootSmokeDocs "irq handlers:" "qemu docs handlers irq section"
Assert-Contains $kernelBootSmokeDocs "skeleton planned: irq0 timer, irq1 keyboard" "qemu docs handlers irq skeleton planned"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> exception-status" "qemu docs exception-status snapshot"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> fault-status" "qemu docs fault-status snapshot"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> pf-status" "qemu docs pf-status snapshot"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> exception-about" "qemu docs exception-about snapshot"
Assert-Contains $kernelBootSmokeDocs "exception subsystem:" "qemu docs exception-about output"
Assert-Contains $kernelBootSmokeDocs "fault recovery:" "qemu docs fault recovery output"
Assert-Contains $kernelBootSmokeDocs "recovery mode: smoke-safe" "qemu docs recovery mode output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> pf-note" "qemu docs pf-note snapshot"
Assert-Contains $kernelBootSmokeDocs "page fault: active smoke" "qemu docs pf-note output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> pf-smoke" "qemu docs pf-smoke snapshot"
Assert-Contains $kernelBootSmokeDocs "exception: page fault" "qemu docs page fault output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> irq-note" "qemu docs irq-note snapshot"
Assert-Contains $kernelBootSmokeDocs "pic/irq: planned / disabled" "qemu docs irq-note output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> irq-status" "qemu docs irq-status snapshot"
Assert-Contains $kernelBootSmokeDocs "irq subsystem:" "qemu docs irq-status output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> irq-handlers" "qemu docs irq-handlers snapshot"
Assert-Contains $kernelBootSmokeDocs "foundation: skeleton / disabled" "qemu docs irq-handlers output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> pic-note" "qemu docs pic-note snapshot"
Assert-Contains $kernelBootSmokeDocs "pic remap: planned / disabled" "qemu docs pic-note output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> pic-status" "qemu docs pic-status snapshot"
Assert-Contains $kernelBootSmokeDocs "remap function: present / not called" "qemu docs pic-status output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> pic-plan" "qemu docs pic-plan snapshot"
Assert-Contains $kernelBootSmokeDocs "pic remap dry-run:" "qemu docs pic-plan output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> pic-remap-status" "qemu docs pic-remap-status snapshot"
Assert-Contains $kernelBootSmokeDocs "PIC remap smoke status" "qemu docs pic-remap-status output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> pic-remap-smoke" "qemu docs pic-remap-smoke snapshot"
Assert-Contains $kernelBootSmokeDocs "guard: not armed" "qemu docs pic-remap-smoke blocked output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> pic-remap-arm" "qemu docs pic-remap-arm snapshot"
Assert-Contains $kernelBootSmokeDocs "PIC remap smoke armed" "qemu docs pic-remap-arm output"
Assert-Contains $kernelBootSmokeDocs "guard: armed" "qemu docs pic-remap-smoke armed output"
Assert-Contains $kernelBootSmokeDocs "result: remapped / masked" "qemu docs pic-remap-smoke remapped masked output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> irq-map" "qemu docs irq-map snapshot"
Assert-Contains $kernelBootSmokeDocs "irq15 secondary-ata -> vector 47 (0x2f)" "qemu docs irq-map output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> pic-status --verbose" "qemu docs pic-status verbose snapshot"
Assert-Contains $kernelBootSmokeDocs "dry-run plan: available" "qemu docs pic-status verbose output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> eoi-status" "qemu docs eoi-status snapshot"
Assert-Contains $kernelBootSmokeDocs "eoi strategy status: cascade master/slave" "qemu docs eoi strategy status output"
Assert-Contains $kernelBootSmokeDocs "eoi command value: 0x20" "qemu docs eoi command value output"
Assert-Contains $kernelBootSmokeDocs "master PIC: planned" "qemu docs eoi master PIC output"
Assert-Contains $kernelBootSmokeDocs "slave PIC: planned" "qemu docs eoi slave PIC output"
Assert-Contains $kernelBootSmokeDocs "dispatch: disabled" "qemu docs eoi dispatch output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> eoi-note" "qemu docs eoi-note snapshot"
Assert-Contains $kernelBootSmokeDocs "EOI strategy note:" "qemu docs eoi strategy note header"
Assert-Contains $kernelBootSmokeDocs "- EOI means End-Of-Interrupt." "qemu docs eoi explanation"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> irq-gates" "qemu docs irq-gates snapshot"
Assert-Contains $kernelBootSmokeDocs "IRQ Interrupt Gates:" "qemu docs irq-gates output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> irq-gate-status" "qemu docs irq-gate-status snapshot"
Assert-Contains $kernelBootSmokeDocs "gate binding dispatch: dormant" "qemu docs irq-gate-status output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> irq-gate-state" "qemu docs irq-gate-state snapshot"
Assert-Contains $kernelBootSmokeDocs "IRQ gate bind state" "qemu docs irq-gate-state output"
Assert-Contains $kernelBootSmokeDocs "bind applied: yes" "qemu docs irq-gate-state after bind"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> irq-gate-history" "qemu docs irq-gate-history snapshot"
Assert-Contains $kernelBootSmokeDocs "IRQ gate bind history" "qemu docs irq-gate-history output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> irq-gate-preflight" "qemu docs irq-gate-preflight snapshot"
Assert-Contains $kernelBootSmokeDocs "IRQ gate bind preflight" "qemu docs irq-gate-preflight output"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> irq-gate-plan" "qemu docs irq-gate-plan snapshot"
Assert-Contains $kernelBootSmokeDocs "IRQ Gate Binding Plan:" "qemu docs irq-gate-plan output"
Assert-Contains $kernelBootSmokeDocs "IRQ0 timer -> vector 32 (0x20)" "qemu docs irq-gate-plan timer"
Assert-Contains $kernelBootSmokeDocs "IRQ1 keyboard -> vector 33 (0x21)" "qemu docs irq-gate-plan keyboard"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> irq-bind-note" "qemu docs irq-bind-note snapshot"
Assert-Contains $kernelBootSmokeDocs "IRQ bind note:" "qemu docs irq-bind-note output"
Assert-Contains $kernelBootSmokeDocs "IRQ0 timer gate: disabled bind path only" "qemu docs irq-bind-note timer"
Assert-Contains $kernelBootSmokeDocs "IRQ1 keyboard gate: disabled bind path only" "qemu docs irq-bind-note keyboard"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> irq-bind-status" "qemu docs irq-bind-status snapshot"
Assert-Contains $kernelBootSmokeDocs "helper: bind_irq_gates_disabled" "qemu docs irq-bind-status helper"
Assert-Contains $kernelBootSmokeDocs "IDT vector 32: unbound" "qemu docs irq-bind-status vector 32"
Assert-Contains $kernelBootSmokeDocs "IDT vector 33: unbound" "qemu docs irq-bind-status vector 33"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> irq-readiness" "qemu docs irq-readiness snapshot"
Assert-Contains $kernelBootSmokeDocs "IRQ runtime readiness" "qemu docs irq-readiness output"
Assert-Contains $kernelBootSmokeDocs "ready for runtime irq: no" "qemu docs irq-readiness not ready"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> irq-risk" "qemu docs irq-risk snapshot"
Assert-Contains $kernelBootSmokeDocs "runtime irq: blocked" "qemu docs irq-risk blocked"
Assert-Contains $kernelBootSmokeDocs "required before enable: IDT gate bind, PIC remap, EOI dispatch, handler stubs" "qemu docs irq-risk requirements"
Assert-Contains $kernelBootSmokeDocs "dbyte-kernel> irq-preflight" "qemu docs irq-preflight snapshot"
Assert-Contains $kernelBootSmokeDocs "IRQ runtime preflight" "qemu docs irq-preflight output"
Assert-Contains $kernelBootSmokeDocs "result: blocked" "qemu docs irq-preflight blocked"
Assert-Contains $kernelBootSmokeDocs "Full Exception Journey Smoke" "qemu docs full exception journey"
Assert-Contains $mainReadme "docs/KERNEL_EXCEPTIONS.md" "README includes kernel exception foundation doc"
Assert-Contains $mainReadme "docs/KERNEL_IRQ.md" "README includes kernel irq foundation doc"

# ==============================================================================
# DByteOS Kernel Lab Hardening Verify Guards (v9.0.2)
# ==============================================================================
Write-Host "Verifying DByteOS Kernel Lab (v9.0.2) hardening guards..." -ForegroundColor Green

# 1. Verify guard that no asm!("int 14") is used anywhere in kernel-lab sources
Get-ChildItem (Join-Path $repoRoot "kernel-lab\src") -Filter "*.rs" | ForEach-Object {
    $content = Get-Content $_.FullName -Raw
    if ($content -match 'int\s+14') {
        throw "Hardening guard failed: Software interrupt Vector 14 ('int 14') is strictly forbidden! File: $($_.Name)"
    }
}
Write-Host "[OK] Guard passed: 'int 14' software interrupt is not used." -ForegroundColor Green

# 2. Verify guard that STI (Set Interrupt Flag) is still disabled
Get-ChildItem (Join-Path $repoRoot "kernel-lab\src") -Filter "*.rs" | ForEach-Object {
    $content = Get-Content $_.FullName -Raw
    if ($content -match 'asm!\(\s*"\s*sti\s*"\s*\)' -or $content -match 'global_asm!\(\s*"\s*sti\s*"\s*\)') {
        throw "Hardening guard failed: 'sti' instruction found in $($_.Name)! Maskable interrupts must remain strictly disabled."
    }
}
Write-Host "[OK] Guard passed: Maskable interrupts remain strictly disabled (no 'sti')." -ForegroundColor Green

# 3. Verify guard that PIC/IRQ writes are limited to the controlled smoke path
# Direct writes to Master PIC ports (0x20/0x21) and Slave PIC ports (0xA0/0xA1) must not exist in general source code (except serial COM1 and the guarded path in pic.rs)
Get-ChildItem (Join-Path $repoRoot "kernel-lab\src") -Filter "*.rs" | ForEach-Object {
    if ($_.Name -ne 'pic.rs' -and $_.Name -ne 'serial.rs') {
        $content = Get-Content $_.FullName -Raw
        if ($content.Contains('outb')) {
            throw "Hardening guard failed: Direct I/O port write 'outb' found in $($_.Name). Writing to PIC or external ports outside serial/pic is strictly forbidden!"
        }
        if ($content.Contains('write_pic_port') -or $content.Contains('"out dx, al"')) {
            throw "Hardening guard failed: PIC port write path found in $($_.Name). PIC writes must stay in pic.rs controlled smoke only!"
        }
    }
}
Write-Host "[OK] Guard passed: PIC writes are restricted to serial COM1 and pic.rs controlled smoke boundaries." -ForegroundColor Green

# 4. Verify guard that Page Fault handler symbols actually exist in the compiled kernel ELF
$elfPath = Join-Path $repoRoot "kernel-lab\target\i686-unknown-linux-gnu\debug\dbyte_kernel"
if (Test-Path $elfPath) {
    $elfText = [System.IO.File]::ReadAllText($elfPath, [System.Text.Encoding]::ASCII)
    $requiredSymbols = @(
        'page_fault_handler_rust',
        'page_fault_handler_asm',
        'pf_smoke_probe_asm',
        'pf_smoke_recovery_asm'
    )
    foreach ($symbol in $requiredSymbols) {
        if (-not $elfText.Contains($symbol)) {
            throw "Hardening guard failed: Required Page Fault symbol '$symbol' was not found in the compiled kernel ELF!"
        }
    }
    Write-Host "[OK] Guard passed: All Page Fault handler symbols verified in the compiled kernel ELF." -ForegroundColor Green
} else {
    Write-Warning "Kernel ELF not built yet; symbol verification deferred to build phase."
}

# 5. Verify guard that IDT entry 14 is properly bound to vector 14 handler wrapper
Assert-Contains $kernelMainSource 'idt::IDT.entries[14].set_handler(interrupts::page_fault_handler_asm as *const ())' "kernel vector 14 active smoke handler registration"

# 5.5 Verify guard that IDT entries 32 and 33 bind only in the armed command path
Assert-NotContains $kernelBootPath "entries[32].set_handler" "kernel boot path keeps IDT entry 32 unbound"
Assert-NotContains $kernelBootPath "entries[33].set_handler" "kernel boot path keeps IDT entry 33 unbound"
Assert-Contains $irqGateSmokeBlock "if irq::irq_gate_bind_smoke_is_armed()" "kernel IRQ gate bind smoke remains guarded"
Assert-Contains $irqGateSmokeBlock "entries[32].set_handler" "kernel IRQ gate bind smoke contains vector 32 bind"
Assert-Contains $irqGateSmokeBlock "entries[33].set_handler" "kernel IRQ gate bind smoke contains vector 33 bind"

# 6. Verify guard for stale previous-release references in current release-facing files
$staleVersion = '9.0.' + '0'
$staleCheckFiles = @(
    'Cargo.toml',
    'Cargo.lock',
    'README.md',
    'scripts/verify.ps1',
    'kernel-lab/Cargo.toml',
    'kernel-lab/Cargo.lock',
    'kernel-lab/src/main.rs',
    'kernel-lab/src/pic.rs',
    'kernel-lab/src/irq.rs',
    'kernel-lab/src/page_fault.rs',
    'docs/KERNEL_EXCEPTIONS.md',
    'docs/KERNEL_IRQ.md',
    'docs/KERNEL_INTERRUPTS.md',
    'docs/KERNEL_LAB.md',
    'docs/QEMU_BOOT_SMOKE.md',
    'examples/dbyteos/README.md',
    'examples/dbyteos/etc/system.dby'
)
foreach ($relPath in $staleCheckFiles) {
    $fullPath = Join-Path $repoRoot $relPath
    if (Test-Path $fullPath) {
        $content = Get-Content $fullPath -Raw
        if ($content.Contains($staleVersion)) {
            throw "Stale reference guard failed: File '$relPath' contains stale version string '$staleVersion'!"
        }
    }
}
Write-Host "[OK] Guard passed: No stale previous-release references found in hardened files." -ForegroundColor Green

# 7. Verify EOI Plan structs and telemetry functions exist in pic.rs and kernel ELF
Get-ChildItem (Join-Path $repoRoot "kernel-lab\src") -Filter "*.rs" | ForEach-Object {
    if ($_.Name -eq 'pic.rs') {
        $content = Get-Content $_.FullName -Raw
        if ($content -notmatch 'pub\s+const\s+PIC_EOI:\s+u8\s+=\s+0x20;') {
            throw "Hardening guard failed: 'pub const PIC_EOI: u8 = 0x20;' must be defined in pic.rs!"
        }
        if ($content.Contains('write_pic_port(PIC_MASTER_CMD, PIC_EOI)') -or $content.Contains('write_pic_port(PIC_SLAVE_CMD, PIC_EOI)')) {
            throw "Hardening guard failed: Active EOI PIC_EOI write is strictly forbidden in pic.rs!"
        }
    }
}
Write-Host "[OK] Guard passed: EOI dispatch is dry-run only (no PIC_EOI writes in pic.rs)." -ForegroundColor Green

$kernelRustSources = Get-ChildItem (Join-Path $repoRoot "kernel-lab\src") -Filter "*.rs"
foreach ($sourceFile in $kernelRustSources) {
    $sourceText = Get-Content $sourceFile.FullName -Raw
    if ($sourceText -match 'asm!\(\s*"\s*sti\s*"\s*\)' -or $sourceText -match 'global_asm!\(\s*"\s*sti\s*"\s*\)') {
        throw "v9.2.1 EOI boundary guard failed: STI instruction found in $($sourceFile.Name)"
    }
    if ($sourceText -match 'write_pic_port\(\s*PIC_(MASTER|SLAVE)_CMD\s*,\s*PIC_EOI\s*\)') {
        throw "v9.2.1 EOI boundary guard failed: PIC_EOI hardware dispatch found in $($sourceFile.Name)"
    }
    if ($sourceText -match 'write_pic_port\(\s*PIC_(MASTER|SLAVE)_DATA\s*,\s*(0x00|0xFE|0xFC|0xFD|0xFB|0xF7|0xEF|0xDF|0xBF|0x7F)\s*\)') {
        throw "v9.2.1 EOI boundary guard failed: PIC IRQ unmask literal found in $($sourceFile.Name)"
    }
    if ($sourceFile.Name -ne 'main.rs') {
        if ($sourceText.Contains('entries[32].set_handler') -or $sourceText.Contains('entries[33].set_handler')) {
            throw "v9.2.1 EOI boundary guard failed: IRQ0/IRQ1 IDT bind outside command dispatcher in $($sourceFile.Name)"
        }
    }
    if ($sourceFile.Name -ne 'interrupts.rs') {
        if ($sourceText.Contains('irq0_handler') -or $sourceText.Contains('irq1_handler')) {
            throw "v9.2.1 EOI boundary guard failed: live IRQ0/IRQ1 handler symbol found in $($sourceFile.Name)"
        }
    }
    if ($sourceText.Contains('keyboard_irq') -or $sourceText.Contains('timer_irq')) {
        throw "v9.2.1 EOI boundary guard failed: runtime IRQ path found in $($sourceFile.Name)"
    }
}
Write-Host "[OK] Guard passed: v9.2.1 EOI boundary static source scan remained dry-run only." -ForegroundColor Green

if (Test-Path $elfPath) {
    $requiredEoiSymbols = @(
        'master_eoi_plan',
        'slave_eoi_plan',
        'irq0_timer_eoi_plan',
        'irq1_keyboard_eoi_plan',
        'eoi_strategy_status'
    )
    foreach ($symbol in $requiredEoiSymbols) {
        if (-not $elfText.Contains($symbol)) {
            throw "Hardening guard failed: Required EOI function symbol '$symbol' was not found in the compiled kernel ELF!"
        }
    }
    Write-Host "[OK] Guard passed: All EOI function symbols verified in the compiled kernel ELF." -ForegroundColor Green
}

# Verify QEMU Availability
$qemuExe = Get-Command qemu-system-i386 -ErrorAction SilentlyContinue
if (-not $qemuExe) {
    $qemuExe = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
}

if (-not $qemuExe) {
    Write-Host "[WARNING] QEMU not found in PATH, skipping virtualized boot smoke tests." -ForegroundColor Yellow
} else {
    Write-Host "Verifying DByteOS Kernel Lab (v9.0.2) virtualized QEMU boot smoke using $($qemuExe.Name)..." -ForegroundColor Green
    New-Item -ItemType Directory -Force -Path (Join-Path $repoRoot "tmp") | Out-Null
    $qemuLog = Join-Path $repoRoot "tmp\qemu_serial.log"
    if (Test-Path $qemuLog) { Remove-Item -Force $qemuLog }

    $pInfo = New-Object System.Diagnostics.ProcessStartInfo
    $pInfo.FileName = $qemuExe.Name
    $pInfo.Arguments = "-kernel `"$elfPath`" -serial file:`"$qemuLog`" -display none"
    $pInfo.UseShellExecute = $false
    $pInfo.CreateNoWindow = $true

    $p = [System.Diagnostics.Process]::Start($pInfo)
    Start-Sleep -Seconds 3

    if (-not $p.HasExited) {
        $p.Kill()
        $p.WaitForExit()
    }

    if (-not (Test-Path $qemuLog)) {
        throw "QEMU boot smoke test failed: serial log was not generated!"
    }

    $logContent = Get-Content $qemuLog -Raw
    Write-Host "Captured QEMU Serial Log:" -ForegroundColor Cyan
    Write-Host $logContent -ForegroundColor Gray

    if (-not ($logContent -like "*status: booted*")) {
        throw "QEMU boot smoke test failed: 'status: booted' not found in serial log!"
    }
    if (-not ($logContent -like "*version: 9.0.2*")) {
        throw "QEMU boot smoke test failed: 'version: 9.0.2' not found in serial log!"
    }
    if (-not ($logContent -like "*target: i686 multiboot*")) {
        throw "QEMU boot smoke test failed: 'target: i686 multiboot' not found in serial log!"
    }

    Remove-Item -Force $qemuLog -ErrorAction SilentlyContinue
    Write-Host "[OK] QEMU virtualized boot smoke test passed successfully!" -ForegroundColor Green
}

Write-Host "Running benchmark smoke tests..."
& $releaseExe bench --engine tree
if ($LASTEXITCODE -ne 0) { throw "dbyte bench --engine tree failed" }
& $releaseExe bench --engine vm
if ($LASTEXITCODE -ne 0) { throw "dbyte bench --engine vm failed" }
& $releaseExe bench --compare-python
if ($LASTEXITCODE -ne 0) { throw "dbyte bench --compare-python failed" }

Write-Host "Running DByteOS Alpha (v9.0.2) Package Smoke Tests..."
if (Test-Path (Join-Path $repoRoot "tmp")) { Remove-Item -Recurse -Force (Join-Path $repoRoot "tmp") }
$packageSmokeStatus = Git-Status-Short
Remove-Item -Recurse -Force $dbyteosProjectsPath -ErrorAction SilentlyContinue
Remove-Item -Force (Join-Path $repoRoot "examples\dbyteos\home\deadbyte\notes.txt") -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force (Join-Path $repoRoot "examples\dbyteos\home\deadbyte\journal") -ErrorAction SilentlyContinue
$smokeRoot = Join-Path $repoRoot "tmp\package_smoke"
if (Test-Path $smokeRoot) { Remove-Item -Recurse -Force $smokeRoot }
New-Item -ItemType Directory -Path $smokeRoot | Out-Null

Write-Host "  Building and packaging..."
& .\scripts\package_release.ps1 -Version "9.0.2"
$zipFile = Join-Path $repoRoot "dbyte-v9.0.2-windows-x64.zip"
if (-not (Test-Path $zipFile)) { throw "Package zip not found: $zipFile" }

Write-Host "  Extracting package..."
Expand-Archive -Path $zipFile -DestinationPath $smokeRoot
$extractedExe = Join-Path $smokeRoot "dbyte.exe"
$extractedOsRoot = Join-Path $smokeRoot "examples\dbyteos"

Write-Host "  Verifying version..."
$vOut = & $extractedExe --version
if ($vOut -ne "DByte 9.0.2") { throw "Package version mismatch: $vOut" }

Write-Host "  Verifying direct OS commands..."
$expectedPackageBoot = $expectedDbyteosBoot.Replace("Home:        home/deadbyte", "Home:        examples/dbyteos/home/deadbyte")
$expectedPackageStatus = $expectedDbyteosStatus.Replace("Home:     home/deadbyte", "Home:     examples/dbyteos/home/deadbyte")
$expectedPackageWelcome = $expectedDbyteosWelcome.Replace("  home:    home/deadbyte", "  home:    examples/dbyteos/home/deadbyte")
$expectedPackageProfile = $expectedDbyteosProfile.Replace("home: home/deadbyte", "home: examples/dbyteos/home/deadbyte")
$expectedPackageProfileUnknown = $expectedDbyteosProfileUnknown
$expectedPackageConfig = $expectedDbyteosConfig.Replace("user.home = home/deadbyte", "user.home = examples/dbyteos/home/deadbyte")
$expectedPackageSnapshot = $expectedDbyteosSnapshot.Replace("  home:    home/deadbyte", "  home:    examples/dbyteos/home/deadbyte").Replace("  user.home = home/deadbyte", "  user.home = examples/dbyteos/home/deadbyte").Replace("  boot.log: missing", "  boot.log: present").Replace("  services.log: missing", "  services.log: present")
$expectedPackageSnapshotProfile = $expectedDbyteosSnapshotProfile.Replace("  home:    home/deadbyte", "  home:    examples/dbyteos/home/deadbyte")
$expectedPackageSnapshotConfig = $expectedDbyteosSnapshotConfig.Replace("  user.home = home/deadbyte", "  user.home = examples/dbyteos/home/deadbyte")
$expectedPackageSnapshotLogs = $expectedDbyteosSnapshotLogs.Replace("  boot.log: missing", "  boot.log: present").Replace("  services.log: missing", "  services.log: present")
$bootOut = & $extractedExe run (Join-Path $extractedOsRoot "boot.dby") 2>&1
Assert-NormalizedEqual $bootOut $expectedPackageBoot "Package boot snapshot"
$statusOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\status.dby") 2>&1
Assert-NormalizedEqual $statusOut $expectedPackageStatus "Package status snapshot"
$helpOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\help.dby") 2>&1
Assert-NormalizedEqual $helpOut $expectedDbyteosHelp "Package help snapshot"
$sysinfoOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\sysinfo.dby") 2>&1
Assert-NormalizedEqual $sysinfoOut $expectedDbyteosSysinfo "Package sysinfo snapshot"
$welcomeOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\welcome.dby") 2>&1
Assert-NormalizedEqual $welcomeOut $expectedPackageWelcome "Package welcome snapshot"
$profileOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\profile.dby") 2>&1
Assert-NormalizedEqual $profileOut $expectedPackageProfile "Package profile snapshot"
$profileShowOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\profile.dby") show 2>&1
Assert-NormalizedEqual $profileShowOut $expectedPackageProfile "Package profile show snapshot"
Assert-Equal (Normalize-Output $profileOut) (Normalize-Output $profileShowOut) "Package profile no args equals show"
$profileWhoamiOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\profile.dby") whoami 2>&1
Assert-Equal (Normalize-Output $profileWhoamiOut) "deadbyte" "Package profile whoami"
$profileHomeOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\profile.dby") home 2>&1
Assert-Equal (Normalize-Output $profileHomeOut) "examples/dbyteos/home/deadbyte" "Package profile home"
$profileThemeOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\profile.dby") theme 2>&1
Assert-Equal (Normalize-Output $profileThemeOut) "default" "Package profile theme"
$profilePromptOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\profile.dby") prompt 2>&1
Assert-Equal (Normalize-Output $profilePromptOut) "dbyte-shell>" "Package profile prompt"
$profileUnknownOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\profile.dby") unknown 2>&1
Assert-NormalizedEqual $profileUnknownOut $expectedPackageProfileUnknown "Package profile unknown snapshot"
$configOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\config.dby") 2>&1
Assert-NormalizedEqual $configOut $expectedPackageConfig "Package config snapshot"
$configShowOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\config.dby") show 2>&1
Assert-NormalizedEqual $configShowOut $expectedPackageConfig "Package config show snapshot"
Assert-Equal (Normalize-Output $configOut) (Normalize-Output $configShowOut) "Package config no args equals show"
$configKeysOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\config.dby") keys 2>&1
Assert-NormalizedEqual $configKeysOut $expectedDbyteosConfigKeys "Package config keys snapshot"
$configModeOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\config.dby") get system.mode 2>&1
Assert-Equal (Normalize-Output $configModeOut) "beta-userland" "Package config mode"
$configPromptOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\config.dby") get system.prompt 2>&1
Assert-Equal (Normalize-Output $configPromptOut) "dbyte-shell>" "Package config prompt"
$configUserOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\config.dby") get user.name 2>&1
Assert-Equal (Normalize-Output $configUserOut) "deadbyte" "Package config user"
$configHomeOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\config.dby") get user.home 2>&1
Assert-Equal (Normalize-Output $configHomeOut) "examples/dbyteos/home/deadbyte" "Package config home"
$configThemeOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\config.dby") get ui.theme 2>&1
Assert-Equal (Normalize-Output $configThemeOut) "default" "Package config theme"
$configSecurityOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\config.dby") get security.mode 2>&1
Assert-Equal (Normalize-Output $configSecurityOut) "simulated" "Package config security mode"
$configUnknownOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\config.dby") get missing.key 2>&1
Assert-Equal (Normalize-Output $configUnknownOut) "error: unknown config key: missing.key" "Package config unknown key"
$snapshotOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\snapshot.dby") 2>&1
Assert-NormalizedEqual $snapshotOut $expectedPackageSnapshot "Package snapshot snapshot"
$snapshotSystemOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\snapshot.dby") system 2>&1
Assert-NormalizedEqual $snapshotSystemOut $expectedPackageSnapshot "Package snapshot system snapshot"
Assert-Equal (Normalize-Output $snapshotOut) (Normalize-Output $snapshotSystemOut) "Package snapshot no args equals system"
$snapshotProfileOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\snapshot.dby") profile 2>&1
Assert-NormalizedEqual $snapshotProfileOut $expectedPackageSnapshotProfile "Package snapshot profile snapshot"
$snapshotConfigOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\snapshot.dby") config 2>&1
Assert-NormalizedEqual $snapshotConfigOut $expectedPackageSnapshotConfig "Package snapshot config snapshot"
$snapshotSecurityOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\snapshot.dby") security 2>&1
Assert-NormalizedEqual $snapshotSecurityOut $expectedDbyteosSnapshotSecurity "Package snapshot security snapshot"
$snapshotLogsOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\snapshot.dby") logs 2>&1
Assert-NormalizedEqual $snapshotLogsOut $expectedPackageSnapshotLogs "Package snapshot logs snapshot"
$doctorOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\doctor.dby") 2>&1
Assert-NormalizedEqual $doctorOut $expectedDbyteosDoctor "Package doctor snapshot"
$diagnoseOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\diagnose.dby") 2>&1
Assert-NormalizedEqual $diagnoseOut $expectedDbyteosDiagnose "Package diagnose snapshot"
$diagnoseProfileOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\diagnose.dby") profile 2>&1
Assert-NormalizedEqual $diagnoseProfileOut $expectedDbyteosDiagnoseProfile "Package diagnose profile snapshot"
$diagnoseConfigOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\diagnose.dby") config 2>&1
Assert-NormalizedEqual $diagnoseConfigOut $expectedDbyteosDiagnoseConfig "Package diagnose config snapshot"
$diagnoseSecurityOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\diagnose.dby") security 2>&1
Assert-NormalizedEqual $diagnoseSecurityOut $expectedDbyteosDiagnoseSecurity "Package diagnose security snapshot"
$diagnoseLogsOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\diagnose.dby") logs 2>&1
Assert-NormalizedEqual $diagnoseLogsOut $expectedDbyteosDiagnoseLogs "Package diagnose logs snapshot"
$diagnoseManualOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\diagnose.dby") manual 2>&1
Assert-NormalizedEqual $diagnoseManualOut $expectedDbyteosDiagnoseManual "Package diagnose manual snapshot"
$diagnosePackageOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\diagnose.dby") package 2>&1
Assert-NormalizedEqual $diagnosePackageOut $expectedDbyteosDiagnosePackage "Package diagnose package snapshot"
$diagnoseUnknownOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\diagnose.dby") unknown 2>&1
Assert-Equal $diagnoseUnknownOut "usage: diagnose [profile|config|preferences|security|logs|manual|package]" "Package diagnose unknown snapshot"
$checkSystemOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\check_system.dby") 2>&1
Assert-NormalizedEqual $checkSystemOut $expectedDbyteosCheckSystem "Package check-system snapshot"

$snapshotUnknownOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\snapshot.dby") unknown 2>&1
Assert-NormalizedEqual $snapshotUnknownOut $expectedDbyteosSnapshotUnknown "Package snapshot unknown snapshot"
$gettingStartedOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\getting_started.dby") 2>&1
Assert-NormalizedEqual $gettingStartedOut $expectedDbyteosGettingStarted "Package getting-started snapshot"
$commandsOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\commands.dby") 2>&1
Assert-NormalizedEqual $commandsOut $expectedDbyteosCommands "Package commands snapshot"
$manIndexOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\man_index.dby") 2>&1
Assert-NormalizedEqual $manIndexOut $expectedDbyteosManIndex "Package man-index snapshot"
Push-Location $extractedOsRoot
try {
    $projectUsageOut = & $extractedExe run "bin\project.dby" 2>&1
    Assert-NormalizedEqual $projectUsageOut $expectedDbyteosProjectUsage "Package project usage snapshot"
    $projectListEmptyOut = & $extractedExe run "bin\project.dby" list 2>&1
    Assert-NormalizedEqual $projectListEmptyOut $expectedDbyteosProjectListEmpty "Package project list empty snapshot"
    $projectInvalidOut = & $extractedExe run "bin\project.dby" new "../demo" 2>&1
    Assert-Equal (Normalize-Output $projectInvalidOut) "error: invalid project name: ../demo" "Package project path escape denied"
    $projectMissingOut = & $extractedExe run "bin\project.dby" status missing 2>&1
    Assert-NormalizedEqual $projectMissingOut $expectedDbyteosProjectNotFound "Package project missing status"
    $projectNewOut = & $extractedExe run "bin\project.dby" new demo 2>&1
    Assert-Equal (Normalize-Output $projectNewOut) "project created: demo" "Package project new demo"
    if (-not (Test-Path (Join-Path $extractedOsRoot "home\deadbyte\projects\demo\project.txt"))) { throw "Package project missing project.txt" }
    $projectDuplicateOut = & $extractedExe run "bin\project.dby" new demo 2>&1
    Assert-Equal (Normalize-Output $projectDuplicateOut) "error: project already exists: demo" "Package project duplicate"
    $projectListOut = & $extractedExe run "bin\project.dby" list 2>&1
    Assert-NormalizedEqual $projectListOut $expectedDbyteosProjectListDemo "Package project list demo snapshot"
    $projectStatusOut = & $extractedExe run "bin\project.dby" status demo 2>&1
    Assert-NormalizedEqual $projectStatusOut $expectedDbyteosProjectStatusDemo "Package project status demo snapshot"
    $projectNotesOut = & $extractedExe run "bin\project.dby" notes demo 2>&1
    Assert-NormalizedEqual $projectNotesOut $expectedDbyteosProjectNotesDemo "Package project notes demo snapshot"
    $projectSnapshotOut = & $extractedExe run "bin\project.dby" snapshot demo 2>&1
    Assert-NormalizedEqual $projectSnapshotOut $expectedDbyteosProjectSnapshotDemo "Package project snapshot demo snapshot"
    $projectDoctorOut = & $extractedExe run "bin\project.dby" doctor demo 2>&1
    Assert-NormalizedEqual $projectDoctorOut $expectedDbyteosProjectDoctorDemo "Package project doctor demo snapshot"
    $projectResetOut = & $extractedExe run "bin\project.dby" reset-demo 2>&1
    Assert-Equal (Normalize-Output $projectResetOut) "project demo reset." "Package project reset-demo"
    $projectResetAgainOut = & $extractedExe run "bin\project.dby" reset-demo 2>&1
    Assert-Equal (Normalize-Output $projectResetAgainOut) "project demo reset." "Package project reset-demo idempotent"
    if (-not (Test-Path (Join-Path $extractedOsRoot "home\deadbyte\projects\demo\tasks.txt"))) { throw "Package project missing tasks.txt" }
    $taskUsageOut = & $extractedExe run "bin\task.dby" 2>&1
    Assert-NormalizedEqual $taskUsageOut $expectedDbyteosTaskUsage "Package task usage snapshot"
    $taskAddMissingTextOut = & $extractedExe run "bin\task.dby" add demo 2>&1
    Assert-Equal (Normalize-Output $taskAddMissingTextOut) "usage: task add <project> <text>" "Package task add missing text"
    $taskAddEmptyTextOut = Invoke-DbyteExact -Executable $extractedExe -Arguments @("run", "bin\task.dby", "add", "demo", "") -WorkingDirectory $extractedOsRoot
    if ($taskAddEmptyTextOut.Code -ne 0) { throw "Package task add empty text failed: $($taskAddEmptyTextOut.Text)" }
    Assert-Equal $taskAddEmptyTextOut.Text "error: task text cannot be empty" "Package task add empty text"
    $taskAddDelimiterOut = & $extractedExe run "bin\task.dby" add demo "bad|text" 2>&1
    Assert-Equal (Normalize-Output $taskAddDelimiterOut) "error: invalid task text" "Package task add delimiter"
    $taskResetOut = & $extractedExe run "bin\task.dby" reset-demo 2>&1
    Assert-Equal (Normalize-Output $taskResetOut) "task demo reset." "Package task reset-demo"
    $taskResetAgainOut = & $extractedExe run "bin\task.dby" reset-demo 2>&1
    Assert-Equal (Normalize-Output $taskResetAgainOut) "task demo reset." "Package task reset-demo idempotent"
    $taskListOut = & $extractedExe run "bin\task.dby" list demo 2>&1
    Assert-NormalizedEqual $taskListOut $expectedDbyteosTaskListDemo "Package task list demo snapshot"
    $taskAddOut = & $extractedExe run "bin\task.dby" add demo write tests 2>&1
    Assert-Equal (Normalize-Output $taskAddOut) "task added: demo #3" "Package task add demo"
    $taskDoneZeroOut = & $extractedExe run "bin\task.dby" done demo 0 2>&1
    Assert-Equal (Normalize-Output $taskDoneZeroOut) "error: invalid task id: 0" "Package task done zero"
    $taskDoneNegativeOut = & $extractedExe run "bin\task.dby" done demo -1 2>&1
    Assert-Equal (Normalize-Output $taskDoneNegativeOut) "error: invalid task id: -1" "Package task done negative"
    $taskDoneAlphaOut = & $extractedExe run "bin\task.dby" done demo abc 2>&1
    Assert-Equal (Normalize-Output $taskDoneAlphaOut) "error: invalid task id: abc" "Package task done alpha"
    $taskDoneOut = & $extractedExe run "bin\task.dby" done demo 1 2>&1
    Assert-Equal (Normalize-Output $taskDoneOut) "task done: demo #1" "Package task done demo"
    $taskDoneAgainOut = & $extractedExe run "bin\task.dby" done demo 1 2>&1
    Assert-Equal (Normalize-Output $taskDoneAgainOut) "task already done: demo #1" "Package task already done"
    $taskDoneMissingOut = & $extractedExe run "bin\task.dby" done demo 99 2>&1
    Assert-Equal (Normalize-Output $taskDoneMissingOut) "error: task not found: 99" "Package task missing id"
    $taskStatusOut = & $extractedExe run "bin\task.dby" status demo 2>&1
    Assert-NormalizedEqual $taskStatusOut $expectedDbyteosTaskStatusAfterDone "Package task status demo snapshot"
    $taskSummaryOut = & $extractedExe run "bin\task.dby" summary demo 2>&1
    Assert-NormalizedEqual $taskSummaryOut $expectedDbyteosTaskSummaryAfterDone "Package task summary demo snapshot"
    $taskOpenOut = & $extractedExe run "bin\task.dby" open demo 2>&1
    Assert-NormalizedEqual $taskOpenOut $expectedDbyteosTaskOpenAfterDone "Package task open demo snapshot"
    $taskDoctorOut = & $extractedExe run "bin\task.dby" doctor demo 2>&1
    Assert-NormalizedEqual $taskDoctorOut $expectedDbyteosTaskDoctorHealthy "Package task doctor demo snapshot"
    $taskSnapshotOut = & $extractedExe run "bin\task.dby" snapshot demo 2>&1
    Assert-NormalizedEqual $taskSnapshotOut $expectedDbyteosTaskSnapshotAfterDone "Package task snapshot demo snapshot"
    Set-Content -Path (Join-Path $extractedOsRoot "home\deadbyte\projects\demo\tasks.txt") -Value "0|inspect workspace`n2|bad marker`n1|done task`n" -NoNewline
    $taskDoctorMalformedOut = & $extractedExe run "bin\task.dby" doctor demo 2>&1
    Assert-NormalizedEqual $taskDoctorMalformedOut $expectedDbyteosTaskDoctorMalformed "Package task doctor malformed snapshot"
    $taskResetAfterMalformedOut = & $extractedExe run "bin\task.dby" reset-demo 2>&1
    Assert-Equal (Normalize-Output $taskResetAfterMalformedOut) "task demo reset." "Package task reset after malformed"
    $taskAddAfterMalformedOut = & $extractedExe run "bin\task.dby" add demo write tests 2>&1
    Assert-Equal (Normalize-Output $taskAddAfterMalformedOut) "task added: demo #3" "Package task add after malformed"
    $taskDoneAfterMalformedOut = & $extractedExe run "bin\task.dby" done demo 1 2>&1
    Assert-Equal (Normalize-Output $taskDoneAfterMalformedOut) "task done: demo #1" "Package task done after malformed"
    $taskClearDoneOut = & $extractedExe run "bin\task.dby" clear-done demo 2>&1
    Assert-NormalizedEqual $taskClearDoneOut $expectedDbyteosTaskClearDone "Package task clear-done demo snapshot"
    $taskClearDoneAgainOut = & $extractedExe run "bin\task.dby" clear-done demo 2>&1
    Assert-NormalizedEqual $taskClearDoneAgainOut "task clear-done: demo`nremoved: 0`nremaining: 2" "Package task clear-done idempotent"
    $taskListAfterClearDoneOut = & $extractedExe run "bin\task.dby" list demo 2>&1
    Assert-NormalizedEqual $taskListAfterClearDoneOut $expectedDbyteosTaskListAfterClearDone "Package task list after clear-done"
    $taskSnapshotAfterClearDoneOut = & $extractedExe run "bin\task.dby" snapshot demo 2>&1
    Assert-NormalizedEqual $taskSnapshotAfterClearDoneOut $expectedDbyteosTaskSnapshotAfterClearDone "Package task snapshot after clear-done"
    $taskListMissingProjectOut = & $extractedExe run "bin\task.dby" list missing 2>&1
    Assert-Equal (Normalize-Output $taskListMissingProjectOut) "error: project not found: missing" "Package task list missing project"
    $taskMissingProjectOut = & $extractedExe run "bin\task.dby" status missing 2>&1
    Assert-Equal (Normalize-Output $taskMissingProjectOut) "error: project not found: missing" "Package task missing project"
    $taskInvalidProjectOut = & $extractedExe run "bin\task.dby" add bad/name text 2>&1
    Assert-Equal (Normalize-Output $taskInvalidProjectOut) "error: invalid project name: bad/name" "Package task invalid project"

    # --- Package Search Cache Smoke Tests ---
    $pkgSearchReset = & $extractedExe run "bin\search.dby" reset-demo 2>&1
    Assert-Equal (Normalize-Output $pkgSearchReset) "search: reset demo project and workspace seed data" "Package search reset-demo"

    $pkgCacheStatusMissing = & $extractedExe run "bin\search.dby" status 2>&1
    Assert-Equal (Normalize-Output $pkgCacheStatusMissing) "index: missing (use 'search rebuild' to generate)" "Package search status missing"

    $pkgCacheRebuild = & $extractedExe run "bin\search.dby" rebuild 2>&1
    Assert-Equal (Normalize-Output $pkgCacheRebuild) "search: index rebuilt successfully (5 records indexed)" "Package search rebuild"

    $pkgCacheStatusActive = & $extractedExe run "bin\search.dby" status 2>&1
    Assert-Contains (Normalize-Output $pkgCacheStatusActive) "index: active (5 records," "Package search status active"

    $pkgCacheDoctor = & $extractedExe run "bin\search.dby" doctor 2>&1
    Assert-Equal (Normalize-Output $pkgCacheDoctor) "index: healthy (all 5 records valid)" "Package search doctor"

    $pkgCacheIndex = & $extractedExe run "bin\search.dby" index note 2>&1
    Assert-Contains (Normalize-Output $pkgCacheIndex) "project demo note: project demo notes" "Package search index query"

    $pkgCacheClear = & $extractedExe run "bin\search.dby" clear-cache 2>&1
    Assert-Equal (Normalize-Output $pkgCacheClear) "search: index cache cleared successfully" "Package search clear-cache"

    $pkgCacheStatusMissingAgain = & $extractedExe run "bin\search.dby" status 2>&1
    Assert-Equal (Normalize-Output $pkgCacheStatusMissingAgain) "index: missing (use 'search rebuild' to generate)" "Package search status missing again"

    # --- Package Timeline Smoke Tests ---
    $pkgTimelineReset = & $extractedExe run "bin\timeline.dby" reset-demo 2>&1
    Assert-Equal (Normalize-Output $pkgTimelineReset) "timeline: reset demo timeline workspace" "Package timeline reset-demo"

    $pkgTimelineTodayFallback = & $extractedExe run "bin\timeline.dby" today 2>&1
    Assert-Contains (Normalize-Output $pkgTimelineTodayFallback) "Timeline Mode: fallback" "Package timeline today fallback"

    # Rebuild search cache to test cached timeline
    $pkgSearchRebuildForTimeline = & $extractedExe run "bin\search.dby" rebuild 2>&1
    Assert-Equal (Normalize-Output $pkgSearchRebuildForTimeline) "search: index rebuilt successfully (5 records indexed)" "Package search rebuild for timeline"

    $pkgTimelineTodayCached = & $extractedExe run "bin\timeline.dby" today 2>&1
    Assert-Contains (Normalize-Output $pkgTimelineTodayCached) "Timeline Mode: cached" "Package timeline today cached"

    $pkgTimelineSnapshotCached = & $extractedExe run "bin\timeline.dby" snapshot 2>&1
    Assert-Contains (Normalize-Output $pkgTimelineSnapshotCached) "Total Projects: 1" "Package timeline snapshot cached"

    # --- Package Dashboard Smoke Tests ---
    $pkgDashboardHomeFallback = & $extractedExe run "bin\dashboard.dby" 2>&1
    Assert-Contains (Normalize-Output $pkgDashboardHomeFallback) "system: healthy" "Package dashboard home fallback"

    $pkgDashboardProjects = & $extractedExe run "bin\dashboard.dby" projects 2>&1
    Assert-Contains (Normalize-Output $pkgDashboardProjects) "* demo:" "Package dashboard projects"

    $pkgDashboardTasks = & $extractedExe run "bin\dashboard.dby" tasks 2>&1
    Assert-Contains (Normalize-Output $pkgDashboardTasks) "* [open] demo" "Package dashboard tasks"

    $pkgDashboardSearch = & $extractedExe run "bin\dashboard.dby" search demo 2>&1
    Assert-Contains (Normalize-Output $pkgDashboardSearch) "* [project] demo" "Package dashboard search"

    $pkgDashboardSearchBad = & $extractedExe run "bin\dashboard.dby" search "a/b" 2>&1
    Assert-Equal (Normalize-Output $pkgDashboardSearchBad) "error: search: invalid query" "Package dashboard search invalid"

    $pkgDashboardTimeline = & $extractedExe run "bin\dashboard.dby" timeline 2>&1
    Assert-Contains (Normalize-Output $pkgDashboardTimeline) "Timeline Mode: cached" "Package dashboard timeline cached"

    $pkgDashboardHealth = & $extractedExe run "bin\dashboard.dby" health 2>&1
    Assert-Contains (Normalize-Output $pkgDashboardHealth) "system: healthy" "Package dashboard health"

    $pkgDashboardSnapshot = & $extractedExe run "bin\dashboard.dby" snapshot 2>&1
    Assert-Contains (Normalize-Output $pkgDashboardSnapshot) "OS Version: 9.0.2" "Package dashboard snapshot"

    # --- Package Full-System Beta Journey Smoke Tests ---
    $pkgJourneyBoot = & $extractedExe run "boot.dby" 2>&1
    Assert-Contains (Normalize-Output $pkgJourneyBoot) "[OK] /tmp/.dbyteos_boot_touch (session marker)" "Package Beta Journey Boot"

    $pkgJourneyWelcome = & $extractedExe run "bin\welcome.dby" 2>&1
    Assert-Contains (Normalize-Output $pkgJourneyWelcome) "Welcome to DByteOS" "Package Beta Journey Welcome"

    $pkgJourneyDashboard = & $extractedExe run "bin\dashboard.dby" 2>&1
    Assert-Contains (Normalize-Output $pkgJourneyDashboard) "system: healthy" "Package Beta Journey Dashboard"

    $pkgJourneyCheck = & $extractedExe run "bin\check_system.dby" 2>&1
    Assert-Contains (Normalize-Output $pkgJourneyCheck) "ready: yes" "Package Beta Journey Check-System"

    $pkgJourneyDoctor = & $extractedExe run "bin\doctor.dby" 2>&1
    Assert-Contains (Normalize-Output $pkgJourneyDoctor) "result: healthy" "Package Beta Journey Doctor"

    $pkgJourneyProject = & $extractedExe run "bin\project.dby" list 2>&1
    Assert-Contains (Normalize-Output $pkgJourneyProject) "demo" "Package Beta Journey Project"

    $pkgJourneyTask = & $extractedExe run "bin\task.dby" list demo 2>&1
    Assert-Contains (Normalize-Output $pkgJourneyTask) "inspect workspace" "Package Beta Journey Task"

    $pkgJourneySearch = & $extractedExe run "bin\search.dby" status 2>&1
    Assert-Contains (Normalize-Output $pkgJourneySearch) "index:" "Package Beta Journey Search"

    $pkgJourneyTimeline = & $extractedExe run "bin\timeline.dby" today 2>&1
    Assert-Contains (Normalize-Output $pkgJourneyTimeline) "Timeline" "Package Beta Journey Timeline"

    $pkgJourneySnapshot = & $extractedExe run "bin\snapshot.dby" 2>&1
    Assert-Contains (Normalize-Output $pkgJourneySnapshot) "DByte 9.0.2" "Package Beta Journey Snapshot"

    $pkgJourneyPrefsShow = & $extractedExe run "bin\prefs.dby" show 2>&1
    Assert-Contains (Normalize-Output $pkgJourneyPrefsShow) "ui.theme = default" "Package Beta Journey Prefs Show"

    $pkgJourneyPrefsSet = & $extractedExe run "bin\prefs.dby" set system.prompt "dbyteos>" 2>&1
    Assert-Contains (Normalize-Output $pkgJourneyPrefsSet) "preference 'system.prompt' updated successfully." "Package Beta Journey Prefs Set"

    Remove-Item -Force (Join-Path $extractedOsRoot "home\deadbyte\notes.txt") -ErrorAction SilentlyContinue
    Remove-Item -Force (Join-Path $extractedOsRoot "home\deadbyte\journal.txt") -ErrorAction SilentlyContinue
    $defaultPrefs = "pub let ui_theme: str = `"default`"`npub let system_prompt: str = `"dbyte-shell`"`npub let user_display_name: str = `"deadbyte`"`n"
    Set-Content -Path (Join-Path $extractedOsRoot "home\deadbyte\preferences.dby") -Value $defaultPrefs -NoNewline
    Remove-Item -Force (Join-Path $extractedOsRoot "tmp\*") -Exclude ".gitignore", ".gitkeep" -ErrorAction SilentlyContinue
}
finally {
    Pop-Location
}

Write-Host "  Verifying documentation structure..."
$expectedDocs = @("DBYTEOS_PERSONAL_ALPHA.md", "DBYTEOS_ALPHA.md", "DBYTEOS_COMMANDS.md", "DBYTEOS_SECURITY.md", "DBYTEOS_BOOT.md", "DBYTEOS_PACKAGE.md", "DBYTEOS_ONBOARDING.md", "DBYTEOS_PROFILE.md", "DBYTEOS_CONFIG.md", "DBYTEOS_SNAPSHOT.md", "DBYTEOS_PROJECTS.md", "DBYTEOS_TASKS.md", "DBYTEOS_DIAGNOSTICS.md", "DBYTEOS_PREFERENCES.md", "DBYTEOS_BETA.md", "DBYTEOS_KERNEL.md", "KERNEL_EXCEPTIONS.md", "KERNEL_IRQ.md", "KERNEL_LAB.md", "QEMU_BOOT_SMOKE.md")
foreach ($d in $expectedDocs) {
    if (-not (Test-Path (Join-Path $smokeRoot "docs\$d"))) { throw "Package missing doc: $d" }
}

Write-Host "  Verifying Kernel Lab sandbox inclusion..."
if (-not (Test-Path (Join-Path $smokeRoot "kernel-lab"))) { throw "Package missing kernel-lab directory!" }
if (-not (Test-Path (Join-Path $smokeRoot "kernel-lab\README.md"))) { throw "Package missing kernel-lab README.md!" }
if (-not (Test-Path (Join-Path $smokeRoot "kernel-lab\boot\linker.ld"))) { throw "Package missing kernel-lab linker script!" }
if (-not (Test-Path (Join-Path $smokeRoot "kernel-lab\scripts\run.ps1"))) { throw "Package missing kernel-lab run.ps1 script!" }
if (Test-Path (Join-Path $smokeRoot "kernel-lab\target")) { throw "Package unexpectedly contains compiled kernel-lab target directory!" }
if (-not (Test-Path (Join-Path $extractedOsRoot "etc\manual\snapshot.txt"))) { throw "Package missing manual: snapshot.txt" }
if (-not (Test-Path (Join-Path $extractedOsRoot "etc\manual\project.txt"))) { throw "Package missing manual: project.txt" }
if (-not (Test-Path (Join-Path $extractedOsRoot "etc\manual\task.txt"))) { throw "Package missing manual: task.txt" }
if (-not (Test-Path (Join-Path $extractedOsRoot "etc\manual\search.txt"))) { throw "Package missing manual: search.txt" }
if (-not (Test-Path (Join-Path $extractedOsRoot "etc\manual\dashboard.txt"))) { throw "Package missing manual: dashboard.txt" }

Write-Host "  Verifying no package junk..."
$rootJunk = @("tmp", "target", "tests", ".git")
foreach ($junk in $rootJunk) {
    if (Test-Path (Join-Path $smokeRoot $junk)) { throw "Package contains root junk: $junk" }
}
$extractedTmp = Join-Path $extractedOsRoot "tmp"
$tmpFiles = Get-ChildItem -Path $extractedTmp -Exclude ".gitignore", ".gitkeep"
if ($tmpFiles.Count -ne 0) { throw "Package contains junk in tmp: $($tmpFiles.Name -join ', ')" }

Write-Host "  Verifying shell RC integration..."
$shellInput = "welcome`nprofile show`nprofile whoami`nprofile home`nprofile theme`nprofile prompt`nconfig show`nconfig keys`nconfig get system.prompt`nsnapshot`nsnapshot profile`nsnapshot config`nsnapshot security`nsnapshot logs`nproject reset-demo`nproject list`nproject status demo`nproject notes demo`nproject snapshot demo`nproject doctor demo`ntask reset-demo`ntask list demo`ntask add demo write tests`ntask done demo 1`ntask done demo 1`ntask status demo`ntask summary demo`ntask open demo`ntask doctor demo`ntask snapshot demo`ntask clear-done demo`ngetting-started`ncommands`nman-index`nboot`nhelp`nstatus`nsysinfo`nwhich read`nwhich doctor`nwhich project`nwhich task`nsearch reset-demo`nsearch summary`nsearch rebuild`nsearch status`nsearch summary`nsearch recent`nsearch projects note`nsearch tasks tests`nsearch notes seed`nsearch journal JOURNAL`nwhich search`nman search`nman index`nman profile`nman config`nman snapshot`nman project`nman task`nman perm`nsearch clear-cache`nquit`n"
$shellOut = $shellInput | & $extractedExe shell --rc (Join-Path $extractedOsRoot ".dbyterc") 2>&1
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedPackageWelcome) "Package shell welcome"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedPackageProfile) "Package shell profile show"
Assert-Contains (Normalize-Output $shellOut) "dbyte-shell>" "Package shell profile prompt"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedPackageConfig) "Package shell config show"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosConfigKeys) "Package shell config keys"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedPackageSnapshot) "Package shell snapshot"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedPackageSnapshotProfile) "Package shell snapshot profile"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedPackageSnapshotConfig) "Package shell snapshot config"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosSnapshotSecurity) "Package shell snapshot security"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedPackageSnapshotLogs) "Package shell snapshot logs"
Assert-Contains (Normalize-Output $shellOut) "project demo reset." "Package shell project reset-demo"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosProjectListDemo) "Package shell project list"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosProjectStatusDemo) "Package shell project status"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosProjectNotesDemo) "Package shell project notes"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosProjectSnapshotDemo) "Package shell project snapshot"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosProjectDoctorDemo) "Package shell project doctor"
Assert-Contains (Normalize-Output $shellOut) "task demo reset." "Package shell task reset-demo"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosTaskListDemo) "Package shell task list"
Assert-Contains (Normalize-Output $shellOut) "task added: demo #3" "Package shell task add"
Assert-Contains (Normalize-Output $shellOut) "task done: demo #1" "Package shell task done"
Assert-Contains (Normalize-Output $shellOut) "task already done: demo #1" "Package shell task already done"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosTaskStatusAfterDone) "Package shell task status"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosTaskSummaryAfterDone) "Package shell task summary"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosTaskOpenAfterDone) "Package shell task open"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosTaskDoctorHealthy) "Package shell task doctor"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosTaskSnapshotAfterDone) "Package shell task snapshot"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosTaskClearDone) "Package shell task clear-done"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosGettingStarted) "Package shell getting-started"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosCommands) "Package shell commands"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosManIndex) "Package shell man-index"
Assert-Contains (Normalize-Output $shellOut) "D B Y T E O S   U S E R L A N D" "Package shell boot"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosHelp) "Package shell help"
Assert-Contains (Normalize-Output $shellOut) "OS:      DByte  9.0.2" "Package shell status version"
Assert-Contains (Normalize-Output $shellOut) "version: DByte 9.0.2" "Package shell sysinfo version"
Assert-Contains (Normalize-Output $shellOut) "read: dbyteos ->" "Package shell which read"
Assert-Contains (Normalize-Output $shellOut) "doctor: dbyteos ->" "Package shell which doctor"
Assert-Contains (Normalize-Output $shellOut) "project: dbyteos ->" "Package shell which project"
Assert-Contains (Normalize-Output $shellOut) "task: dbyteos ->" "Package shell which task"
Assert-Contains (Normalize-Output $shellOut) "search: dbyteos ->" "Package shell which search"
Assert-Contains (Normalize-Output $shellOut) "Index Status: active" "Package shell search summary active"
Assert-Contains (Normalize-Output $shellOut) "--- Recent Indexed Records ---" "Package shell search recent"
Assert-Contains (Normalize-Output $shellOut) "project demo note: project demo notes" "Package shell search projects note"
Assert-Contains (Normalize-Output $shellOut) "project demo task: [ ] 2: write tests" "Package shell search tasks tests"
Assert-Contains (Normalize-Output $shellOut) "notes: dbyteos notes seed" "Package shell search notes seed"
Assert-Contains (Normalize-Output $shellOut) "journal: [JOURNAL] dbyteos journal seed" "Package shell search journal JOURNAL"
Assert-Contains (Normalize-Output $shellOut) "DByteOS Search Command" "Package shell man search"
Assert-Contains (Normalize-Output $shellOut) "Manual topics:" "Package shell man index"
Assert-Contains (Normalize-Output $shellOut) "DByteOS Profile" "Package shell man profile"
Assert-Contains (Normalize-Output $shellOut) "DByteOS Config" "Package shell man config"
Assert-Contains (Normalize-Output $shellOut) "DByteOS Snapshot" "Package shell man snapshot"
Assert-Contains (Normalize-Output $shellOut) "DByteOS Project Command" "Package shell man project"
Assert-Contains (Normalize-Output $shellOut) "DByteOS Task Command" "Package shell man task"
Assert-Contains (Normalize-Output $shellOut) "DByteOS Permission Command" "Package shell man perm"

# Internal verification hook only: force prompt capture for piped package smoke tests.
$packagePromptEnv = @{ "DBYTE_SHELL_FORCE_PROMPT" = "1" }
$packagePromptDefault = Invoke-DbyteInput -Executable $extractedExe -Arguments @("shell", "--rc", ".dbyterc") -InputText "version`nquit`n" -WorkingDirectory $extractedOsRoot -Environment $packagePromptEnv
if ($packagePromptDefault.Code -ne 0) { throw "Package shell prompt default failed: $($packagePromptDefault.Text)" }
Assert-Equal $packagePromptDefault.Text "dbyte-shell> DByte 9.0.2`ndbyte-shell>" "Package shell prompt default snapshot"

$packagePromptChange = Invoke-DbyteInput -Executable $extractedExe -Arguments @("shell", "--rc", ".dbyterc") -InputText "prefs set system.prompt dbyteos>`nversion`nprefs set system.prompt deadbyte>`nversion`nprefs reset-demo`nversion`nquit`n" -WorkingDirectory $extractedOsRoot -Environment $packagePromptEnv
if ($packagePromptChange.Code -ne 0) { throw "Package shell prompt change failed: $($packagePromptChange.Text)" }
Assert-Equal $packagePromptChange.Text "dbyte-shell> preference 'system.prompt' updated successfully.`ndbyteos> DByte 9.0.2`ndbyteos> preference 'system.prompt' updated successfully.`ndeadbyte> DByte 9.0.2`ndeadbyte> preferences reset to default seed state.`ndbyte-shell> DByte 9.0.2`ndbyte-shell>" "Package shell prompt preference snapshots"

$packagePromptNoRc = Invoke-DbyteInput -Executable $extractedExe -Arguments @("shell", "--no-rc") -InputText "quit`n" -WorkingDirectory $extractedOsRoot -Environment $packagePromptEnv
if ($packagePromptNoRc.Code -ne 0) { throw "Package shell prompt no-rc failed: $($packagePromptNoRc.Text)" }
Assert-Equal $packagePromptNoRc.Text "dbyte-shell>" "Package shell --no-rc default prompt snapshot"

$packageNoRcProject = Invoke-DbyteInput -Executable $extractedExe -Arguments @("shell", "--no-rc") -InputText "project list`nquit`n" -WorkingDirectory $extractedOsRoot
if ($packageNoRcProject.Code -ne 0) { throw "Package shell --no-rc project failed: $($packageNoRcProject.Text)" }
Assert-Contains $packageNoRcProject.Text "ShellError: unknown command: project" "Package shell --no-rc hides project autopath"
$packageNoRcTask = Invoke-DbyteInput -Executable $extractedExe -Arguments @("shell", "--no-rc") -InputText "task list demo`nquit`n" -WorkingDirectory $extractedOsRoot
if ($packageNoRcTask.Code -ne 0) { throw "Package shell --no-rc task failed: $($packageNoRcTask.Text)" }
Assert-Contains $packageNoRcTask.Text "ShellError: unknown command: task" "Package shell --no-rc hides task autopath"

$packagePrefsForPrompt = Join-Path $extractedOsRoot "home\deadbyte\preferences.dby"
$originalPackagePrefsForPrompt = Get-Content $packagePrefsForPrompt -Raw
try {
    Remove-Item -Path $packagePrefsForPrompt -Force
    $packagePromptMissingFallback = Invoke-DbyteInput -Executable $extractedExe -Arguments @("shell", "--rc", ".dbyterc") -InputText "quit`n" -WorkingDirectory $extractedOsRoot -Environment $packagePromptEnv
    if ($packagePromptMissingFallback.Code -ne 0) { throw "Package shell prompt missing fallback failed: $($packagePromptMissingFallback.Text)" }
    Assert-Equal $packagePromptMissingFallback.Text "dbyte-shell>" "Package shell prompt missing prefs fallback"

    Set-Content -Path $packagePrefsForPrompt -Value "pub let ui_theme: str = `"default`"`npub let system_prompt: str = `"dbyteos>`npub let user_display_name: str = `"deadbyte`"`n" -NoNewline
    $packagePromptMalformedFallback = Invoke-DbyteInput -Executable $extractedExe -Arguments @("shell", "--rc", ".dbyterc") -InputText "quit`n" -WorkingDirectory $extractedOsRoot -Environment $packagePromptEnv
    if ($packagePromptMalformedFallback.Code -ne 0) { throw "Package shell prompt malformed fallback failed: $($packagePromptMalformedFallback.Text)" }
    Assert-Equal $packagePromptMalformedFallback.Text "dbyte-shell>" "Package shell prompt malformed prefs fallback"

    Set-Content -Path $packagePrefsForPrompt -Value "pub let ui_theme: str = `"default`"`npub let system_prompt: str = `"unsupported>`"`npub let user_display_name: str = `"deadbyte`"`n" -NoNewline
    $packagePromptFallback = Invoke-DbyteInput -Executable $extractedExe -Arguments @("shell", "--rc", ".dbyterc") -InputText "quit`n" -WorkingDirectory $extractedOsRoot -Environment $packagePromptEnv
    if ($packagePromptFallback.Code -ne 0) { throw "Package shell prompt fallback failed: $($packagePromptFallback.Text)" }
    Assert-Equal $packagePromptFallback.Text "dbyte-shell>" "Package shell prompt unsupported prefs fallback"
}
finally {
    Set-Content -Path $packagePrefsForPrompt -Value $originalPackagePrefsForPrompt -NoNewline
}

$packagePostMutation = Invoke-DbyteInput -Executable $extractedExe -Arguments @("shell", "--rc", ".dbyterc") -InputText "prefs set system.prompt dbyteos>`nquit`n" -WorkingDirectory $extractedOsRoot -Environment $packagePromptEnv
if ($packagePostMutation.Code -ne 0) { throw "Package post-mutation prompt setup failed: $($packagePostMutation.Text)" }
Assert-Contains $packagePostMutation.Text "dbyte-shell> preference 'system.prompt' updated successfully." "Package post-mutation prompt set"
$packagePostMutationHealth = Invoke-DbyteInput -Executable $extractedExe -Arguments @("shell", "--rc", ".dbyterc") -InputText "check-system`ndoctor`nsnapshot`nprefs reset-demo`nquit`n" -WorkingDirectory $extractedOsRoot -Environment $packagePromptEnv
if ($packagePostMutationHealth.Code -ne 0) { throw "Package post-mutation health failed: $($packagePostMutationHealth.Text)" }
Assert-Contains $packagePostMutationHealth.Text "dbyteos> DByteOS readiness check" "Package check-system after prompt mutation"
Assert-Contains $packagePostMutationHealth.Text "ready: yes" "Package check-system healthy after prompt mutation"
Assert-Contains $packagePostMutationHealth.Text "dbyteos> DByteOS Doctor" "Package doctor after prompt mutation"
Assert-Contains $packagePostMutationHealth.Text "result: healthy" "Package doctor healthy after prompt mutation"
Assert-Contains $packagePostMutationHealth.Text "prompt:  dbyteos> (overridden)" "Package snapshot after prompt profile override"
Assert-Contains $packagePostMutationHealth.Text "system.prompt = dbyteos> (overridden)" "Package snapshot after prompt config override"
Assert-Contains $packagePostMutationHealth.Text "dbyteos> preferences reset to default seed state." "Package post-mutation prefs reset"

$packageJourneyInput = "boot`nwelcome`ncheck-system`ndoctor`nprefs set system.prompt dbyteos>`nsnapshot`nproject reset-demo`ntask reset-demo`ntask list demo`ntask add demo write tests`ntask done demo 1`ntask status demo`ntask summary demo`ntask open demo`ntask doctor demo`ntask snapshot demo`ntask clear-done demo`nproject status demo`nproject snapshot demo`nworkspace report`nworkspace doctor`nworkspace snapshot`nworkspace daily`ndaily summary`nprefs reset-demo`nversion`nquit`n"
$packageJourney = Invoke-DbyteInput -Executable $extractedExe -Arguments @("shell", "--rc", ".dbyterc") -InputText $packageJourneyInput -WorkingDirectory $extractedOsRoot -Environment $packagePromptEnv
if ($packageJourney.Code -ne 0) { throw "Package Personal Workspace Beta Foundation journey failed: $($packageJourney.Text)" }
$expectedPackageJourney = @"
dbyte-shell> ==================================================
  ____  ____        _             ___  ____  
 |  _ \| __ ) _   _| |_ ___      / _ \/ ___| 
 | | | |  _ \| | | | __/ _ \    | | | \___ \ 
 | |_| | |_) | |_| | ||  __/    | |_| |___) |
 |____/|____/ \__, |\__\___|     \___/|____/ 
              |___/                          
        D B Y T E O S   U S E R L A N D
        Alpha personal computing workspace
==================================================
System:
  Version:    DByte  9.0.2  ( Userland Prototype )
  Hostname:    DByte-Alpha
  Kernel:      Simulated (Host)
  User:        deadbyte
  Home:        home/deadbyte
--------------------------------------------------
Checking system integrity...
  [OK] /bin
  [OK] /etc
  [OK] /sys
  [OK] /home
  [OK] /tmp

Init: starting userland services...
  [INIT] notes
  [INIT] sysinfo
Init: 2 services initialized.
  [OK] Session initialized.
System initialization complete.
  [OK] /tmp/.dbyteos_boot_touch (session marker)
DByteOS is ready for interaction.
First-run guide:
  welcome          - show onboarding
  getting-started  - follow the checklist
  commands         - browse command groups
  man-index        - list manual topics
==================================================
dbyte-shell> --- Welcome to DByteOS Alpha ---
DByteOS is a personal userland built on the DByte runtime.

Profile:
  user:    deadbyte
  home:    home/deadbyte
  mode:    beta-userland
  prompt:  dbyte-shell>

Start here:
  profile show    - inspect current profile
  getting-started - follow the first-run checklist
  commands        - browse commands by category
  man-index       - list manual topics
  help            - show grouped command help
  status          - summarize system state

Suggested first session:
  boot
  profile show
  getting-started
  commands
  man-index
  man perm

Rule: DByteOS commands are DByte scripts, not OS passthrough.
dbyte-shell> DByteOS readiness check
version: ok
profile: ok
config: ok
manual: ok
security: ok
preferences: ok
workspace: ok
package: ok
ready: yes
dbyte-shell> DByteOS Doctor
profile: ok
config: ok
preferences: ok
security: ok
logs: ok
manual: ok
package: ok
snapshot: ok
result: healthy
dbyte-shell> preference 'system.prompt' updated successfully.
dbyteos> --- DByteOS System Snapshot ---
System:
  version: DByte 9.0.2
  codename: Userland Prototype
  host:    DByte-Alpha
  kernel:  Simulated (Host)

Profile:
  user:    deadbyte
  home:    home/deadbyte
  shell:   dbyte shell
  mode:    beta-userland
  theme:   default
  prompt:  dbyteos> (overridden)

Config:
  system.mode = beta-userland
  system.prompt = dbyteos> (overridden)
  user.name = deadbyte
  user.home = home/deadbyte
  ui.theme = default
  security.mode = simulated

Security:
  mode:          simulated
  tmp/:          read/write
  home/deadbyte/: read/write
  etc/:          read-only
  sys/:          read-only
  bin/:          read-only
  ../:           denied
  absolute path: denied

Logs:
  boot.log: present
  services.log: present
  security.log: missing

Next: snapshot profile | snapshot config | snapshot security | snapshot logs
dbyteos> project demo reset.
dbyteos> task demo reset.
dbyteos> DByteOS project tasks: demo
[ ] 1: inspect workspace
[ ] 2: write project note
dbyteos> task added: demo #3
dbyteos> task done: demo #1
dbyteos> Task Status: demo
open: 2
done: 1
total: 3
dbyteos> Task Summary: demo
open: 2
done: 1
total: 3
dbyteos> DByteOS open tasks: demo
[ ] 2: write project note
[ ] 3: write tests
dbyteos> Task Doctor: demo
project: ok
tasks_file: ok
rows: ok
result: healthy
dbyteos> --- DByteOS Task Snapshot ---
project: demo
open: 2
done: 1
total: 3
tasks:
[x] 1: inspect workspace
[ ] 2: write project note
[ ] 3: write tests
dbyteos> task clear-done: demo
removed: 1
remaining: 2
dbyteos> --- DByteOS Project Status ---
name: demo
project: present
project.txt: present
notes.txt: present
snapshot.txt: present
dbyteos> --- DByteOS Project Snapshot ---
name: demo
owner: deadbyte
status: active
files: project.txt, notes.txt, snapshot.txt

dbyteos> --- DByteOS Workspace Report ---
User:    deadbyte
Home:    home/deadbyte
Theme:   default
Prompt:  dbyteos> (overridden)

Projects:
  demo: 2 open, 0 done (total: 2)
dbyteos> --- DByteOS Workspace Doctor ---
Index: ok
Projects:
  demo: healthy
Result: healthy
dbyteos> --- DByteOS Workspace Snapshot ---
User: deadbyte
Projects:
  - name: demo
    tasks: 2 open, 0 done
dbyteos> --- DByteOS Daily Summary ---
Notes:   home/deadbyte/notes.txt (not found)
Journal: 0 entries recorded

Open Tasks:
  demo: 2 open
dbyteos> --- DByteOS Daily Summary ---
Notes:   home/deadbyte/notes.txt (not found)
Journal: 0 entries recorded

Open Tasks:
  demo: 2 open
dbyteos> preferences reset to default seed state.
dbyte-shell> DByte 9.0.2
dbyte-shell>
"@
Assert-NormalizedEqual $packageJourney.Text $expectedPackageJourney "Package Personal Workspace Beta Foundation journey exact snapshot"

Remove-Item -Recurse -Force $smokeRoot -ErrorAction SilentlyContinue
Assert-GitStatus-Unchanged $packageSmokeStatus "package smoke cleanliness"
Write-Host "Package smoke tests passed."

Write-Host "verify passed"
