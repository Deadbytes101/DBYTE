from pathlib import Path
import hashlib, json, socket, subprocess, time, uuid

from v13_release_identity import FROZEN_BASE_HEAD, expected_head

REPO = Path(__file__).resolve().parent.parent
COMPILER = REPO / "target" / "debug" / "dbytec.exe"
QEMU = Path(r"C:\Program Files\qemu\qemu-system-x86_64.exe")
EXPECTED_HEAD = expected_head()
ROOT = REPO / "tmp" / "v13_g4" / ("dbfs-persist-" + uuid.uuid4().hex)
ROOT.mkdir(parents=True)
COMMAND_INDEX = 0
SECTOR = 512
DISK_BYTES = 8 * 1024 * 1024
FS_GUARD_LBA = 63
INODE_LBA = 69
DATA_LBA = 85
FS_GUARD = b"DBYTEV13DBFSRW01"
OLD_PAYLOAD = b"DBYTE V13 DBFS1 READONLY\n"
NEW_PAYLOAD = bytes((((i * 37) + 0x41) & 0xff) for i in range(128))
NEW_SECTOR = NEW_PAYLOAD + bytes(SECTOR - len(NEW_PAYLOAD))

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest().upper()

def sha_bytes(data):
    return hashlib.sha256(data).hexdigest().upper()

def fnv1a(data):
    h = 2166136261
    for value in data:
        h = ((h ^ value) * 16777619) & 0xffffffff
    return h

def assert_true(value, message):
    if not value:
        raise RuntimeError(message)

def run(args, description, cwd=REPO):
    global COMMAND_INDEX
    COMMAND_INDEX += 1
    p = subprocess.run([str(x) for x in args], cwd=cwd, text=True,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = ROOT / f"command-{COMMAND_INDEX:03d}.txt"
    log.write_text(
        f"CWD={cwd}\nCOMMAND={' '.join(map(str,args))}\nEXIT={p.returncode}\n{p.stdout}",
        encoding="utf-8")
    if p.returncode:
        raise RuntimeError(f"{description} failed ({p.returncode}): {log}")
    return p.stdout

def wslpath(path):
    return run(["wsl.exe", "-d", "Debian", "--", "wslpath", "-a", "-u",
                str(path).replace("\\", "/")], "translate WSL path").strip()

def put_u32(buf, offset, value):
    buf[offset:offset + 4] = int(value & 0xffffffff).to_bytes(4, "little")

def source_guard():
    root = REPO / "examples/native_v13_machine"
    storage = (root / "storage_ide.dby").read_text(encoding="utf-8")
    sector = (root / "sector_service.dby").read_text(encoding="utf-8")
    dbfsro = (root / "dbfs1_ro.dby").read_text(encoding="utf-8")
    vfsro = (root / "vfs_ro.dby").read_text(encoding="utf-8")
    writer = (root / "storage_ide_write_test.dby").read_text(encoding="utf-8")
    dbfsrw = (root / "dbfs1_rw_test.dby").read_text(encoding="utf-8")
    vfsrw = (root / "vfs_rw_test.dby").read_text(encoding="utf-8")
    kernel = (root / "kernel_g4_dbfs_persist_write.dby").read_text(encoding="utf-8")
    assert_true("native.io_out16(" not in storage, "production storage gained OUT16")
    assert_true("COMMAND_WRITE" not in storage and "FLUSH_CACHE" not in storage,
                "production storage gained media write")
    assert_true("write" not in sector.lower() and "flush" not in sector.lower(),
                "sector service escaped read-only boundary")
    assert_true("storage_ide_write_test" not in dbfsro and "write" not in vfsro.lower(),
                "read-only filesystem boundary changed")
    assert_true('const FS_GUARD_LBA: u32 = 63 as u32' in writer, "filesystem guard LBA drifted")
    assert_true('const FS_FIRST_LBA: u32 = 85 as u32' in writer, "filesystem first LBA drifted")
    assert_true('const FS_SECOND_LBA: u32 = 69 as u32' in writer, "filesystem second LBA drifted")
    assert_true("pub fn write_sector" not in writer, "generic ATA write API exposed")
    assert_true('import "dbfs1_ro.dby" as dbfs' in dbfsrw, "RW DBFS not layered on DBFS")
    assert_true("native.io_" not in dbfsrw, "RW DBFS bypasses storage boundary")
    assert_true('import "vfs_ro.dby" as ro' in vfsrw, "RW VFS does not retain RO facade")
    assert_true("storage_ide" not in vfsrw and "native.io_" not in vfsrw,
                "RW VFS leaks hardware boundary")
    assert_true("vfsrw.replace(" in kernel, "kernel bypasses writable VFS")
    assert_true("storage_ide" not in kernel and "write_test" not in kernel,
                "kernel bypasses filesystem write layer")

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
    disk[FS_GUARD_LBA*SECTOR:FS_GUARD_LBA*SECTOR+len(FS_GUARD)] = FS_GUARD
    super_off = 64 * SECTOR
    disk[super_off:super_off+5] = b"DBFS1"
    put_u32(disk, super_off+8, 1); put_u32(disk, super_off+12, 512)
    put_u32(disk, super_off+16, 64); put_u32(disk, super_off+20, 16320)
    put_u32(disk, super_off+24, 65); put_u32(disk, super_off+28, 4)
    put_u32(disk, super_off+32, 69); put_u32(disk, super_off+36, 64)
    put_u32(disk, super_off+40, 8); put_u32(disk, super_off+44, 77)
    put_u32(disk, super_off+48, 16307); put_u32(disk, super_off+52, 1)
    put_u32(disk, super_off+56, 64); put_u32(disk, super_off+60, fnv1a(disk[super_off:super_off+60]))
    bitmap = 65 * SECTOR
    disk[bitmap] = 0xff
    disk[bitmap+1] = 0x01
    inode_base = 69 * SECTOR
    root_inode = inode_base + 64
    put_u32(disk, root_inode, 1); put_u32(disk, root_inode+4, 2)
    put_u32(disk, root_inode+8, 4096); put_u32(disk, root_inode+12, 8)
    for index in range(8):
        put_u32(disk, root_inode+16+(index*4), index)
    file_inode = inode_base + 128
    put_u32(disk, file_inode, 1); put_u32(disk, file_inode+4, 1)
    put_u32(disk, file_inode+8, 25); put_u32(disk, file_inode+12, 1)
    put_u32(disk, file_inode+16, 8)
    for index in range(1, 8):
        put_u32(disk, file_inode+16+(index*4), 0xffffffff)
    directory = 77 * SECTOR
    put_u32(disk, directory, 2); put_u32(disk, directory+4, 1)
    put_u32(disk, directory+8, 9)
    disk[directory+12:directory+21] = b"HELLO.TXT"
    disk[DATA_LBA*SECTOR:DATA_LBA*SECTOR+len(OLD_PAYLOAD)] = OLD_PAYLOAD
    if kind == "bad-guard":
        disk[FS_GUARD_LBA*SECTOR] ^= 1
    elif kind == "baseline-mismatch":
        put_u32(disk, file_inode+8, 24)
    elif kind != "valid":
        raise RuntimeError(f"unknown fixture {kind}")
    path.write_bytes(disk)
    return {"path": str(path), "kind": kind, "sha256": sha(path)}

def expected_after(before):
    out = bytearray(before)
    out[DATA_LBA*SECTOR:(DATA_LBA+1)*SECTOR] = NEW_SECTOR
    put_u32(out, INODE_LBA*SECTOR + 128 + 8, len(NEW_PAYLOAD))
    return bytes(out)

def build_iso(label, opt, source, stem, date):
    out = ROOT / f"{label}-o{opt}-{stem}"
    out.mkdir()
    obj = out / f"{stem}.o"
    elf = out / f"{stem}.elf"
    base = [COMPILER, "build", source, "--target", "x86_64-dbyte-none", "--opt-level", opt]
    run(base + ["--emit", "obj", "-o", obj], f"build {stem} {label} O{opt} object")
    run(base + ["--emit", "elf", "-o", elf], f"build {stem} {label} O{opt} ELF")
    run(["wsl.exe", "-d", "Debian", "--", "grub-file", "--is-x86-multiboot2", wslpath(elf)],
        f"grub-file {stem}")
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
    if stem == "dbfs-write":
        normalized = " ".join(dis.replace("\t", " ").split())
        assert_true("out dx,ax" in normalized, "OUT16 absent from DBFS writer object")
    return {"label": label, "opt": opt, "stem": stem, "obj": str(obj),
            "elf": str(elf), "iso": str(iso), "obj_sha256": sha(obj),
            "elf_sha256": sha(elf), "iso_sha256": sha(iso), "iso_bytes": iso.stat().st_size}

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
    raise RuntimeError(f"serial timeout {needles}; qemu={process.poll()}; serial={read_shared(serial)}")

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
            try:
                file.close()
            except Exception:
                pass
        if sock:
            try:
                sock.close()
            except Exception:
                pass
        if process.poll() is None:
            process.kill()
            process.wait()

def positive_pair(write_build, read_build, ram):
    out = ROOT / f"run-o{write_build['opt']}-{ram}m-positive"
    out.mkdir()
    disk = out / "test.raw"
    make_fixture(disk, "valid")
    before = disk.read_bytes()
    expected = expected_after(before)
    before_sha = sha_bytes(before)
    expected_sha = sha_bytes(expected)
    write_run = run_vm(write_build["iso"], ram, disk, out / "write-boot",
                       "DBYTE V13 G4 DBFS PERSIST WRITE",
                       ["G4 DBFS PERSIST WRITE COMPLETE", "G4DBFSW ERROR"])
    text = write_run["text"].replace("\r\n", "\n")
    new_hash = fnv1a(NEW_PAYLOAD)
    assert_true("G4DBFSW ERROR" not in text, "DBFS writer reported error")
    assert_true("DBFS MUTATE OK inode=00000002 old=00000019 new=00000080\n" in text,
                "DBFS mutation receipt mismatch")
    assert_true("DBFS WRITE OK words=00000200 flush=00000001\n" in text,
                "DBFS write counters mismatch")
    assert_true(f"DBFS READBACK OK bytes=00000080 hash={new_hash:08X}\n" in text,
                "DBFS same-boot readback mismatch")
    assert_true("G4 DBFS PERSIST WRITE COMPLETE\n" in text, "DBFS write completion missing")
    stats = write_run["stats"]
    assert_true(stats["wr_bytes"] == 1024, "QEMU write bytes not exactly two sectors")
    assert_true(stats["wr_operations"] == 2, "QEMU write operations not exactly two")
    assert_true(stats["flush_operations"] == 1, "QEMU flush count not exactly one")
    assert_true(stats["failed_wr_operations"] == 0, "QEMU recorded failed write")
    observed = disk.read_bytes()
    assert_true(observed == expected, "host byte oracle mismatch after DBFS mutation")
    assert_true(sha_bytes(observed) == expected_sha, "expected image SHA mismatch")
    read_run = run_vm(read_build["iso"], ram, disk, out / "reboot-read",
                      "DBYTE V13 G4 DBFS PERSIST REBOOT",
                      ["G4 DBFS PERSIST REBOOT COMPLETE", "G4DBFSR ERROR"])
    read_text = read_run["text"].replace("\r\n", "\n")
    assert_true("G4DBFSR ERROR" not in read_text, "DBFS reboot reader reported error")
    assert_true("DBFS REMOUNT OK\n" in read_text, "DBFS remount receipt missing")
    assert_true("DBFS STAT OK inode=00000002 size=00000080\n" in read_text,
                "DBFS reboot stat mismatch")
    assert_true(f"DBFS READ OK bytes=00000080 hash={new_hash:08X}\n" in read_text,
                "DBFS reboot read mismatch")
    assert_true("G4 DBFS PERSIST REBOOT COMPLETE\n" in read_text,
                "DBFS reboot completion missing")
    assert_true(read_run["stats"]["wr_bytes"] == 0, "reboot reader wrote disk bytes")
    assert_true(sha(disk) == expected_sha, "reboot reader mutated persistent image")
    return {"opt": write_build["opt"], "ram": ram, "kind": "positive",
            "before_sha256": before_sha, "after_write_sha256": expected_sha,
            "after_reboot_sha256": sha(disk), "payload_sha256": sha_bytes(NEW_PAYLOAD),
            "payload_hash": f"{new_hash:08X}", "writer_stats": stats,
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
    run_result = run_vm(write_build["iso"], 64, disk, out / "write-boot",
                        "DBYTE V13 G4 DBFS PERSIST WRITE",
                        ["G4DBFSW ERROR", "G4 DBFS PERSIST WRITE COMPLETE"])
    text = run_result["text"].replace("\r\n", "\n")
    expected = "00000034" if kind == "bad-guard" else "00000048"
    label = "replace" if kind == "bad-guard" else "baseline"
    assert_true(f"G4DBFSW ERROR {label}={expected}\n" in text,
                f"{kind} rejection mismatch")
    assert_true("G4 DBFS PERSIST WRITE COMPLETE" not in text,
                f"{kind} falsely completed")
    assert_true(disk.read_bytes() == before, f"{kind} mutated disposable image")
    stats = run_result["stats"]
    assert_true(stats["wr_bytes"] == 0, f"{kind} performed a write")
    assert_true(stats["wr_operations"] == 0, f"{kind} performed a write operation")
    assert_true(stats["flush_operations"] == 0, f"{kind} performed a flush")
    return {"opt": write_build["opt"], "ram": 64, "kind": kind,
            "disk_sha256": sha(disk), "stats": stats,
            "serial_sha256": run_result["serial_sha256"],
            "trace_sha256": run_result["trace_sha256"]}

def run_raw_predecessor():
    output = run(["python.exe", str(REPO / "scripts/verify_v13_generation4_durability.py")],
                 "fresh raw durability predecessor")
    (ROOT / "raw-durability-predecessor.txt").write_text(output, encoding="utf-8")
    marker = "EVIDENCE ROOT:"
    line = next((x for x in output.splitlines() if x.startswith(marker)), None)
    assert_true(line is not None, "raw durability predecessor evidence root missing")
    evidence = Path(line[len(marker):].strip())
    result_path = evidence / "RESULT.json"
    result = json.loads(result_path.read_text(encoding="utf-8"))
    assert_true(result.get("status") == "PASS", "raw durability predecessor not PASS")
    assert_true(result.get("generation") == "v13-g4-virtual-ata-durability-qemu",
                "wrong raw durability predecessor generation")
    assert_true(result.get("real_storage") == "DISARMED", "raw predecessor armed real storage")
    return {"root": str(evidence), "result_sha256": sha(result_path)}

def main():
    head = run(["git.exe", "rev-parse", "HEAD"], "HEAD").strip()
    assert_true(head == EXPECTED_HEAD, "unexpected HEAD")
    tag = run(["git.exe", "rev-parse", "v12.4.0^{}"], "frozen tag").strip()
    assert_true(tag == FROZEN_BASE_HEAD, "v12.4.0 tag drifted")
    source_guard()
    assert_true(len(FS_GUARD) == 16, "filesystem guard length drift")
    assert_true(len(OLD_PAYLOAD) == 25, "old payload length drift")
    assert_true(fnv1a(OLD_PAYLOAD) == 0x77C07A2B, "old payload oracle drift")
    assert_true(len(NEW_PAYLOAD) == 128, "new payload length drift")
    raw_predecessor = run_raw_predecessor()
    epoch = int(run(["git.exe", "show", "-s", "--format=%ct", "HEAD"], "epoch").strip())
    date = time.strftime("%Y%m%d%H%M%S00", time.gmtime(epoch))
    manifest = source_manifest()
    (ROOT / "input-manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    fixture_probe = ROOT / "fixture-probe.raw"
    make_fixture(fixture_probe, "valid")
    before_probe = fixture_probe.read_bytes()
    after_probe = expected_after(before_probe)
    changed_sectors = sorted({i // SECTOR for i, (a, b) in enumerate(zip(before_probe, after_probe)) if a != b})
    assert_true(changed_sectors == [69, 85], f"host oracle changed unexpected sectors {changed_sectors}")
    builds = []
    for opt in ("0", "1"):
        for stem, source in (
            ("dbfs-write", "examples/native_v13_machine/kernel_g4_dbfs_persist_write.dby"),
            ("dbfs-read", "examples/native_v13_machine/kernel_g4_dbfs_persist_read.dby")):
            a = build_iso("A", opt, source, stem, date)
            b = build_iso("B", opt, source, stem, date)
            assert_true(a["obj_sha256"] == b["obj_sha256"], f"O{opt} {stem} object nondeterministic")
            assert_true(a["elf_sha256"] == b["elf_sha256"], f"O{opt} {stem} ELF nondeterministic")
            assert_true(a["iso_sha256"] == b["iso_sha256"], f"O{opt} {stem} ISO nondeterministic")
            builds.extend([a, b])
    (ROOT / "builds.json").write_text(json.dumps(builds, indent=2), encoding="utf-8")
    receipts = []
    for opt in ("0", "1"):
        write_build = next(x for x in builds if x["label"] == "A" and x["opt"] == opt and x["stem"] == "dbfs-write")
        read_build = next(x for x in builds if x["label"] == "A" and x["opt"] == opt and x["stem"] == "dbfs-read")
        for ram in (32, 64):
            receipts.append(positive_pair(write_build, read_build, ram))
        receipts.append(negative_case(write_build, "bad-guard"))
        receipts.append(negative_case(write_build, "baseline-mismatch"))
    (ROOT / "qemu-receipts.json").write_text(json.dumps(receipts, indent=2), encoding="utf-8")
    run(["git.exe", "diff", "--check"], "diff check")
    tracked = run(["git.exe", "diff", "--binary"], "tracked diff")
    (ROOT / "tracked.diff").write_text(tracked, encoding="utf-8")
    selected_write = next(x for x in builds if x["label"] == "A" and x["opt"] == "1" and x["stem"] == "dbfs-write")
    selected_read = next(x for x in builds if x["label"] == "A" and x["opt"] == "1" and x["stem"] == "dbfs-read")
    result = {
        "status": "PASS", "generation": "v13-g4-dbfs-persistence-qemu", "head": head,
        "compiler_sha256": sha(COMPILER), "manifest_sha256": sha(ROOT / "input-manifest.json"),
        "selected_write_object_sha256": selected_write["obj_sha256"],
        "selected_write_elf_sha256": selected_write["elf_sha256"],
        "selected_write_iso_sha256": selected_write["iso_sha256"],
        "selected_read_object_sha256": selected_read["obj_sha256"],
        "selected_read_elf_sha256": selected_read["elf_sha256"],
        "selected_read_iso_sha256": selected_read["iso_sha256"],
        "old_payload_hash": f"{fnv1a(OLD_PAYLOAD):08X}",
        "new_payload_hash": f"{fnv1a(NEW_PAYLOAD):08X}",
        "new_payload_sha256": sha_bytes(NEW_PAYLOAD), "changed_sectors": changed_sectors,
        "qemu_receipts": len(receipts),
        "positive_pairs": sum(1 for x in receipts if x["kind"] == "positive"),
        "negative_receipts": sum(1 for x in receipts if x["kind"] != "positive"),
        "raw_durability_predecessor": "PASS", "raw_predecessor": raw_predecessor,
        "write_path": "VFS RW test -> DBFS existing-file replace -> guarded ATA PIO -> FLUSH",
        "cache_mode": "writeback", "real_biostar": "PENDING", "real_storage": "DISARMED",
        "g5": "PENDING", "v13_final": "NOT_CREATED"
    }
    (ROOT / "RESULT.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print("V13 GENERATION 4 DBFS PERSISTENCE VERIFIER: PASS")
    print("EVIDENCE ROOT:", ROOT)
    print("NEW PAYLOAD HASH:", result["new_payload_hash"])
    print("QEMU RECEIPTS:", len(receipts))

if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("V13 GENERATION 4 DBFS PERSISTENCE VERIFIER: FAIL")
        print("EVIDENCE ROOT:", ROOT)
        raise