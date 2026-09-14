"""Focused package-boundary tests for the v13.2 Linux release candidate."""

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
SCRIPTS = Path(__file__).resolve().parent
ROOT = SCRIPTS.parent
SPEC = importlib.util.spec_from_file_location(
    "package_release_linux", SCRIPTS / "package_release_linux.py"
)
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)


class PackageBoundaryTests(unittest.TestCase):
    def test_protected_user_paths_and_generated_trees_are_excluded(self):
        paths = package.tracked_package_paths(ROOT)
        self.assertTrue(package.PROTECTED_WORKTREE_PATHS.isdisjoint(paths))
        self.assertFalse(any("tmp" in Path(path).parts for path in paths))

    def test_unsafe_member_paths_fail_closed(self):
        for path in ("../escape", "/absolute", "C:/drive", "back\\slash"):
            with self.subTest(path=path), self.assertRaises(package.PackageError):
                package.validate_path(path)

    def test_linux_release_docs_are_current(self):
        package.validate_release_docs(ROOT)

    def test_linux_install_guide_is_packaged(self):
        paths = package.tracked_package_paths(ROOT)
        self.assertIn("INSTALL.md", paths)

    def test_legacy_linux_install_guide_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "README.md").write_text(
                (ROOT / "README.md").read_text(encoding="utf-8"),
                encoding="utf-8",
            )
            install = (ROOT / "INSTALL.md").read_text(encoding="utf-8")
            (root / "INSTALL.md").write_text(
                install + "\ndbyte-v11.6.0-windows-x64.zip\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(package.PackageError, "forbidden"):
                package.validate_release_docs(root)

if __name__ == "__main__":
    unittest.main()
