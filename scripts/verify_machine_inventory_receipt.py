"""Validate a DByte machine-inventory receipt without granting hardware authority.

Exit 0: structurally valid and ready for target-selection review.
Exit 1: malformed, tampered, or acceptance state is unsafe.
Exit 2: structurally valid but incomplete for target-selection review.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path
import sys

sys.dont_write_bytecode = True

EXPECTED_SCHEMA = "dbyte-machine-inventory-v1"
SAFE_ACCEPTANCE = {
    "primary_real_hw_target": "UNSELECTED",
    "storage_driver": "UNSELECTED",
    "input_driver": "UNSELECTED",
    "dedicated_test_disk": "UNSELECTED",
    "disk_write": "NO WRITE",
    "real_hardware": "PENDING",
    "full_os": "PENDING",
}


IDENTITY_CORE_FIELDS = (
    "board_model",
    "board_vendor",
    "cpu_model",
    "cpu_vendor",
)
IDENTITY_DISCRIMINATOR_FIELDS = ("board_serial", "system_uuid")
IDENTITY_FIELDS = IDENTITY_CORE_FIELDS + IDENTITY_DISCRIMINATOR_FIELDS
IDENTITY_PLACEHOLDER_PREFIXES = (
    "unknown",
    "none",
    "default",
    "to be filled",
)


def _fact_value(value):
    if isinstance(value, dict) and "value" in value:
        return value.get("value")
    return None


def _fact_has_observation(value):
    if not isinstance(value, dict):
        return False
    if value.get("status") != "OBSERVED":
        return False
    observed = value.get("value")
    if observed is None:
        return False
    if isinstance(observed, (list, dict, str)) and not observed:
        return False
    return True


def _record_value(record, name):
    if not isinstance(record, dict):
        return None
    value = record.get(name)
    if isinstance(value, dict) and "value" in value:
        if value.get("status") != "OBSERVED":
            return None
        return value.get("value")
    return value


def _fact_records(value):
    if not _fact_has_observation(value):
        return []
    observed = value.get("value")
    return observed if isinstance(observed, list) else []


def _meaningful_text(value):
    if not isinstance(value, str):
        return False
    normalized = value.strip()
    if not normalized:
        return False
    return not normalized.lower().startswith(IDENTITY_PLACEHOLDER_PREFIXES)


def _identity_value_is_meaningful(value):
    if not isinstance(value, str) or not value.strip():
        return False
    normalized = value.strip().lower()
    if normalized.startswith(IDENTITY_PLACEHOLDER_PREFIXES):
        return False
    compact = normalized.replace("-", "")
    return not (compact and set(compact) in ({"0"}, {"f"}))


def _identity_is_complete(identity):
    if not isinstance(identity, dict):
        return False
    if any(field not in identity for field in IDENTITY_FIELDS):
        return False
    if not all(_identity_value_is_meaningful(identity.get(field)) for field in IDENTITY_CORE_FIELDS):
        return False
    return any(
        _identity_value_is_meaningful(identity.get(field))
        for field in IDENTITY_DISCRIMINATOR_FIELDS
    )


def _storage_controller_observed(value):
    for record in _fact_records(value):
        class_code = _record_value(record, "class_code")
        if not isinstance(class_code, str):
            continue
        normalized = class_code.strip().lower()
        if normalized.startswith("0x"):
            normalized = normalized[2:]
        if re.fullmatch(r"[0-9a-f]{6}", normalized) and normalized.startswith("01"):
            return True
    return False


def _input_transport_observed(value):
    for record in _fact_records(value):
        transport = _record_value(record, "transport")
        if isinstance(transport, str) and transport.strip().upper() in {"USB", "PS/2"}:
            return True
    return False


def _disk_identity_observed(value):
    for record in _fact_records(value):
        for field in ("DeviceID", "device_path", "name"):
            if _meaningful_text(_record_value(record, field)):
                return True
    return False


def _long_mode(observations):
    direct = observations.get("long_mode")
    if isinstance(direct, dict):
        return direct.get("value") if direct.get("status") == "OBSERVED" else None
    cpu = observations.get("cpu")
    if isinstance(cpu, dict):
        nested = cpu.get("long_mode")
        if isinstance(nested, dict):
            return nested.get("value") if nested.get("status") == "OBSERVED" else None
    return None


def _disk_fact(observations):
    for name in ("disks", "disks_and_partitions"):
        if name in observations:
            return observations[name]
    return None


def validate_receipt(receipt):
    errors = []
    blockers = []
    if not isinstance(receipt, dict):
        return {
            "status": "FAIL", "selection_ready": False, "write_authorized": False,
            "errors": ["RECEIPT_NOT_OBJECT"], "blockers": [],
        }
    if receipt.get("schema") != EXPECTED_SCHEMA:
        errors.append("SCHEMA_MISMATCH")
    if receipt.get("collector") not in {"windows", "linux"}:
        errors.append("COLLECTOR_UNSUPPORTED")

    fingerprint = receipt.get("fingerprint")
    if not isinstance(fingerprint, dict):
        errors.append("FINGERPRINT_MISSING")
        fingerprint = {}
    preimage = fingerprint.get("preimage")
    supplied_hash = fingerprint.get("sha256")
    identity = fingerprint.get("identity")
    if not isinstance(preimage, str) or not isinstance(supplied_hash, str):
        errors.append("FINGERPRINT_FIELDS_MISSING")
    else:
        recomputed = hashlib.sha256(preimage.encode("utf-8")).hexdigest()
        if supplied_hash.lower() != recomputed:
            errors.append("FINGERPRINT_SHA256_MISMATCH")
        try:
            preimage_identity = json.loads(preimage)
        except json.JSONDecodeError:
            errors.append("FINGERPRINT_PREIMAGE_INVALID")
        else:
            if not isinstance(preimage_identity, dict) or preimage_identity != identity:
                errors.append("FINGERPRINT_IDENTITY_MISMATCH")
    if not isinstance(identity, dict):
        errors.append("FINGERPRINT_IDENTITY_MISSING")
    elif fingerprint.get("complete") is True and not _identity_is_complete(identity):
        errors.append("FINGERPRINT_COMPLETE_INCONSISTENT")
    if fingerprint.get("complete") is not True:
        blockers.append("INCOMPLETE_FINGERPRINT")

    acceptance = receipt.get("acceptance")
    if not isinstance(acceptance, dict):
        errors.append("ACCEPTANCE_MISSING")
        acceptance = {}
    for field, expected in SAFE_ACCEPTANCE.items():
        if acceptance.get(field) != expected:
            errors.append(f"UNSAFE_ACCEPTANCE_{field}")

    if receipt.get("fixture") is not False:
        blockers.append("FIXTURE_RECEIPT")

    observations = receipt.get("observations")
    if not isinstance(observations, dict):
        errors.append("OBSERVATIONS_MISSING")
        observations = {}
    if _long_mode(observations) is not True:
        blockers.append("LONG_MODE_NOT_OBSERVED")
    topology = {
        "PCI_TOPOLOGY_UNAVAILABLE": observations.get("pci"),
        "STORAGE_CONTROLLER_UNAVAILABLE": observations.get("storage_controllers"),
        "INPUT_DEVICE_UNAVAILABLE": observations.get("input_devices"),
        "DISK_INVENTORY_UNAVAILABLE": _disk_fact(observations),
    }
    for code, value in topology.items():
        if not _fact_has_observation(value):
            blockers.append(code)
    if _fact_has_observation(observations.get("storage_controllers")) and not _storage_controller_observed(
        observations.get("storage_controllers")
    ):
        blockers.append("STORAGE_CONTROLLER_IDENTITY_UNAVAILABLE")
    if _fact_has_observation(observations.get("input_devices")) and not _input_transport_observed(
        observations.get("input_devices")
    ):
        blockers.append("INPUT_TRANSPORT_UNAVAILABLE")
    disk_fact = _disk_fact(observations)
    if _fact_has_observation(disk_fact) and not _disk_identity_observed(disk_fact):
        blockers.append("DISK_IDENTITY_UNAVAILABLE")

    errors = sorted(set(errors))
    blockers = sorted(set(blockers))
    if errors:
        status = "FAIL"
    elif blockers:
        status = "INCOMPLETE"
    else:
        status = "PASS"
    return {
        "status": status,
        "schema": receipt.get("schema"),
        "collector": receipt.get("collector"),
        "fingerprint_sha256": fingerprint.get("sha256"),
        "identity_complete": fingerprint.get("complete") is True,
        "fixture": receipt.get("fixture"),
        "long_mode_observed": _long_mode(observations) is True,
        "selection_ready": status == "PASS",
        "write_authorized": False,
        "acceptance_preserved": not any(
            error.startswith("UNSAFE_ACCEPTANCE_") for error in errors
        ),
        "errors": errors,
        "blockers": blockers,
    }


def _decode_receipt(data):
    if data.startswith((b"\xff\xfe", b"\xfe\xff")):
        return data.decode("utf-16")
    return data.decode("utf-8-sig")

def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("receipt", type=Path, help="unmodified machine inventory receipt JSON")
    args = parser.parse_args(argv)
    try:
        data = args.receipt.read_bytes()
        receipt = json.loads(_decode_receipt(data))
        report = validate_receipt(receipt)
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        report = {
            "status": "FAIL",
            "selection_ready": False,
            "write_authorized": False,
            "errors": [f"RECEIPT_READ_ERROR:{type(exc).__name__}"],
            "blockers": [],
        }
        data = b""
    report["receipt_sha256"] = hashlib.sha256(data).hexdigest()
    print(json.dumps(report, sort_keys=True, indent=2))
    if report["status"] == "PASS":
        return 0
    if report["status"] == "INCOMPLETE":
        return 2
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
