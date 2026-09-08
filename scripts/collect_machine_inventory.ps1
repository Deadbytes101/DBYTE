# Read-only Windows observations. JSON stdout; diagnostics stderr. No elevation.
param([string]$FixturePath = '')
$ErrorActionPreference = 'Stop'

function Fact($Value, [string]$Source, [string]$Status = '') {
    if (-not $Status) { $Status = if ($null -eq $Value -or ($Value -is [string] -and -not $Value.Trim()) -or ($Value -is [array] -and $Value.Count -eq 0)) { 'UNKNOWN' } else { 'OBSERVED' } }
    return [ordered]@{ status = $Status; value = $Value; source = $Source }
}
function Field($Object, [string]$Name) {
    if ($null -eq $Object) { return $null }
    return $Object.$Name
}
function Record($Object, [string[]]$Names, [string]$Source) {
    $result = [ordered]@{}
    foreach ($name in $Names) { $result[$name] = Fact (Field $Object $name) $Source }
    return $result
}
$queryErrors = [System.Collections.Generic.List[object]]::new()
function Query([string]$Class, [string]$Namespace = 'root/cimv2') {
    try {
        if (Get-Command Get-CimInstance -ErrorAction SilentlyContinue) {
            return @(Get-CimInstance -Namespace $Namespace -ClassName $Class -ErrorAction Stop)
        }
        return @(Get-WmiObject -Namespace $Namespace -Class $Class -ErrorAction Stop)
    } catch {
        $queryErrors.Add((Fact $_.Exception.Message "$Namespace/$Class" 'UNAVAILABLE'))
        [Console]::Error.WriteLine("INVENTORY_QUERY_UNAVAILABLE: $Namespace/$Class")
        return @()
    }
}
if ($FixturePath) {
    $raw = Get-Content -Raw -LiteralPath $FixturePath | ConvertFrom-Json
} else {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'INVENTORY_PLATFORM_MISMATCH: Windows required' }
    $raw = [pscustomobject]@{
        cpu = @(Query 'Win32_Processor'); board = @(Query 'Win32_BaseBoard')
        system = @(Query 'Win32_ComputerSystem'); product = @(Query 'Win32_ComputerSystemProduct')
        os = @(Query 'Win32_OperatingSystem'); pnp = @(Query 'Win32_PnPEntity')
        disks = @(Query 'Win32_DiskDrive'); partitions = @(Query 'Win32_DiskPartition')
        logical = @(Query 'Win32_LogicalDisk'); disk_partition = @(Query 'Win32_DiskDriveToDiskPartition')
        logical_partition = @(Query 'Win32_LogicalDiskToPartition')
        storage_disks = @(Query 'MSFT_Disk' 'root/Microsoft/Windows/Storage')
        scsi_associations = @(Query 'Win32_SCSIControllerDevice')
        ide_associations = @(Query 'Win32_IDEControllerDevice')
        keyboard = @(Query 'Win32_Keyboard'); video = @(Query 'Win32_VideoController')
        network = @(Query 'Win32_NetworkAdapter'); firmware = $null
        pnp_parents = @()
    }
    # PEFirmwareType is current-boot evidence only; absent registry value is UNKNOWN.
    try { $raw.firmware = (Get-ItemProperty -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Control' -Name PEFirmwareType -ErrorAction Stop).PEFirmwareType } catch { }
    if (Get-Command Get-PnpDeviceProperty -ErrorAction SilentlyContinue) {
        $raw.pnp_parents = @(
            foreach ($key in $raw.keyboard) {
                $id = [string]$key.PNPDeviceID
                $visited = @{}
                for ($depth = 0; $depth -lt 16 -and $id -and -not $visited.ContainsKey($id); $depth++) {
                    $visited[$id] = $true
                    try {
                        $parent = (Get-PnpDeviceProperty -InstanceId $id -KeyName 'DEVPKEY_Device_Parent' -ErrorAction Stop).Data
                        [pscustomobject]@{ child = $id; parent = $parent }
                        $id = [string]$parent
                    } catch { break }
                }
            }
        )
    }
}
$cpu = @($raw.cpu) | Select-Object -First 1
$board = @($raw.board) | Select-Object -First 1
$product = @($raw.product) | Select-Object -First 1
$system = @($raw.system) | Select-Object -First 1
$osInfo = @($raw.os) | Select-Object -First 1
$pci = @(
    foreach ($dev in @($raw.pnp | Sort-Object PNPDeviceID)) {
        $id = [string]$dev.PNPDeviceID
        if ($id -notmatch '^PCI\\VEN_([0-9A-F]{4})&DEV_([0-9A-F]{4})') { continue }
        $vendor = $Matches[1]; $device = $Matches[2]; $classCode = $null
        foreach ($compatible in @($dev.CompatibleID) + @($dev.HardwareID)) {
            if ($compatible -match 'CC_([0-9A-F]{6})(?:$|&)') { $classCode = $Matches[1]; break }
        }
        $row = [pscustomobject]@{ id=$id; vendor=$vendor; device=$device; class_code=$classCode
            name=$dev.Name; service=$dev.Service; status=$dev.Status }
        Record $row @('id','vendor','device','class_code','name','service','status') 'Win32_PnPEntity hardware/compatible IDs (OS-reported)'
    }
)
$mode = switch ([string]$raw.firmware) { '2' { 'UEFI' }; '1' { 'LEGACY' }; default { $null } }
$identity = [ordered]@{
    board_model = (Field $board 'Product'); board_serial = (Field $board 'SerialNumber')
    board_vendor = (Field $board 'Manufacturer'); cpu_model = (Field $cpu 'Name')
    cpu_vendor = (Field $cpu 'Manufacturer'); system_uuid = (Field $product 'UUID')
}
foreach ($name in @($identity.Keys)) {
    if ($identity[$name] -is [string]) { $identity[$name] = $identity[$name].Trim() }
}
# Windows serializer preimage is included. Compare fingerprints from the same collector;
# cross-OS comparisons must inspect the normalized fields rather than assume formatting equality.
$preimage = $identity | ConvertTo-Json -Compress
$sha = [Security.Cryptography.SHA256]::Create()
try { $hash = ([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($preimage)))).Replace('-','').ToLowerInvariant() } finally { $sha.Dispose() }
$complete = $true
foreach ($value in $identity.Values) {
    if (-not $value -or $value -match '^(To be filled|Default|Unknown|None)' -or ([string]$value).Replace('-','') -match '^(0+|F+)$') { $complete = $false }
}
$diskRows = @(
    foreach ($disk in @($raw.disks | Sort-Object DeviceID)) {
        Record $disk @('DeviceID','Index','Model','SerialNumber','InterfaceType','Size','MediaType','Capabilities','PNPDeviceID','SCSIPort','SCSIBus','SCSITargetId','SCSILogicalUnit') 'Win32_DiskDrive; InterfaceType is OS-reported, not inferred controller mode'
    }
)
$keys = @(
    foreach ($key in @($raw.keyboard | Sort-Object PNPDeviceID)) {
        $transport = $null; $id = [string]$key.PNPDeviceID
        for ($depth=0; $depth -lt 16 -and $id; $depth++) {
            if ($id -match '^USB\\') { $transport='USB'; break }
            $binding = @($raw.pnp | Where-Object { $_.PNPDeviceID -eq $id }) | Select-Object -First 1
            if ($binding.Service -eq 'i8042prt') { $transport='PS/2'; break }
            $parent = @($raw.pnp_parents | Where-Object { $_.child -eq $id }) | Select-Object -First 1
            $id = [string]$parent.parent
        }
        $r = Record $key @('Name','PNPDeviceID','Description','Status') 'Win32_Keyboard'
        $r['transport'] = Fact $transport 'PnP parent chain or i8042prt service; HID alone is inconclusive'
        $r
    }
)
$obs = [ordered]@{
    cpu = (Record $cpu @('Manufacturer','Name','Family','Stepping','Revision','ProcessorId','Architecture','AddressWidth','DataWidth') 'Win32_Processor (OS-reported; not raw CPUID family/model decoding)')
    cpuid_features = (Fact $null 'no direct CPUID instruction executed' 'UNAVAILABLE')
    long_mode = (Fact $(if ($cpu -and $cpu.Architecture -eq 9 -and $cpu.DataWidth -eq 64) { $true } else { $null }) 'Win32_Processor architecture/data width (OS-reported)')
    ram = (Fact (Field $system 'TotalPhysicalMemory') 'Win32_ComputerSystem bytes')
    motherboard = (Record $board @('Manufacturer','Product','Version','SerialNumber') 'Win32_BaseBoard')
    firmware = [ordered]@{ current_boot_mode=(Fact $mode 'PEFirmwareType registry current boot'); uefi_capable=(Fact $(if ($mode -eq 'UEFI') {$true} else {$null}) 'current boot evidence only'); bios_only=(Fact $null 'legacy boot does not establish BIOS-only firmware') }
    pci = (Fact $pci 'Win32_PnPEntity')
    storage_controllers = (Fact @($pci | Where-Object { $_.class_code.value -match '^01' }) 'PCI class codes; no guessed firmware mode')
    usb_controllers = (Fact @($pci | Where-Object { $_.class_code.value -match '^0C03' }) 'PCI class codes')
    storage_active_mode = (Fact $null 'class/prog-if plus bound service are evidence; no firmware setting inferred')
    input_devices = (Fact $keys 'Win32_Keyboard with bounded PnP parent traversal')
    input_parent_chain = (Fact @($raw.pnp_parents | Sort-Object child,parent) 'Get-PnpDeviceProperty DEVPKEY_Device_Parent')
    i8042 = (Fact @($raw.pnp | Where-Object { $_.Service -eq 'i8042prt' } | ForEach-Object { Record $_ @('PNPDeviceID','Name','Service') 'Win32_PnPEntity' }) 'i8042prt binding; empty does not establish absence')
    disks = (Fact $diskRows 'Win32_DiskDrive')
    partitions = (Fact @($raw.partitions | Sort-Object DiskIndex,Index | ForEach-Object { Record $_ @('DeviceID','DiskIndex','Index','StartingOffset','Size','Type','BootPartition','PrimaryPartition') 'Win32_DiskPartition' }) 'Win32_DiskPartition')
    storage_disk_flags = (Fact @($raw.storage_disks | Sort-Object Number | ForEach-Object { Record $_ @('Number','Path','Model','SerialNumber','Size','BusType','IsBoot','IsSystem','IsOffline','PartitionStyle') 'MSFT_Disk' }) 'MSFT_Disk; may be unavailable on older Windows')
    disk_partition_links = (Fact @($raw.disk_partition | Sort-Object Antecedent,Dependent | ForEach-Object { Record $_ @('Antecedent','Dependent') 'Win32_DiskDriveToDiskPartition' }) 'WMI association')
    logical_partition_links = (Fact @($raw.logical_partition | Sort-Object Antecedent,Dependent | ForEach-Object { Record $_ @('Antecedent','Dependent') 'Win32_LogicalDiskToPartition' }) 'WMI association')
    controller_links = (Fact @(@($raw.scsi_associations)+@($raw.ide_associations) | Sort-Object Antecedent,Dependent | ForEach-Object { Record $_ @('Antecedent','Dependent') 'Win32_SCSIControllerDevice/Win32_IDEControllerDevice' }) 'WMI association; unresolved links do not authorize writes')
    boot_system = (Record $osInfo @('SystemDrive','BootDevice','WindowsDirectory') 'Win32_OperatingSystem; correlate with associations and MSFT_Disk flags')
    gpu = (Fact @($raw.video | Sort-Object PNPDeviceID | ForEach-Object { Record $_ @('PNPDeviceID','Name','CurrentHorizontalResolution','CurrentVerticalResolution') 'Win32_VideoController; OS mode is not a GOP receipt' }) 'Win32_VideoController')
    network = (Fact @($raw.network | Sort-Object PNPDeviceID | ForEach-Object { Record $_ @('PNPDeviceID','Name','PhysicalAdapter') 'Win32_NetworkAdapter' }) 'Win32_NetworkAdapter')
}
$privilege = 'UNKNOWN'
if (-not $FixturePath) {
    $principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
    $privilege = if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { 'administrator' } else { 'unprivileged' }
}
$receipt = [ordered]@{
    schema='dbyte-machine-inventory-v1'; collector='windows'; fixture=[bool]$FixturePath
    collected_utc=[DateTime]::UtcNow.ToString('o')
    environment=(Record ([pscustomobject]@{os=(Field $osInfo 'Caption'); powershell=$PSVersionTable.PSVersion.ToString(); privilege=$privilege}) @('os','powershell','privilege') 'collector environment')
    query_errors=@($queryErrors.ToArray()); observations=$obs
    fingerprint=[ordered]@{sha256=$hash; preimage=$preimage; complete=$complete; identity=$identity; policy='comparison aid only; never disk-write authorization'}
    acceptance=[ordered]@{primary_real_hw_target='UNSELECTED';storage_driver='UNSELECTED';input_driver='UNSELECTED';dedicated_test_disk='UNSELECTED';disk_write='NO WRITE';real_hardware='PENDING';full_os='PENDING'}
}
$receipt | ConvertTo-Json -Depth 20
