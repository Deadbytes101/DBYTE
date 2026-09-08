from pathlib import Path
import hashlib, json, subprocess, time, uuid

from v13_release_identity import FROZEN_BASE_HEAD, expected_head

REPO = Path(__file__).resolve().parent.parent
COMPILER = REPO / "target/debug/dbytec.exe"
QEMU = Path(r"C:\Program Files\qemu\qemu-system-x86_64.exe")
EXPECTED_HEAD = expected_head()
ROOT = REPO / "tmp/v13_g5" / ("module-bootstrap-" + uuid.uuid4().hex)
ROOT.mkdir(parents=True)
COMMAND_INDEX = 0
KERNEL = "examples/native_v13_machine/kernel_g5_module_probe.dby"
USER = "examples/native_v13_machine/programs64/hello.dby"

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest().upper()

def assert_true(value, message):
    if not value:
        raise RuntimeError(message)

def run(args, description):
    global COMMAND_INDEX
    COMMAND_INDEX += 1
    p = subprocess.run([str(x) for x in args], cwd=REPO, text=True,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = ROOT / f"command-{COMMAND_INDEX:03d}.txt"
    log.write_text(f"COMMAND={' '.join(map(str,args))}\nEXIT={p.returncode}\n{p.stdout}", encoding="utf-8")
    if p.returncode:
        raise RuntimeError(f"{description} failed ({p.returncode}): {log}")
    return p.stdout

def wslpath(path):
    return run(["wsl.exe", "-d", "Debian", "--", "wslpath", "-a", "-u",
                str(path).replace("\\", "/")], "wslpath").strip()

def source_guard():
    mb2 = (REPO / "examples/native_v13_machine/multiboot2.dby").read_text(encoding="utf-8")
    boot = (REPO / "examples/native_v13_machine/boot_info.dby").read_text(encoding="utf-8")
    assert_true("const TAG_MODULE: u32 = 3 as u32" in mb2, "module tag support missing")
    assert_true("boot.append_module(start, length)" in mb2, "module normalization missing")
    assert_true("module_in_staged_memory" in boot, "module memory-map coverage gate missing")
    assert_true("return KIND_RESERVED" in boot, "module reservation overlay missing")
    assert_true("MAX_MODULE_BYTES" in boot, "module size bound missing")

def read_shared(path):
    try:
        return Path(path).read_text(encoding="ascii", errors="replace")
    except (FileNotFoundError, PermissionError):
        return ""

def wait_serial(serial, process, timeout=30):
    deadline = time.time() + timeout
    while time.time() < deadline:
        text = read_shared(serial)
        if "G5 MODULE BOOTSTRAP COMPLETE" in text or "G5MOD ERROR" in text:
            return text
        if process.poll() is not None:
            break
        time.sleep(0.05)
    raise RuntimeError(f"module bootstrap timeout; qemu={process.poll()}; serial={read_shared(serial)}")

def build_set(label, opt, date):
    out = ROOT / f"{label}-o{opt}"
    out.mkdir()
    kernel = out / "kernel.elf"
    user = out / "hello.elf"
    run([COMPILER, "build", KERNEL, "--target", "x86_64-dbyte-none", "--opt-level", opt,
         "--emit", "elf", "-o", kernel], f"build kernel {label} O{opt}")
    run([COMPILER, "build", USER, "--target", "x86_64-dbyte-user", "--opt-level", opt,
         "--emit", "elf", "-o", user], f"build user {label} O{opt}")
    tree = out / "iso"
    grub = tree / "boot/grub"
    grub.mkdir(parents=True)
    (tree / "boot/kernel.elf").write_bytes(kernel.read_bytes())
    (tree / "boot/hello.elf").write_bytes(user.read_bytes())
    (grub / "grub.cfg").write_text(
        "set timeout=0\nset default=0\nmenuentry 'DByte G5 module bootstrap' {\n"
        " multiboot2 /boot/kernel.elf\n module2 /boot/hello.elf hello64\n boot\n}\n",
        encoding="ascii")
    iso = out / "g5-module.iso"
    run(["wsl.exe", "-d", "Debian", "--", "grub-mkrescue", "-o", wslpath(iso),
         f"--modification-date={date}", "--set_all_file_dates", date, wslpath(tree)],
        f"deterministic ISO {label} O{opt}")
    return {"label": label, "opt": opt, "kernel": str(kernel), "user": str(user), "iso": str(iso),
            "kernel_sha256": sha(kernel), "user_sha256": sha(user), "iso_sha256": sha(iso),
            "iso_bytes": iso.stat().st_size}

def boot(build, ram):
    out = ROOT / f"run-o{build['opt']}-{ram}m"
    out.mkdir()
    serial = out / "serial.log"
    trace = out / "trace.log"
    args = [str(QEMU), "-cdrom", build["iso"], "-m", f"{ram}M", "-display", "none",
            "-monitor", "none", "-serial", f"file:{serial}", "-d", "int,cpu_reset,guest_errors",
            "-D", str(trace), "-no-reboot", "-no-shutdown"]
    (out / "qemu-command.txt").write_text(" ".join(args), encoding="utf-8")
    p = subprocess.Popen(args, cwd=REPO, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    try:
        text = wait_serial(serial, p)
        assert_true("G5MOD ERROR" not in text, f"module probe error O{build['opt']} {ram}M")
        assert_true("MODULE OK addr=" in text, "module receipt missing")
        assert_true("G5 MODULE BOOTSTRAP COMPLETE" in text, "module completion missing")
    finally:
        if p.poll() is None:
            p.kill()
        p.wait()
    trace_text = read_shared(trace)
    assert_true("Triple fault" not in trace_text, "triple fault observed")
    for token in ("v=06", "v=0d", "v=0e"):
        assert_true(token not in trace_text, f"CPU fault observed: {token}")
    line = next(x for x in text.splitlines() if x.startswith("MODULE OK addr="))
    return {"opt": build["opt"], "ram": ram, "receipt": line,
            "serial_sha256": sha(serial), "trace_sha256": sha(trace)}

def main():
    head = run(["git.exe", "rev-parse", "HEAD"], "HEAD").strip()
    assert_true(head == EXPECTED_HEAD, "unexpected HEAD")
    tag = run(["git.exe", "rev-parse", "v12.4.0^{}"], "frozen tag").strip()
    assert_true(tag == FROZEN_BASE_HEAD, "v12.4.0 tag drifted")
    source_guard()
    compiler = run(["python.exe", str(REPO / "scripts/verify_v13_generation5_user64_compiler.py")],
                   "fresh user64 compiler predecessor")
    (ROOT / "compiler-predecessor.txt").write_text(compiler, encoding="utf-8")
    run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
         str(REPO / "scripts/verify_v13_boot_info_regression.ps1")], "BootInfo regression")
    epoch = int(run(["git.exe", "show", "-s", "--format=%ct", "HEAD"], "epoch").strip())
    date = time.strftime("%Y%m%d%H%M%S00", time.gmtime(epoch))
    builds = []
    for opt in ("0", "1"):
        a = build_set("A", opt, date)
        b = build_set("B", opt, date)
        assert_true(a["kernel_sha256"] == b["kernel_sha256"], f"O{opt} kernel nondeterministic")
        assert_true(a["user_sha256"] == b["user_sha256"], f"O{opt} user nondeterministic")
        assert_true(a["iso_sha256"] == b["iso_sha256"], f"O{opt} ISO nondeterministic")
        builds.extend([a, b])
    (ROOT / "builds.json").write_text(json.dumps(builds, indent=2), encoding="utf-8")
    receipts = []
    for opt in ("0", "1"):
        build = next(x for x in builds if x["label"] == "A" and x["opt"] == opt)
        for ram in (32, 64):
            receipts.append(boot(build, ram))
    (ROOT / "qemu-receipts.json").write_text(json.dumps(receipts, indent=2), encoding="utf-8")
    run(["git.exe", "diff", "--check"], "diff check")
    selected = next(x for x in builds if x["label"] == "A" and x["opt"] == "1")
    result = {
        "status": "PASS", "generation": "v13-g5-multiboot2-user-module-qemu", "head": head,
        "compiler_predecessor": "PASS", "bootinfo_regression": "PASS",
        "selected_kernel_sha256": selected["kernel_sha256"],
        "selected_user_sha256": selected["user_sha256"],
        "selected_iso_sha256": selected["iso_sha256"], "selected_iso_bytes": selected["iso_bytes"],
        "qemu_receipts": len(receipts), "transport": "GRUB module2 -> Multiboot2 tag 3 -> canonical BootInfo",
        "module_memory_policy": "RESERVED_FROM_FRAME_ALLOCATOR", "ring3_runtime": "PENDING",
        "real_biostar": "PENDING", "real_storage": "DISARMED", "v13_final": "NOT_CREATED"
    }
    (ROOT / "RESULT.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print("V13 GENERATION 5 MODULE BOOTSTRAP VERIFIER: PASS")
    print("EVIDENCE ROOT:", ROOT)
    print("SELECTED ISO SHA256:", result["selected_iso_sha256"])
    print("QEMU RECEIPTS:", len(receipts))

if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("V13 GENERATION 5 MODULE BOOTSTRAP VERIFIER: FAIL")
        print("EVIDENCE ROOT:", ROOT)
        raise
