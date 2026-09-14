"""Regression tests for v13.2 Linux package identity handling."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location(
    "package_verify", ROOT / "verify_v13_2_linux_package.py"
)
package_verify = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package_verify)
import package_release_linux
import package_v13_2_source

TAG_OBJECT = "b8dfb2e9c11f5803fc371b0cfb00645527419a12"
TAG_COMMIT = "867bebea05d796855590099d2340de09f2ced4d5"


class PackageIdentityTests(unittest.TestCase):
    def test_final_source_transfer_does_not_require_git_metadata(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload = b"native source transfer\n"
            (root / "payload.txt").write_bytes(payload)
            digest = hashlib.sha256(payload).hexdigest()
            expected = "ab" * 32
            manifest = {
                "schema": "dbyte-v13.2-source-transfer-v1",
                "version": "13.2.0",
                "frozen_v13_1_tag_object": TAG_OBJECT,
                "frozen_v13_1_tag_commit": TAG_COMMIT,
                "candidate_identity": expected,
                "entries": [
                    {"path": "payload.txt", "bytes": len(payload), "sha256": digest}
                ],
            }
            (root / "source-manifest.json").write_text(
                json.dumps(manifest, indent=2, sort_keys=True) + "\n",
                encoding="utf-8",
            )
            (root / "SHA256SUMS").write_bytes(
                f"{digest}  payload.txt\n".encode()
            )
            evidence = root / "evidence"
            identity = package_verify.resolve_candidate_identity(
                root=root,
                evidence=evidence,
                final=True,
                expected_identity=expected,
            )
            self.assertEqual(identity, expected)
            self.assertTrue((evidence / "source-manifest-check.json").is_file())

    def test_release_path_selection_can_use_source_manifest_without_git(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            paths = [
                "README.md",
                "INSTALL.md",
                "LICENSE",
                "runtime/v13.2.0/linux_io.dby",
                "docs/LINUX_NATIVE_HOST_PORTABILITY.md",
                "examples/hello.dby",
            ]
            for relative in paths:
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(relative + "\n", encoding="utf-8")
            manifest = {"entries": [{"path": relative} for relative in paths]}
            manifest_path = root / "source-manifest.json"
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            selected = package_release_linux.tracked_package_paths(
                root, source_manifest=manifest_path
            )
            self.assertIn("README.md", selected)
            self.assertIn("runtime/v13.2.0/linux_io.dby", selected)
            self.assertNotIn("examples/hello.dby", selected)

    def test_source_transfer_untracked_provenance_uses_git_untracked_set_only(self):
        with mock.patch.object(
            package_v13_2_source,
            "run",
            return_value="docs/new.md\nruntime/new.dby\n",
        ) as run:
            paths = package_v13_2_source.untracked_candidate_paths(Path("."))
        self.assertEqual(paths, {"docs/new.md", "runtime/new.dby"})
        run.assert_called_once_with(
            Path("."), "git", "ls-files", "--others", "--exclude-standard"
        )


if __name__ == "__main__":
    unittest.main()
