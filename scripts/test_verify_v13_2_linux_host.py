"""Fail-closed tests for v13.2 physical Linux-host binding."""

import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import unittest

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

HOST_SPEC = importlib.util.spec_from_file_location(
    "verify_v13_2_linux_host", ROOT / "verify_v13_2_linux_host.py"
)
host_verifier = importlib.util.module_from_spec(HOST_SPEC)
HOST_SPEC.loader.exec_module(host_verifier)

RECEIPT_SPEC = importlib.util.spec_from_file_location(
    "test_machine_inventory_receipt", ROOT / "test_machine_inventory_receipt.py"
)
receipt_tests = importlib.util.module_from_spec(RECEIPT_SPEC)
RECEIPT_SPEC.loader.exec_module(receipt_tests)

CANDIDATE_SPEC = importlib.util.spec_from_file_location(
    "v13_2_candidate_identity", ROOT / "v13_2_candidate_identity.py"
)
candidate_identity = importlib.util.module_from_spec(CANDIDATE_SPEC)
CANDIDATE_SPEC.loader.exec_module(candidate_identity)


class PhysicalBindingTests(unittest.TestCase):
    def test_protected_worktree_presence_does_not_change_candidate_identity(self):
        payload = {
            "schema": "dbyte-v13.2-candidate-identity-v1",
            "head": "a" * 40,
            "frozen_v13_1_tag_object": "b" * 40,
            "frozen_v13_1_tag_commit": "c" * 40,
            "included_dirty_inputs": [{"path": "Cargo.toml", "sha256": "d" * 64}],
            "excluded_dirty_inputs": [],
        }
        first = candidate_identity.compute_candidate_identity(payload)
        payload["excluded_dirty_inputs"] = [
            {"path": "examples/smoke.dby", "reason": "user-owned-protected"}
        ]
        self.assertEqual(first, candidate_identity.compute_candidate_identity(payload))

    def test_exact_live_identity_is_accepted_without_authorizing_storage(self):
        accepted = receipt_tests.base_receipt()
        live = copy.deepcopy(accepted)
        live["collected_utc"] = "2026-09-14T00:00:00+00:00"
        expected = accepted["fingerprint"]["sha256"]
        binding = host_verifier.bind_physical_machine(accepted, live, expected)
        self.assertEqual(binding["accepted_fingerprint_sha256"], expected)
        self.assertEqual(binding["live_fingerprint_sha256"], expected)
        self.assertEqual(live["acceptance"]["disk_write"], "NO WRITE")

    def test_different_live_machine_is_rejected(self):
        accepted = receipt_tests.base_receipt()
        live = receipt_tests.base_receipt()
        live["fingerprint"]["identity"]["system_uuid"] = (
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        )
        preimage = json.dumps(
            live["fingerprint"]["identity"], sort_keys=True, separators=(",", ":")
        )
        live["fingerprint"]["preimage"] = preimage
        live["fingerprint"]["sha256"] = hashlib.sha256(preimage.encode()).hexdigest()
        with self.assertRaisesRegex(
            host_verifier.VerificationError,
            "BIOSTAR_LIVE_FINGERPRINT_MISMATCH",
        ):
            host_verifier.bind_physical_machine(
                accepted, live, accepted["fingerprint"]["sha256"]
            )

    def test_fixture_or_environment_marker_cannot_promote_final(self):
        accepted = receipt_tests.base_receipt()
        live = copy.deepcopy(accepted)
        live["fixture"] = True
        with self.assertRaisesRegex(
            host_verifier.VerificationError,
            "BIOSTAR_LIVE_INVENTORY_INVALID",
        ):
            host_verifier.bind_physical_machine(
                accepted, live, accepted["fingerprint"]["sha256"]
            )
        source = (ROOT / "verify_v13_2_linux_host.py").read_text(encoding="utf-8")
        self.assertNotIn("DBYTE_BIOSTAR_HOST", source)


if __name__ == "__main__":
    unittest.main()
