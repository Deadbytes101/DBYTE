param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $Args
)

$ErrorActionPreference = "Stop"

$Root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$HelperSource = Join-Path $PSScriptRoot "native\dbyte-audio.c"
$HelperExe = Join-Path $PSScriptRoot "native\dbyte-audio.exe"

function Build-Helper {
    if (Test-Path $HelperExe) {
        return
    }

    $cl = Get-Command cl.exe -ErrorAction SilentlyContinue
    if ($cl) {
        Push-Location (Join-Path $PSScriptRoot "native")
        try {
            & cl.exe /nologo /W4 /O2 /Fe:dbyte-audio.exe dbyte-audio.c winmm.lib
        } finally {
            Pop-Location
        }
        return
    }

    $gcc = Get-Command gcc.exe -ErrorAction SilentlyContinue
    if ($gcc) {
        & gcc.exe -O2 -Wall -Wextra -o $HelperExe $HelperSource -lwinmm
        return
    }

    throw "No C compiler found. Install Visual Studio Build Tools or MinGW gcc.exe."
}

Build-Helper

Push-Location $Root
try {
    & cargo run -q -p dbyte_cli -- run .\personal_tools\bytedeck\musicplayer.dby @Args
} finally {
    Pop-Location
}
