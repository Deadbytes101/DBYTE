"""Adversarial tests for the v13.2 source-transfer manifest."""

import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
SCRIPTS = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "verify_v13_2_source_manifest", SCRIPTS / "verify_v13_2_source_manifest.py"
)
verifier = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verifier)


class SourceManifestTests(unittest.TestCase):
    def make_fixture(self, root: Path) -> tuple[Path, str]:
        identity = "a" * 64
        data = b"[workspace]\n"
        (root / "Cargo.toml").write_bytes(data)
        entry = {
            "path": "Cargo.toml",
            "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
            "mode": 0o644,
            "provenance": "candidate",
        }
        manifest = {
            "schema": "dbyte-v13.2-source-transfer-v1",
            "version": "13.2.0",
            "candidate_identity": identity,
            "frozen_v13_1_tag_object": verifier.TAG_OBJECT,
            "frozen_v13_1_tag_commit": verifier.TAG_COMMIT,
            "entries": [entry],
            "excluded": [],
        }
        path = root / "source-manifest.json"
        path.write_text(json.dumps(manifest), encoding="utf-8")
        (root / "SHA256SUMS").write_bytes(
            f"{entry['sha256']}  Cargo.toml\n".encode()
        )
        return path, identity

    def test_exact_source_manifest_passes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, identity = self.make_fixture(root)
            result = verifier.verify(root, manifest, identity)
            self.assertEqual(result["status"], "PASS")

    def test_changed_or_extra_source_fails_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, identity = self.make_fixture(root)
            (root / "Cargo.toml").write_text("changed", encoding="utf-8")
            with self.assertRaisesRegex(verifier.ManifestError, "FILE_MISMATCH"):
                verifier.verify(root, manifest, identity)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, identity = self.make_fixture(root)
            (root / "unexpected.txt").write_text("unexpected", encoding="utf-8")
            with self.assertRaisesRegex(verifier.ManifestError, "UNEXPECTED_FILE"):
                verifier.verify(root, manifest, identity)


if __name__ == "__main__":
    unittest.main()
