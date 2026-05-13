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
if ($versionOut -ne "DByte 3.0.0") {
    throw "Version mismatch: expected 'DByte 3.0.0', got '$versionOut'"
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
$shellInput = "help`nversion`npwd`ncd `"$shellRoot`"`ncd missing-dir`nls`nrun hello.dby`ncheck hello.dby`nrun defs.dby`n: let y: int = 40`n: print(y + 2)`n: print(from_file(20))`nalias hi = run hello.dby`nwhich help`nwhich hi`nwhich missing`naliases`nhi`nalias run = ls`nunalias hi`nhi`nnot_a_real_cmd`nquit`n"
$shellBasic = Invoke-DbyteInput -Arguments @("shell", "--no-rc") -InputText $shellInput
if ($shellBasic.Code -ne 0) { throw "shell basic command failed: $($shellBasic.Text)" }
Assert-Contains $shellBasic.Text "DByte shell commands" "shell help"
Assert-Contains $shellBasic.Text "alias <name> = <command>" "shell registry alias help"
Assert-Contains $shellBasic.Text "which <name>" "shell registry which help"
Assert-Contains $shellBasic.Text "DByte 3.0.0" "shell version"
Assert-Contains $shellBasic.Text "ShellError: failed to cd" "shell invalid cd"
Assert-Contains $shellBasic.Text "hello.dby" "shell ls"
Assert-Contains $shellBasic.Text "shell file ok" "shell run file"
Assert-Contains $shellBasic.Text "no type errors found" "shell check file"
Assert-Contains $shellBasic.Text "42" "shell code persistence"
Assert-Contains $shellBasic.Text "help: built-in" "shell which built-in"
Assert-Contains $shellBasic.Text "hi: alias -> run hello.dby" "shell which alias"
Assert-Contains $shellBasic.Text "missing: not found" "shell which missing"
Assert-Contains $shellBasic.Text "hi = run hello.dby" "shell aliases list"
Assert-Contains $shellBasic.Text "ShellError: alias cannot override built-in command: run" "shell alias built-in collision"
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

$shellBadAliasRoot = Join-Path $interactiveRoot "shell-bad-alias-rc"
New-Item -ItemType Directory -Path $shellBadAliasRoot | Out-Null
Set-Content -Path (Join-Path $shellBadAliasRoot ".dbyterc") -Value "let ok: int = 1`n@shell alias cd = ls" -NoNewline
$shellBadAlias = Invoke-DbyteInput -Arguments @("shell") -InputText "quit`n" -WorkingDirectory $shellBadAliasRoot
if ($shellBadAlias.Code -eq 0) { throw "shell bad alias rc unexpectedly passed: $($shellBadAlias.Text)" }
Assert-Contains $shellBadAlias.Text "ShellError:" "shell rc alias collision"
Assert-Contains $shellBadAlias.Text "line 2: alias cannot override built-in command: cd" "shell rc alias collision line number"

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

Write-Host "Running Sanctum System Workspace (v3.0.0) smoke tests..."
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

Write-Host "Running DByteOS Userland Prototype (v3.0.0) smoke tests..."
$dbyteosRoot = Join-Path $repoRoot "examples\dbyteos"
$dbyteosStatus = Git-Status-Short
try {
    # 1. Boot sequence
    $dbyteosBoot = Invoke-Dbyte -Arguments @("run", "boot.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosBoot.Code -ne 0) { throw "dbyteos boot failed: $($dbyteosBoot.Text)" }
    Assert-Contains $dbyteosBoot.Text "D B Y T E O S   U S E R L A N D" "dbyteos boot banner"
    Assert-Contains $dbyteosBoot.Text "[OK] /bin" "dbyteos boot bin check"
    
    # 2. System status
    $dbyteosStatusReport = Invoke-Dbyte -Arguments @("run", "bin\status.dby") -WorkingDirectory $dbyteosRoot
    if ($dbyteosStatusReport.Code -ne 0) { throw "dbyteos status failed: $($dbyteosStatusReport.Text)" }
    Assert-Contains $dbyteosStatusReport.Text "--- DByteOS System Status ---" "dbyteos status banner"
    Assert-Contains $dbyteosStatusReport.Text "bin: [PRESENT]" "dbyteos status bin ok"

    # 3. Shell aliases
    $dbyteosShell = Invoke-DbyteInput -Arguments @("shell", "--rc", ".dbyterc") -InputText "status`nclean`nquit`n" -WorkingDirectory $dbyteosRoot
    if ($dbyteosShell.Code -ne 0) { throw "dbyteos shell failed: $($dbyteosShell.Text)" }
    Assert-Contains $dbyteosShell.Text "--- DByteOS System Status ---" "dbyteos shell status alias"
    Assert-Contains $dbyteosShell.Text "DByteOS: Cleaning /tmp and artifacts..." "dbyteos shell clean alias"

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

$EXPECTED_VERSION = "3.0.0"

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

Write-Host "verify passed"
