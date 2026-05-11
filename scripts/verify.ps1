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
    param([string[]]$Arguments)

    $oldPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $output = & $cli @Arguments 2>&1
        $code = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $oldPreference
    }
    return [PSCustomObject]@{
        Code = $code
        Text = Normalize-Output $output
    }
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

$EXPECTED_VERSION = "1.9.1"

$DBYTE_BIN = "target/release/dbyte.exe"
$releaseExe = Join-Path $repoRoot "target\release\dbyte.exe"
& $cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$version = & $releaseExe --version
if ($version -notmatch $EXPECTED_VERSION) { throw "version check failed: got '$version'" }

Write-Host "Running benchmark smoke tests..."
& $releaseExe bench --engine tree
if ($LASTEXITCODE -ne 0) { throw "dbyte bench --engine tree failed" }
& $releaseExe bench --engine vm
if ($LASTEXITCODE -ne 0) { throw "dbyte bench --engine vm failed" }
& $releaseExe bench --compare-python
if ($LASTEXITCODE -ne 0) { throw "dbyte bench --compare-python failed" }

Write-Host "verify passed"
