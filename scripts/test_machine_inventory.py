"""Offline saved-observation tests. Does not inspect this host's hardware."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('inventory', ROOT/'machine_inventory_linux.py')
inventory = importlib.util.module_from_spec(spec)
spec.loader.exec_module(inventory)


class LinuxTests(unittest.TestCase):
    def setUp(self):
        self.raw = json.loads((ROOT/'tests/machine_inventory/linux.json').read_text())

    def test_legacy_never_proves_bios_only(self):
        r = inventory.normalize(self.raw, True)
        self.assertIsNone(r['observations']['firmware']['bios_only']['value'])
        self.assertIsNone(r['observations']['firmware']['uefi_capable']['value'])
        self.assertEqual(r['acceptance']['disk_write'], 'NO WRITE')

    def test_raid_backing_is_not_collapsed_to_single_disk(self):
        roles = inventory.normalize(self.raw)['observations']['boot_system_disk']['value']
        self.assertEqual(roles[0]['backing_devices'], ['sda', 'sdb'])
        self.assertTrue(roles[0]['complete'])

    def test_missing_tools_permissions_and_unknown_input(self):
        o = inventory.normalize(self.raw)['observations']
        self.assertEqual(o['lsblk']['status'], 'UNAVAILABLE')
        self.assertEqual(o['lspci']['status'], 'UNAVAILABLE')
        self.assertEqual(o['input_devices']['value'][0]['transport']['status'], 'UNKNOWN')
        self.assertEqual(o['direct_cpuid']['status'], 'UNAVAILABLE')

    def test_fingerprint_stable_under_numbering_and_sensitive_to_board(self):
        first = inventory.normalize(self.raw)['fingerprint']
        self.raw['disks'].reverse()
        self.raw['os'] = 'another OS version'
        self.assertEqual(first['sha256'], inventory.normalize(self.raw)['fingerprint']['sha256'])
        self.assertEqual(first['sha256'], hashlib.sha256(first['preimage'].encode()).hexdigest())
        self.raw['board']['board_serial'] = 'BOARD-B'
        self.assertNotEqual(first['sha256'], inventory.normalize(self.raw)['fingerprint']['sha256'])

    def test_unknown_graph_and_incomplete_identity_fail_closed(self):
        self.raw['disks'] = []
        self.raw['board'] = {}
        r = inventory.normalize(self.raw)
        self.assertFalse(r['fingerprint']['complete'])
        self.assertFalse(r['observations']['boot_system_disk']['value'][0]['complete'])
        self.assertEqual(r['acceptance']['disk_write'], 'NO WRITE')

    def test_disk_identity_change_never_grants_write(self):
        self.raw['disks'][0]['serial'] = 'REPLACEMENT'
        r = inventory.normalize(self.raw)
        self.assertEqual(r['observations']['disks_and_partitions']['value'][1]['serial']['value'], 'REPLACEMENT')
        self.assertEqual(r['acceptance']['dedicated_test_disk'], 'UNSELECTED')

    def test_empty_receipt_has_no_invented_devices(self):
        r = inventory.normalize({})
        self.assertEqual(r['observations']['pci']['status'], 'UNKNOWN')
        self.assertFalse(r['fingerprint']['complete'])

    def test_command_missing_permission_and_timeout_are_unavailable(self):
        with mock.patch.object(inventory.shutil, 'which', return_value=None):
            self.assertEqual(inventory.command(['lspci'])['status'], 'UNAVAILABLE')
        with mock.patch.object(inventory.shutil, 'which', return_value='/usr/bin/lspci'):
            with mock.patch.object(inventory.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, '', 'permission denied')):
                self.assertEqual(inventory.command(['lspci'])['status'], 'UNAVAILABLE')
            with mock.patch.object(inventory.subprocess, 'run', side_effect=subprocess.TimeoutExpired('lspci', 20)):
                self.assertEqual(inventory.command(['lspci'])['status'], 'UNAVAILABLE')

    def test_partition_parent_and_unknown_slave_are_explicit(self):
        self.raw['disks'].append({'name':'sda1', 'major_minor':'8:1', 'partition_number':'1',
                                  'sysfs_path':'/sys/devices/block/sda/sda1', 'slaves':[]})
        self.raw['mountinfo'] = '25 0 8:1 / /boot/efi rw - vfat /dev/sda1 rw'
        self.assertEqual(inventory.mounted_roles(self.raw)[0]['backing_devices'], ['sda'])
        self.raw['disks'][-1]['sysfs_path'] = '/sys/devices/block/missing/sda1'
        self.assertFalse(inventory.mounted_roles(self.raw)[0]['complete'])


class WindowsTests(unittest.TestCase):
    def collect(self, raw=None):
        shell = shutil.which('powershell.exe') or shutil.which('pwsh')
        if not shell:
            self.skipTest('PowerShell unavailable; Windows fixture execution still required before delivery')
        with tempfile.TemporaryDirectory(prefix='dbyte-inventory-test-') as directory:
            path = ROOT/'tests/machine_inventory/windows.json'
            if raw is not None:
                path = Path(directory)/'observations.json'
                path.write_text(json.dumps(raw), encoding='utf-8')
            run = subprocess.run([shell, '-NoProfile', '-File', str(ROOT/'collect_machine_inventory.ps1'),
                                  '-FixturePath', str(path)], capture_output=True, text=True, timeout=30)
        self.assertEqual(run.returncode, 0, run.stderr)
        return json.loads(run.stdout)

    def test_powershell_saved_os_observations(self):
        r = self.collect()
        self.assertTrue(r['fixture'])
        self.assertEqual(r['observations']['firmware']['current_boot_mode']['value'], 'LEGACY')
        self.assertIsNone(r['observations']['firmware']['bios_only']['value'])
        self.assertEqual(r['observations']['storage_controllers']['value'][0]['class_code']['value'], '010601')
        self.assertEqual(r['observations']['input_devices']['value'][0]['transport']['status'], 'UNKNOWN')
        self.assertTrue(r['observations']['storage_disk_flags']['value'][0]['IsSystem']['value'])
        self.assertEqual(r['acceptance']['disk_write'], 'NO WRITE')
        self.assertEqual(r['fingerprint']['sha256'], hashlib.sha256(r['fingerprint']['preimage'].encode()).hexdigest())

    def test_empty_observations_and_changed_identity(self):
        empty = self.collect({})
        self.assertFalse(empty['fingerprint']['complete'])
        self.assertEqual(empty['observations']['pci']['status'], 'UNKNOWN')
        raw = json.loads((ROOT/'tests/machine_inventory/windows.json').read_text())
        original = self.collect(raw)
        raw['board'][0]['SerialNumber'] = 'BOARD-B'
        changed = self.collect(raw)
        self.assertNotEqual(original['fingerprint']['sha256'], changed['fingerprint']['sha256'])
        self.assertEqual(changed['acceptance']['disk_write'], 'NO WRITE')

    def test_usb_parent_and_uefi_observed(self):
        raw = json.loads((ROOT/'tests/machine_inventory/windows.json').read_text())
        raw['firmware'] = 2
        raw['pnp_parents'] = [{'child':'HID\\fixture','parent':'USB\\fixture'}]
        r = self.collect(raw)
        self.assertEqual(r['observations']['input_devices']['value'][0]['transport']['value'], 'USB')
        self.assertTrue(r['observations']['firmware']['uefi_capable']['value'])

    def test_i8042_binding_not_hid_name(self):
        raw = json.loads((ROOT/'tests/machine_inventory/windows.json').read_text())
        raw['pnp'].append({'PNPDeviceID':'HID\\fixture','Service':'i8042prt'})
        r = self.collect(raw)
        self.assertEqual(r['observations']['input_devices']['value'][0]['transport']['value'], 'PS/2')


if __name__ == '__main__':
    unittest.main()
