$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$python = (Get-Command python.exe -ErrorAction Stop).Source

function Invoke-InventoryTest([string]$Path) {
    & $python -B $Path
    if ($LASTEXITCODE -ne 0) {
        throw "machine inventory test failed: $Path (exit $LASTEXITCODE)"
    }
}

Push-Location $repo
try {
    Invoke-InventoryTest 'scripts/test_machine_inventory.py'
    Invoke-InventoryTest 'scripts/test_machine_inventory_receipt.py'
    Write-Host 'MACHINE INVENTORY TOOLING: PASS'
} finally {
    Pop-Location
}
