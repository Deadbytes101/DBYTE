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
$versionOut = & $cli --version
if ($versionOut -ne "DByte 4.4.1") {
    throw "Version mismatch: expected 'DByte 4.4.1', got '$versionOut'"
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

function Expected-File($path) {
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

function Invoke-DbyteInput {
    param(
        [string[]]$Arguments,
        [string]$InputText,
        [string]$WorkingDirectory = $repoRoot
    )

    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = $cli
    $quotedArgs = foreach ($arg in $Arguments) {
        '"' + $arg.Replace('"', '\"') + '"'
    }
    $psi.Arguments = ($quotedArgs -join " ")
    $psi.WorkingDirectory = (Resolve-Path $WorkingDirectory).Path
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true

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

$disasmResult = Invoke-Dbyte -Arguments @("disasm", "tests\vm\disasm_smoke.dby")
if ($disasmResult.Code -ne 0) { throw "disasm smoke failed: $($disasmResult.Text)" }
Assert-Equal $disasmResult.Text (Expected-File "tests\vm\disasm_smoke.disasm") "disasm smoke"

$traceResult = Invoke-Dbyte -Arguments @("run", "--vm", "--trace", "tests\vm\trace_smoke.dby")
if ($traceResult.Code -ne 0) { throw "trace smoke failed: $($traceResult.Text)" }
Assert-Equal $traceResult.Text (Expected-File "tests\vm\trace_smoke.trace") "trace smoke"

$arityResult = Invoke-Dbyte -Arguments @("run", "--vm", "--no-check", "tests\vm\arity_mismatch.dby")
if ($arityResult.Code -eq 0) { throw "vm arity mismatch unexpectedly passed" }
Assert-Contains $arityResult.Text (Expected-File "tests\vm\arity_mismatch.err") "vm arity mismatch"

$returnResult = Invoke-Dbyte -Arguments @("run", "--vm", "--no-check", "tests\vm\return_outside_function.dby")
if ($returnResult.Code -eq 0) { throw "vm return outside function unexpectedly passed" }
Assert-Contains $returnResult.Text (Expected-File "tests\vm\return_outside_function.err") "vm return outside function"

$divisionResult = Invoke-Dbyte -Arguments @("run", "--vm", "--no-check", "tests\vm\vm_division_by_zero.dby")
if ($divisionResult.Code -eq 0) { throw "vm division by zero unexpectedly passed" }
Assert-Contains $divisionResult.Text (Expected-File "tests\vm\vm_division_by_zero.err") "vm division by zero"

$listResult = Invoke-Dbyte -Arguments @("run", "--vm", "--no-check", "tests\vm\vm_list_oob.dby")
if ($listResult.Code -eq 0) { throw "vm list out of bounds unexpectedly passed" }
Assert-Contains $listResult.Text (Expected-File "tests\vm\vm_list_oob.err") "vm list out of bounds"

Write-Host "Running VM fast path disasm checks..."

$loopDisasm = Invoke-Dbyte -Arguments @("disasm", "benchmarks\loop_sum.dby")
if ($loopDisasm.Code -ne 0) { throw "loop_sum disasm failed: $($loopDisasm.Text)" }
Assert-Contains $loopDisasm.Text "STORE_LOCAL_I64" "loop_sum typed store"
Assert-Contains $loopDisasm.Text "ADD_LOCAL_I64" "loop_sum direct local add"
Assert-Contains $loopDisasm.Text "ADD_LOCAL_CONST_I64" "loop_sum direct const increment"
Assert-Contains $loopDisasm.Text "JUMP_IF_NOT_LT_LOCAL_CONST_I64" "loop_sum direct local less-than jump"

$largeLoopDisasm = Invoke-Dbyte -Arguments @("disasm", "benchmarks\loop_sum_large.dby")
if ($largeLoopDisasm.Code -ne 0) { throw "loop_sum_large disasm failed: $($largeLoopDisasm.Text)" }
Assert-Contains $largeLoopDisasm.Text "STORE_LOCAL_I64" "loop_sum_large typed store"
Assert-Contains $largeLoopDisasm.Text "ADD_LOCAL_I64" "loop_sum_large direct local add"
Assert-Contains $largeLoopDisasm.Text "ADD_LOCAL_CONST_I64" "loop_sum_large direct const increment"
Assert-Contains $largeLoopDisasm.Text "JUMP_IF_NOT_LT_LOCAL_CONST_I64" "loop_sum_large direct local less-than jump"

$compareLoopDisasm = Invoke-Dbyte -Arguments @("disasm", "benchmarks\int_compare_loop.dby")
if ($compareLoopDisasm.Code -ne 0) { throw "int_compare_loop disasm failed: $($compareLoopDisasm.Text)" }
Assert-Contains $compareLoopDisasm.Text "JUMP_IF_NOT_GE_LOCAL_CONST_I64" "int_compare_loop direct greater-equal jump"
Assert-Contains $compareLoopDisasm.Text "JUMP_IF_NOT_LE_LOCAL_CONST_I64" "int_compare_loop direct less-equal jump"
Assert-Contains $compareLoopDisasm.Text "JUMP_IF_NOT_LT_LOCAL_CONST_I64" "int_compare_loop direct loop condition jump"

$fallbackLocalDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\generic_local_fallback.dby")
if ($fallbackLocalDisasm.Code -ne 0) { throw "generic local fallback disasm failed: $($fallbackLocalDisasm.Text)" }
Assert-Contains $fallbackLocalDisasm.Text "STORE_LOCAL 0 ; nums" "generic list local fallback store"
Assert-Contains $fallbackLocalDisasm.Text "LOAD_LOCAL 0 ; nums" "generic list local fallback load"

$directLocalRhsDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\direct_add_local_rhs.dby")
if ($directLocalRhsDisasm.Code -ne 0) { throw "direct local rhs disasm failed: $($directLocalRhsDisasm.Text)" }
Assert-Contains $directLocalRhsDisasm.Text "ADD_LOCAL_I64" "direct local rhs add fast path"

$commutedAddDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\fallback_commuted_add.dby")
if ($commutedAddDisasm.Code -ne 0) { throw "fallback commuted add disasm failed: $($commutedAddDisasm.Text)" }
Assert-NotContains $commutedAddDisasm.Text "ADD_LOCAL_I64" "commuted add avoids direct local add"

$mulAssignDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\fallback_mul_assign.dby")
if ($mulAssignDisasm.Code -ne 0) { throw "fallback mul assign disasm failed: $($mulAssignDisasm.Text)" }
Assert-Contains $mulAssignDisasm.Text "MUL_I64" "mul assign uses typed stack multiply"
Assert-NotContains $mulAssignDisasm.Text "ADD_LOCAL_I64" "mul assign avoids direct local add"
Assert-NotContains $mulAssignDisasm.Text "ADD_LOCAL_CONST_I64" "mul assign avoids direct const add"

$lenAddDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\fallback_len_add.dby")
if ($lenAddDisasm.Code -ne 0) { throw "fallback len add disasm failed: $($lenAddDisasm.Text)" }
Assert-Contains $lenAddDisasm.Text "CALL len 1" "len add keeps builtin call"
Assert-Contains $lenAddDisasm.Text "ADD_I64" "len add uses typed stack add"
Assert-NotContains $lenAddDisasm.Text "ADD_LOCAL_I64" "len add avoids direct local add"
Assert-NotContains $lenAddDisasm.Text "ADD_LOCAL_CONST_I64" "len add avoids direct const add"

$binaryDisasm = Invoke-Dbyte -Arguments @("disasm", "benchmarks\binary_read_u32.dby")
if ($binaryDisasm.Code -ne 0) { throw "binary_read_u32 disasm failed: $($binaryDisasm.Text)" }
Assert-Contains $binaryDisasm.Text "READ_U32_LE" "binary_read_u32 intrinsic"

$bufferDisasm = Invoke-Dbyte -Arguments @("disasm", "benchmarks\buffer_replace.dby")
if ($bufferDisasm.Code -ne 0) { throw "buffer_replace disasm failed: $($bufferDisasm.Text)" }
Assert-Contains $bufferDisasm.Text "BUFFER_REPLACE" "buffer_replace intrinsic"

$binaryAliasDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\binary_alias_u32.dby")
if ($binaryAliasDisasm.Code -ne 0) { throw "binary alias disasm failed: $($binaryAliasDisasm.Text)" }
Assert-Contains $binaryAliasDisasm.Text "READ_U32_LE" "binary alias intrinsic"

$bufferAliasDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\buffer_alias_ops.dby")
if ($bufferAliasDisasm.Code -ne 0) { throw "buffer alias disasm failed: $($bufferAliasDisasm.Text)" }
Assert-Contains $bufferAliasDisasm.Text "BUFFER_FIND" "buffer alias find intrinsic"
Assert-Contains $bufferAliasDisasm.Text "BUFFER_REPLACE" "buffer alias replace intrinsic"

$bufferLoadSaveDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\patching\load_save_roundtrip.dby")
if ($bufferLoadSaveDisasm.Code -ne 0) { throw "buffer load/save disasm failed: $($bufferLoadSaveDisasm.Text)" }
Assert-Contains $bufferLoadSaveDisasm.Text "BUFFER_LOAD" "buffer load intrinsic"
Assert-Contains $bufferLoadSaveDisasm.Text "BUFFER_SAVE" "buffer save intrinsic"

$fsExistsDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\patching\fs_exists.dby")
if ($fsExistsDisasm.Code -ne 0) { throw "fs exists disasm failed: $($fsExistsDisasm.Text)" }
Assert-Contains $fsExistsDisasm.Text "CALL_NATIVE FsExists" "fs exists native call"

$bufferLoadFallbackDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\patching\member_call_fallback_buffer_load.dby")
if ($bufferLoadFallbackDisasm.Code -ne 0) { throw "buffer load fallback disasm failed: $($bufferLoadFallbackDisasm.Text)" }
Assert-Contains $bufferLoadFallbackDisasm.Text "MEMBER_CALL load 1" "non-std buffer load fallback member call"
Assert-NotContains $bufferLoadFallbackDisasm.Text "BUFFER_LOAD" "non-std buffer load fallback avoids intrinsic"

$fallbackDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\typed\fallback_member_call.dby")
if ($fallbackDisasm.Code -ne 0) { throw "fallback member call disasm failed: $($fallbackDisasm.Text)" }
Assert-Contains $fallbackDisasm.Text "MEMBER_CALL u32_le 2" "non-std fallback member call"
Assert-NotContains $fallbackDisasm.Text "READ_U32_LE" "non-std fallback avoids binary intrinsic"

$directCallDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\function_fastpath\direct_call_disasm.dby")
if ($directCallDisasm.Code -ne 0) { throw "direct function call disasm failed: $($directCallDisasm.Text)" }
Assert-Contains $directCallDisasm.Text "ADD_I64_STACK" "direct function call fast path"
Assert-Contains $directCallDisasm.Text "RETURN_I64" "typed int return fast path"
Assert-NotContains $directCallDisasm.Text "CALL add 2" "direct function avoids string call"


$directReturnDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\call_i64_to_local_disasm.dby")
if ($directReturnDisasm.Code -ne 0) { throw "direct return-to-local disasm failed: $($directReturnDisasm.Text)" }
Assert-Contains $directReturnDisasm.Text "ADD_I64_STACK" "direct return-to-local fast path"
Assert-Contains $directReturnDisasm.Text "RETURN_I64" "direct return-to-local typed return"
Assert-NotContains $directReturnDisasm.Text "CALL add 2" "direct return-to-local avoids string call"


$letInitDirectReturnDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\let_init_i64_to_local.dby")
if ($letInitDirectReturnDisasm.Code -ne 0) { throw "let init direct return-to-local disasm failed: $($letInitDirectReturnDisasm.Text)" }
Assert-Contains $letInitDirectReturnDisasm.Text "STORE_LOCAL_I64_STACK" "let init direct return-to-local fast path"

$earlyReturnDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\early_return_i64_to_local.dby")
if ($earlyReturnDisasm.Code -ne 0) { throw "early return direct return-to-local disasm failed: $($earlyReturnDisasm.Text)" }
Assert-Contains $earlyReturnDisasm.Text "STORE_LOCAL_I64_STACK" "early return direct return-to-local fast path"

$nestedArgFallbackDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\nested_call_fallback.dby")
if ($nestedArgFallbackDisasm.Code -ne 0) { throw "nested argument fallback disasm failed: $($nestedArgFallbackDisasm.Text)" }
Assert-Contains $nestedArgFallbackDisasm.Text "CALL_FN" "nested argument still uses direct function id fallback"
Assert-NotContains $nestedArgFallbackDisasm.Text "CALL_FN_I64_TO_LOCAL" "nested argument avoids direct return-to-local"

$directReturnGenericDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\generic_return_no_fastpath.dby")
if ($directReturnGenericDisasm.Code -ne 0) { throw "direct return generic fallback disasm failed: $($directReturnGenericDisasm.Text)" }
Assert-Contains $directReturnGenericDisasm.Text "STORE_LOCAL" "direct return generic fallback uses direct id"
Assert-Contains $directReturnGenericDisasm.Text "RETURN" "direct return generic fallback keeps generic return"
Assert-NotContains $directReturnGenericDisasm.Text "CALL_FN_I64_TO_LOCAL" "direct return generic fallback avoids direct return-to-local"
Assert-NotContains $directReturnGenericDisasm.Text "RETURN_I64" "direct return generic fallback avoids return_i64"

$directReturnNonIntDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\non_int_return_no_fastpath.dby")
if ($directReturnNonIntDisasm.Code -ne 0) { throw "direct return non-int fallback disasm failed: $($directReturnNonIntDisasm.Text)" }
Assert-Contains $directReturnNonIntDisasm.Text "STORE_LOCAL" "direct return non-int fallback uses direct id"
Assert-NotContains $directReturnNonIntDisasm.Text "CALL_FN_I64_TO_LOCAL" "direct return non-int fallback avoids direct return-to-local"

$directReturnBuiltinDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\builtin_len_no_fastpath.dby")
if ($directReturnBuiltinDisasm.Code -ne 0) { throw "direct return builtin fallback disasm failed: $($directReturnBuiltinDisasm.Text)" }
Assert-Contains $directReturnBuiltinDisasm.Text "CALL len 1" "direct return builtin fallback keeps builtin call"
Assert-Contains $directReturnBuiltinDisasm.Text "STORE_LOCAL_I64" "direct return builtin fallback stores typed local"
Assert-NotContains $directReturnBuiltinDisasm.Text "CALL_FN_I64_TO_LOCAL" "direct return builtin fallback avoids direct return-to-local"

$directReturnStdMemberDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\direct_return\std_member_no_fastpath.dby")
if ($directReturnStdMemberDisasm.Code -ne 0) { throw "direct return std member fallback disasm failed: $($directReturnStdMemberDisasm.Text)" }
Assert-Contains $directReturnStdMemberDisasm.Text "MEMBER_CALL max 2" "direct return std member fallback keeps member dispatch"
Assert-Contains $directReturnStdMemberDisasm.Text "STORE_LOCAL_I64" "direct return std member fallback stores typed local"
Assert-NotContains $directReturnStdMemberDisasm.Text "CALL_FN_I64_TO_LOCAL" "direct return std member fallback avoids direct return-to-local"

$directReturnMemberFallback = Invoke-Dbyte -Arguments @("disasm", "tests\vm\call_fastpath\member_call_not_call_fn.dby")
if ($directReturnMemberFallback.Code -ne 0) { throw "direct return member fallback disasm failed: $($directReturnMemberFallback.Text)" }
Assert-Contains $directReturnMemberFallback.Text "MEMBER_CALL max 2" "direct return member fallback keeps member dispatch"
Assert-NotContains $directReturnMemberFallback.Text "CALL_FN_I64_TO_LOCAL" "direct return member fallback avoids direct return-to-local"

$i64StackChainDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\i64_stack\typed_call_chain_disasm.dby")
if ($i64StackChainDisasm.Code -ne 0) { throw "i64 stack chain disasm failed: $($i64StackChainDisasm.Text)" }
Assert-Contains $i64StackChainDisasm.Text "STORE_LOCAL_I64_STACK" "i64 stack direct typed call"
Assert-Contains $i64StackChainDisasm.Text "RETURN_I64_TO_I64_STACK" "i64 stack typed return"
Assert-Contains $i64StackChainDisasm.Text "ADD_I64_STACK" "i64 stack typed add"
Assert-NotContains $i64StackChainDisasm.Text "CALL inc 1" "i64 stack chain avoids string call"

$i64StackAssignDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\i64_stack\assign_call_plus_local.dby")
if ($i64StackAssignDisasm.Code -ne 0) { throw "i64 stack assign disasm failed: $($i64StackAssignDisasm.Text)" }
Assert-Contains $i64StackAssignDisasm.Text "STORE_LOCAL_I64_STACK" "i64 stack assignment call result"
Assert-Contains $i64StackAssignDisasm.Text "STORE_LOCAL_I64_STACK" "i64 stack assignment stores typed local"

$i64StackFallbackDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\i64_stack\generic_return_no_i64_stack_call.dby")
if ($i64StackFallbackDisasm.Code -ne 0) { throw "i64 stack generic fallback disasm failed: $($i64StackFallbackDisasm.Text)" }
Assert-Contains $i64StackFallbackDisasm.Text "STORE_LOCAL" "i64 stack generic fallback keeps direct id"
Assert-NotContains $i64StackFallbackDisasm.Text "CALL_FN_I64_TO_I64_STACK" "i64 stack generic fallback avoids typed call"
Assert-NotContains $i64StackFallbackDisasm.Text "RETURN_I64_TO_I64_STACK" "i64 stack generic fallback avoids typed return"

$i64StackMemberFallbackDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\i64_stack\member_std_no_i64_stack_call.dby")
if ($i64StackMemberFallbackDisasm.Code -ne 0) { throw "i64 stack member fallback disasm failed: $($i64StackMemberFallbackDisasm.Text)" }
Assert-Contains $i64StackMemberFallbackDisasm.Text "MEMBER_CALL max 2" "i64 stack std member fallback keeps member dispatch"
Assert-NotContains $i64StackMemberFallbackDisasm.Text "CALL_FN_I64_TO_I64_STACK" "i64 stack std member fallback avoids typed call"

$i64StackHardeningDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\i64_stack_hardening\i64_stack_call_chain.dby")
if ($i64StackHardeningDisasm.Code -ne 0) { throw "i64 stack hardening disasm failed: $($i64StackHardeningDisasm.Text)" }
Assert-Contains $i64StackHardeningDisasm.Text "CONST_I64_STACK" "i64 stack hardening uses typed constants"
Assert-Contains $i64StackHardeningDisasm.Text "CALL_FN_I64_TO_I64_STACK" "i64 stack hardening uses typed call chain"
Assert-Contains $i64StackHardeningDisasm.Text "RETURN_I64_TO_I64_STACK" "i64 stack hardening uses typed return"

$i64StackHardeningGenericDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\i64_stack_hardening\i64_stack_generic_return_fallback.dby")
if ($i64StackHardeningGenericDisasm.Code -ne 0) { throw "i64 stack hardening generic fallback disasm failed: $($i64StackHardeningGenericDisasm.Text)" }
Assert-Contains $i64StackHardeningGenericDisasm.Text "STORE_LOCAL" "i64 stack hardening generic fallback keeps direct id"
Assert-NotContains $i64StackHardeningGenericDisasm.Text "CALL_FN_I64_TO_I64_STACK" "i64 stack hardening generic fallback avoids typed call"
Assert-NotContains $i64StackHardeningGenericDisasm.Text "RETURN_I64_TO_I64_STACK" "i64 stack hardening generic fallback avoids typed return"

$nestedCallDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\function_fastpath\nested_function_call.dby")
if ($nestedCallDisasm.Code -ne 0) { throw "nested function call disasm failed: $($nestedCallDisasm.Text)" }
Assert-Contains $nestedCallDisasm.Text "ADD_I64_STACK" "nested function i64 stack direct call fast path"
Assert-Contains $nestedCallDisasm.Text "RETURN_I64_TO_I64_STACK" "nested function i64 stack return fast path"


$genericCallDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\function_fastpath\generic_function_fallback.dby")
if ($genericCallDisasm.Code -ne 0) { throw "generic function call disasm failed: $($genericCallDisasm.Text)" }
Assert-Contains $genericCallDisasm.Text "STORE_LOCAL" "generic user function inlined"
Assert-Contains $genericCallDisasm.Text "RETURN" "generic return keeps generic return path"
Assert-NotContains $genericCallDisasm.Text "RETURN_I64" "generic return avoids typed int return"

$discardCallDisasm = Invoke-Dbyte -Arguments @("disasm", "benchmarks\function_call.dby")
if ($discardCallDisasm.Code -ne 0) { throw "function_call disasm failed: $($discardCallDisasm.Text)" }
Assert-Contains $discardCallDisasm.Text "POP_I64_STACK" "discarded function call avoids return stack traffic"
Assert-NotContains $discardCallDisasm.Text "CALL work 1" "discarded function avoids string call"

$callFnHardeningDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\call_fastpath\call_fn_disasm.dby")
if ($callFnHardeningDisasm.Code -ne 0) { throw "call_fn hardening disasm failed: $($callFnHardeningDisasm.Text)" }
Assert-Contains $callFnHardeningDisasm.Text "ADD_I64_STACK" "call_fn hardening direct call inlined"
Assert-Contains $callFnHardeningDisasm.Text "RETURN_I64" "call_fn hardening typed return"
Assert-NotContains $callFnHardeningDisasm.Text "CALL add 2" "call_fn hardening avoids string lookup"


$returnI64Disasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\call_fastpath\return_i64_correctness.dby")
if ($returnI64Disasm.Code -ne 0) { throw "return_i64 disasm failed: $($returnI64Disasm.Text)" }
Assert-Contains $returnI64Disasm.Text "RETURN_I64" "int function uses return_i64"

$discardHardeningDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\call_fastpath\discard_call_stack_clean.dby")
if ($discardHardeningDisasm.Code -ne 0) { throw "discard call hardening disasm failed: $($discardHardeningDisasm.Text)" }
Assert-Contains $discardHardeningDisasm.Text "POP_I64_STACK" "discarded call hardening inlined"
Assert-NotContains $discardHardeningDisasm.Text "CALL value 1" "discarded call hardening avoids string lookup"

$genericFallbackDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\call_fastpath\generic_call_fallback.dby")
if ($genericFallbackDisasm.Code -ne 0) { throw "generic call fallback disasm failed: $($genericFallbackDisasm.Text)" }
Assert-Contains $genericFallbackDisasm.Text "STORE_LOCAL" "generic user function inlined"
Assert-Contains $genericFallbackDisasm.Text "RETURN" "generic function keeps generic return"
Assert-NotContains $genericFallbackDisasm.Text "RETURN_I64" "generic function avoids return_i64"

$memberFallbackDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\vm\call_fastpath\member_call_not_call_fn.dby")
if ($memberFallbackDisasm.Code -ne 0) { throw "member call fallback disasm failed: $($memberFallbackDisasm.Text)" }
Assert-Contains $memberFallbackDisasm.Text "MEMBER_CALL max 2" "member call keeps member dispatch"
Assert-NotContains $memberFallbackDisasm.Text "CALL_FN" "member call avoids direct function opcode"

$recursionDisasm = Invoke-Dbyte -Arguments @("disasm", "tests\functions\recursion_factorial.dby")
if ($recursionDisasm.Code -ne 0) { throw "recursion factorial disasm failed: $($recursionDisasm.Text)" }
Assert-Contains $recursionDisasm.Text "CALL_FN" "recursive function direct call"
Assert-NotContains $recursionDisasm.Text "CALL fact" "recursive function avoids string call"

$frameDispatchTypedArgs = Invoke-Dbyte -Arguments @("disasm", "tests\vm\frame_dispatch\typed_args_correctness.dby")
if ($frameDispatchTypedArgs.Code -ne 0) { throw "frame dispatch typed args disasm failed: $($frameDispatchTypedArgs.Text)" }
Assert-Contains $frameDispatchTypedArgs.Text "ADD_I64_STACK" "frame dispatch direct user call inlined"
Assert-Contains $frameDispatchTypedArgs.Text "RETURN_I64" "frame dispatch typed int return"
Assert-NotContains $frameDispatchTypedArgs.Text "CALL add 2" "frame dispatch avoids string call"


$frameDispatchDiscard = Invoke-Dbyte -Arguments @("disasm", "tests\vm\frame_dispatch\discard_call_stack_clean.dby")
if ($frameDispatchDiscard.Code -ne 0) { throw "frame dispatch discard disasm failed: $($frameDispatchDiscard.Text)" }
Assert-Contains $frameDispatchDiscard.Text "POP_I64_STACK" "frame dispatch discarded call inlined"

$frameDispatchGeneric = Invoke-Dbyte -Arguments @("disasm", "tests\vm\frame_dispatch\generic_return_fallback.dby")
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
Assert-Contains $shellBasic.Text "DByte shell commands" "shell help"
Assert-Contains $shellBasic.Text "alias <name> = <command>" "shell registry alias help"
Assert-Contains $shellBasic.Text "which <name>" "shell registry which help"
Assert-Contains $shellBasic.Text "DByte 4.4.1" "shell version"
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
    $runNoRc = Invoke-Dbyte -Arguments @("run", "main.dby")
    if ($runNoRc.Code -ne 0) { throw "run loaded rc unexpectedly: $($runNoRc.Text)" }
    Assert-Equal $runNoRc.Text "run ignores rc" "run ignores rc"
    $checkNoRc = Invoke-Dbyte -Arguments @("check", "main.dby")
    if ($checkNoRc.Code -ne 0) { throw "check loaded rc unexpectedly: $($checkNoRc.Text)" }
    Assert-Contains $checkNoRc.Text "no type errors found" "check ignores rc"
    $newNoRcRoot = Join-Path $runNoRcRoot "new-no-rc"
    New-Item -ItemType Directory -Path $newNoRcRoot | Out-Null
    Push-Location $newNoRcRoot
    try {
        $newNoRc = Invoke-Dbyte -Arguments @("new", "rcsafe")
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
    $result = Invoke-Dbyte -Arguments @("run", "personal_tools\$($tool.Path)")
    if ($result.Code -ne 0) { throw "personal tool from repo root failed [$($tool.Name)]: $($result.Text)" }
    Assert-PersonalToolOutput $tool.Name $result.Text
}
Assert-GitStatus-Unchanged $personalToolsStatus "personal tools repo-root run cleanliness"

Push-Location (Join-Path $repoRoot "personal_tools")
try {
    foreach ($tool in $personalToolFiles) {
        $result = Invoke-Dbyte -Arguments @("run", $tool.Path)
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

$personalHexArgs = Invoke-Dbyte -Arguments @("run", "personal_tools\hexdump.dby", $personalArgsFile)
if ($personalHexArgs.Code -ne 0) { throw "personal hexdump args failed: $($personalHexArgs.Text)" }
Assert-Contains $personalHexArgs.Text "bytes: 10" "personal hexdump args size"
Assert-Contains $personalHexArgs.Text "0000: 00deadbeef007856" "personal hexdump args first row"
Assert-Contains $personalHexArgs.Text "0008: 3412" "personal hexdump args second row"

$personalHexRange = Invoke-Dbyte -Arguments @("run", "personal_tools\hexdump.dby", $personalArgsFile, "1", "6")
if ($personalHexRange.Code -ne 0) { throw "personal hexdump range failed: $($personalHexRange.Text)" }
Assert-Contains $personalHexRange.Text "range: 1 6" "personal hexdump range header"
Assert-Contains $personalHexRange.Text "1 : deadbeef0078" "personal hexdump range row"

$personalBinArgs = Invoke-Dbyte -Arguments @("run", "personal_tools\bininfo.dby", $personalArgsFile)
if ($personalBinArgs.Code -ne 0) { throw "personal bininfo args failed: $($personalBinArgs.Text)" }
Assert-Contains $personalBinArgs.Text "bytes: 10" "personal bininfo args size"
Assert-Contains $personalBinArgs.Text "first8: 00deadbeef007856" "personal bininfo args first bytes"

$personalFindArgs = Invoke-Dbyte -Arguments @("run", "personal_tools\find_bytes.dby", $personalArgsFile, "DEADBEEF")
if ($personalFindArgs.Code -ne 0) { throw "personal find args failed: $($personalFindArgs.Text)" }
Assert-Contains $personalFindArgs.Text "pattern: 1" "personal find args offset"
Assert-Contains $personalFindArgs.Text "pattern: 1 0x1" "personal find args hex offset"

$personalFindInvalidHex = Invoke-Dbyte -Arguments @("run", "personal_tools\find_bytes.dby", $personalArgsFile, "NOTHEX")
if ($personalFindInvalidHex.Code -ne 0) { throw "personal find invalid hex failed: $($personalFindInvalidHex.Text)" }
Assert-Equal $personalFindInvalidHex.Text "error: invalid hex_pattern" "personal find invalid hex"

$personalPatchArgs = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalArgsFile, "DEADBEEF", "CAFEBABE")
if ($personalPatchArgs.Code -ne 0) { throw "personal patch args failed: $($personalPatchArgs.Text)" }
Assert-Contains $personalPatchArgs.Text "patched first match at offset 1" "personal patch args offset"
Assert-Contains $personalPatchArgs.Text "wrote $personalArgsFile.patched" "personal patch args output path"
Assert-Contains $personalPatchArgs.Text "patched_hex: 00cafebabe0078563412" "personal patch args bytes"
Assert-Equal (Bytes-Hex $personalArgsFile) "00deadbeef0078563412" "personal patch original unchanged"
Assert-Equal (Bytes-Hex "$personalArgsFile.patched") "00cafebabe0078563412" "personal patch output bytes"

$personalPatchFirstMatch = Join-Path $personalArgsRoot "first-match.bin"
[System.IO.File]::WriteAllBytes($personalPatchFirstMatch, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x11, 0xde, 0xad, 0xbe, 0xef, 0x22))
$personalPatchFirstMatchResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalPatchFirstMatch, "DEADBEEF", "CAFEBABE")
if ($personalPatchFirstMatchResult.Code -ne 0) { throw "personal patch first-match failed: $($personalPatchFirstMatchResult.Text)" }
Assert-Contains $personalPatchFirstMatchResult.Text "patched first match at offset 1" "personal patch first-match offset"
Assert-Equal (Bytes-Hex $personalPatchFirstMatch) "00deadbeef11deadbeef22" "personal patch first-match original unchanged"
Assert-Equal (Bytes-Hex "$personalPatchFirstMatch.patched") "00cafebabe11deadbeef22" "personal patch first-match output bytes"

$personalPatchAll = Join-Path $personalArgsRoot "all.bin"
[System.IO.File]::WriteAllBytes($personalPatchAll, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x11, 0xde, 0xad, 0xbe, 0xef, 0x22))
$personalPatchAllResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--all", $personalPatchAll, "DEADBEEF", "CAFEBABE")
if ($personalPatchAllResult.Code -ne 0) { throw "personal patch all failed: $($personalPatchAllResult.Text)" }
Assert-Contains $personalPatchAllResult.Text "patched count: 2" "personal patch all count"
Assert-Equal (Bytes-Hex $personalPatchAll) "00deadbeef11deadbeef22" "personal patch all original unchanged"
Assert-Equal (Bytes-Hex "$personalPatchAll.patched") "00cafebabe11cafebabe22" "personal patch all output bytes"

$personalPatchOffset = Join-Path $personalArgsRoot "offset.bin"
[System.IO.File]::WriteAllBytes($personalPatchOffset, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x11))
$personalPatchOffsetResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--offset", "1", $personalPatchOffset, "CAFEBABE")
if ($personalPatchOffsetResult.Code -ne 0) { throw "personal patch offset failed: $($personalPatchOffsetResult.Text)" }
Assert-Contains $personalPatchOffsetResult.Text "patched offset 1" "personal patch offset marker"
Assert-Equal (Bytes-Hex $personalPatchOffset) "00deadbeef11" "personal patch offset original unchanged"
Assert-Equal (Bytes-Hex "$personalPatchOffset.patched") "00cafebabe11" "personal patch offset output bytes"

$personalPatchOffsetOob = Join-Path $personalArgsRoot "offset-oob.bin"
[System.IO.File]::WriteAllBytes($personalPatchOffsetOob, [byte[]](0x00, 0xde, 0xad))
$personalPatchOffsetOobResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--offset", "2", $personalPatchOffsetOob, "CAFEBABE")
if ($personalPatchOffsetOobResult.Code -ne 0) { throw "personal patch offset oob failed: $($personalPatchOffsetOobResult.Text)" }
Assert-Equal $personalPatchOffsetOobResult.Text "error: offset out of bounds" "personal patch offset oob"
if (Test-Path "$personalPatchOffsetOob.patched") { throw "personal patch offset oob unexpectedly wrote output" }

$personalPatchOffsetBadDecimal = Join-Path $personalArgsRoot "offset-bad-decimal.bin"
[System.IO.File]::WriteAllBytes($personalPatchOffsetBadDecimal, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef))
$personalPatchOffsetBadDecimalResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--offset", "nope", $personalPatchOffsetBadDecimal, "CAFEBABE")
if ($personalPatchOffsetBadDecimalResult.Code -ne 0) { throw "personal patch offset bad decimal failed: $($personalPatchOffsetBadDecimalResult.Text)" }
Assert-Equal $personalPatchOffsetBadDecimalResult.Text "error: offset must be a decimal integer" "personal patch offset bad decimal"
if (Test-Path "$personalPatchOffsetBadDecimal.patched") { throw "personal patch offset bad decimal unexpectedly wrote output" }

$personalPatchInvalidHex = Join-Path $personalArgsRoot "invalid-hex.bin"
[System.IO.File]::WriteAllBytes($personalPatchInvalidHex, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef))
$personalPatchInvalidHexResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalPatchInvalidHex, "NOTHEX", "CAFEBABE")
if ($personalPatchInvalidHexResult.Code -ne 0) { throw "personal patch invalid hex failed: $($personalPatchInvalidHexResult.Text)" }
Assert-Equal $personalPatchInvalidHexResult.Text "error: invalid find_hex" "personal patch invalid hex"
if (Test-Path "$personalPatchInvalidHex.patched") { throw "personal patch invalid hex unexpectedly wrote output" }

$personalPatchMissing = Join-Path $personalArgsRoot "missing.bin"
[System.IO.File]::WriteAllBytes($personalPatchMissing, [byte[]](0x01, 0x02, 0x03, 0x04))
$personalPatchMissingResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalPatchMissing, "DEADBEEF", "CAFEBABE")
if ($personalPatchMissingResult.Code -ne 0) { throw "personal patch missing failed: $($personalPatchMissingResult.Text)" }
Assert-Equal $personalPatchMissingResult.Text "pattern not found" "personal patch missing output"
if (Test-Path "$personalPatchMissing.patched") { throw "personal patch missing unexpectedly wrote output" }

$personalPatchUnequalFile = Join-Path $personalArgsRoot "unequal.bin"
[System.IO.File]::WriteAllBytes($personalPatchUnequalFile, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x00))
$personalPatchUnequal = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalPatchUnequalFile, "DEADBEEF", "CAFE")
if ($personalPatchUnequal.Code -ne 0) { throw "personal patch unequal failed: $($personalPatchUnequal.Text)" }
Assert-Equal $personalPatchUnequal.Text "error: find_hex and replace_hex must have the same byte length" "personal patch unequal length"
Assert-Equal (Bytes-Hex $personalPatchUnequalFile) "00deadbeef00" "personal patch unequal original unchanged"
if (Test-Path "$personalPatchUnequalFile.patched") { throw "personal patch unequal unexpectedly wrote output" }

$personalPatchOffsetNeg = Join-Path $personalArgsRoot "offset-neg.bin"
[System.IO.File]::WriteAllBytes($personalPatchOffsetNeg, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef))
$personalPatchOffsetNegResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--offset", "-1", $personalPatchOffsetNeg, "CAFEBABE")
if ($personalPatchOffsetNegResult.Code -ne 0) { throw "personal patch offset negative failed: $($personalPatchOffsetNegResult.Text)" }
Assert-Equal $personalPatchOffsetNegResult.Text "error: offset must be a non-negative decimal integer" "personal patch offset negative"
if (Test-Path "$personalPatchOffsetNeg.patched") { throw "personal patch offset negative unexpectedly wrote output" }

$personalPatchBadReplace = Join-Path $personalArgsRoot "bad-replace.bin"
[System.IO.File]::WriteAllBytes($personalPatchBadReplace, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef))
$personalPatchBadReplaceResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalPatchBadReplace, "DEADBEEF", "ZZZZZZZZ")
if ($personalPatchBadReplaceResult.Code -ne 0) { throw "personal patch invalid replace failed: $($personalPatchBadReplaceResult.Text)" }
Assert-Equal $personalPatchBadReplaceResult.Text "error: invalid replace_hex" "personal patch invalid replace"
if (Test-Path "$personalPatchBadReplace.patched") { throw "personal patch invalid replace unexpectedly wrote output" }

$personalPatchOffsetBadReplaceResult = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--offset", "0", $personalPatchBadReplace, "NOTHEX")
if ($personalPatchOffsetBadReplaceResult.Code -ne 0) { throw "personal patch offset invalid replace failed: $($personalPatchOffsetBadReplaceResult.Text)" }
Assert-Equal $personalPatchOffsetBadReplaceResult.Text "error: invalid replace_hex" "personal patch offset invalid replace"
if (Test-Path "$personalPatchBadReplace.patched") { throw "personal patch offset invalid replace unexpectedly wrote output" }

$personalFindNoMatch = Join-Path $personalArgsRoot "no-pattern.bin"
[System.IO.File]::WriteAllBytes($personalFindNoMatch, [byte[]](0x01, 0x02, 0x03, 0x04))
$personalFindNoMatchResult = Invoke-Dbyte -Arguments @("run", "personal_tools\find_bytes.dby", $personalFindNoMatch, "DEADBEEF")
if ($personalFindNoMatchResult.Code -ne 0) { throw "personal find no match failed: $($personalFindNoMatchResult.Text)" }
Assert-Contains $personalFindNoMatchResult.Text "pattern: not found" "personal find no match"

$personalHexOob = Invoke-Dbyte -Arguments @("run", "personal_tools\hexdump.dby", $personalArgsFile, "11", "1")
if ($personalHexOob.Code -ne 0) { throw "personal hexdump offset oob failed: $($personalHexOob.Text)" }
Assert-Equal $personalHexOob.Text "error: offset out of bounds" "personal hexdump offset oob"

$personalHexClamp = Invoke-Dbyte -Arguments @("run", "personal_tools\hexdump.dby", $personalArgsFile, "1", "999")
if ($personalHexClamp.Code -ne 0) { throw "personal hexdump length clamp failed: $($personalHexClamp.Text)" }
Assert-Contains $personalHexClamp.Text "range: 1 9" "personal hexdump length clamp header"
Assert-Contains $personalHexClamp.Text "1 : deadbeef00785634" "personal hexdump length clamp row1"
Assert-Contains $personalHexClamp.Text "9 : 12" "personal hexdump length clamp row2"

$personalHexNeg = Invoke-Dbyte -Arguments @("run", "personal_tools\hexdump.dby", $personalArgsFile, "-1", "4")
if ($personalHexNeg.Code -ne 0) { throw "personal hexdump negative offset failed: $($personalHexNeg.Text)" }
Assert-Equal $personalHexNeg.Text "error: offset must be a non-negative decimal integer" "personal hexdump negative offset"

$personalHexTwoArgs = Invoke-Dbyte -Arguments @("run", "personal_tools\hexdump.dby", $personalArgsFile, "0")
if ($personalHexTwoArgs.Code -ne 0) { throw "personal hexdump two args failed: $($personalHexTwoArgs.Text)" }
Assert-Contains $personalHexTwoArgs.Text "usage: hexdump <file> [offset length]" "personal hexdump two args usage line"
Assert-Contains $personalHexTwoArgs.Text "-h, --help" "personal hexdump two args options"
Assert-Contains $personalHexTwoArgs.Text "example: dbyte run personal_tools/hexdump.dby sample.bin 0 16" "personal hexdump two args example"

$personalU32BadOffset = Invoke-Dbyte -Arguments @("run", "personal_tools\read_u32_table.dby", $personalArgsFile, "nope", "1")
if ($personalU32BadOffset.Code -ne 0) { throw "personal u32 bad offset failed: $($personalU32BadOffset.Text)" }
Assert-Equal $personalU32BadOffset.Text "error: offset must be a decimal integer" "personal u32 bad offset"

$personalU32NegCount = Invoke-Dbyte -Arguments @("run", "personal_tools\read_u32_table.dby", $personalArgsFile, "0", "-1")
if ($personalU32NegCount.Code -ne 0) { throw "personal u32 negative count failed: $($personalU32NegCount.Text)" }
Assert-Equal $personalU32NegCount.Text "error: count must be a non-negative decimal integer" "personal u32 negative count"

$personalU32OobStart = Invoke-Dbyte -Arguments @("run", "personal_tools\read_u32_table.dby", $personalArgsFile, "11", "1")
if ($personalU32OobStart.Code -ne 0) { throw "personal u32 start offset oob failed: $($personalU32OobStart.Text)" }
Assert-Equal $personalU32OobStart.Text "error: offset out of bounds" "personal u32 start offset oob"

$personalU32Args = Invoke-Dbyte -Arguments @("run", "personal_tools\read_u32_table.dby", $personalArgsFile)
if ($personalU32Args.Code -ne 0) { throw "personal u32 args failed: $($personalU32Args.Text)" }
Assert-Contains $personalU32Args.Text "0 -> 3199065600" "personal u32 args first row"
Assert-Contains $personalU32Args.Text "4 -> 1450705135" "personal u32 args second row"

$personalU32Range = Invoke-Dbyte -Arguments @("run", "personal_tools\read_u32_table.dby", $personalArgsFile, "6", "1")
if ($personalU32Range.Code -ne 0) { throw "personal u32 range failed: $($personalU32Range.Text)" }
Assert-Contains $personalU32Range.Text "6 -> 305419896" "personal u32 range row"

$personalSpacedRoot = Join-Path $personalArgsRoot "path with spaces"
New-Item -ItemType Directory -Path $personalSpacedRoot | Out-Null
$personalSpacedFile = Join-Path $personalSpacedRoot "quoted sample.bin"
[System.IO.File]::WriteAllBytes($personalSpacedFile, [byte[]](0x00, 0xde, 0xad, 0xbe, 0xef, 0x00))
$personalFindSpaced = Invoke-Dbyte -Arguments @("run", "personal_tools\find_bytes.dby", $personalSpacedFile, "DEADBEEF")
if ($personalFindSpaced.Code -ne 0) { throw "personal find spaced path failed: $($personalFindSpaced.Text)" }
Assert-Contains $personalFindSpaced.Text "pattern: 1 0x1" "personal find spaced path"

$personalHexSpaced = Invoke-Dbyte -Arguments @("run", "personal_tools\hexdump.dby", $personalSpacedFile)
if ($personalHexSpaced.Code -ne 0) { throw "personal hexdump spaced path failed: $($personalHexSpaced.Text)" }
Assert-Contains $personalHexSpaced.Text "bytes: 6" "personal hexdump spaced path size"

$personalBinSpaced = Invoke-Dbyte -Arguments @("run", "personal_tools\bininfo.dby", $personalSpacedFile)
if ($personalBinSpaced.Code -ne 0) { throw "personal bininfo spaced path failed: $($personalBinSpaced.Text)" }
Assert-Contains $personalBinSpaced.Text "bytes: 6" "personal bininfo spaced path size"

$personalPatchSpaced = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalSpacedFile, "DEADBEEF", "CAFEBABE")
if ($personalPatchSpaced.Code -ne 0) { throw "personal patch spaced path failed: $($personalPatchSpaced.Text)" }
Assert-Contains $personalPatchSpaced.Text "patched first match at offset 1" "personal patch spaced path"
Assert-Equal (Bytes-Hex $personalSpacedFile) "00deadbeef00" "personal patch spaced original unchanged"

$personalU32Spaced = Invoke-Dbyte -Arguments @("run", "personal_tools\read_u32_table.dby", $personalSpacedFile, "1", "1")
if ($personalU32Spaced.Code -ne 0) { throw "personal u32 spaced path failed: $($personalU32Spaced.Text)" }
Assert-Contains $personalU32Spaced.Text "1 -> 4022250974" "personal u32 spaced path row"

if (Test-Path "$personalSpacedFile.patched") {
    Remove-Item -Force "$personalSpacedFile.patched"
}

$personalUsageFind = Invoke-Dbyte -Arguments @("run", "personal_tools\find_bytes.dby", $personalArgsFile)
if ($personalUsageFind.Code -ne 0) { throw "personal find usage failed: $($personalUsageFind.Text)" }
Assert-Contains $personalUsageFind.Text "usage: find_bytes <file> <hex_pattern>" "personal find usage line"
Assert-Contains $personalUsageFind.Text "-h, --help" "personal find usage options"
Assert-Contains $personalUsageFind.Text "example: dbyte run personal_tools/find_bytes.dby sample.bin DEADBEEF" "personal find usage example"

$personalUsagePatch = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $personalArgsFile, "DEADBEEF")
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
    $helpResult = Invoke-Dbyte -Arguments @("run", "personal_tools\$($toolEntry.Path)", "--help")
    if ($helpResult.Code -ne 0) { throw "$($toolEntry.Name) --help failed: $($helpResult.Text)" }
    Assert-Contains $helpResult.Text "usage:" "$($toolEntry.Name) --help contains usage:"
    Assert-Contains $helpResult.Text "-h, --help" "$($toolEntry.Name) --help contains -h flag"
    Assert-Contains $helpResult.Text "example:" "$($toolEntry.Name) --help contains example:"

    $shortHelpResult = Invoke-Dbyte -Arguments @("run", "personal_tools\$($toolEntry.Path)", "-h")
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

$patchOutFirst = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", $patchOutSrc, "DEADBEEF", "CAFEBABE", "--out", $patchOutDst)
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

$patchAllOut = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--all", $patchAllOutSrc, "DEADBEEF", "CAFEBABE", "--out", $patchAllOutDst)
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

$patchOffOut = Invoke-Dbyte -Arguments @("run", "personal_tools\patch_bytes.dby", "--offset", "1", $patchOffOutSrc, "CAFEBABE", "--out", $patchOffOutDst)
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
    $sanctumStatusRoot = Invoke-Dbyte -Arguments @("run", "examples\sanctum\sanctum_status.dby")
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

Write-Host "Running DByteOS Command Set (v4.4.1) smoke tests..."
$dbyteosRoot = Join-Path $repoRoot "examples\dbyteos"
$dbyteosStatus = Git-Status-Short
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
  Version:    DByte  4.4.1  ( Userland Prototype )
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
--- DByteOS Alpha Help ---
System:
  boot             - initialize the DByteOS userland
  status           - summarize system state
  sysinfo          - display version and identity
  whoami           - print the current user
  profile          - show profile identity
  config           - show read-only preferences

Discovery:
  welcome          - show the onboarding entry point
  getting-started  - show the first-run checklist
  commands         - browse commands by category
  help             - display this command guide
  man <topic>      - display manual entry for a command
  man-index        - list manual topics
  path             - display path config or resolve commands
  which <command>  - locate commands from the shell

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
  home             - print home path
  tmp              - print temp path
  env              - display environment variables

Services/Logs:
  services         - manage system services
  log              - read DByteOS session logs

Try: welcome, profile show, config show, getting-started, commands
"@
$expectedDbyteosStatus = @"
--- DByteOS System Status ---
Summary:
  OS:      DByte  4.4.1
  Host:     DByte-Alpha
  User:     deadbyte
  Home:     home/deadbyte

Profile:
  Mode:     alpha-userland
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
version: DByte 4.4.1
codename: Userland Prototype
host: DByte-Alpha
kernel: Simulated (Host)
user: deadbyte
home: examples/dbyteos/home/deadbyte
shell: dbyte shell
mode: alpha-userland
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
  mode:    alpha-userland
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
  config           - show read-only preferences

Discovery:
  welcome          - show the onboarding entry point
  getting-started  - show the first-run checklist
  commands         - list commands by category
  help             - show grouped command help
  man <topic>      - read a manual topic
  man-index        - list manual topics
  which <command>  - resolve a command
  path             - show command search roots

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
  env
  path

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
  services
  log

Use: man <topic>
"@
$expectedDbyteosProfile = @"
--- DByteOS Profile ---
user: deadbyte
home: home/deadbyte
shell: dbyte shell
mode: alpha-userland
theme: default
prompt: dbyte-shell>
os_version: 4.4.1
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
system.mode = alpha-userland
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
    Assert-Equal $dbyteosConfigMode.Text "alpha-userland" "dbyteos config mode"

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

    $dbyteosShell = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "status`nclean`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShell.Code -ne 0) { throw "dbyteos shell failed: $($dbyteosShell.Text)" }
    Assert-Contains $dbyteosShell.Text "--- DByteOS System Status ---" "dbyteos shell status alias"
    Assert-Contains $dbyteosShell.Text "sweep complete" "dbyteos shell clean alias sweep"

    $dbyteosShellHelp = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "help`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellHelp.Code -ne 0) { throw "dbyteos shell help failed: $($dbyteosShellHelp.Text)" }
    Assert-Contains $dbyteosShellHelp.Text (Normalize-Output $expectedDbyteosHelp) "dbyteos shell help snapshot"
    Assert-Contains $dbyteosShellHelp.Text "--- DByteOS Alpha Help ---" "dbyteos shell help output (aliased)"
    Assert-Contains $dbyteosShellHelp.Text "System:" "dbyteos shell help system"
    Assert-Contains $dbyteosShellHelp.Text "Discovery:" "dbyteos shell help discovery"
    Assert-Contains $dbyteosShellHelp.Text "perm             - inspect permission policy" "dbyteos shell help perm"
    Assert-Contains $dbyteosShellHelp.Text "profile          - show profile identity" "dbyteos shell help profile"
    Assert-Contains $dbyteosShellHelp.Text "config           - show read-only preferences" "dbyteos shell help config"
    Assert-Contains $dbyteosShellHelp.Text "Try: welcome, profile show, config show, getting-started, commands" "dbyteos shell help try line"

    $dbyteosShellWhichHelpAliased = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "which help`nquit`n" -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosShellWhichHelpAliased.Text "help: alias -> run bin/help.dby" "which help with alias"

    $dbyteosShellNoRcHelp = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "help`nquit`n" -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosShellNoRcHelp.Text "DByte shell commands:" "shell --no-rc help remains built-in"

    $dbyteosShellNoRcWhichHelp = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "which help`nquit`n" -WorkingDirectory $dbyteosRoot
    Assert-Contains $dbyteosShellNoRcWhichHelp.Text "help: built-in" "which help without alias remains built-in (autopath blocked)"

    $dbyteosShellNoRcOnboarding = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "welcome`nprofile`nconfig`ngetting-started`ncommands`nman-index`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellNoRcOnboarding.Code -ne 0) { throw "dbyteos shell --no-rc onboarding guard failed: $($dbyteosShellNoRcOnboarding.Text)" }
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: welcome" "dbyteos shell --no-rc hides welcome"
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: profile" "dbyteos shell --no-rc hides profile"
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: config" "dbyteos shell --no-rc hides config"
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: getting-started" "dbyteos shell --no-rc hides getting-started"
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: commands" "dbyteos shell --no-rc hides commands"
    Assert-Contains $dbyteosShellNoRcOnboarding.Text "ShellError: unknown command: man-index" "dbyteos shell --no-rc hides man-index"

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

    $dbyteosOnboardingShell = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "welcome`nprofile show`nprofile whoami`nprofile home`nprofile theme`nprofile prompt`nconfig show`nconfig keys`nconfig get system.prompt`ngetting-started`ncommands`nman-index`nhelp`nman index`nquit`n" -WorkingDirectory $dbyteosRoot
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
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosGettingStarted) "dbyteos shell getting-started"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosCommands) "dbyteos shell commands"
    Assert-Contains $dbyteosOnboardingShell.Text (Normalize-Output $expectedDbyteosManIndex) "dbyteos shell man-index"
    Assert-Contains $dbyteosOnboardingShell.Text "Manual topics:" "dbyteos shell man index"

    $dbyteosOnboardingManuals = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "man welcome`nman profile`nman config`nman getting-started`nman commands`nman index`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosOnboardingManuals.Code -ne 0) { throw "dbyteos onboarding manuals failed: $($dbyteosOnboardingManuals.Text)" }
    Assert-Contains $dbyteosOnboardingManuals.Text "DByteOS Welcome" "dbyteos man welcome"
    Assert-Contains $dbyteosOnboardingManuals.Text "DByteOS Profile" "dbyteos man profile"
    Assert-Contains $dbyteosOnboardingManuals.Text "DByteOS Config" "dbyteos man config"
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

    $dbyteosShellWhichCd = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "which cd`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellWhichCd.Code -ne 0) { throw "dbyteos shell which cd failed: $($dbyteosShellWhichCd.Text)" }
    Assert-Contains $dbyteosShellWhichCd.Text "cd: built-in" "dbyteos shell which built-in"

    $dbyteosShellInspectArgs = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "inspect boot.dby`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellInspectArgs.Code -ne 0) { throw "dbyteos shell inspect args failed: $($dbyteosShellInspectArgs.Text)" }
    Assert-Contains $dbyteosShellInspectArgs.Text "Inspecting file:" "dbyteos shell inspect passes args"

    $dbyteosShellNoRc = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "status`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellNoRc.Code -ne 0) { throw "dbyteos shell --no-rc failed: $($dbyteosShellNoRc.Text)" }
    Assert-Contains $dbyteosShellNoRc.Text "ShellError: unknown command: status" "dbyteos shell --no-rc hides os aliases"

    $dbyteosShellNoRcCat = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "cat tmp/write_demo.txt`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellNoRcCat.Code -ne 0) { throw "dbyteos shell --no-rc cat failed: $($dbyteosShellNoRcCat.Text)" }
    Assert-Contains $dbyteosShellNoRcCat.Text "ShellError: unknown command: cat" "dbyteos shell --no-rc hides cat alias"

    $dbyteosShellNoRcRead = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText "read tmp/verify_v32.txt`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShellNoRcRead.Code -ne 0) { throw "dbyteos shell --no-rc read failed: $($dbyteosShellNoRcRead.Text)" }
    Assert-Contains $dbyteosShellNoRcRead.Text "ShellError: unknown command: read" "dbyteos shell --no-rc hides read alias"

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

    $dbyteosSysinfoRoot = Invoke-Dbyte -Arguments @("run", "examples\dbyteos\bin\sysinfo.dby") -WorkingDirectory $repoRoot
    if ($dbyteosSysinfoRoot.Code -ne 0) { throw "dbyteos sysinfo from root failed: $($dbyteosSysinfoRoot.Text)" }
    Assert-NormalizedEqual $dbyteosSysinfoRoot.Text $expectedDbyteosSysinfo "dbyteos sysinfo snapshot"
    Assert-Contains $dbyteosSysinfoRoot.Text "DByteOS Alpha Userland" "dbyteos sysinfo banner"
    Assert-Contains $dbyteosSysinfoRoot.Text "version: DByte 4.4.1" "dbyteos sysinfo version"
    Assert-Contains $dbyteosSysinfoRoot.Text "codename: Userland Prototype" "dbyteos sysinfo codename"
    Assert-Contains $dbyteosSysinfoRoot.Text "guide: run help, status, or man <topic>" "dbyteos sysinfo guide"

    foreach ($profileText in @($dbyteosProfileDirect.Text, $dbyteosWelcomeDirect.Text, $dbyteosStatusReport.Text, $dbyteosSysinfoRoot.Text)) {
        Assert-Contains $profileText $dbyteosConfigUser.Text "dbyteos profile user sync"
        Assert-Contains $profileText $dbyteosConfigMode.Text "dbyteos profile mode sync"
        Assert-Contains $profileText $dbyteosConfigPrompt.Text "dbyteos profile prompt sync"
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
    Assert-Contains $dbyteosProfileRoot.Text "mode: alpha-userland" "dbyteos profile mode"
    Assert-Contains $dbyteosProfileRoot.Text "theme: default" "dbyteos profile theme"
    Assert-Contains $dbyteosProfileRoot.Text "prompt: dbyte-shell>" "dbyteos profile prompt"
    Assert-Contains $dbyteosProfileRoot.Text "os_version: 4.4.1" "dbyteos profile os version"

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

    Write-Host "Running DByteOS Security/Permissions (v4.4.1) smoke tests..."
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
    Assert-Contains $dbyteosReadEtc.Text "pub let os_version: str = `"4.4.1`"" "read etc allowed"
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
    
    Write-Host "Running DByteOS Security Enforcement Expansion (v4.4.1) smoke tests..."
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
    Assert-Contains $dbyteosEnforcement.Text "os_version: str = `"4.4.1`"" "cat etc allowed"
    Assert-Contains $dbyteosEnforcement.Text "error: permission denied: path escape tmp/../etc/system.dby" "cat escape denied"
    Assert-Contains $dbyteosEnforcement.Text "touch: ok" "touch tmp allowed"
    Assert-Contains $dbyteosEnforcement.Text "error: permission denied: touch etc/security_touch.txt" "touch etc denied"
    Assert-Contains $dbyteosEnforcement.Text "Inspecting file:" "inspect bin allowed"
    Assert-Contains $dbyteosEnforcement.Text "error: permission denied: inspect unknown/file" "inspect unknown root denied"
    Assert-Contains $dbyteosEnforcement.Text "DENY cat tmp/../etc/system.dby" "security log cat denied"
    Assert-Contains $dbyteosEnforcement.Text "DENY touch etc/security_touch.txt" "security log touch denied"
    Assert-Contains $dbyteosEnforcement.Text "DENY inspect unknown/file" "security log inspect denied"
    Assert-Contains $dbyteosEnforcement.Text "workspace sweep complete" "enforcement clean sweep"

    Write-Host "Running DByteOS Security Enforcement Hardening (v4.4.1) smoke tests..."
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

    Write-Host "Verifying DByteOS Alpha Userland (v4.4.1) documentation..."
    $dbyteDocs = @("DBYTEOS_ALPHA.md", "DBYTEOS_COMMANDS.md", "DBYTEOS_SECURITY.md", "DBYTEOS_BOOT.md", "DBYTEOS_PACKAGE.md", "DBYTEOS_ONBOARDING.md", "DBYTEOS_PROFILE.md", "DBYTEOS_CONFIG.md")
    foreach ($doc in $dbyteDocs) {
        $p = Join-Path $repoRoot "docs/$doc"
        if (-not (Test-Path $p)) { throw "DByteOS doc missing: $doc" }
    }
    $mainReadme = Get-Content (Join-Path $repoRoot "README.md") -Raw
    Assert-Contains $mainReadme "DByteOS Alpha Userland (v4.4.1)" "README alpha positioning"
    Assert-Contains $mainReadme "docs/DBYTEOS_ALPHA.md" "README alpha link"
    Assert-Contains $mainReadme "docs/DBYTEOS_ONBOARDING.md" "README onboarding link"
    Assert-Contains $mainReadme "docs/DBYTEOS_PROFILE.md" "README profile link"
    Assert-Contains $mainReadme "docs/DBYTEOS_CONFIG.md" "README config link"
    Assert-Contains $mainReadme "docs/DBYTEOS_PACKAGE.md" "README package guide link"
    Assert-Contains $mainReadme "Smoke-test a zip release" "README zip quickstart"
    Assert-Contains $mainReadme "dbyte shell --rc examples/dbyteos/.dbyterc" "README shell quickstart command"
    Assert-Contains $mainReadme "welcome" "README onboarding welcome command"
    Assert-Contains $mainReadme "profile show" "README profile show command"
    Assert-Contains $mainReadme "config show" "README config show command"
    Assert-Contains $mainReadme "getting-started" "README onboarding getting-started command"
    Assert-Contains $mainReadme "commands" "README onboarding commands command"
    Assert-Contains $mainReadme "man-index" "README onboarding man-index command"
    Assert-Contains (Normalize-Output $mainReadme) "welcome`nprofile show`nconfig show`ngetting-started`ncommands`nman-index`nboot`nhelp`nstatus`nsysinfo`nwhich read`nman index`nman perm`nquit" "README package quickstart command sequence"
    Assert-Contains $mainReadme "which read" "README package quickstart which command"
    Assert-Contains $mainReadme "man perm" "README package quickstart man command"
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_ALPHA.md"))) { throw "README alpha link target missing" }
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_ONBOARDING.md"))) { throw "README onboarding link target missing" }
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_PROFILE.md"))) { throw "README profile link target missing" }
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_CONFIG.md"))) { throw "README config link target missing" }
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_PACKAGE.md"))) { throw "README package link target missing" }
    
    $osReadme = Get-Content (Join-Path $repoRoot "examples/dbyteos/README.md") -Raw
    Assert-Contains $osReadme "DByteOS Alpha Userland (v4.4.1)" "OS README alpha positioning"
    Assert-Contains $osReadme '| `cat` | View file contents |' "OS README command table"
    Assert-Contains $osReadme "Package Smoke" "OS README package smoke"
    Assert-Contains $osReadme ".\dbyte.exe --version" "OS README package version smoke"
    Assert-Contains $osReadme ".\dbyte.exe shell --rc examples/dbyteos/.dbyterc" "OS README package shell smoke"
    Assert-Contains $osReadme "profile show" "OS README profile smoke"
    Assert-Contains $osReadme "config show" "OS README config smoke"
    Assert-Contains $osReadme "sysinfo" "OS README package sysinfo smoke"
    if (-not (Test-Path (Join-Path $repoRoot "docs\DBYTEOS_SECURITY.md"))) { throw "OS README security link target missing" }
    $packageGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_PACKAGE.md") -Raw
    Assert-Contains $packageGuide "DByteOS Package Smoke Guide" "package guide title"
    Assert-Contains $packageGuide ".\dbyte.exe --version" "package guide version smoke"
    Assert-Contains $packageGuide ".\dbyte.exe shell --rc examples/dbyteos/.dbyterc" "package guide shell quickstart"
    Assert-Contains $packageGuide "welcome" "package guide welcome command"
    Assert-Contains $packageGuide "profile show" "package guide profile show command"
    Assert-Contains $packageGuide "config show" "package guide config show command"
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
    Assert-Contains $onboardingGuide "welcome" "onboarding guide welcome"
    Assert-Contains $onboardingGuide "profile show" "onboarding guide profile show"
    Assert-Contains $onboardingGuide "config show" "onboarding guide config show"
    Assert-Contains $onboardingGuide "getting-started" "onboarding guide getting-started"
    Assert-Contains $onboardingGuide "man-index" "onboarding guide man-index"
    $profileGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_PROFILE.md") -Raw
    Assert-Contains $profileGuide "DByteOS Profile" "profile guide title"
    Assert-Contains $profileGuide "profile show" "profile guide show"
    Assert-Contains $profileGuide "alpha-userland" "profile guide mode"
    Assert-Contains $profileGuide "read-only DByteOS config layer" "profile guide config source"
    $configGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_CONFIG.md") -Raw
    Assert-Contains $configGuide "DByteOS Config" "config guide title"
    Assert-Contains $configGuide "config show" "config guide show"
    Assert-Contains $configGuide "system.prompt = dbyte-shell>" "config guide prompt"
    Assert-Contains $configGuide "read-only in v4.4.1" "config guide read-only"

    $alphaGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_ALPHA.md") -Raw
    $bootGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_BOOT.md") -Raw
    $securityGuide = Get-Content (Join-Path $repoRoot "docs/DBYTEOS_SECURITY.md") -Raw
    Assert-NotContains $alphaGuide "file:///C:/Users/" "alpha guide avoids local absolute links"
    Assert-NotContains $bootGuide "file:///C:/Users/" "boot guide avoids local absolute links"
    Assert-NotContains $securityGuide "file:///C:/Users/" "security guide avoids local absolute links"
    Assert-Contains $alphaGuide "[Home](../README.md)" "alpha guide relative home link"
    Assert-Contains $bootGuide "[Alpha Status](DBYTEOS_ALPHA.md)" "boot guide relative alpha link"
    Assert-Contains $securityGuide "[Boot](DBYTEOS_BOOT.md)" "security guide relative boot link"

    $staleReleaseVersion = "4.4." + "0"
    $staleReleasePatterns = @("v$staleReleaseVersion", "DByte $staleReleaseVersion", "dbyte-v$staleReleaseVersion")
    $releaseRefFiles = @(
        "Cargo.toml",
        "Cargo.lock",
        "README.md",
        "INSTALL.md",
        "LANGUAGE_SPEC.md",
        "scripts\verify.ps1",
        "scripts\package_release.ps1",
        "docs\DBYTEOS_ALPHA.md",
        "docs\DBYTEOS_CONFIG.md",
        "docs\DBYTEOS_PROFILE.md",
        "examples\dbyteos\README.md",
        "examples\dbyteos\etc\system.dby",
        "examples\dbyteos\etc\manual\profile.txt"
    )
    foreach ($releaseRefFile in $releaseRefFiles) {
        $releaseRefText = Get-Content (Join-Path $repoRoot $releaseRefFile) -Raw
        foreach ($stalePattern in $staleReleasePatterns) {
            Assert-NotContains $releaseRefText $stalePattern "stale release ref $releaseRefFile"
        }
    }


    $inspectSource = Get-Content (Join-Path $dbyteosRoot "bin\inspect.dby") -Raw
    # v4.4.1 enforcement confirmed via smoke tests above
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

    $dbyteosCmdShell = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "whoami`nsysinfo`nhome`ntmp`nprofile`npath`nenv`nwhich cat`nnotes`nmkdir-demo`nwrite tmp/shell_chain.txt shell chain ok`nread tmp/shell_chain.txt`nwrite-demo`ncat tmp/write_demo.txt`nclean`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosCmdShell.Code -ne 0) { throw "dbyteos command shell chain failed: $($dbyteosCmdShell.Text)" }
    Assert-Contains $dbyteosCmdShell.Text "deadbyte" "dbyteos shell whoami"
    Assert-Contains $dbyteosCmdShell.Text "version: DByte 4.4.1" "dbyteos shell sysinfo"
    Assert-Contains $dbyteosCmdShell.Text "home/deadbyte" "dbyteos shell home"
    Assert-Contains $dbyteosCmdShell.Text "wrote tmp/write_demo.txt" "dbyteos shell write-demo"
    Assert-Contains $dbyteosCmdShell.Text "os_version: 4.4.1" "dbyteos shell profile"
    Assert-Contains $dbyteosCmdShell.Text "mode: alpha-userland" "dbyteos shell profile mode"
    Assert-Contains $dbyteosCmdShell.Text "PATH=/bin:/tmp:/home/deadbyte" "dbyteos shell path"
    Assert-Contains $dbyteosCmdShell.Text "cat: dbyteos ->" "dbyteos shell chain which cat autopath"
    Assert-Contains $dbyteosCmdShell.Text "mkdir-demo: ok" "dbyteos shell mkdir-demo"
    Assert-Contains $dbyteosCmdShell.Text "shell chain ok" "dbyteos shell read after write"
    Assert-Contains $dbyteosCmdShell.Text "dbyteos write_demo ok" "dbyteos shell cat"

    Write-Host "Running DByteOS Notes Workflow (v4.4.1) smoke tests..."
    $dbyteosNotesWorkflow = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "notes clear-demo`nnotes read`nnotes add First Note`nnotes read`nnotes append Second Note`nnotes read`nnotes list`nclean`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosNotesWorkflow.Code -ne 0) { throw "dbyteos notes workflow failed: $($dbyteosNotesWorkflow.Text)" }
    Assert-Contains $dbyteosNotesWorkflow.Text "notes: reset to seed state" "notes clear-demo"
    Assert-Contains $dbyteosNotesWorkflow.Text "dbyteos notes seed" "notes read seed"
    Assert-Contains $dbyteosNotesWorkflow.Text "notes: added" "notes add First Note"
    Assert-Contains $dbyteosNotesWorkflow.Text "First Note" "notes read First Note"
    Assert-Contains $dbyteosNotesWorkflow.Text "notes: appended" "notes append Second Note"
    Assert-Contains $dbyteosNotesWorkflow.Text "First Note`nSecond Note" "notes read both lines"
    Assert-Contains $dbyteosNotesWorkflow.Text "notes: home/deadbyte/notes.txt (exists)" "notes list"

    Write-Host "Running DByteOS Notes Hardening (v4.4.1) smoke tests..."
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
    
    Write-Host "Running DByteOS Init Services (v4.4.1) smoke tests..."
    $dbyteosInitServices = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "boot`nservices list`nservices status`nservices run notes`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosInitServices.Code -ne 0) { throw "dbyteos init services failed: $($dbyteosInitServices.Text)" }
    Assert-Contains $dbyteosInitServices.Text "Init: starting userland services..." "init start"
    Assert-Contains $dbyteosInitServices.Text "[INIT] notes" "init notes service"
    Assert-Contains $dbyteosInitServices.Text "[INIT] sysinfo" "init sysinfo service"
    Assert-Contains $dbyteosInitServices.Text "System State: Initialized" "services status ok"
    Assert-Contains $dbyteosInitServices.Text "[ACTIVE] notes" "services status notes"
    Assert-Contains $dbyteosInitServices.Text "services: running notes..." "services run notes"
    
    Write-Host "Running DByteOS Journal/Logger (v4.4.1) smoke tests..."
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

    Assert-Contains $dbyteosCmdShell.Text "workspace sweep complete" "dbyteos shell clean sweep"

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

Push-Location (Join-Path $repoRoot "tests\project\basic")
try {
    $result = Invoke-Dbyte -Arguments @("run")
    if ($result.Code -ne 0) { throw "basic project run failed: $($result.Text)" }
    $expected = (Get-Content "expected.out" -Raw).Trim()
    Assert-Equal $result.Text $expected "basic project run"
    $vmResult = Invoke-Dbyte -Arguments @("run", "--vm")
    if ($vmResult.Code -ne 0) { throw "basic project vm run failed: $($vmResult.Text)" }
    Assert-Equal $vmResult.Text $expected "basic project vm run"
    $checkResult = Invoke-Dbyte -Arguments @("check")
    if ($checkResult.Code -ne 0) { throw "basic project check failed: $($checkResult.Text)" }
    Assert-Contains $checkResult.Text "no type errors found" "basic project check"
}
finally {
    Pop-Location
}

Push-Location (Join-Path $repoRoot "tests\project\missing_manifest")
try {
    $result = Invoke-Dbyte -Arguments @("run")
    if ($result.Code -eq 0) { throw "missing manifest project unexpectedly passed" }
    $expected = (Get-Content "expected.err" -Raw).Trim()
    Assert-Contains $result.Text $expected "missing manifest project"
}
finally {
    Pop-Location
}

Push-Location (Join-Path $repoRoot "tests\project\missing_entry")
try {
    $result = Invoke-Dbyte -Arguments @("run")
    if ($result.Code -eq 0) { throw "missing entry project unexpectedly passed" }
    $expected = (Get-Content "expected.err" -Raw).Trim()
    Assert-Contains $result.Text $expected "missing entry project"
}
finally {
    Pop-Location
}

Push-Location (Join-Path $repoRoot "tests\project\invalid_manifest")
try {
    $result = Invoke-Dbyte -Arguments @("run")
    if ($result.Code -eq 0) { throw "invalid manifest project unexpectedly passed" }
    $expected = (Get-Content "expected.err" -Raw).Trim()
    Assert-Contains $result.Text $expected "invalid manifest project"
}
finally {
    Pop-Location
}

Push-Location (Join-Path $repoRoot "tests\project\nested_run\src\tools")
try {
    $result = Invoke-Dbyte -Arguments @("run")
    if ($result.Code -ne 0) { throw "nested project run failed: $($result.Text)" }
    $expected = (Get-Content "..\..\expected.out" -Raw).Trim()
    Assert-Equal $result.Text $expected "nested project run"
    $vmResult = Invoke-Dbyte -Arguments @("run", "--vm")
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
    $result = Invoke-Dbyte -Arguments @("new", "scanner")
    if ($result.Code -ne 0) { throw "dbyte new failed: $($result.Text)" }
    Push-Location "scanner"
    try {
        $runResult = Invoke-Dbyte -Arguments @("run")
        if ($runResult.Code -ne 0) { throw "new project run failed: $($runResult.Text)" }
        Assert-Equal $runResult.Text "hello from scanner" "new project run"
        $vmRunResult = Invoke-Dbyte -Arguments @("run", "--vm")
        if ($vmRunResult.Code -ne 0) { throw "new project vm run failed: $($vmRunResult.Text)" }
        Assert-Equal $vmRunResult.Text "hello from scanner" "new project vm run"
        $testResult = Invoke-Dbyte -Arguments @("test")
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

$EXPECTED_VERSION = "4.4.1"

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

Write-Host "Running benchmark smoke tests..."
& $releaseExe bench --engine tree
if ($LASTEXITCODE -ne 0) { throw "dbyte bench --engine tree failed" }
& $releaseExe bench --engine vm
if ($LASTEXITCODE -ne 0) { throw "dbyte bench --engine vm failed" }
& $releaseExe bench --compare-python
if ($LASTEXITCODE -ne 0) { throw "dbyte bench --compare-python failed" }

Write-Host "Running DByteOS Alpha (v4.4.1) Package Smoke Tests..."
$packageSmokeStatus = Git-Status-Short
$smokeRoot = Join-Path $repoRoot "tmp\package_smoke"
if (Test-Path $smokeRoot) { Remove-Item -Recurse -Force $smokeRoot }
New-Item -ItemType Directory -Path $smokeRoot | Out-Null

Write-Host "  Building and packaging..."
& powershell -ExecutionPolicy Bypass -File .\scripts\package_release.ps1 -Version "4.4.1"
$zipFile = Join-Path $repoRoot "dbyte-v4.4.1-windows-x64.zip"
if (-not (Test-Path $zipFile)) { throw "Package zip not found: $zipFile" }

Write-Host "  Extracting package..."
Expand-Archive -Path $zipFile -DestinationPath $smokeRoot
$extractedExe = Join-Path $smokeRoot "dbyte.exe"
$extractedOsRoot = Join-Path $smokeRoot "examples\dbyteos"

Write-Host "  Verifying version..."
$vOut = & $extractedExe --version
if ($vOut -ne "DByte 4.4.1") { throw "Package version mismatch: $vOut" }

Write-Host "  Verifying direct OS commands..."
$expectedPackageBoot = $expectedDbyteosBoot.Replace("Home:        home/deadbyte", "Home:        examples/dbyteos/home/deadbyte")
$expectedPackageStatus = $expectedDbyteosStatus.Replace("Home:     home/deadbyte", "Home:     examples/dbyteos/home/deadbyte")
$expectedPackageWelcome = $expectedDbyteosWelcome.Replace("  home:    home/deadbyte", "  home:    examples/dbyteos/home/deadbyte")
$expectedPackageProfile = $expectedDbyteosProfile.Replace("home: home/deadbyte", "home: examples/dbyteos/home/deadbyte")
$expectedPackageProfileUnknown = $expectedDbyteosProfileUnknown
$expectedPackageConfig = $expectedDbyteosConfig.Replace("user.home = home/deadbyte", "user.home = examples/dbyteos/home/deadbyte")
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
Assert-Equal (Normalize-Output $configModeOut) "alpha-userland" "Package config mode"
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
$gettingStartedOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\getting_started.dby") 2>&1
Assert-NormalizedEqual $gettingStartedOut $expectedDbyteosGettingStarted "Package getting-started snapshot"
$commandsOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\commands.dby") 2>&1
Assert-NormalizedEqual $commandsOut $expectedDbyteosCommands "Package commands snapshot"
$manIndexOut = & $extractedExe run (Join-Path $extractedOsRoot "bin\man_index.dby") 2>&1
Assert-NormalizedEqual $manIndexOut $expectedDbyteosManIndex "Package man-index snapshot"

Write-Host "  Verifying documentation structure..."
$expectedDocs = @("DBYTEOS_ALPHA.md", "DBYTEOS_COMMANDS.md", "DBYTEOS_SECURITY.md", "DBYTEOS_BOOT.md", "DBYTEOS_PACKAGE.md", "DBYTEOS_ONBOARDING.md", "DBYTEOS_PROFILE.md", "DBYTEOS_CONFIG.md")
foreach ($d in $expectedDocs) {
    if (-not (Test-Path (Join-Path $smokeRoot "docs\$d"))) { throw "Package missing doc: $d" }
}

Write-Host "  Verifying no package junk..."
$rootJunk = @("tmp", "target", "tests", ".git")
foreach ($junk in $rootJunk) {
    if (Test-Path (Join-Path $smokeRoot $junk)) { throw "Package contains root junk: $junk" }
}
$extractedTmp = Join-Path $extractedOsRoot "tmp"
$tmpFiles = Get-ChildItem -Path $extractedTmp -Exclude ".gitignore", ".gitkeep"
if ($tmpFiles.Count -ne 0) { throw "Package contains junk in tmp: $($tmpFiles.Name -join ', ')" }

Write-Host "  Verifying shell RC integration..."
$shellInput = "welcome`nprofile show`nprofile whoami`nprofile home`nprofile theme`nprofile prompt`nconfig show`nconfig keys`nconfig get system.prompt`ngetting-started`ncommands`nman-index`nboot`nhelp`nstatus`nsysinfo`nwhich read`nman index`nman profile`nman config`nman perm`nquit`n"
$shellOut = $shellInput | & $extractedExe shell --rc (Join-Path $extractedOsRoot ".dbyterc") 2>&1
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedPackageWelcome) "Package shell welcome"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedPackageProfile) "Package shell profile show"
Assert-Contains (Normalize-Output $shellOut) "dbyte-shell>" "Package shell profile prompt"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedPackageConfig) "Package shell config show"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosConfigKeys) "Package shell config keys"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosGettingStarted) "Package shell getting-started"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosCommands) "Package shell commands"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosManIndex) "Package shell man-index"
Assert-Contains (Normalize-Output $shellOut) "D B Y T E O S   U S E R L A N D" "Package shell boot"
Assert-Contains (Normalize-Output $shellOut) (Normalize-Output $expectedDbyteosHelp) "Package shell help"
Assert-Contains (Normalize-Output $shellOut) "OS:      DByte  4.4.1" "Package shell status version"
Assert-Contains (Normalize-Output $shellOut) "version: DByte 4.4.1" "Package shell sysinfo version"
Assert-Contains (Normalize-Output $shellOut) "read: dbyteos ->" "Package shell which read"
Assert-Contains (Normalize-Output $shellOut) "Manual topics:" "Package shell man index"
Assert-Contains (Normalize-Output $shellOut) "DByteOS Profile" "Package shell man profile"
Assert-Contains (Normalize-Output $shellOut) "DByteOS Config" "Package shell man config"
Assert-Contains (Normalize-Output $shellOut) "DByteOS Permission Command" "Package shell man perm"

Remove-Item -Recurse -Force $smokeRoot
Assert-GitStatus-Unchanged $packageSmokeStatus "package smoke cleanliness"
Write-Host "Package smoke tests passed."

Write-Host "verify passed"
