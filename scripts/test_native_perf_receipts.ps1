$ErrorActionPreference = 'Stop'
$path = Join-Path $PSScriptRoot 'verify_native_machine_code_performance.ps1'
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Verifier parse failed' }
if ($ast.Extent.Text.Contains('Remove-Item -LiteralPath $proofRoot -Recurse')) { throw 'Next run deletes prior measurement evidence' }
foreach ($name in @('Save-MeasurementReceipt', 'Measure-WindowsPair')) {
    $definition = $ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name }, $true)
    if (-not $definition) { throw "Missing receipt function: $name" }
    . ([scriptblock]::Create($definition.Extent.Text))
}
$proofRoot = Join-Path $env:TEMP ('dbyte-native-receipt-test-' + [guid]::NewGuid().ToString('N'))
$measureScript = 'measurement.py'
$raw = '{"stable":false,"attempts":[{"attempt":1,"baseline":{"relative_mad":0.08}}]}'
function Invoke-Checked { return $raw }
$manifest = [pscustomobject]@{
    warmups = 3; measurements = 9; maximum_attempts = 3
    maximum_relative_mad = 0.05; windows_affinity_cpu = 10
    windows_settle_milliseconds = 1000; sample_repetitions = 8
}
$result = Measure-WindowsPair 'baseline.exe' 'candidate.exe' 0 $manifest
if ($result.stable) { throw 'Unstable result must remain unstable' }
$files = @(Get-ChildItem -LiteralPath (Join-Path $proofRoot 'measurements') -Filter '*.json')
if ($files.Count -ne 1) { throw 'Unstable raw measurement was not preserved' }
$receipt = Get-Content -LiteralPath $files[0].FullName -Raw | ConvertFrom-Json
if ($receipt.stdout -cne $raw) { throw 'Raw distributions changed' }
if ($receipt.platform -ne 'windows' -or $receipt.arguments -notcontains '--sample-repetitions') { throw 'Command binding missing' }
Measure-WindowsPair 'baseline.exe' 'candidate.exe' 0 $manifest | Out-Null
if (@(Get-ChildItem -LiteralPath (Join-Path $proofRoot 'measurements') -Filter '*.json').Count -ne 2) { throw 'Repeated measurements overwrote evidence' }
Write-Output "NATIVE PERFORMANCE RECEIPT TESTS: PASS (unstable preservation, command binding, unique paths); evidence=$proofRoot"
