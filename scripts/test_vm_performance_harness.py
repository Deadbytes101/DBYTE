from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
HELPER = ROOT / "scripts" / "vm_perf_pair.py"
CORE = ROOT / "scripts" / "verify_vm_performance.ps1"
CPU6502 = ROOT / "scripts" / "verify_vm_6502.ps1"
MANIFEST = ROOT / "benchmarks" / "vm" / "corpus.json"


def load_helper():
    if not HELPER.is_file():
        raise AssertionError("paired VM performance helper is missing")
    spec = importlib.util.spec_from_file_location("vm_perf_pair", HELPER)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class VmPerformanceHarnessTests(unittest.TestCase):
    def test_verifiers_use_paired_affinity_helper(self):
        for path in (CORE, CPU6502):
            text = path.read_text(encoding="utf-8")
            self.assertIn("vm_perf_pair.py", text, path.name)
            self.assertIn("windows_affinity_cpu", text, path.name)

    def test_manifest_declares_windows_affinity(self):
        manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
        cpu = manifest.get("windows_affinity_cpu")
        self.assertIsInstance(cpu, int)
        self.assertGreaterEqual(cpu, 0)

    def test_pair_order_alternates(self):
        helper = load_helper()
        self.assertEqual(helper.measurement_order(0), ("baseline", "candidate"))
        self.assertEqual(helper.measurement_order(1), ("candidate", "baseline"))

    @unittest.skipUnless(os.name == "nt", "Windows affinity contract")
    def test_windows_launch_is_pinned_and_high_priority(self):
        helper = load_helper()
        code = (
            "import ctypes,json; from ctypes import wintypes; "
            "k=ctypes.WinDLL('kernel32',use_last_error=True); "
            "k.GetCurrentProcess.restype=wintypes.HANDLE; "
            "k.GetProcessAffinityMask.argtypes=[wintypes.HANDLE,ctypes.POINTER(ctypes.c_size_t),ctypes.POINTER(ctypes.c_size_t)]; "
            "k.GetProcessAffinityMask.restype=wintypes.BOOL; "
            "k.GetPriorityClass.argtypes=[wintypes.HANDLE]; k.GetPriorityClass.restype=wintypes.DWORD; "
            "h=k.GetCurrentProcess(); pm=ctypes.c_size_t(); sm=ctypes.c_size_t(); "
            "ok=k.GetProcessAffinityMask(h,ctypes.byref(pm),ctypes.byref(sm)); "
            "print(json.dumps({'ok':bool(ok),'mask':pm.value,'priority':k.GetPriorityClass(h)}))"
        )
        stdout, stderr, exit_code = helper.run_command(
            [sys.executable, "-c", code], cpu=0
        )
        self.assertEqual(exit_code, 0, stderr.decode(errors="replace"))
        payload = json.loads(stdout.decode())
        self.assertEqual(payload["mask"], 1)
        self.assertEqual(payload["priority"], helper.HIGH_PRIORITY_CLASS)


if __name__ == "__main__":
    unittest.main()
