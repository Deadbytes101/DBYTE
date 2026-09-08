from pathlib import Path
import hashlib
import json
import subprocess
import time
import uuid

from v13_release_identity import FROZEN_BASE_HEAD, expected_head

REPO = Path(__file__).resolve().parent.parent
COMPILER = REPO / "target/debug/dbytec.exe"
QEMU = Path(r"C:\Program Files\qemu\qemu-system-x86_64.exe")
EXPECTED_HEAD = expected_head()
ROOT = REPO / "tmp/v13_g5" / ("fault-containment-" + uuid.uuid4().hex)
ROOT.mkdir(parents=True)
COMMAND_INDEX = 0
KERNEL = "examples/native_v13_machine/kernel_g5_faults.dby"
FAULT_USER = "examples/native_v13_machine/programs64/pagefault.dby"
SURVIVOR_USER = "examples/native_v13_machine/programs64/hello.dby"
TASK_REGRESSION = "examples/native_v13_machine/task_address64_regression.dby"


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest().upper()


def assert_true(value, message):
    if not value:
        raise RuntimeError(message)
def run(args, description, expect=0):
    global COMMAND_INDEX
    COMMAND_INDEX += 1
    p = subprocess.run([str(x) for x in args], cwd=REPO, text=True,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = ROOT / f"command-{COMMAND_INDEX:03d}.txt"
    log.write_text(
        f"COMMAND={' '.join(map(str, args))}\nEXIT={p.returncode}\n{p.stdout}",
        encoding="utf-8")
    if p.returncode != expect:
        raise RuntimeError(
            f"{description} exit {p.returncode}, expected {expect}: {log}")
    return p.stdout


def wslpath(path):
    win = str(Path(path).resolve()).replace("\\", "/")
    return run(["wsl.exe", "-d", "Debian", "--", "wslpath", "-a", "-u", win],
               "wslpath").strip()


def read_shared(path):
    try:
        return Path(path).read_text(encoding="ascii", errors="replace")
    except (FileNotFoundError, PermissionError):
        return ""
def source_guard():
    selector = (REPO / "crates/dbyte_native/src/native_select_x86_64.rs").read_text(encoding="utf-8")
    user = (REPO / "examples/native_v13_machine/user_space64.dby").read_text(encoding="utf-8")
    irq = (REPO / "examples/native_v13_machine/interrupts64.dby").read_text(encoding="utf-8")
    kernel = (REPO / KERNEL).read_text(encoding="utf-8")
    assert_true("| NativeMirInst::TaskAddress {" in selector,
                "x86_64 task_address selector seam missing")
    assert_true("fn contain_fault" in user, "user fault containment missing")
    assert_true("pub interrupt(error_code, cpl3_transition) fn user_page_fault" in user,
                "CPL3 page-fault handler missing")
    assert_true("(*frame).ss = irq.kernel_data_selector() as u64" in user,
                "kernel IRETQ SS restoration missing")
    assert_true("(*frame).rsp = (irq.ring0_stack_top() - (8 as usize)) as u64" in user,
                "kernel IRETQ stack restoration missing")
    assert_true("install_user_fault_gates" in irq,
                "CPL3 fault gates not exposed")
    assert_true("task_address(recover)" in kernel,
                "kernel recovery continuation not wired")
    assert_true("start_user(1 as usize)" in kernel,
                "survivor user is not launched after fault")


def predecessor_gate():
    output = run(["python.exe", str(REPO / "scripts/verify_v13_generation5_ring3.py")],
                 "fresh Ring3 predecessor")
    (ROOT / "ring3-predecessor.txt").write_text(output, encoding="utf-8")
def task_address_gate():
    receipts = []
    for opt in ("0", "1"):
        obj = ROOT / f"task-address-o{opt}.o"
        run([COMPILER, "build", TASK_REGRESSION, "--target", "x86_64-dbyte-none",
             "--opt-level", opt, "--emit", "obj", "-o", obj],
            f"task_address O{opt} object")
        reloc = run(["objdump.exe", "-r", obj], f"task_address O{opt} relocations")
        assert_true("continuation" in reloc, f"task_address O{opt} relocation target missing")
        assert_true("PC32" in reloc, f"task_address O{opt} is not PC-relative")
        receipts.append({"opt": opt, "object_sha256": sha(obj)})
    return receipts


def build_iso(label, opt, date):
    out = ROOT / f"{label}-o{opt}"
    out.mkdir()
    kernel = out / "kernel.elf"
    fault = out / "pagefault.elf"
    survivor = out / "hello.elf"
    run([COMPILER, "build", KERNEL, "--target", "x86_64-dbyte-none",
         "--opt-level", opt, "--emit", "elf", "-o", kernel],
        f"kernel {label} O{opt}")
    run([COMPILER, "build", FAULT_USER, "--target", "x86_64-dbyte-user",
         "--opt-level", opt, "--emit", "elf", "-o", fault],
        f"fault user {label} O{opt}")
    run([COMPILER, "build", SURVIVOR_USER, "--target", "x86_64-dbyte-user",
         "--opt-level", opt, "--emit", "elf", "-o", survivor],
        f"survivor user {label} O{opt}")
    tree = out / "iso"
    grub = tree / "boot/grub"
    grub.mkdir(parents=True)
    (tree / "boot/kernel.elf").write_bytes(kernel.read_bytes())
    (tree / "boot/pagefault.elf").write_bytes(fault.read_bytes())
    (tree / "boot/hello.elf").write_bytes(survivor.read_bytes())
    (grub / "grub.cfg").write_text(
        "set timeout=0\nset default=0\nmenuentry 'DByte G5 fault containment' {\n"
        " multiboot2 /boot/kernel.elf\n"
        " module2 /boot/pagefault.elf pagefault64\n"
        " module2 /boot/hello.elf hello64\n"
        " boot\n}\n", encoding="ascii")
    iso = out / "g5-fault-containment.iso"
    run(["wsl.exe", "-d", "Debian", "--", "grub-mkrescue", "-o", wslpath(iso),
         f"--modification-date={date}", "--set_all_file_dates", date, wslpath(tree)],
        f"deterministic ISO {label} O{opt}")
    return {"label": label, "opt": opt, "kernel": str(kernel),
            "fault": str(fault), "survivor": str(survivor), "iso": str(iso),
            "kernel_sha256": sha(kernel), "fault_sha256": sha(fault),
            "survivor_sha256": sha(survivor), "iso_sha256": sha(iso),
            "iso_bytes": iso.stat().st_size}
def wait_serial(serial, process, timeout=35):
    deadline = time.time() + timeout
    entry_resets = None
    text = ""
    while time.time() < deadline:
        text = read_shared(serial)
        trace = serial.parent / "trace.log"
        if text and entry_resets is None and trace.exists():
            entry_resets = read_shared(trace).count("CPU Reset")
        if "G5 USER FAULT CONTAINMENT COMPLETE" in text:
            return text, entry_resets
        if "G5FAULT " in text or "G5 KERNEL #" in text:
            return text, entry_resets
        if process.poll() is not None:
            break
        time.sleep(0.05)
    raise RuntimeError(f"QEMU timeout/exit={process.poll()}; serial={text}")


def qemu_args(iso, ram, serial, trace):
    return [str(QEMU), "-cdrom", str(iso), "-m", f"{ram}M",
            "-display", "none", "-monitor", "none",
            "-serial", f"file:{serial}",
            "-d", "int,cpu_reset,guest_errors", "-D", str(trace),
            "-no-reboot", "-no-shutdown"]
def boot(build, ram):
    out = ROOT / f"run-o{build['opt']}-{ram}m"
    out.mkdir()
    serial = out / "serial.log"
    trace = out / "trace.log"
    args = qemu_args(build["iso"], ram, serial, trace)
    (out / "qemu-command.txt").write_text(" ".join(args), encoding="utf-8")
    process = subprocess.Popen(args, cwd=REPO, stdout=subprocess.DEVNULL,
                               stderr=subprocess.PIPE, text=True)
    try:
        text, entry_resets = wait_serial(serial, process)
        assert_true("G5FAULT " not in text, f"G5 failure O{build['opt']} {ram}M")
        assert_true("G5 KERNEL #" not in text, f"kernel fault O{build['opt']} {ram}M")
        expected = [
            "USER64 FAULT reason=0000000E addr=00000000C1000000 error=0000000000000007 owned=00000000",
            "USER #PF CONTAINED addr=00000000C1000000 error=0000000000000007",
            "HELLO FROM DBYTE64 USER",
            "USER64 EXIT code=00000000",
            "SURVIVOR USER EXITED",
            "G5 USER FAULT CONTAINMENT COMPLETE",
        ]
        for token in expected:
            assert_true(token in text, f"missing receipt {token!r}")
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
    trace_text = read_shared(trace)
    assert_true("Triple fault" not in trace_text,
                f"triple fault O{build['opt']} {ram}M")
    assert_true("v=06" not in trace_text,
                f"unexpected #UD O{build['opt']} {ram}M")
    assert_true("v=0d" not in trace_text,
                f"unexpected #GP O{build['opt']} {ram}M")
    page_faults = trace_text.count("v=0e")
    assert_true(page_faults == 1,
                f"expected exactly one hostile #PF, saw {page_faults}")
    final_resets = trace_text.count("CPU Reset")
    assert_true(entry_resets is not None, "no reset baseline sampled")
    assert_true(final_resets == entry_resets,
                f"CPU reset drift O{build['opt']} {ram}M")
    return {
        "opt": build["opt"], "ram": ram,
        "serial_sha256": sha(serial), "trace_sha256": sha(trace),
        "entry_resets": entry_resets, "final_resets": final_resets,
        "hostile_page_faults": page_faults,
        "fault_address": "0x00000000C1000000",
        "fault_error": "0x7", "survivor_exit": 0,
    }


def source_manifest():
    paths = run(["git.exe", "ls-files", "--cached", "--others", "--exclude-standard"],
                "source enumeration").splitlines()
    rows = []
    for rel in sorted(set(paths)):
        if rel.startswith(("crates/", "examples/native_v13_machine/", "scripts/")) or rel.startswith("Cargo."):
            path = REPO / rel
            if path.is_file():
                rows.append({"path": rel.replace("\\", "/"), "sha256": sha(path)})
    return rows


def main():
    head = run(["git.exe", "rev-parse", "HEAD"], "HEAD").strip()
    assert_true(head == EXPECTED_HEAD, "unexpected HEAD")
    tag = run(["git.exe", "rev-parse", "v12.4.0^{}"], "frozen tag").strip()
    assert_true(tag == FROZEN_BASE_HEAD, "v12.4.0 tag drifted")
    source_guard()
    predecessor_gate()
    run(["cargo.exe", "build", "-q", "-p", "dbyte_cli", "--bin", "dbytec"],
        "fresh compiler")
    task_receipts = task_address_gate()
    (ROOT / "task-address-receipts.json").write_text(
        json.dumps(task_receipts, indent=2), encoding="utf-8")
    epoch = int(run(["git.exe", "show", "-s", "--format=%ct", "HEAD"], "epoch").strip())
    date = time.strftime("%Y%m%d%H%M%S00", time.gmtime(epoch))
    builds = []
    for opt in ("0", "1"):
        a = build_iso("A", opt, date)
        b = build_iso("B", opt, date)
        for key in ("kernel_sha256", "fault_sha256", "survivor_sha256", "iso_sha256"):
            assert_true(a[key] == b[key], f"O{opt} nondeterministic {key}")
        builds.extend([a, b])
    (ROOT / "builds.json").write_text(json.dumps(builds, indent=2), encoding="utf-8")
    receipts = []
    for opt in ("0", "1"):
        selected = next(x for x in builds if x["label"] == "A" and x["opt"] == opt)
        for ram in (32, 64):
            receipts.append(boot(selected, ram))
    (ROOT / "qemu-receipts.json").write_text(
        json.dumps(receipts, indent=2), encoding="utf-8")
    manifest = source_manifest()
    manifest_path = ROOT / "input-manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    run(["git.exe", "diff", "--check"], "diff check")
    tracked = run(["git.exe", "diff", "--binary"], "tracked diff")
    (ROOT / "tracked.diff").write_text(tracked, encoding="utf-8")
    selected = next(x for x in builds if x["label"] == "A" and x["opt"] == "1")
    result = {
        "status": "PASS",
        "generation": "v13-g5-hostile-user-fault-containment-qemu",
        "head": head,
        "ring3_predecessor": "PASS",
        "task_address64": "PASS_FOCUSED",
        "selected_kernel_sha256": selected["kernel_sha256"],
        "selected_fault_user_sha256": selected["fault_sha256"],
        "selected_survivor_user_sha256": selected["survivor_sha256"],
        "selected_iso_sha256": selected["iso_sha256"],
        "selected_iso_bytes": selected["iso_bytes"],
        "manifest_sha256": sha(manifest_path),
        "qemu_receipts": len(receipts),
        "hostile_fault": "PAGE_FAULT_CPL3",
        "fault_address": "0x00000000C1000000",
        "fault_error": "0x7",
        "user_reclaim": "PASS_QEMU",
        "kernel_survival": "PASS_QEMU",
        "survivor_user": "PASS_QEMU",
        "worker_syscall_boundary": "PENDING",
        "owner_qualified_fds": "PENDING",
        "interactive_shell": "PENDING",
        "reboot_persistence": "PENDING",
        "real_biostar": "PENDING",
        "real_storage": "DISARMED",
        "v13_final": "NOT_CREATED",
    }
    (ROOT / "RESULT.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print("V13 GENERATION 5 USER FAULT CONTAINMENT VERIFIER: PASS")
    print("EVIDENCE ROOT:", ROOT)
    print("SELECTED ISO SHA256:", result["selected_iso_sha256"])
    print("QEMU RECEIPTS:", len(receipts))


if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("V13 GENERATION 5 USER FAULT CONTAINMENT VERIFIER: FAIL")
        print("EVIDENCE ROOT:", ROOT)
        raise
