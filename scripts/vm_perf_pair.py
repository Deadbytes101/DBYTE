#!/usr/bin/env python3
from __future__ import annotations

import argparse
import ctypes
import json
import os
import statistics
import subprocess
import sys
import tempfile

HIGH_PRIORITY_CLASS = 0x00000080


def measurement_order(index: int) -> tuple[str, str]:
    if index % 2 == 0:
        return ("baseline", "candidate")
    return ("candidate", "baseline")


def sample_stats(samples: list[int]) -> dict[str, object]:
    ordered = sorted(samples)
    median = int(statistics.median(ordered))
    deviations = sorted(abs(value - median) for value in ordered)
    mad = int(statistics.median(deviations))
    return {
        "median_ns": median,
        "min_ns": ordered[0],
        "max_ns": ordered[-1],
        "mad_ns": mad,
        "relative_mad": (mad / median) if median else 0.0,
        "samples_ns": ordered,
    }


if os.name == "nt":
    import msvcrt
    from ctypes import wintypes

    CREATE_SUSPENDED = 0x00000004
    STARTF_USESTDHANDLES = 0x00000100
    INFINITE = 0xFFFFFFFF
    WAIT_FAILED = 0xFFFFFFFF

    class STARTUPINFO(ctypes.Structure):
        _fields_ = [
            ("cb", wintypes.DWORD), ("lpReserved", wintypes.LPWSTR),
            ("lpDesktop", wintypes.LPWSTR), ("lpTitle", wintypes.LPWSTR),
            ("dwX", wintypes.DWORD), ("dwY", wintypes.DWORD),
            ("dwXSize", wintypes.DWORD), ("dwYSize", wintypes.DWORD),
            ("dwXCountChars", wintypes.DWORD), ("dwYCountChars", wintypes.DWORD),
            ("dwFillAttribute", wintypes.DWORD), ("dwFlags", wintypes.DWORD),
            ("wShowWindow", wintypes.WORD), ("cbReserved2", wintypes.WORD),
            ("lpReserved2", ctypes.POINTER(ctypes.c_ubyte)),
            ("hStdInput", wintypes.HANDLE), ("hStdOutput", wintypes.HANDLE),
            ("hStdError", wintypes.HANDLE),
        ]

    class PROCESS_INFORMATION(ctypes.Structure):
        _fields_ = [
            ("hProcess", wintypes.HANDLE), ("hThread", wintypes.HANDLE),
            ("dwProcessId", wintypes.DWORD), ("dwThreadId", wintypes.DWORD),
        ]

    KERNEL32 = ctypes.WinDLL("kernel32", use_last_error=True)
    KERNEL32.CreateProcessW.argtypes = [
        wintypes.LPCWSTR, wintypes.LPWSTR, ctypes.c_void_p, ctypes.c_void_p,
        wintypes.BOOL, wintypes.DWORD, ctypes.c_void_p, wintypes.LPCWSTR,
        ctypes.POINTER(STARTUPINFO), ctypes.POINTER(PROCESS_INFORMATION),
    ]
    KERNEL32.CreateProcessW.restype = wintypes.BOOL
    KERNEL32.SetProcessAffinityMask.argtypes = [wintypes.HANDLE, ctypes.c_size_t]
    KERNEL32.SetProcessAffinityMask.restype = wintypes.BOOL
    KERNEL32.GetPriorityClass.argtypes = [wintypes.HANDLE]
    KERNEL32.GetPriorityClass.restype = wintypes.DWORD
    KERNEL32.ResumeThread.argtypes = [wintypes.HANDLE]
    KERNEL32.ResumeThread.restype = wintypes.DWORD
    KERNEL32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    KERNEL32.WaitForSingleObject.restype = wintypes.DWORD
    KERNEL32.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    KERNEL32.GetExitCodeProcess.restype = wintypes.BOOL
    KERNEL32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
    KERNEL32.TerminateProcess.restype = wintypes.BOOL
    KERNEL32.CloseHandle.argtypes = [wintypes.HANDLE]
    KERNEL32.CloseHandle.restype = wintypes.BOOL


def _windows_handle(file_obj):
    handle = msvcrt.get_osfhandle(file_obj.fileno())
    os.set_handle_inheritable(handle, True)
    return handle


def _run_windows(command: list[str], cpu: int) -> tuple[bytes, bytes, int]:
    stdin_file = open(os.devnull, "rb")
    stdout_file = tempfile.TemporaryFile()
    stderr_file = tempfile.TemporaryFile()
    handles = [stdin_file, stdout_file, stderr_file]
    pi = PROCESS_INFORMATION()
    try:
        si = STARTUPINFO()
        si.cb = ctypes.sizeof(si)
        si.dwFlags = STARTF_USESTDHANDLES
        si.hStdInput = _windows_handle(stdin_file)
        si.hStdOutput = _windows_handle(stdout_file)
        si.hStdError = _windows_handle(stderr_file)
        command_line = ctypes.create_unicode_buffer(subprocess.list2cmdline(command))
        flags = CREATE_SUSPENDED | HIGH_PRIORITY_CLASS
        if not KERNEL32.CreateProcessW(
            command[0], command_line, None, None, True, flags, None, None,
            ctypes.byref(si), ctypes.byref(pi),
        ):
            raise OSError(ctypes.get_last_error(), "CreateProcessW failed")
        for file_obj in handles:
            os.set_handle_inheritable(msvcrt.get_osfhandle(file_obj.fileno()), False)
        try:
            if not KERNEL32.SetProcessAffinityMask(pi.hProcess, 1 << cpu):
                raise OSError(ctypes.get_last_error(), "SetProcessAffinityMask failed")
            priority = KERNEL32.GetPriorityClass(pi.hProcess)
            if priority != HIGH_PRIORITY_CLASS:
                raise RuntimeError(f"unexpected Windows priority class: 0x{priority:08x}")
            if KERNEL32.ResumeThread(pi.hThread) == 0xFFFFFFFF:
                raise OSError(ctypes.get_last_error(), "ResumeThread failed")
            if KERNEL32.WaitForSingleObject(pi.hProcess, INFINITE) == WAIT_FAILED:
                raise OSError(ctypes.get_last_error(), "WaitForSingleObject failed")
            code = wintypes.DWORD()
            if not KERNEL32.GetExitCodeProcess(pi.hProcess, ctypes.byref(code)):
                raise OSError(ctypes.get_last_error(), "GetExitCodeProcess failed")
            stdout_file.seek(0)
            stderr_file.seek(0)
            return stdout_file.read(), stderr_file.read(), int(code.value)
        except Exception:
            if pi.hProcess:
                KERNEL32.TerminateProcess(pi.hProcess, 1)
                KERNEL32.WaitForSingleObject(pi.hProcess, INFINITE)
            raise
        finally:
            if pi.hThread:
                KERNEL32.CloseHandle(pi.hThread)
            if pi.hProcess:
                KERNEL32.CloseHandle(pi.hProcess)
    finally:
        for file_obj in handles:
            file_obj.close()


def run_command(command: list[str], cpu: int) -> tuple[bytes, bytes, int]:
    if cpu < 0 or cpu >= ctypes.sizeof(ctypes.c_size_t) * 8:
        raise ValueError(f"CPU affinity index out of range: {cpu}")
    if os.name == "nt":
        return _run_windows(command, cpu)
    process = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return process.stdout, process.stderr, process.returncode


def invoke_runner(
    runner: str, source: str, iterations: int, cpu: int
) -> dict[str, object]:
    command = [
        runner,
        source,
        "--warmups", "0",
        "--measurements", "1",
        "--iterations", str(iterations),
    ]
    stdout, stderr, exit_code = run_command(command, cpu)
    if exit_code != 0:
        raise RuntimeError(
            f"runner exit {exit_code}: {runner}; stderr={stderr[:256]!r}"
        )
    try:
        receipt = json.loads(stdout.decode("utf-8"))
    except Exception as error:
        raise RuntimeError(f"invalid runner JSON from {runner}: {error}") from error
    if receipt.get("warmups") != 0 or receipt.get("measurements") != 1:
        raise RuntimeError("runner one-sample contract mismatch")
    if receipt.get("iterations") != iterations:
        raise RuntimeError("runner iteration contract mismatch")
    return receipt


def verify_receipt(
    receipt: dict[str, object], canonical: dict[str, str] | None
) -> dict[str, str]:
    current = {
        "source_sha256": str(receipt.get("source_sha256", "")),
        "output_sha256": str(receipt.get("output_sha256", "")),
    }
    if not current["source_sha256"] or not current["output_sha256"]:
        raise RuntimeError("runner receipt lacks source/output SHA-256")
    if canonical is None:
        return current
    if current != canonical:
        raise RuntimeError(f"runner observable divergence: {current} != {canonical}")
    return canonical


def aggregate_receipt(
    stats: dict[str, object], canonical: dict[str, str],
    warmups: int, measurements: int, iterations: int
) -> dict[str, object]:
    return {
        **canonical,
        "warmups": warmups,
        "measurements": measurements,
        "iterations": iterations,
        **stats,
    }


def measure_pair(args: argparse.Namespace) -> dict[str, object]:
    canonical = None
    attempts = []
    for attempt in range(1, args.attempts + 1):
        for index in range(args.warmups):
            for label in measurement_order(index):
                runner = args.baseline if label == "baseline" else args.candidate
                receipt = invoke_runner(runner, args.source, args.iterations, args.cpu_affinity)
                canonical = verify_receipt(receipt, canonical)

        baseline_samples: list[int] = []
        candidate_samples: list[int] = []
        for index in range(args.measurements):
            for label in measurement_order(index):
                runner = args.baseline if label == "baseline" else args.candidate
                receipt = invoke_runner(runner, args.source, args.iterations, args.cpu_affinity)
                canonical = verify_receipt(receipt, canonical)
                sample = int(receipt["median_ns"])
                if label == "baseline":
                    baseline_samples.append(sample)
                else:
                    candidate_samples.append(sample)

        baseline_stats = sample_stats(baseline_samples)
        candidate_stats = sample_stats(candidate_samples)
        stable = (
            baseline_stats["relative_mad"] <= args.maximum_relative_mad
            and candidate_stats["relative_mad"] <= args.maximum_relative_mad
        )
        baseline = aggregate_receipt(
            baseline_stats, canonical, args.warmups, args.measurements, args.iterations
        )
        candidate = aggregate_receipt(
            candidate_stats, canonical, args.warmups, args.measurements, args.iterations
        )
        record = {
            "attempt": attempt,
            "stable": stable,
            "baseline": baseline,
            "candidate": candidate,
            "ratio": baseline_stats["median_ns"] / candidate_stats["median_ns"],
        }
        attempts.append(record)
        if stable:
            return {
                "stable": True,
                "selected_attempt": attempt,
                "cpu_affinity": args.cpu_affinity,
                "priority_class": "high" if os.name == "nt" else "default",
                "launch_control": (
                    "suspended_affinity_high_then_resume"
                    if os.name == "nt" else "direct"
                ),
                "sample_order": "alternating",
                "source_sha256": canonical["source_sha256"],
                "output_sha256": canonical["output_sha256"],
                "attempts": attempts,
            }

    return {
        "stable": False,
        "selected_attempt": None,
        "cpu_affinity": args.cpu_affinity,
        "priority_class": "high" if os.name == "nt" else "default",
        "launch_control": (
            "suspended_affinity_high_then_resume"
            if os.name == "nt" else "direct"
        ),
        "sample_order": "alternating",
        "source_sha256": canonical["source_sha256"] if canonical else None,
        "output_sha256": canonical["output_sha256"] if canonical else None,
        "attempts": attempts,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("baseline")
    parser.add_argument("candidate")
    parser.add_argument("source")
    parser.add_argument("--iterations", type=int, required=True)
    parser.add_argument("--warmups", type=int, required=True)
    parser.add_argument("--measurements", type=int, required=True)
    parser.add_argument("--attempts", type=int, required=True)
    parser.add_argument("--maximum-relative-mad", type=float, required=True)
    parser.add_argument("--cpu-affinity", type=int, required=True)
    args = parser.parse_args()
    if args.iterations < 1 or args.measurements < 1 or args.attempts < 1:
        parser.error("iterations, measurements and attempts must be positive")
    if args.warmups < 0 or args.cpu_affinity < 0:
        parser.error("warmups and CPU affinity must be non-negative")
    return args


def main() -> int:
    result = measure_pair(parse_args())
    print(json.dumps(result, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(json.dumps({"stable": False, "error": str(error)}, separators=(",", ":")))
        raise SystemExit(2)
