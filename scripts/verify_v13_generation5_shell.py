from pathlib import Path
import argparse, hashlib, json, re, shutil, socket, struct, subprocess, time, uuid

from v13_release_identity import FROZEN_BASE_HEAD, expected_head

REPO = Path(__file__).resolve().parent.parent
COMPILER = REPO / "target/debug/dbytec.exe"
QEMU = Path(r"C:\Program Files\qemu\qemu-system-x86_64.exe")
EXPECTED_HEAD = expected_head()
ROOT = REPO / "tmp/v13_g5" / ("shell-" + uuid.uuid4().hex)
ROOT.mkdir(parents=True)
KERNEL = "examples/native_v13_machine/kernel_g5_shell.dby"
PROGRAMS = {
    "worker": "examples/native_v13_machine/programs64/worker_probe.dby",
    "hello": "examples/native_v13_machine/programs64/hello.dby",
    "fault": "examples/native_v13_machine/programs64/pagefault.dby",
}
COMMAND_INDEX = 0


def assert_true(value, message):
    if not value:
        raise RuntimeError(message)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest().upper()

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


def fnv1a(data):
    value = 0x811C9DC5
    for byte in data:
        value = ((value ^ byte) * 0x01000193) & 0xFFFFFFFF
    return value

def put_u32(buffer, offset, value):
    struct.pack_into("<I", buffer, offset, value)


def base_fixture():
    disk = bytearray(8 * 1024 * 1024)
    super_off = 64 * 512
    disk[super_off:super_off + 5] = b"DBFS1"
    fields = [(8, 1), (12, 512), (16, 64), (20, 16320), (24, 65), (28, 4),
              (32, 69), (36, 64), (40, 8), (44, 77), (48, 16307),
              (52, 1), (56, 64)]
    for offset, value in fields:
        put_u32(disk, super_off + offset, value)
    put_u32(disk, super_off + 60, fnv1a(disk[super_off:super_off + 60]))
    inode_base = 69 * 512
    root = inode_base + 64
    for offset, value in ((0, 1), (4, 2), (8, 4096), (12, 8)):
        put_u32(disk, root + offset, value)
    for index in range(8):
        put_u32(disk, root + 16 + 4 * index, index)
    inode = inode_base + 128
    for offset, value in ((0, 1), (4, 1), (8, 128), (12, 1), (16, 8)):
        put_u32(disk, inode + offset, value)
    for index in range(1, 8):
        put_u32(disk, inode + 16 + 4 * index, 0xFFFFFFFF)
    bitmap = 65 * 512
    disk[bitmap] = 0xFF
    disk[bitmap + 1] = 0x01
    directory = 77 * 512
    for offset, value in ((0, 2), (4, 1), (8, 11)):
        put_u32(disk, directory + offset, value)
    disk[directory + 12:directory + 23] = b"PERSIST.TXT"
    payload = bytearray(128)
    payload[:8] = b"DBYTEFS1"
    payload[8:12] = b"\n42\n"
    for index in range(12, 128):
        payload[index] = (index * 31 + 0x5D) & 0xFF
    disk[85 * 512:85 * 512 + 128] = payload
    return disk


def install_file(disk, inode_number, entry_index, name, image, first_block):
    assert_true(64 <= len(image) <= 4096, f"{name} is outside DBFS1 file limit")
    blocks = (len(image) + 511) // 512
    assert_true(blocks <= 8, f"{name} exceeds direct-block limit")
    inode = 69 * 512 + inode_number * 64
    for offset, value in ((0, 1), (4, 1), (8, len(image)), (12, blocks)):
        put_u32(disk, inode + offset, value)
    for index in range(8):
        put_u32(disk, inode + 16 + index * 4,
                first_block + index if index < blocks else 0xFFFFFFFF)
    for block in range(first_block, first_block + blocks):
        disk[65 * 512 + block // 8] |= 1 << (block % 8)
    entry = 77 * 512 + entry_index * 64
    encoded = name.encode("ascii")
    for offset, value in ((0, inode_number), (4, 1), (8, len(encoded))):
        put_u32(disk, entry + offset, value)
    disk[entry + 12:entry + 12 + len(encoded)] = encoded
    start = (77 + first_block) * 512
    disk[start:start + len(image)] = image
    return first_block + blocks


def make_fixture(programs, label):
    disk = base_fixture()
    next_block = 9
    next_block = install_file(disk, 3, 1, "WORKER.ELF", programs["worker"], next_block)
    next_block = install_file(disk, 4, 2, "HELLO.ELF", programs["hello"], next_block)
    next_block = install_file(disk, 5, 3, "FAULT.ELF", programs["fault"], next_block)
    expected_next = 9 + sum((len(programs[name]) + 511) // 512
                            for name in ("worker", "hello", "fault"))
    assert_true(next_block == expected_next, "shell fixture block placement drift")
    bitmap = 65 * 512
    for block in range(next_block):
        assert_true((disk[bitmap + block // 8] & (1 << (block % 8))) != 0,
                    f"shell fixture missing allocated block {block}")
    assert_true((disk[bitmap + next_block // 8] & (1 << (next_block % 8))) == 0,
                "shell fixture first-free block is unexpectedly allocated")
    output = ROOT / f"shell-{label}.raw"
    output.write_bytes(disk)
    for name, (inode_number, entry_index) in {
        "WORKER.ELF": (3, 1), "HELLO.ELF": (4, 2), "FAULT.ELF": (5, 3)
    }.items():
        entry = 77 * 512 + entry_index * 64
        assert_true(struct.unpack_from("<I", disk, entry)[0] == inode_number,
                    f"{name} directory inode mismatch")
    return output

def source_guard():
    shell = (REPO / KERNEL).read_text(encoding="utf-8")
    inp = (REPO / "examples/native_v13_machine/input_ps2.dby").read_text(encoding="utf-8")
    loader = (REPO / "examples/native_v13_machine/user_file64.dby").read_text(encoding="utf-8")
    assert_true('boot.module_count() != (0 as usize)' in shell,
                "shell must reject GRUB user modules")
    for token in ('launch("/HELLO.ELF"', 'launch("/WORKER.ELF"', 'launch("/FAULT.ELF"'):
        assert_true(token in shell, f"disk program route missing: {token}")
    assert_true("status = file.load(path, path_length)" in shell,
                "shell launch path does not use shared disk ELF loader")
    assert_true("input.wait_scancode(&scan)" in shell, "shell is not wired to PS/2 input ring")
    assert_true("pub fn load(path: str" in loader and "user.load_image" in loader,
                "shared disk ELF loader missing")
    assert_true("0xd2" not in inp.lower(), "i8042 D2 injection is forbidden")
    assert_true("pub interrupt(no_error_code) fn irq1" in inp, "real IRQ1 handler missing")


def predecessor_gate():
    text = run(["powershell.exe", "-NoProfile", "-Command",
                "python.exe scripts/verify_v13_generation5_worker.py --disk-backed"],
               "fresh disk-backed worker predecessor")
    (ROOT / "worker-predecessor.txt").write_text(text, encoding="utf-8")
    assert_true("VERIFIER: PASS" in text, "disk-backed worker predecessor did not PASS")


def build_set(label, opt, date):
    out = ROOT / f"{label}-o{opt}"
    out.mkdir()
    kernel = out / "kernel.elf"
    run([COMPILER, "build", KERNEL, "--target", "x86_64-dbyte-none",
         "--opt-level", opt, "--emit", "elf", "-o", kernel],
        f"shell kernel {label} O{opt}")
    users = {}
    user_hashes = {}
    for name, source in PROGRAMS.items():
        path = out / f"{name}.elf"
        run([COMPILER, "build", source, "--target", "x86_64-dbyte-user",
             "--opt-level", opt, "--emit", "elf", "-o", path],
            f"shell {name} {label} O{opt}")
        data = path.read_bytes()
        assert_true(len(data) <= 4096, f"{name} O{opt} exceeds DBFS1 file limit")
        users[name] = data
        user_hashes[name] = sha(path)
    tree = out / "iso"
    grub = tree / "boot/grub"
    grub.mkdir(parents=True)
    (tree / "boot/kernel.elf").write_bytes(kernel.read_bytes())
    (grub / "grub.cfg").write_text(
        "set timeout=0\nset default=0\nmenuentry 'DByte G5 shell' {\n"
        " multiboot2 /boot/kernel.elf\n boot\n}\n", encoding="ascii")
    iso = out / "g5-shell.iso"
    run(["wsl.exe", "-d", "Debian", "--", "grub-mkrescue", "-o", wslpath(iso),
         f"--modification-date={date}", "--set_all_file_dates", date, wslpath(tree)],
        f"deterministic shell ISO {label} O{opt}")
    fixture = make_fixture(users, f"{label}-o{opt}")
    return {"label": label, "opt": opt, "kernel": str(kernel), "iso": str(iso),
            "fixture": str(fixture), "kernel_sha256": sha(kernel),
            "iso_sha256": sha(iso), "iso_bytes": iso.stat().st_size,
            "fixture_sha256": sha(fixture), "user_sha256": user_hashes}


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def qmp_connect(port, process):
    deadline = time.time() + 10
    while time.time() < deadline:
        try:
            sock = socket.create_connection(("127.0.0.1", port), timeout=1)
            stream = sock.makefile("rwb", buffering=0)
            json.loads(stream.readline())
            return sock, stream
        except OSError:
            if process.poll() is not None:
                raise RuntimeError("QEMU exited before QMP connect")
            time.sleep(0.05)
    raise RuntimeError("QMP connect timeout")


def qmp_cmd(stream, name, args=None, ident=1):
    request = {"execute": name, "id": ident}
    if args is not None:
        request["arguments"] = args
    stream.write((json.dumps(request, separators=(",", ":")) + "\n").encode())
    while True:
        line = stream.readline()
        if not line:
            raise RuntimeError("QMP closed")
        response = json.loads(line)
        if response.get("id") == ident:
            if "error" in response:
                raise RuntimeError(f"QMP {name} error: {response['error']}")
            return response.get("return")


def key_event(stream, qcode, down, ident):
    qmp_cmd(stream, "input-send-event", {"events": [{"type": "key", "data": {
        "down": down, "key": {"type": "qcode", "data": qcode}}}]}, ident)


def send_key(stream, qcode, ident):
    key_event(stream, qcode, True, ident)
    time.sleep(0.02)
    key_event(stream, qcode, False, ident + 1)
    time.sleep(0.02)
    return ident + 2


def send_text(stream, text, ident):
    for char in text:
        qcode = "spc" if char == " " else char
        ident = send_key(stream, qcode, ident)
    return ident


def send_command(stream, text, ident, edit_hello=False):
    if edit_hello:
        ident = send_text(stream, "hellp", ident)
        ident = send_key(stream, "backspace", ident)
        ident = send_text(stream, "o", ident)
    else:
        ident = send_text(stream, text, ident)
    ident = send_key(stream, "ret", ident)
    return ident


def send_quit(stream, ident):
    ident = send_text(stream, "quit", ident)
    key_event(stream, "ret", True, ident)
    return ident + 1


def wait_serial(serial, needle, process, timeout=20):
    deadline = time.time() + timeout
    while time.time() < deadline:
        text = read_shared(serial)
        if needle in text:
            return text
        if process.poll() is not None:
            break
        time.sleep(0.05)
    raise RuntimeError(f"timeout waiting for {needle!r}; qemu={process.poll()}; serial={read_shared(serial)}")


def prompt_count(serial, expected, process, timeout=20):
    deadline = time.time() + timeout
    while time.time() < deadline:
        text = read_shared(serial)
        if text.count("dbyte> ") >= expected:
            return text
        if process.poll() is not None:
            break
        time.sleep(0.05)
    raise RuntimeError(f"timeout waiting for prompt {expected}; serial={read_shared(serial)}")


def trace_gate(trace, entry_resets, label):
    text = read_shared(trace)
    assert_true("Triple fault" not in text, f"{label} triple fault")
    assert_true("v=06" not in text, f"{label} unexpected #UD")
    assert_true("v=0d" not in text, f"{label} unexpected #GP")
    page_faults = text.count("v=0e")
    assert_true(page_faults == 1, f"{label} expected exactly one contained #PF, saw {page_faults}")
    final_resets = text.count("CPU Reset")
    assert_true(entry_resets is not None, f"{label} no reset baseline")
    assert_true(final_resets == entry_resets, f"{label} CPU reset drift")
    return final_resets, page_faults


def qemu_args(build, disk, ram, serial, trace, port):
    return [str(QEMU), "-cdrom", build["iso"], "-boot", "order=d", "-m", f"{ram}M",
            "-display", "none", "-monitor", "none", "-serial", f"file:{serial}",
            "-d", "int,cpu_reset,guest_errors", "-D", str(trace),
            "-no-reboot", "-no-shutdown", "-drive",
            f"file={disk},format=raw,if=ide,index=0,media=disk,cache=directsync",
            "-qmp", f"tcp:127.0.0.1:{port},server=on,wait=off"]


def boot_shell(build, ram, case="positive"):
    out = ROOT / f"run-o{build['opt']}-{ram}m-{case}"
    out.mkdir()
    disk = out / "shell.raw"
    shutil.copyfile(build["fixture"], disk)
    before = sha(disk)
    serial = out / "serial.log"
    trace = out / "trace.log"
    port = free_port()
    args = qemu_args(build, disk, ram, serial, trace, port)
    (out / "qemu-command.txt").write_text(" ".join(args), encoding="utf-8")
    process = subprocess.Popen(args, cwd=REPO, stdout=subprocess.DEVNULL,
                               stderr=subprocess.PIPE, text=True)
    sock = stream = None
    try:
        sock, stream = qmp_connect(port, process)
        qmp_cmd(stream, "qmp_capabilities", ident=10)
        text = wait_serial(serial, "SHELL READY disk=DBFS1 input=PS2", process, 35)
        prompt_count(serial, 1, process)
        entry_resets = read_shared(trace).count("CPU Reset")
        ident = 20
        ident = send_command(stream, "hello", ident, edit_hello=True)
        wait_serial(serial, "HELLO FROM DBYTE64 USER", process)
        wait_serial(serial, "SHELL RETURN id=00000001 exit=00000000", process)
        prompt_count(serial, 2, process)
        ident = send_command(stream, "worker", ident)
        wait_serial(serial, "WORKER USER START", process)
        wait_serial(serial, "WORKER USER OK", process)
        wait_serial(serial, "SHELL RETURN id=00000002 exit=00000000", process)
        prompt_count(serial, 3, process)
        ident = send_command(stream, "fault", ident)
        wait_serial(serial, "USER64 FAULT reason=0000000E", process)
        wait_serial(serial, "SHELL FAULT id=00000003 reason=0000000E", process)
        prompt_count(serial, 4, process)
        ident = send_command(stream, "status", ident)
        text = wait_serial(serial, "SHELL STATUS runs=00000003 commands=00000004 faults=00000001", process)
        prompt_count(serial, 5, process)
        assert_true(" drops=00000000" in text, "input ring dropped data before status")
        ident = send_command(stream, "help", ident)
        wait_serial(serial, "SHELL COMMANDS hello worker fault status help quit", process)
        prompt_count(serial, 6, process)
        ident = send_command(stream, "nope", ident)
        wait_serial(serial, "SHELL UNKNOWN len=00000004", process)
        prompt_count(serial, 7, process)
        ident = send_command(stream, "a" * 40, ident)
        wait_serial(serial, "SHELL UNKNOWN len=0000001F", process)
        prompt_count(serial, 8, process)
        ident = send_quit(stream, ident)
        text = wait_serial(serial, "G5 SHELL INTERACTIVE COMPLETE", process, 15)
        expected_input = "SHELL INPUT irq=000000A7 eoi=000000A7 pushes=000000A7 drops=00000000 malformed=00000000"
        assert_true(expected_input in text, "final PS/2 IRQ/EOI/ring receipt mismatch")
        assert_true("SHELL COMPLETE runs=00000003 commands=00000008 faults=00000001" in text,
                    "shell run/command/fault receipt mismatch")
        assert_true("G5SHELL " not in text and "G5 KERNEL #" not in text,
                    "shell reported kernel failure")
    finally:
        if stream:
            try:
                qmp_cmd(stream, "quit", ident=900)
            except Exception:
                pass
        if sock:
            try:
                sock.close()
            except Exception:
                pass
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    after = sha(disk)
    assert_true(before == after, f"shell read-only run mutated disk O{build['opt']} {ram}M")
    final_resets, page_faults = trace_gate(trace, entry_resets,
                                           f"shell O{build['opt']} {ram}M")
    return {"opt": build["opt"], "ram": ram, "case": case,
            "disk_before": before, "disk_after": after,
            "serial_sha256": sha(serial), "trace_sha256": sha(trace),
            "entry_resets": entry_resets, "final_resets": final_resets,
            "contained_page_faults": page_faults, "input_events": 167,
            "runs": 3, "commands": 8, "faults": 1}


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


def main(focused=False):
    head = run(["git.exe", "rev-parse", "HEAD"], "HEAD").strip()
    assert_true(head == EXPECTED_HEAD, "unexpected HEAD")
    tag = run(["git.exe", "rev-parse", "v12.4.0^{}"], "frozen tag").strip()
    assert_true(tag == FROZEN_BASE_HEAD, "v12.4.0 tag drifted")
    source_guard()
    if not focused:
        predecessor_gate()
    run(["cargo.exe", "build", "-q", "-p", "dbyte_cli", "--bin", "dbytec"],
        "fresh compiler")
    epoch = int(run(["git.exe", "show", "-s", "--format=%ct", "HEAD"], "epoch").strip())
    date = time.strftime("%Y%m%d%H%M%S00", time.gmtime(epoch))
    builds = []
    if focused:
        builds.append(build_set("A", "1", date))
    else:
        for opt in ("0", "1"):
            a = build_set("A", opt, date)
            b = build_set("B", opt, date)
            for key in ("kernel_sha256", "iso_sha256", "fixture_sha256"):
                assert_true(a[key] == b[key], f"O{opt} nondeterministic {key}")
            assert_true(a["user_sha256"] == b["user_sha256"],
                        f"O{opt} user programs are nondeterministic")
            builds.extend([a, b])
    (ROOT / "builds.json").write_text(json.dumps(builds, indent=2), encoding="utf-8")
    receipts = []
    if focused:
        receipts.append(boot_shell(builds[0], 64, "focused"))
    else:
        for opt in ("0", "1"):
            selected = next(x for x in builds if x["label"] == "A" and x["opt"] == opt)
            for ram in (32, 64):
                receipts.append(boot_shell(selected, ram))
    (ROOT / "qemu-receipts.json").write_text(json.dumps(receipts, indent=2), encoding="utf-8")
    run(["git.exe", "diff", "--check"], "diff check")
    manifest = source_manifest()
    manifest_path = ROOT / "input-manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    selected = next(x for x in builds if x["label"] == "A" and x["opt"] == "1")
    result = {
        "status": "FOCUSED_PASS" if focused else "PASS",
        "generation": "v13-g5-interactive-ps2-disk-shell-qemu",
        "head": head,
        "worker_disk_predecessor": "SKIPPED_FOCUSED" if focused else "PASS",
        "disk_programs": "PASS_QEMU",
        "interactive_ps2_shell": "PASS_QEMU",
        "hostile_user_survival_in_shell": "PASS_QEMU",
        "line_editing": "PASS_QEMU",
        "line_overflow_bound": "PASS_QEMU",
        "disk_mutation": "NONE_OBSERVED",
        "selected_kernel_sha256": selected["kernel_sha256"],
        "selected_iso_sha256": selected["iso_sha256"],
        "selected_iso_bytes": selected["iso_bytes"],
        "selected_fixture_sha256": selected["fixture_sha256"],
        "selected_user_sha256": selected["user_sha256"],
        "manifest_sha256": sha(manifest_path),
        "qemu_receipts": len(receipts),
        "reboot_persistence": "PENDING",
        "real_biostar": "PENDING",
        "real_storage": "DISARMED",
        "v13_final": "NOT_CREATED",
    }
    (ROOT / "RESULT.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print("V13 GENERATION 5 INTERACTIVE SHELL VERIFIER:", result["status"])
    print("EVIDENCE ROOT:", ROOT)
    print("SELECTED ISO SHA256:", result["selected_iso_sha256"])
    print("QEMU RECEIPTS:", len(receipts))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--focused", action="store_true",
                        help="Skip expensive predecessor/determinism matrix; never emits PASS")
    args = parser.parse_args()
    try:
        main(args.focused)
    except Exception:
        print("V13 GENERATION 5 INTERACTIVE SHELL VERIFIER: FAIL")
        print("EVIDENCE ROOT:", ROOT)
        raise
