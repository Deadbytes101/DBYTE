"""Read-only OS observations; never opens a block device or authorizes writes."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys


def fact(value, source, status=None):
    return {"status": status or ("UNKNOWN" if value is None or value == [] or value == '' else "OBSERVED"),
            "value": value, "source": source}


def record(values, source):
    return {key: fact(value, source) for key, value in sorted(values.items())}


def read(path):
    try:
        return Path(path).read_text(errors="replace").strip() or None
    except (OSError, ValueError):
        return None


def command(args):
    if not shutil.which(args[0]):
        return fact(None, args[0], "UNAVAILABLE")
    try:
        result = subprocess.run(args, capture_output=True, text=True, timeout=20,
                                env=dict(os.environ, LC_ALL="C"), check=False)
        if result.returncode:
            return fact({"exit_code": result.returncode, "stderr": result.stderr.strip()},
                        " ".join(args), "UNAVAILABLE")
        return fact(result.stdout.strip(), " ".join(args))
    except (OSError, subprocess.TimeoutExpired) as exc:
        return fact(str(exc), " ".join(args), "UNAVAILABLE")


def bounded_text(path, limit=262144):
    """Read pseudo-files without conflating denial, emptiness and truncation."""
    try:
        with Path(path).open('rb') as stream:
            data = stream.read(limit + 1)
        if len(data) > limit:
            return fact(None, str(path) + ' (size limit exceeded)', 'UNAVAILABLE')
        return fact(data.decode('utf-8', errors='replace').strip(), str(path))
    except OSError as exc:
        return fact(None, str(path) + ': ' + str(exc), 'UNAVAILABLE')


def directory_inventory(path, limit=256):
    """List names/associations only; do not read ACPI binary tables or devices."""
    try:
        entries = []
        with os.scandir(path) as iterator:
            for entry in iterator:
                if len(entries) == limit:
                    return fact(None, str(path) + ' (entry limit exceeded)', 'UNAVAILABLE')
                entries.append({'name': entry.name, 'resolved_path': os.path.realpath(entry.path)})
        return fact(sorted(entries, key=lambda item: item['name']), str(path))
    except OSError as exc:
        return fact(None, str(path) + ': ' + str(exc), 'UNAVAILABLE')


def discovery_observations(pci):
    sata = []
    for device in sorted(pci, key=lambda item: item['address']):
        address = device['address']
        if str(device.get('class_code')).startswith('0x0106') and re.fullmatch(
                r'[0-9a-fA-F]{4}:[0-9a-fA-F]{2}:[0-9a-fA-F]{2}\.[0-7]', address):
            if len(sata) == 64:
                return fact(None, 'SATA controller limit exceeded', 'UNAVAILABLE')
            sata.append({'address': address, 'detail': command(['lspci', '-Dnnvv', '-s', address])})
    messages = command(['dmesg', '--color=never'])
    if messages['status'] == 'OBSERVED':
        lines = [line for line in messages['value'].splitlines()
                 if re.search(r'\b(?:ahci|ata\d*|sata|ide|scsi|sd\w*)\b', line, re.IGNORECASE)]
        messages = fact(lines, 'dmesg --color=never; storage lines only') if len(lines) <= 4096 else fact(
            None, 'dmesg storage lines exceed 4096', 'UNAVAILABLE')
    return {'sata_pci_detail': fact(sata, 'discovered PCI SATA class; lspci -Dnnvv'),
            'ata_hosts': directory_inventory('/sys/class/ata_host'),
            'scsi_hosts': directory_inventory('/sys/class/scsi_host'),
            'kernel_storage_messages': messages,
            'interrupts': bounded_text('/proc/interrupts'),
            'acpi_tables': directory_inventory('/sys/firmware/acpi/tables')}


def probe():
    cpu = {}
    for line in (read('/proc/cpuinfo') or '').splitlines():
        if not line.strip() and cpu:
            break
        if ':' in line:
            key, value = line.split(':', 1)
            cpu[key.strip()] = value.strip()
    mem = dict(line.split(':', 1) for line in (read('/proc/meminfo') or '').splitlines() if ':' in line)
    dmi = Path('/sys/class/dmi/id')
    pci = []
    for dev in sorted(Path('/sys/bus/pci/devices').glob('*')):
        pci.append({"address": dev.name, "vendor": read(dev/'vendor'), "device": read(dev/'device'),
                    "class_code": read(dev/'class'), "driver": (dev/'driver').resolve().name if (dev/'driver').exists() else None})
    inputs = []
    for dev in sorted(Path('/sys/class/input').glob('event*')):
        path = str((dev/'device').resolve())
        inputs.append({"path": path, "name": read(dev/'device/name'),
                       "bus_type": read(dev/'device/id/bustype'),
                       "key_capabilities": read(dev/'device/capabilities/key'),
                       "transport": "USB" if '/usb' in path else ("PS/2" if '/i8042/' in path else None)})
    block = command(['lsblk', '--json', '--bytes', '--paths', '--output',
                     'NAME,KNAME,PKNAME,TYPE,SIZE,MODEL,SERIAL,TRAN,RM,MOUNTPOINT'])
    disks = []
    for dev in sorted(Path('/sys/class/block').glob('*')):
        # Include partitions and virtual devices so multi-disk backing is visible.
        disks.append({"name": dev.name, "device_path": '/dev/'+dev.name, "major_minor": read(dev/'dev'),
                      "sysfs_path": str(dev.resolve()), "size_512_sectors": read(dev/'size'),
                      "logical_block_size": read(dev/'queue/logical_block_size'),
                      "model": read(dev/'device/model'), "serial": read(dev/'device/serial'),
                      "removable": read(dev/'removable'), "partition_number": read(dev/'partition'),
                      "partition_start_512_sectors": read(dev/'start'),
                      "slaves": sorted(p.name for p in (dev/'slaves').glob('*')),
                      "holders": sorted(p.name for p in (dev/'holders').glob('*')),
                      "controller_path": str((dev/'device').resolve()) if (dev/'device').exists() else None})
    return {"cpu": cpu, "memory": mem, "board": {k: read(dmi/k) for k in
            ('board_vendor', 'board_name', 'board_serial', 'product_uuid')},
            "firmware_mode": 'UEFI' if Path('/sys/firmware/efi').is_dir() else 'UNKNOWN',
            "pci": pci, "inputs": inputs, "disks": disks,
            "discovery": discovery_observations(pci),
            "mountinfo": read('/proc/self/mountinfo'), "lsblk": block,
            "lspci": command(['lspci', '-Dnnk']), "lsblk_version": command(['lsblk', '--version']),
            "lspci_version": command(['lspci', '--version']),
            "i8042": [str(p) for p in Path('/sys/bus/platform/devices').glob('i8042*')],
            "os": platform.platform(), "privilege": 'root' if os.geteuid() == 0 else 'unprivileged'}


def mounted_roles(raw):
    """Expose every leaf backing root/boot/ESP mounts; never select a test disk."""
    devices = {d['name']: d for d in raw.get('disks', [])}
    by_number = {d.get('major_minor'): d for d in devices.values() if d.get('major_minor')}
    results = []
    for line in (raw.get('mountinfo') or '').splitlines():
        fields = line.split()
        if len(fields) < 6 or fields[4] not in ('/', '/boot', '/boot/efi', '/efi'):
            continue
        device = by_number.get(fields[2])
        leaves, seen = [], set()
        pending = [device['name']] if device else []
        unresolved = device is None
        while pending:
            name = pending.pop()
            if name in seen:
                unresolved = True
                continue
            seen.add(name)
            item = devices.get(name)
            if item is None:
                unresolved = True
            elif item.get('slaves'):
                pending.extend(item['slaves'])
            elif item.get('partition_number'):
                parent = Path(item.get('sysfs_path', '')).parent.name
                if parent in devices:
                    pending.append(parent)
                else:
                    unresolved = True
            else:
                leaves.append(name)
        results.append({"mount": fields[4], "major_minor": fields[2],
                        "backing_devices": sorted(leaves), "complete": not unresolved})
    return results


def identity_value_is_meaningful(value):
    if not isinstance(value, str) or not value.strip():
        return False
    normalized = value.strip().lower()
    if normalized.startswith(('unknown', 'none', 'default', 'to be filled')):
        return False
    compact = normalized.replace('-', '')
    return not (compact and set(compact) in ({'0'}, {'f'}))


def normalize(raw, fixture=False):
    cpu, board = raw.get('cpu', {}), raw.get('board', {})
    identity = {"board_vendor": board.get('board_vendor'), "board_model": board.get('board_name'),
                "board_serial": board.get('board_serial'), "system_uuid": board.get('product_uuid'),
                "cpu_vendor": cpu.get('vendor_id'), "cpu_model": cpu.get('model name')}
    identity = {k: v.strip() if isinstance(v, str) else v for k, v in identity.items()}
    complete = all(identity_value_is_meaningful(identity[name]) for name in
                   ('board_vendor', 'board_model', 'cpu_vendor', 'cpu_model')) and any(
                       identity_value_is_meaningful(identity[name]) for name in ('board_serial', 'system_uuid'))
    # Canonical JSON preimage is included for independent recomputation.
    preimage = json.dumps(identity, ensure_ascii=True, sort_keys=True, separators=(',', ':'))
    flags = cpu.get('flags')
    pci = sorted(raw.get('pci', []), key=lambda d: d.get('address', ''))
    sections = {
        "cpu": record({"vendor": cpu.get('vendor_id'), "model": cpu.get('model name'),
                        "family": cpu.get('cpu family'), "model_number": cpu.get('model'),
                        "stepping": cpu.get('stepping'), "features": sorted(flags.split()) if flags else None,
                        "long_mode": 'lm' in flags.split() if flags else None}, '/proc/cpuinfo (OS-reported; not direct CPUID)'),
        "direct_cpuid": fact(None, 'not executed', 'UNAVAILABLE'),
        "ram": fact(raw.get('memory', {}).get('MemTotal', '').strip() or None, '/proc/meminfo MemTotal (OS-usable kB)'),
        "motherboard": record(board, '/sys/class/dmi/id'),
        "firmware": {"current_boot_mode": fact(raw.get('firmware_mode'), '/sys/firmware/efi',
                                               'OBSERVED' if raw.get('firmware_mode') == 'UEFI' else 'UNKNOWN'),
                     "uefi_capable": fact(True if raw.get('firmware_mode') == 'UEFI' else None, 'current boot evidence only'),
                     "bios_only": fact(None, 'cannot infer from absent EFI sysfs')},
        "pci": fact([record(d, '/sys/bus/pci/devices') for d in pci], '/sys/bus/pci/devices'),
        "storage_controllers": fact([record(d, '/sys/bus/pci/devices') for d in pci if str(d.get('class_code')).startswith('0x01')], '/sys/bus/pci/devices'),
        "usb_controllers": fact([record(d, '/sys/bus/pci/devices') for d in pci if str(d.get('class_code')).startswith('0x0c03')], '/sys/bus/pci/devices'),
        "gpu": fact([record(d, '/sys/bus/pci/devices') for d in pci if str(d.get('class_code')).startswith('0x03')], '/sys/bus/pci/devices; no GOP mode inferred'),
        "network": fact([record(d, '/sys/bus/pci/devices') for d in pci if str(d.get('class_code')).startswith('0x02')], '/sys/bus/pci/devices'),
        "input_devices": fact([record(d, '/sys/class/input') for d in sorted(raw.get('inputs', []), key=lambda d: d.get('path', ''))], '/sys/class/input; key capabilities are evidence, not a guessed keyboard'),
        "i8042": fact(raw.get('i8042') or None, '/sys/bus/platform/devices; absence is inconclusive'),
        "disks_and_partitions": fact([record(d, '/sys/class/block') for d in sorted(raw.get('disks', []), key=lambda d: d.get('name', ''))], '/sys/class/block'),
        "mounted_backing": fact(raw.get('mountinfo'), '/proc/self/mountinfo; mount namespace only'),
        "discovery": raw.get('discovery', fact(None, 'not collected; rerun collector', 'UNAVAILABLE')),
        "lsblk": raw.get('lsblk', fact(None, 'lsblk', 'UNAVAILABLE')),
        "lspci": raw.get('lspci', fact(None, 'lspci', 'UNAVAILABLE')),
        "storage_active_mode": fact(None, 'PCI class/prog-if and driver are evidence; no firmware setting inferred'),
        "boot_system_disk": fact(mounted_roles(raw) or None, '/proc/self/mountinfo major:minor + sysfs slaves/partition graph; unmounted boot media remain UNKNOWN')}
    return {"schema": 'dbyte-machine-inventory-v1', "collector": 'linux', "fixture": fixture,
            "collected_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "environment": record({"os": raw.get('os'), "privilege": raw.get('privilege'),
                                   "python": platform.python_version()}, 'collector environment'),
            "tools": {"lsblk": raw.get('lsblk_version', fact(None, 'lsblk', 'UNAVAILABLE')),
                      "lspci": raw.get('lspci_version', fact(None, 'lspci', 'UNAVAILABLE'))},
            "observations": sections,
            "fingerprint": {"sha256": hashlib.sha256(preimage.encode()).hexdigest(), "preimage": preimage,
                            "complete": complete, "identity": identity,
                            "policy": 'comparison aid only; never disk-write authorization'},
            "acceptance": {"primary_real_hw_target": 'UNSELECTED', "storage_driver": 'UNSELECTED',
                           "input_driver": 'UNSELECTED', "dedicated_test_disk": 'UNSELECTED',
                           "disk_write": 'NO WRITE', "real_hardware": 'PENDING', "full_os": 'PENDING'}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--fixture', type=Path, help='offline test observation JSON; always marks receipt as fixture')
    args = parser.parse_args()
    if not args.fixture and sys.platform != 'linux':
        parser.error('INVENTORY_PLATFORM_MISMATCH: Linux required')
    raw = json.loads(args.fixture.read_text()) if args.fixture else probe()
    print(json.dumps(normalize(raw, bool(args.fixture)), ensure_ascii=True, sort_keys=True, indent=2))


if __name__ == '__main__':
    main()
