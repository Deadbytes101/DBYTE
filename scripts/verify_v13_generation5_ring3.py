from pathlib import Path
import hashlib, json, re, struct, subprocess, time, uuid

from v13_release_identity import FROZEN_BASE_HEAD, expected_head

REPO = Path(__file__).resolve().parent.parent
COMPILER = REPO / "target/debug/dbytec.exe"
QEMU = Path(r"C:\Program Files\qemu\qemu-system-x86_64.exe")
EXPECTED_HEAD = expected_head()
ROOT = REPO / "tmp/v13_g5" / ("ring3-" + uuid.uuid4().hex)
ROOT.mkdir(parents=True)
COMMAND_INDEX = 0
KERNEL = "examples/native_v13_machine/kernel_g5_ring3.dby"
USER = "examples/native_v13_machine/programs64/hello.dby"


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
    log.write_text(f"COMMAND={' '.join(map(str,args))}\nEXIT={p.returncode}\n{p.stdout}", encoding="utf-8")
    if p.returncode != expect:
        raise RuntimeError(f"{description} exit {p.returncode}, expected {expect}: {log}")
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
    user = (REPO / "examples/native_v13_machine/user_space64.dby").read_text(encoding="utf-8")
    irq = (REPO / "examples/native_v13_machine/interrupts64.dby").read_text(encoding="utf-8")
    kernel = (REPO / KERNEL).read_text(encoding="utf-8")
    assert_true("pub interrupt(no_error_code, cpl3_transition) fn syscall" in user,
                "CPL3 syscall handler missing")
    assert_true("native.interrupt_resume((&RESUME[0]) as usize)" in user,
                "IRETQ resume path missing")
    assert_true("native.cpu_write_cr3(PML4_FRAME as usize)" in user,
                "private CR3 entry missing")
    assert_true("pub fn validate_user_range" in user and "pub fn copy_from_user" in user,
                "copyin validation boundary missing")
    assert_true("install_user_syscall_gate(interrupt_address(128, syscall))" in user,
                "INT80 gate installation missing")
    assert_true("return set_gate_attr(128 as usize, address, 0 as u8, 0xee as u8)" in irq,
                "DPL3 interrupt gate contract missing")
    assert_true("boot.module_count() != (1 as usize)" in kernel,
                "single module ownership gate missing")
    assert_true("user.enter()" in kernel, "ring3 entry is not wired")


def predecessor_gates():
    g3 = run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
              str(REPO / "scripts/verify_v13_generation3.ps1")],
             "fresh G3 predecessor")
    (ROOT / "g3-predecessor.txt").write_text(g3, encoding="utf-8")
    assert_true("VERIFIER: PASS" in g3, "G3 predecessor did not report PASS")
    mod = run(["python.exe", str(REPO / "scripts/verify_v13_generation5_module_bootstrap.py")],
              "fresh module bootstrap predecessor")
    (ROOT / "module-bootstrap-predecessor.txt").write_text(mod, encoding="utf-8")
    assert_true("VERIFIER: PASS" in mod, "module bootstrap predecessor did not report PASS")


def build_iso(label, opt, date):
    out = ROOT / f"{label}-o{opt}"
    out.mkdir()
    kernel = out / "kernel.elf"
    user = out / "hello.elf"
    run([COMPILER, "build", KERNEL, "--target", "x86_64-dbyte-none", "--opt-level", opt,
         "--emit", "elf", "-o", kernel], f"build ring3 kernel {label} O{opt}")
    run([COMPILER, "build", USER, "--target", "x86_64-dbyte-user", "--opt-level", opt,
         "--emit", "elf", "-o", user], f"build hello user {label} O{opt}")
    tree = out / "iso"
    grub = tree / "boot/grub"
    grub.mkdir(parents=True)
    (tree / "boot/kernel.elf").write_bytes(kernel.read_bytes())
    (tree / "boot/hello.elf").write_bytes(user.read_bytes())
    (grub / "grub.cfg").write_text(
        "set timeout=0\nset default=0\nmenuentry 'DByte G5 ring3' {\n"
        " multiboot2 /boot/kernel.elf\n module2 /boot/hello.elf hello64\n boot\n}\n",
        encoding="ascii")
    iso = out / "g5-ring3.iso"
    run(["wsl.exe", "-d", "Debian", "--", "grub-mkrescue", "-o", wslpath(iso),
         f"--modification-date={date}", "--set_all_file_dates", date, wslpath(tree)],
        f"deterministic ring3 ISO {label} O{opt}")
    return {"label": label, "opt": opt, "kernel": str(kernel), "user": str(user),
            "iso": str(iso), "kernel_sha256": sha(kernel), "user_sha256": sha(user),
            "iso_sha256": sha(iso), "iso_bytes": iso.stat().st_size}


def wait_serial(serial, process, tokens, timeout=35):
    entry_resets = None
    deadline = time.time() + timeout
    text = ""
    while time.time() < deadline:
        text = read_shared(serial)
        if text and entry_resets is None:
            trace = serial.parent / "trace.log"
            if trace.exists():
                entry_resets = read_shared(trace).count("CPU Reset")
        if any(token in text for token in tokens):
            return text, entry_resets
        if process.poll() is not None:
            break
        time.sleep(0.05)
    raise RuntimeError(f"QEMU timeout/exit={process.poll()}; serial={text}")


def trace_gate(trace, entry_resets, label):
    text = read_shared(trace)
    assert_true("Triple fault" not in text, f"{label} triple fault observed")
    for token in ("v=06", "v=0d", "v=0e"):
        assert_true(token not in text, f"{label} CPU fault observed: {token}")
    final_resets = text.count("CPU Reset")
    assert_true(entry_resets is not None, f"{label} no entry reset sample")
    assert_true(final_resets == entry_resets, f"{label} CPU reset after kernel entry")
    return final_resets


def qemu_args(iso, ram, serial, trace):
    return [str(QEMU), "-cdrom", str(iso), "-m", f"{ram}M", "-display", "none",
            "-monitor", "none", "-serial", f"file:{serial}",
            "-d", "int,cpu_reset,guest_errors", "-D", str(trace),
            "-no-reboot", "-no-shutdown"]


def boot_positive(build, ram):
    out = ROOT / f"run-o{build['opt']}-{ram}m"
    out.mkdir()
    serial = out / "serial.log"
    trace = out / "trace.log"
    args = qemu_args(build["iso"], ram, serial, trace)
    (out / "qemu-command.txt").write_text(" ".join(args), encoding="utf-8")
    p = subprocess.Popen(args, cwd=REPO, stdout=subprocess.DEVNULL,
                         stderr=subprocess.PIPE, text=True)
    try:
        text, entry_resets = wait_serial(
            serial, p, ("G5 RING3 COMPLETE", "G5 ERROR", "G2B FATAL", "G2B PAGEFAULT"))
        assert_true("G5 ERROR" not in text, f"positive ring3 boot error O{build['opt']} {ram}M")
        assert_true("HELLO FROM DBYTE64 USER" in text, "user stdout receipt missing")
        assert_true("USER64 EXIT code=00000000" in text, "user exit receipt mismatch")
        load = re.search(r"USER64 LOAD entry=00000000004000B0 cr3=[0-9A-F]{16} pages=([0-9A-F]{8})", text)
        reclaim = re.search(r"RECLAIM OK frames=([0-9A-F]{8}) owned=([0-9A-F]{8}) syscalls=([0-9A-F]{8})", text)
        assert_true(load is not None and reclaim is not None, "load/reclaim receipt missing")
        user_pages = int(load.group(1), 16)
        reclaimed = int(reclaim.group(1), 16)
        assert_true(reclaimed == user_pages + 5,
                    "reclaim count does not equal user pages plus five private table frames")
        assert_true(int(reclaim.group(2), 16) == 0, "user frames remain owned after exit")
        assert_true(int(reclaim.group(3), 16) == 2, "user syscall count mismatch")
        assert_true("G5 RING3 COMPLETE" in text, "ring3 completion missing")
    finally:
        if p.poll() is None:
            p.kill()
        p.wait()
    final_resets = trace_gate(trace, entry_resets, f"O{build['opt']} {ram}M")
    return {"opt": build["opt"], "ram": ram,
            "serial_sha256": sha(serial), "trace_sha256": sha(trace),
            "entry_resets": entry_resets, "final_resets": final_resets,
            "receipt": "G5 RING3 COMPLETE"}


def mutate_user(source, name):
    data = bytearray(Path(source).read_bytes())
    assert_true(data[:4] == b"\x7fELF", "negative source is not ELF")
    if name == "bad-magic":
        data[0] = 0
        expected = "G5 ERROR ELF LOAD=0000002B"
    elif name == "rwx-segment":
        phoff = struct.unpack_from("<Q", data, 32)[0]
        struct.pack_into("<I", data, phoff + 4, 7)
        expected = "G5 ERROR ELF LOAD=00000015"
    elif name == "bad-entry":
        struct.pack_into("<Q", data, 24, 0x1000)
        expected = "G5 ERROR ELF LOAD=0000002F"
    elif name == "bad-phnum":
        struct.pack_into("<H", data, 56, 17)
        expected = "G5 ERROR ELF LOAD=0000002E"
    else:
        raise RuntimeError(f"unknown negative mutation {name}")
    out = ROOT / f"negative-{name}.elf"
    out.write_bytes(data)
    return out, expected


def build_negative_iso(name, kernel, user, date):
    out = ROOT / f"negative-{name}"
    grub = out / "iso/boot/grub"
    grub.mkdir(parents=True)
    (out / "iso/boot/kernel.elf").write_bytes(Path(kernel).read_bytes())
    (out / "iso/boot/hello.elf").write_bytes(Path(user).read_bytes())
    (grub / "grub.cfg").write_text(
        "set timeout=0\nset default=0\nmenuentry 'DByte G5 ring3 negative' {\n"
        " multiboot2 /boot/kernel.elf\n module2 /boot/hello.elf hello64\n boot\n}\n",
        encoding="ascii")
    iso = out / "negative.iso"
    run(["wsl.exe", "-d", "Debian", "--", "grub-mkrescue", "-o", wslpath(iso),
         f"--modification-date={date}", "--set_all_file_dates", date,
         wslpath(out / "iso")], f"negative ISO {name}")
    return iso


def boot_negative(name, iso, expected):
    out = ROOT / f"negative-run-{name}"
    out.mkdir()
    serial = out / "serial.log"
    trace = out / "trace.log"
    args = qemu_args(iso, 64, serial, trace)
    (out / "qemu-command.txt").write_text(" ".join(args), encoding="utf-8")
    p = subprocess.Popen(args, cwd=REPO, stdout=subprocess.DEVNULL,
                         stderr=subprocess.PIPE, text=True)
    try:
        text, entry_resets = wait_serial(serial, p, (expected, "G2B FATAL", "G2B PAGEFAULT"))
        assert_true(expected in text, f"negative {name} did not reject cleanly")
        assert_true("G5 RING3 COMPLETE" not in text, f"negative {name} reached ring3 completion")
    finally:
        if p.poll() is None:
            p.kill()
        p.wait()
    final_resets = trace_gate(trace, entry_resets, f"negative {name}")
    return {"name": name, "expected": expected, "iso_sha256": sha(iso),
            "serial_sha256": sha(serial), "trace_sha256": sha(trace),
            "entry_resets": entry_resets, "final_resets": final_resets}


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
    predecessor_gates()
    run(["cargo.exe", "build", "-q", "-p", "dbyte_cli", "--bin", "dbytec"],
        "fresh compiler")
    epoch = int(run(["git.exe", "show", "-s", "--format=%ct", "HEAD"], "epoch").strip())
    date = time.strftime("%Y%m%d%H%M%S00", time.gmtime(epoch))
    builds = []
    for opt in ("0", "1"):
        a = build_iso("A", opt, date)
        b = build_iso("B", opt, date)
        assert_true(a["kernel_sha256"] == b["kernel_sha256"], f"O{opt} kernel nondeterministic")
        assert_true(a["user_sha256"] == b["user_sha256"], f"O{opt} user nondeterministic")
        assert_true(a["iso_sha256"] == b["iso_sha256"], f"O{opt} ISO nondeterministic")
        builds.extend([a, b])
    (ROOT / "builds.json").write_text(json.dumps(builds, indent=2), encoding="utf-8")
    receipts = []
    for opt in ("0", "1"):
        selected = next(x for x in builds if x["label"] == "A" and x["opt"] == opt)
        for ram in (32, 64):
            receipts.append(boot_positive(selected, ram))
    (ROOT / "qemu-receipts.json").write_text(json.dumps(receipts, indent=2), encoding="utf-8")
    selected = next(x for x in builds if x["label"] == "A" and x["opt"] == "1")
    negatives = []
    for name in ("bad-magic", "rwx-segment", "bad-entry", "bad-phnum"):
        mutated, expected = mutate_user(selected["user"], name)
        iso = build_negative_iso(name, selected["kernel"], mutated, date)
        negatives.append(boot_negative(name, iso, expected))
    (ROOT / "negative-receipts.json").write_text(json.dumps(negatives, indent=2), encoding="utf-8")
    manifest = source_manifest()
    (ROOT / "input-manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    run(["git.exe", "diff", "--check"], "diff check")
    tracked = run(["git.exe", "diff", "--binary"], "tracked diff")
    (ROOT / "tracked.diff").write_text(tracked, encoding="utf-8")
    result = {
        "status": "PASS",
        "generation": "v13-g5-ring3-int80-qemu",
        "head": head,
        "g3_predecessor": "PASS",
        "module_bootstrap_predecessor": "PASS",
        "selected_kernel_sha256": selected["kernel_sha256"],
        "selected_user_sha256": selected["user_sha256"],
        "selected_iso_sha256": selected["iso_sha256"],
        "selected_iso_bytes": selected["iso_bytes"],
        "manifest_sha256": sha(ROOT / "input-manifest.json"),
        "qemu_receipts": len(receipts),
        "negative_receipts": len(negatives),
        "private_address_spaces": "PASS_QEMU",
        "copyin_validation": "PASS_QEMU",
        "int80_syscall_boundary": "PASS_QEMU",
        "exit_reclaim": "PASS_QEMU",        "worker_syscall_boundary": "PENDING",
        "owner_qualified_fds": "PENDING",
        "hostile_user_fault_containment": "PENDING",
        "interactive_shell": "PENDING",
        "reboot_persistence": "PENDING",
        "real_biostar": "PENDING",
        "real_storage": "DISARMED",
        "v13_final": "NOT_CREATED",
    }
    (ROOT / "RESULT.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print("V13 GENERATION 5 RING3 VERIFIER: PASS")
    print("EVIDENCE ROOT:", ROOT)
    print("SELECTED ISO SHA256:", result["selected_iso_sha256"])
    print("QEMU RECEIPTS:", len(receipts))
    print("NEGATIVE RECEIPTS:", len(negatives))


if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("V13 GENERATION 5 RING3 VERIFIER: FAIL")
        print("EVIDENCE ROOT:", ROOT)
        raise