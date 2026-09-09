"""Tests for read-only machine inventory receipt validation."""
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "inventory_receipt", ROOT / "verify_machine_inventory_receipt.py"
)
receipt_verifier = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(receipt_verifier)


def fact(value, status="OBSERVED"):
    return {"status": status, "value": value, "source": "test"}


def base_receipt():
    identity = {
        "board_model": "TEST-BOARD",
        "board_serial": "BOARD-1",
        "board_vendor": "TEST",
        "cpu_model": "TEST64",
        "cpu_vendor": "GenuineIntel",
        "system_uuid": "11111111-2222-3333-4444-555555555555",
    }
    preimage = json.dumps(identity, sort_keys=True, separators=(",", ":"))
    return {
        "schema": "dbyte-machine-inventory-v1",
        "collector": "windows",
        "fixture": False,
        "collected_utc": "2026-09-09T00:00:00+00:00",
        "observations": {
            "long_mode": fact(True),
            "pci": fact([{"id": fact("PCI\\VEN_1234&DEV_5678")}]),
            "storage_controllers": fact([{"class_code": fact("010601")}]),
            "input_devices": fact([{"transport": fact("PS/2")}]),
            "disks": fact([{"DeviceID": fact("PHYSICALDRIVE1")}]),
        },
        "fingerprint": {
            "sha256": hashlib.sha256(preimage.encode()).hexdigest(),
            "preimage": preimage,
            "complete": True,
            "identity": identity,
            "policy": "comparison aid only; never disk-write authorization",
        },
        "acceptance": {
            "primary_real_hw_target": "UNSELECTED",
            "storage_driver": "UNSELECTED",
            "input_driver": "UNSELECTED",
            "dedicated_test_disk": "UNSELECTED",
            "disk_write": "NO WRITE",
            "real_hardware": "PENDING",
            "full_os": "PENDING",
        },
    }


class ReceiptTests(unittest.TestCase):
    def test_real_complete_receipt_is_selection_ready(self):
        result = receipt_verifier.validate_receipt(base_receipt())
        self.assertEqual(result["status"], "PASS")
        self.assertTrue(result["selection_ready"])
        self.assertFalse(result["write_authorized"])
    def test_fixture_never_becomes_selection_ready(self):
        receipt = base_receipt()
        receipt["fixture"] = True
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "INCOMPLETE")
        self.assertIn("FIXTURE_RECEIPT", result["blockers"])

    def test_fingerprint_mismatch_fails(self):
        receipt = base_receipt()
        receipt["fingerprint"]["sha256"] = "0" * 64
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "FAIL")
        self.assertIn("FINGERPRINT_SHA256_MISMATCH", result["errors"])

    def test_identity_must_match_hashed_preimage(self):
        receipt = base_receipt()
        receipt["fingerprint"]["identity"]["board_model"] = "TAMPERED"
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "FAIL")
        self.assertIn("FINGERPRINT_IDENTITY_MISMATCH", result["errors"])

    def test_complete_flag_cannot_override_incomplete_identity(self):
        receipt = base_receipt()
        receipt["fingerprint"]["identity"]["board_serial"] = "Unknown"
        receipt["fingerprint"]["identity"]["system_uuid"] = None
        receipt["fingerprint"]["preimage"] = json.dumps(
            receipt["fingerprint"]["identity"], sort_keys=True, separators=(",", ":")
        )
        receipt["fingerprint"]["sha256"] = hashlib.sha256(
            receipt["fingerprint"]["preimage"].encode()
        ).hexdigest()
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "FAIL")
        self.assertIn("FINGERPRINT_COMPLETE_INCONSISTENT", result["errors"])

    def test_legacy_board_without_serial_can_use_system_uuid(self):
        receipt = base_receipt()
        receipt["fingerprint"]["identity"]["board_serial"] = None
        receipt["fingerprint"]["preimage"] = json.dumps(
            receipt["fingerprint"]["identity"], sort_keys=True, separators=(",", ":")
        )
        receipt["fingerprint"]["sha256"] = hashlib.sha256(
            receipt["fingerprint"]["preimage"].encode()
        ).hexdigest()
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "PASS")
        self.assertTrue(result["selection_ready"])
        self.assertFalse(result["write_authorized"])

    def test_acceptance_escalation_fails_closed(self):
        receipt = base_receipt()
        receipt["acceptance"]["disk_write"] = "AUTHORIZED"
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "FAIL")
        self.assertFalse(result["write_authorized"])
        self.assertIn("UNSAFE_ACCEPTANCE_disk_write", result["errors"])

    def test_unknown_long_mode_is_incomplete(self):
        receipt = base_receipt()
        receipt["observations"]["long_mode"] = fact(None, "UNKNOWN")
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "INCOMPLETE")
        self.assertIn("LONG_MODE_NOT_OBSERVED", result["blockers"])

    def test_unknown_input_transport_is_incomplete(self):
        receipt = base_receipt()
        receipt["observations"]["input_devices"] = fact(
            [{"transport": fact(None, "UNKNOWN")}]
        )
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "INCOMPLETE")
        self.assertIn("INPUT_TRANSPORT_UNAVAILABLE", result["blockers"])

    def test_placeholder_input_transport_is_incomplete(self):
        receipt = base_receipt()
        receipt["observations"]["input_devices"] = fact(
            [{"transport": fact("UNKNOWN")}]
        )
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "INCOMPLETE")
        self.assertIn("INPUT_TRANSPORT_UNAVAILABLE", result["blockers"])

    def test_unrecognized_input_transport_is_incomplete(self):
        receipt = base_receipt()
        receipt["observations"]["input_devices"] = fact(
            [{"transport": fact("BLUETOOTH")}]
        )
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "INCOMPLETE")
        self.assertIn("INPUT_TRANSPORT_UNAVAILABLE", result["blockers"])

    def test_storage_controller_requires_storage_class(self):
        receipt = base_receipt()
        receipt["observations"]["storage_controllers"] = fact(
            [{"class_code": fact("020000")}]
        )
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "INCOMPLETE")
        self.assertIn("STORAGE_CONTROLLER_IDENTITY_UNAVAILABLE", result["blockers"])

    def test_storage_controller_requires_full_pci_class_code(self):
        receipt = base_receipt()
        receipt["observations"]["storage_controllers"] = fact(
            [{"class_code": fact("01")}]
        )
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "INCOMPLETE")
        self.assertIn("STORAGE_CONTROLLER_IDENTITY_UNAVAILABLE", result["blockers"])

    def test_storage_controller_rejects_non_hex_class_code(self):
        receipt = base_receipt()
        receipt["observations"]["storage_controllers"] = fact(
            [{"class_code": fact("01ZZZZ")}]
        )
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "INCOMPLETE")
        self.assertIn("STORAGE_CONTROLLER_IDENTITY_UNAVAILABLE", result["blockers"])

    def test_disk_inventory_requires_device_identity(self):
        receipt = base_receipt()
        receipt["observations"]["disks"] = fact([{}])
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "INCOMPLETE")
        self.assertIn("DISK_IDENTITY_UNAVAILABLE", result["blockers"])

    def test_disk_inventory_rejects_placeholder_identity(self):
        receipt = base_receipt()
        receipt["observations"]["disks"] = fact(
            [{"DeviceID": fact("Unknown")}]
        )
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "INCOMPLETE")
        self.assertIn("DISK_IDENTITY_UNAVAILABLE", result["blockers"])

    def test_linux_long_mode_shape_is_accepted(self):
        receipt = base_receipt()
        receipt["collector"] = "linux"
        del receipt["observations"]["long_mode"]
        receipt["observations"]["cpu"] = {"long_mode": fact(True)}
        result = receipt_verifier.validate_receipt(receipt)
        self.assertEqual(result["status"], "PASS")
    def test_cli_reports_hash_only_summary(self):
        with tempfile.TemporaryDirectory(prefix="dbyte-receipt-") as directory:
            path = Path(directory) / "receipt.json"
            path.write_text(json.dumps(base_receipt()), encoding="utf-8")
            run = subprocess.run(
                [sys.executable, "-B", str(ROOT / "verify_machine_inventory_receipt.py"), str(path)],
                capture_output=True,
                text=True,
                timeout=20,
            )
        self.assertEqual(run.returncode, 0, run.stderr)
        report = json.loads(run.stdout)
        self.assertEqual(report["status"], "PASS")
        self.assertEqual(len(report["receipt_sha256"]), 64)
        self.assertNotIn("BOARD-1", run.stdout)


if __name__ == "__main__":
    unittest.main()
