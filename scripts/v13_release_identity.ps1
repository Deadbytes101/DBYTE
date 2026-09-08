$script:V13FrozenBaseHead = 'a1a5c2458065f6f295d1c3e77df62a34846226e3'

function Get-V13ExpectedHead {
    $value = [Environment]::GetEnvironmentVariable('DBYTE_V13_EXPECTED_HEAD')
    if ([string]::IsNullOrWhiteSpace($value)) {
        return $script:V13FrozenBaseHead
    }
    $value = $value.Trim().ToLowerInvariant()
    if ($value -notmatch '^[0-9a-f]{40}$') {
        throw 'V13_EXPECTED_HEAD_INVALID'
    }
    return $value
}
