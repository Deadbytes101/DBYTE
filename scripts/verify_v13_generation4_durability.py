from pathlib import Path
import hashlib, json, socket, subprocess, time, uuid

from v13_release_identity import FROZEN_BASE_HEAD, expected_head

REPO = Path(__file__).resolve().parent.parent
COMPILER = REPO / "target" / "debug" / "dbytec.exe"
QEMU = Path(r"C:\Program Files\qemu\qemu-system-x86_64.exe")
EXPECTED_HEAD = expected_head()
ROOT = REPO / "tmp" / "v13_g4" / ("durability-" + uuid.uuid4().hex)
ROOT.mkdir(parents=True)
COMMAND_INDEX = 0
SECTOR = 512
DISK_BYTES = 8 * 1024 * 1024
GUARD_LBA = 7
SCRATCH_LBA = 8
GUARD = b"DBYTEV13VWRITE01"
PATTERN = bytes((((i * 29) + 0x63) & 0xff) for i in range(SECTOR))

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest().upper()

def sha_bytes(data):
    return hashlib.sha256(data).hexdigest().upper()

def sector_hash(data):
    h = 0xC0DEC0DE
    for value in data:
        h = (((h << 5) | (h >> 27)) & 0xffffffff) ^ value
    return h

def run(args, description, cwd=REPO):
    global COMMAND_INDEX
    COMMAND_INDEX += 1
    p = subprocess.run([str(x) for x in args], cwd=cwd, text=True,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = ROOT / f"command-{COMMAND_INDEX:03d}.txt"
    log.write_text(f"CWD={cwd}\nCOMMAND={' '.join(map(str,args))}\nEXIT={p.returncode}\n{p.stdout}", encoding="utf-8")
    if p.returncode:
        raise RuntimeError(f"{description} failed ({p.returncode}): {log}")
    return p.stdout

def assert_true(value, message):
    if not value:
        raise RuntimeError(message)

def wslpath(path):
    return run(["wsl.exe", "-d", "Debian", "--", "wslpath", "-a", "-u",
                str(path).replace("\\", "/")], "translate WSL path").strip()

def source_guard():
    ro = (REPO / "examples/native_v13_machine/storage_ide.dby").read_text(encoding="utf-8")
    svc = (REPO / "examples/native_v13_machine/sector_service.dby").read_text(encoding="utf-8")
    wr = (REPO / "examples/native_v13_machine/storage_ide_write_test.dby").read_text(encoding="utf-8")
    assert_true("native.io_out16(" not in ro, "read-only storage gained OUT16")
    assert_true("COMMAND_WRITE" not in ro and "FLUSH_CACHE" not in ro, "read-only storage gained media write")
    assert_true("write" not in svc.lower() and "flush" not in svc.lower(), "sector service escaped read-only boundary")
    assert_true('const FIXTURE_SECTORS: u32 = 0x00004000 as u32' in wr, "fixture size guard missing")
    assert_true('const GUARD_LBA: u32 = 7 as u32' in wr, "guard LBA changed")
    assert_true('const SCRATCH_LBA: u32 = 8 as u32' in wr, "scratch LBA changed")
    assert_true('const COMMAND_WRITE: u8 = 0x30 as u8' in wr, "ATA WRITE command missing")
    assert_true('const COMMAND_FLUSH_CACHE: u8 = 0xe7 as u8' in wr, "ATA FLUSH command missing")
    assert_true("native.io_out16(DATA_PORT" in wr, "PIO data write missing")
    assert_true("pub fn write_sector" not in wr, "generic write API forbidden in durability card")

def source_manifest():
    paths = run(["git.exe", "ls-files", "--cached", "--others", "--exclude-standard"],
                "source enumeration").splitlines()
    rows = []
    for rel in sorted(set(paths)):
        if rel.startswith(("crates/", "runtime/", "examples/native_v13_machine/", "scripts/")) or rel.startswith("Cargo."):
            path = REPO / rel
            if path.is_file():
                rows.append({"path": rel.replace("\\", "/"), "sha256": sha(path)})
    return rows

def make_fixture(path, kind="valid"):
    disk = bytearray(DISK_BYTES)
    disk[GUARD_LBA * SECTOR:GUARD_LBA * SECTOR + len(GUARD)] = GUARD
    if kind == "bad-guard":
        disk[GUARD_LBA * SECTOR] ^= 1
    elif kind == "dirty-scratch":
        disk[SCRATCH_LBA * SECTOR] = 0xA5
    elif kind != "valid":
        raise RuntimeError(f"unknown fixture kind {kind}")
    path.write_bytes(disk)
    return {"path": str(path), "kind": kind, "sha256": sha(path)}

def build_iso(label, opt, source, stem, date):
    out = ROOT / f"{label}-o{opt}-{stem}"
    out.mkdir()
    obj = out / f"{stem}.o"
    elf = out / f"{stem}.elf"
    base = [COMPILER, "build", source, "--target", "x86_64-dbyte-none", "--opt-level", opt]
    run(base + ["--emit", "obj", "-o", obj], f"build {stem} {label} O{opt} object")
    run(base + ["--emit", "elf", "-o", elf], f"build {stem} {label} O{opt} ELF")
    run(["wsl.exe", "-d", "Debian", "--", "grub-file", "--is-x86-multiboot2", wslpath(elf)], "grub-file")
    tree = out / "iso"
    grub = tree / "boot" / "grub"
    grub.mkdir(parents=True)
    (tree / "boot" / "kernel.elf").write_bytes(elf.read_bytes())
    (grub / "grub.cfg").write_text(
        f"set timeout=0\nset default=0\nmenuentry 'DByte v13 G4 {stem}' {{\n multiboot2 /boot/kernel.elf\n boot\n}}\n",
        encoding="ascii")
    iso = out / f"{stem}.iso"
    run(["wsl.exe", "-d", "Debian", "--", "grub-mkrescue", "-o", wslpath(iso),
         f"--modification-date={date}", "--set_all_file_dates", date, wslpath(tree)],
        f"deterministic {stem} ISO")
    dis = run(["objdump.exe", "-d", "-Mintel", obj], f"disassemble {stem} object")
    (out / "disassembly.txt").write_text(dis, encoding="utf-8")
    if stem == "write":
        normalized = " ".join(dis.replace("\t", " ").split())
        assert_true("out dx,ax" in normalized, "OUT16 not present in writer object machine code")
    return {"label": label, "opt": opt, "stem": stem, "obj": str(obj), "elf": str(elf), "iso": str(iso),
            "obj_sha256": sha(obj), "elf_sha256": sha(elf), "iso_sha256": sha(iso),
            "iso_bytes": iso.stat().st_size}

def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]

def qmp_connect(port, process):
    deadline = time.time() + 10
    while time.time() < deadline:
        try:
            sock = socket.create_connection(("127.0.0.1", port), timeout=1)
            file = sock.makefile("rwb", buffering=0)
            json.loads(file.readline())
            return sock, file
        except OSError:
            if process.poll() is not None:
                raise RuntimeError("QEMU exited before QMP connect")
            time.sleep(0.05)
    raise RuntimeError("QMP connect timeout")

def qmp_cmd(file, name, args=None, ident=1):
    request = {"execute": name, "id": ident}
    if args is not None:
        request["arguments"] = args
    file.write((json.dumps(request, separators=(",", ":")) + "\n").encode())
    while True:
        line = file.readline()
        if not line:
            raise RuntimeError("QMP closed")
        obj = json.loads(line)
        if obj.get("id") == ident:
            if "error" in obj:
                raise RuntimeError(f"QMP {name} error: {obj['error']}")
            return obj.get("return")

def read_shared(path):
    try:
        return Path(path).read_text(encoding="ascii", errors="replace")
    except (FileNotFoundError, PermissionError):
        return ""

def wait_serial(serial, needles, process, timeout):
    if isinstance(needles, str):
        needles = [needles]
    deadline = time.time() + timeout
    while time.time() < deadline:
        text = read_shared(serial)
        if any(needle in text for needle in needles):
            return text
        if process.poll() is not None:
            break
        time.sleep(0.05)
    raise RuntimeError(f"serial timeout for {needles}; qemu={process.poll()}; serial={read_shared(serial)}")

def trace_check(trace, entry_resets):
    text = read_shared(trace)
    assert_true("Triple fault" not in text, "triple fault observed")
    for token in ("v=06", "v=0d", "v=0e"):
        assert_true(token not in text, f"CPU fault observed: {token}")
    final_resets = text.count("CPU Reset")
    assert_true(final_resets == entry_resets, "CPU reset after kernel entry")
    return final_resets

def block_stats(file, ident):
    result = qmp_cmd(file, "query-blockstats", ident=ident)
    stats = []
    def walk(value):
        if isinstance(value, dict):
            if "stats" in value and isinstance(value["stats"], dict):
                stats.append(value["stats"])
            for child in value.values():
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)
    walk(result)
    return {
        "wr_bytes": max([int(s.get("wr_bytes", 0)) for s in stats] or [0]),
        "wr_operations": max([int(s.get("wr_operations", 0)) for s in stats] or [0]),
        "flush_operations": max([int(s.get("flush_operations", 0)) for s in stats] or [0]),
        "failed_wr_operations": max([int(s.get("failed_wr_operations", 0)) for s in stats] or [0]),
    }

def qemu_args(iso, ram, disk, serial, trace, port):
    return [str(QEMU), "-cdrom", str(iso), "-boot", "order=d", "-m", f"{ram}M",
            "-display", "none", "-monitor", "none", "-serial", f"file:{serial}",
            "-d", "int,cpu_reset,guest_errors", "-D", str(trace), "-no-reboot", "-no-shutdown",
            "-drive", f"file={disk},format=raw,if=ide,index=0,media=disk,cache=writeback",
            "-qmp", f"tcp:127.0.0.1:{port},server=on,wait=off"]

def run_vm(iso, ram, disk, out, entry_needle, outcome_needles, timeout=45):
    out.mkdir()
    serial = out / "serial.log"
    trace = out / "trace.log"
    port = free_port()
    args = qemu_args(iso, ram, disk, serial, trace, port)
    (out / "qemu-command.txt").write_text(" ".join(args), encoding="utf-8")
    process = subprocess.Popen(args, cwd=REPO, stdout=subprocess.DEVNULL,
                               stderr=subprocess.PIPE, text=True)
    sock = file = None
    try:
        sock, file = qmp_connect(port, process)
        qmp_cmd(file, "qmp_capabilities", ident=10)
        wait_serial(serial, entry_needle, process, 30)
        entry_resets = read_shared(trace).count("CPU Reset")
        text = wait_serial(serial, outcome_needles, process, timeout)
        stats = block_stats(file, 20)
        qmp_cmd(file, "quit", ident=99)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill(); process.wait()
        final_resets = trace_check(trace, entry_resets)
        return {"text": text, "stats": stats, "serial_sha256": sha(serial),
                "trace_sha256": sha(trace), "entry_resets": entry_resets,
                "final_resets": final_resets}
    finally:
        if file:
            try: file.close()
            except Exception: pass
        if sock:
            try: sock.close()
            except Exception: pass
        if process.poll() is None:
            process.kill(); process.wait()

def positive_pair(write_build, read_build, ram):
    out = ROOT / f"run-o{write_build['opt']}-{ram}m-positive"
    out.mkdir()
    disk = out / "test.raw"
    make_fixture(disk, "valid")
    before = disk.read_bytes()
    before_sha = sha_bytes(before)
    write_run = run_vm(write_build["iso"], ram, disk, out / "write-boot",
                       "DBYTE V13 G4 DURABILITY WRITE",
                       ["G4 DURABILITY WRITE COMPLETE", "G4WRITE ERROR"])
    text = write_run["text"].replace("\r\n", "\n")
    expected_hash = sector_hash(PATTERN)
    assert_true("G4WRITE ERROR" not in text, "writer reported error")
    assert_true(f"WRITE OK lba=00000008 words=00000100 flush=00000001\n" in text,
                "writer receipt mismatch")
    assert_true(f"READBACK OK hash={expected_hash:08X}\n" in text, "writer readback hash mismatch")
    assert_true("G4 DURABILITY WRITE COMPLETE\n" in text, "writer completion missing")
    stats = write_run["stats"]
    assert_true(stats["wr_bytes"] >= 512, "QEMU saw no sector write")
    assert_true(stats["wr_operations"] >= 1, "QEMU saw no write operation")
    assert_true(stats["flush_operations"] >= 1, "QEMU saw no guest flush operation")
    assert_true(stats["failed_wr_operations"] == 0, "QEMU recorded failed write")
    after_write = disk.read_bytes()
    assert_true(after_write[:SCRATCH_LBA*SECTOR] == before[:SCRATCH_LBA*SECTOR], "bytes before scratch LBA changed")
    assert_true(after_write[(SCRATCH_LBA+1)*SECTOR:] == before[(SCRATCH_LBA+1)*SECTOR:], "bytes after scratch LBA changed")
    assert_true(after_write[SCRATCH_LBA*SECTOR:(SCRATCH_LBA+1)*SECTOR] == PATTERN, "scratch sector mismatch")
    after_write_sha = sha_bytes(after_write)
    assert_true(after_write_sha != before_sha, "writer did not mutate disposable image")
    read_run = run_vm(read_build["iso"], ram, disk, out / "reboot-read",
                      "DBYTE V13 G4 DURABILITY REBOOT",
                      ["G4 DURABILITY REBOOT READ COMPLETE", "G4READ ERROR"])
    read_text = read_run["text"].replace("\r\n", "\n")
    assert_true("G4READ ERROR" not in read_text, "reboot reader reported error")
    assert_true(f"REBOOT READ OK lba=00000008 hash={expected_hash:08X}\n" in read_text,
                "reboot read receipt mismatch")
    assert_true("G4 DURABILITY REBOOT READ COMPLETE\n" in read_text, "reboot completion missing")
    assert_true(sha(disk) == after_write_sha, "read-only reboot mutated disk")
    return {"opt": write_build["opt"], "ram": ram, "kind": "positive",
            "before_sha256": before_sha, "after_write_sha256": after_write_sha,
            "after_reboot_sha256": sha(disk), "pattern_sha256": sha_bytes(PATTERN),
            "pattern_hash": f"{expected_hash:08X}", "writer_stats": stats,
            "write_serial_sha256": write_run["serial_sha256"],
            "write_trace_sha256": write_run["trace_sha256"],
            "read_serial_sha256": read_run["serial_sha256"],
            "read_trace_sha256": read_run["trace_sha256"]}

def negative_case(write_build, kind):
    out = ROOT / f"run-o{write_build['opt']}-64m-{kind}"
    out.mkdir()
    disk = out / "test.raw"
    make_fixture(disk, kind)
    before = disk.read_bytes()
    expected_error = "0000002A" if kind == "bad-guard" else "0000002D"
    run_result = run_vm(write_build["iso"], 64, disk, out / "write-boot",
                        "DBYTE V13 G4 DURABILITY WRITE",
                        ["G4WRITE ERROR", "G4 DURABILITY WRITE COMPLETE"])
    text = run_result["text"].replace("\r\n", "\n")
    assert_true(f"G4WRITE ERROR arm={expected_error}\n" in text, f"{kind} rejection mismatch")
    assert_true("G4 DURABILITY WRITE COMPLETE" not in text, f"{kind} falsely completed")
    assert_true(disk.read_bytes() == before, f"{kind} mutated disposable image")
    stats = run_result["stats"]
    assert_true(stats["wr_bytes"] == 0, f"{kind} performed a write")
    assert_true(stats["wr_operations"] == 0, f"{kind} performed a write operation")
    assert_true(stats["flush_operations"] == 0, f"{kind} performed a flush")
    return {"opt": write_build["opt"], "ram": 64, "kind": kind,
            "disk_sha256": sha(disk), "stats": stats,
            "serial_sha256": run_result["serial_sha256"],
            "trace_sha256": run_result["trace_sha256"]}

def main():
    head = run(["git.exe", "rev-parse", "HEAD"], "HEAD").strip()
    assert_true(head == EXPECTED_HEAD, "unexpected HEAD")
    tag = run(["git.exe", "rev-parse", "v12.4.0^{}"], "frozen tag").strip()
    assert_true(tag == FROZEN_BASE_HEAD, "v12.4.0 tag drifted")
    source_guard()
    run(["cargo.exe", "build", "-q", "-p", "dbyte_cli", "--bin", "dbytec"], "fresh compiler")
    g3 = run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
              str(REPO / "scripts/verify_v13_generation3.ps1")], "fresh G3 predecessor")
    (ROOT / "g3-predecessor.txt").write_text(g3, encoding="utf-8")
    epoch = int(run(["git.exe", "show", "-s", "--format=%ct", "HEAD"], "epoch").strip())
    date = time.strftime("%Y%m%d%H%M%S00", time.gmtime(epoch))
    manifest = source_manifest()
    (ROOT / "input-manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    builds = []
    for opt in ("0", "1"):
        for stem, source in (("write", "examples/native_v13_machine/kernel_g4_durability_write.dby"),
                             ("read", "examples/native_v13_machine/kernel_g4_durability_read.dby")):
            a = build_iso("A", opt, source, stem, date)
            b = build_iso("B", opt, source, stem, date)
            assert_true(a["obj_sha256"] == b["obj_sha256"], f"O{opt} {stem} object nondeterministic")
            assert_true(a["elf_sha256"] == b["elf_sha256"], f"O{opt} {stem} ELF nondeterministic")
            assert_true(a["iso_sha256"] == b["iso_sha256"], f"O{opt} {stem} ISO nondeterministic")
            builds.extend([a, b])
    (ROOT / "builds.json").write_text(json.dumps(builds, indent=2), encoding="utf-8")
    receipts = []
    for opt in ("0", "1"):
        write_build = next(x for x in builds if x["label"] == "A" and x["opt"] == opt and x["stem"] == "write")
        read_build = next(x for x in builds if x["label"] == "A" and x["opt"] == opt and x["stem"] == "read")
        for ram in (32, 64):
            receipts.append(positive_pair(write_build, read_build, ram))
        receipts.append(negative_case(write_build, "bad-guard"))
        receipts.append(negative_case(write_build, "dirty-scratch"))
    (ROOT / "qemu-receipts.json").write_text(json.dumps(receipts, indent=2), encoding="utf-8")
    run(["git.exe", "diff", "--check"], "diff check")
    tracked = run(["git.exe", "diff", "--binary"], "tracked diff")
    (ROOT / "tracked.diff").write_text(tracked, encoding="utf-8")
    selected_write = next(x for x in builds if x["label"] == "A" and x["opt"] == "1" and x["stem"] == "write")
    selected_read = next(x for x in builds if x["label"] == "A" and x["opt"] == "1" and x["stem"] == "read")
    result = {
        "status": "PASS", "generation": "v13-g4-virtual-ata-durability-qemu", "head": head,
        "compiler_sha256": sha(COMPILER), "manifest_sha256": sha(ROOT / "input-manifest.json"),
        "selected_write_object_sha256": selected_write["obj_sha256"],
        "selected_write_elf_sha256": selected_write["elf_sha256"],
        "selected_write_iso_sha256": selected_write["iso_sha256"],
        "selected_read_object_sha256": selected_read["obj_sha256"],
        "selected_read_elf_sha256": selected_read["elf_sha256"],
        "selected_read_iso_sha256": selected_read["iso_sha256"],
        "pattern_sha256": sha_bytes(PATTERN), "pattern_hash": f"{sector_hash(PATTERN):08X}",
        "qemu_receipts": len(receipts),
        "positive_pairs": sum(1 for x in receipts if x["kind"] == "positive"),
        "negative_receipts": sum(1 for x in receipts if x["kind"] != "positive"),
        "cache_mode": "writeback", "guest_flush_required": True,
        "g3_predecessor": "PASS", "write_scope": "disposable raw 8MiB fixture LBA8 only",
        "production_read_path": "READ_ONLY", "real_biostar": "PENDING",
        "real_storage": "DISARMED", "dbfs_writable": "PENDING", "v13_final": "NOT_CREATED"
    }
    (ROOT / "RESULT.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print("V13 GENERATION 4 VIRTUAL ATA DURABILITY VERIFIER: PASS")
    print("EVIDENCE ROOT:", ROOT)
    print("PATTERN HASH:", result["pattern_hash"])
    print("QEMU RECEIPTS:", len(receipts))

if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("V13 GENERATION 4 VIRTUAL ATA DURABILITY VERIFIER: FAIL")
        print("EVIDENCE ROOT:", ROOT)
        raise
