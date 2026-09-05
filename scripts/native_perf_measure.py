#!/usr/bin/env python3
import argparse
import ctypes
import hashlib
import json
import os
import statistics
import subprocess
import sys
import time


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


if os.name == "nt":
    import msvcrt
    import tempfile
    from ctypes import wintypes

    CREATE_SUSPENDED = 0x00000004
    HIGH_PRIORITY_CLASS = 0x00000080
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


def _windows_temp_handle(file_obj):
    handle = msvcrt.get_osfhandle(file_obj.fileno())
    os.set_handle_inheritable(handle, True)
    return handle


def _invoke_windows(path: str, expected_exit: int, cpu: int):
    stdin_file = open(os.devnull, "rb")
    stdout_file = tempfile.TemporaryFile()
    stderr_file = tempfile.TemporaryFile()
    handles = [stdin_file, stdout_file, stderr_file]
    pi = PROCESS_INFORMATION()
    try:
        si = STARTUPINFO()
        si.cb = ctypes.sizeof(si)
        si.dwFlags = STARTF_USESTDHANDLES
        si.hStdInput = _windows_temp_handle(stdin_file)
        si.hStdOutput = _windows_temp_handle(stdout_file)
        si.hStdError = _windows_temp_handle(stderr_file)
        command_line = ctypes.create_unicode_buffer(subprocess.list2cmdline([path]))
        flags = CREATE_SUSPENDED | HIGH_PRIORITY_CLASS
        if not KERNEL32.CreateProcessW(
            path, command_line, None, None, True, flags, None, None,
            ctypes.byref(si), ctypes.byref(pi),
        ):
            raise OSError(ctypes.get_last_error(), "CreateProcessW failed")
        for file_obj in handles:
            os.set_handle_inheritable(msvcrt.get_osfhandle(file_obj.fileno()), False)
        try:
            if not KERNEL32.SetProcessAffinityMask(pi.hProcess, 1 << cpu):
                raise OSError(ctypes.get_last_error(), "SetProcessAffinityMask failed")
            priority = KERNEL32.GetPriorityClass(pi.hProcess)
            if priority == 0:
                raise OSError(ctypes.get_last_error(), "GetPriorityClass failed")
            if priority != HIGH_PRIORITY_CLASS:
                raise RuntimeError(f"unexpected Windows priority class: 0x{priority:08x}")
            started = time.perf_counter_ns()
            if KERNEL32.ResumeThread(pi.hThread) == 0xFFFFFFFF:
                raise OSError(ctypes.get_last_error(), "ResumeThread failed")
            if KERNEL32.WaitForSingleObject(pi.hProcess, INFINITE) == WAIT_FAILED:
                raise OSError(ctypes.get_last_error(), "WaitForSingleObject failed")
            elapsed = time.perf_counter_ns() - started
            code = wintypes.DWORD()
            if not KERNEL32.GetExitCodeProcess(pi.hProcess, ctypes.byref(code)):
                raise OSError(ctypes.get_last_error(), "GetExitCodeProcess failed")
            stdout_file.seek(0)
            stderr_file.seek(0)
            stdout = stdout_file.read()
            stderr = stderr_file.read()
            if code.value != expected_exit:
                raise RuntimeError(
                    f"{path}: exit {code.value}, expected {expected_exit}; "
                    f"stderr={stderr[:256]!r}"
                )
            return elapsed, {
                "stdout_sha256": digest(stdout), "stderr_sha256": digest(stderr),
                "stdout_bytes": len(stdout), "stderr_bytes": len(stderr),
            }
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

def invoke(path: str, expected_exit: int, cpu: int):
    if os.name == "nt":
        return _invoke_windows(os.path.abspath(path), expected_exit, cpu)
    started = time.perf_counter_ns()
    process = subprocess.Popen([path], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    stdout, stderr = process.communicate()
    elapsed = time.perf_counter_ns() - started
    if process.returncode != expected_exit:
        raise RuntimeError(
            f"{path}: exit {process.returncode}, expected {expected_exit}; "
            f"stderr={stderr[:256]!r}"
        )
    return elapsed, {
        "stdout_sha256": digest(stdout), "stderr_sha256": digest(stderr),
        "stdout_bytes": len(stdout), "stderr_bytes": len(stderr),
    }

def sample_stats(samples):
    ordered = sorted(samples)
    median = int(statistics.median(ordered))
    deviations = sorted(abs(value - median) for value in ordered)
    mad = int(statistics.median(deviations))
    relative_mad = (mad / median) if median else 0.0
    return {
        "median_ns": median,
        "min_ns": ordered[0],
        "max_ns": ordered[-1],
        "mad_ns": mad,
        "relative_mad": relative_mad,
        "samples_ns": ordered,
    }


def assert_signature(current, expected, label):
    if expected is None:
        return current
    if current != expected:
        raise RuntimeError(f"observable output drift for {label}: {current} != {expected}")
    return expected


def invoke_sample(path: str, expected_exit: int, cpu: int, repetitions: int):
    elapsed_total = 0
    signature = None
    for repetition in range(repetitions):
        elapsed, current = invoke(path, expected_exit, cpu)
        signature = assert_signature(
            current, signature, f"sample repetition {repetition + 1}"
        )
        elapsed_total += elapsed
    return elapsed_total, signature


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("baseline")
    parser.add_argument("candidate")
    parser.add_argument("--expected-exit", type=int, required=True)
    parser.add_argument("--warmups", type=int, required=True)
    parser.add_argument("--measurements", type=int, required=True)
    parser.add_argument("--attempts", type=int, required=True)
    parser.add_argument("--maximum-relative-mad", type=float, required=True)
    parser.add_argument("--cpu-affinity", type=int, required=True)
    parser.add_argument("--settle-milliseconds", type=int, default=0)
    parser.add_argument("--sample-repetitions", type=int, default=1)
    args = parser.parse_args()

    if args.cpu_affinity < 0:
        raise ValueError("cpu affinity must be non-negative")
    if args.settle_milliseconds < 0:
        raise ValueError("settle milliseconds must be non-negative")
    if args.sample_repetitions < 1:
        raise ValueError("sample repetitions must be positive")
    if os.name != "nt" and hasattr(os, "sched_getaffinity"):
        inherited = set(os.sched_getaffinity(0))
        if inherited != {args.cpu_affinity}:
            raise RuntimeError(
                f"runner affinity {sorted(inherited)} does not match requested CPU {args.cpu_affinity}"
            )

    canonical_signature = None
    attempts = []
    for attempt in range(1, args.attempts + 1):
        for _ in range(args.warmups):
            _, base_signature = invoke_sample(
                args.baseline, args.expected_exit, args.cpu_affinity,
                args.sample_repetitions,
            )
            canonical_signature = assert_signature(
                base_signature, canonical_signature, "baseline warmup"
            )
            _, candidate_signature = invoke_sample(
                args.candidate, args.expected_exit, args.cpu_affinity,
                args.sample_repetitions,
            )
            canonical_signature = assert_signature(
                candidate_signature, canonical_signature, "candidate warmup"
            )

        # Newly written Windows executables can still be undergoing host-side
        # inspection after the untimed warmups.  Keep that environmental work
        # outside the measured window without changing the workload, sample
        # count, stability threshold, or alternating launch order.
        if args.settle_milliseconds:
            time.sleep(args.settle_milliseconds / 1000.0)

        baseline_samples = []
        candidate_samples = []
        for index in range(args.measurements):
            order = ("baseline", "candidate") if index % 2 == 0 else ("candidate", "baseline")
            for label in order:
                path = args.baseline if label == "baseline" else args.candidate
                elapsed, signature = invoke_sample(
                    path, args.expected_exit, args.cpu_affinity,
                    args.sample_repetitions,
                )
                canonical_signature = assert_signature(signature, canonical_signature, label)
                if label == "baseline":
                    baseline_samples.append(elapsed)
                else:
                    candidate_samples.append(elapsed)
        baseline = sample_stats(baseline_samples)
        candidate = sample_stats(candidate_samples)
        stable = (
            baseline["relative_mad"] <= args.maximum_relative_mad
            and candidate["relative_mad"] <= args.maximum_relative_mad
        )
        record = {
            "attempt": attempt,
            "stable": stable,
            "baseline": baseline,
            "candidate": candidate,
            "ratio": baseline["median_ns"] / candidate["median_ns"],
        }
        attempts.append(record)
        if stable:
            print(json.dumps({
                "stable": True,
                "selected_attempt": attempt,
                "cpu_affinity": args.cpu_affinity,
                "priority_class": "high" if os.name == "nt" else "inherited",
                "launch_control": "suspended_affinity_high_then_resume" if os.name == "nt" else "inherited",
                "settle_milliseconds": args.settle_milliseconds,
                "sample_repetitions": args.sample_repetitions,
                "signature": canonical_signature,
                "attempts": attempts,
            }, separators=(",", ":")))
            return 0

    print(json.dumps({
        "stable": False,
        "selected_attempt": None,
        "cpu_affinity": args.cpu_affinity,
        "priority_class": "high" if os.name == "nt" else "inherited",
        "launch_control": "suspended_affinity_high_then_resume" if os.name == "nt" else "inherited",
        "settle_milliseconds": args.settle_milliseconds,
        "sample_repetitions": args.sample_repetitions,
        "signature": canonical_signature,
        "attempts": attempts,
    }, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(json.dumps({"stable": False, "error": str(error)}, separators=(",", ":")))
        raise SystemExit(2)
