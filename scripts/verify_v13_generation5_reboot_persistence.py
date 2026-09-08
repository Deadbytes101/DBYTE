from pathlib import Path
import hashlib, json, re, shutil, socket, struct, subprocess, time, uuid

from v13_release_identity import FROZEN_BASE_HEAD, expected_head

REPO = Path(__file__).resolve().parent.parent
COMPILER = REPO / "target/debug/dbytec.exe"
QEMU = Path(r"C:\Program Files\qemu\qemu-system-x86_64.exe")
EXPECTED_HEAD = expected_head()
ROOT = REPO / "tmp/v13_g5" / ("reboot-persist-" + uuid.uuid4().hex)
ROOT.mkdir(parents=True)
COMMAND_INDEX = 0
SECTOR = 512
DISK_BYTES = 8 * 1024 * 1024
GUARD = b"DBYTEV13DBFSRW01"
OLD_PAYLOAD = b"DBYTE V13 DBFS1 READONLY\n"
NEW_PAYLOAD = bytes((((i * 37) + 0x41) & 0xff) for i in range(128))
NEW_SECTOR = NEW_PAYLOAD + bytes(SECTOR - 128)

def assert_true(value, message):
    if not value:
        raise RuntimeError(message)

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest().upper()

def sha_bytes(data):
    return hashlib.sha256(data).hexdigest().upper()
def fnv1a(data):
    value = 0x811C9DC5
    for byte in data:
        value = ((value ^ byte) * 0x01000193) & 0xffffffff
    return value

def put_u32(buf, offset, value):
    struct.pack_into("<I", buf, offset, value)

def run(args, description, expect=0):
    global COMMAND_INDEX
    COMMAND_INDEX += 1
    proc = subprocess.run([str(x) for x in args], cwd=REPO, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = ROOT / f"command-{COMMAND_INDEX:03d}.txt"
    log.write_text(
        f"COMMAND={' '.join(map(str,args))}\nEXIT={proc.returncode}\n{proc.stdout}",
        encoding="utf-8")
    if proc.returncode != expect:
        raise RuntimeError(f"{description} exit {proc.returncode}, expected {expect}: {log}")
    return proc.stdout

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
    root = REPO / "examples/native_v13_machine"
    storage = (root / "storage_ide.dby").read_text(encoding="utf-8")
    sector = (root / "sector_service.dby").read_text(encoding="utf-8")
    writer = (root / "storage_ide_write_test.dby").read_text(encoding="utf-8")
    kernel = (root / "kernel_g5_persistence.dby").read_text(encoding="utf-8")
    probe = (root / "programs64/persistence_probe.dby").read_text(encoding="utf-8")
    assert_true("native.io_out16(" not in storage and "COMMAND_WRITE" not in storage,
                "production storage escaped read-only boundary")
    assert_true("write" not in sector.lower() and "flush" not in sector.lower(),
                "sector service escaped read-only boundary")
    assert_true("pub fn write_sector" not in writer, "generic ATA write API exposed")
    assert_true('file.load("/PERSIST.ELF", 12 as usize)' in kernel,
                "G5 persistence does not load disk ELF")
    assert_true("storage_ide_write_test" not in kernel and "vfs_rw_test" not in kernel,
                "G5 persistence reader gained write path")
    assert_true('let path: str = "/HELLO.TXT"' in probe,
                "persistence probe path drifted")
    assert_true("expected_byte(index)" in probe and "count != 128" in probe,
                "persistence user oracle missing")
def make_fixture(path, user_image):
    disk = bytearray(DISK_BYTES)
    disk[63*SECTOR:63*SECTOR+len(GUARD)] = GUARD
    super_off = 64 * SECTOR
    disk[super_off:super_off+5] = b"DBFS1"
    for offset, value in ((8,1),(12,512),(16,64),(20,16320),(24,65),(28,4),
                          (32,69),(36,64),(40,8),(44,77),(48,16307),(52,1),(56,64)):
        put_u32(disk, super_off + offset, value)
    put_u32(disk, super_off + 60, fnv1a(disk[super_off:super_off+60]))
    bitmap = 65 * SECTOR
    disk[bitmap] = 0xff
    disk[bitmap + 1] = 0x01
    inode_base = 69 * SECTOR
    root_inode = inode_base + 64
    for offset, value in ((0,1),(4,2),(8,4096),(12,8)):
        put_u32(disk, root_inode + offset, value)
    for index in range(8):
        put_u32(disk, root_inode + 16 + 4*index, index)
    hello_inode = inode_base + 128
    for offset, value in ((0,1),(4,1),(8,25),(12,1),(16,8)):
        put_u32(disk, hello_inode + offset, value)
    for index in range(1,8):
        put_u32(disk, hello_inode + 16 + 4*index, 0xffffffff)
    directory = 77 * SECTOR
    for offset, value in ((0,2),(4,1),(8,9)):
        put_u32(disk, directory + offset, value)
    disk[directory+12:directory+21] = b"HELLO.TXT"
    disk[85*SECTOR:85*SECTOR+len(OLD_PAYLOAD)] = OLD_PAYLOAD
    blocks = (len(user_image) + 511) // 512
    assert_true(64 <= len(user_image) <= 4096 and blocks <= 8,
                "persistence user ELF outside DBFS1 limits")
    user_inode = inode_base + 3 * 64
    for offset, value in ((0,1),(4,1),(8,len(user_image)),(12,blocks)):
        put_u32(disk, user_inode + offset, value)
    for index in range(8):
        put_u32(disk, user_inode + 16 + 4*index,
                9 + index if index < blocks else 0xffffffff)
    for block in range(9, 9 + blocks):
        disk[bitmap + block // 8] |= 1 << (block % 8)
    entry = directory + 64
    for offset, value in ((0,3),(4,1),(8,11)):
        put_u32(disk, entry + offset, value)
    disk[entry+12:entry+23] = b"PERSIST.ELF"
    start = (77 + 9) * SECTOR
    disk[start:start+len(user_image)] = user_image
    path.write_bytes(disk)
    return path
def expected_after(before):
    out = bytearray(before)
    out[85*SECTOR:86*SECTOR] = NEW_SECTOR
    put_u32(out, 69*SECTOR + 128 + 8, 128)
    return bytes(out)

def build_kernel_iso(label, opt, source, stem, date):
    out = ROOT / f"{label}-o{opt}-{stem}"
    out.mkdir()
    obj = out / f"{stem}.o"
    elf = out / f"{stem}.elf"
    base = [COMPILER, "build", source, "--target", "x86_64-dbyte-none",
            "--opt-level", opt]
    run(base + ["--emit", "obj", "-o", obj], f"{stem} {label} O{opt} object")
    run(base + ["--emit", "elf", "-o", elf], f"{stem} {label} O{opt} ELF")
    tree = out / "iso"
    grub = tree / "boot/grub"
    grub.mkdir(parents=True)
    (tree / "boot/kernel.elf").write_bytes(elf.read_bytes())
    (grub / "grub.cfg").write_text(
        "set timeout=0\nset default=0\nmenuentry 'DByte G5 persist' {\n"
        " multiboot2 /boot/kernel.elf\n boot\n}\n", encoding="ascii")
    iso = out / f"{stem}.iso"
    run(["wsl.exe", "-d", "Debian", "--", "grub-mkrescue", "-o", wslpath(iso),
         f"--modification-date={date}", "--set_all_file_dates", date, wslpath(tree)],
        f"deterministic {stem} ISO")
    if stem == "dbfs-write":
        dis = run(["objdump.exe", "-d", "-Mintel", obj], "writer disassembly")
        assert_true("out    dx,ax" in dis or "out dx,ax" in " ".join(dis.split()),
                    "writer object missing OUT16")
    return {"label": label, "opt": opt, "stem": stem, "obj": str(obj),
            "elf": str(elf), "iso": str(iso), "obj_sha256": sha(obj),
            "elf_sha256": sha(elf), "iso_sha256": sha(iso),
            "iso_bytes": iso.stat().st_size}

def build_user(label, opt):
    out = ROOT / f"{label}-o{opt}-user"
    out.mkdir()
    elf = out / "persist.elf"
    run([COMPILER, "build",
         "examples/native_v13_machine/programs64/persistence_probe.dby",
         "--target", "x86_64-dbyte-user", "--opt-level", opt,
         "--emit", "elf", "-o", elf], f"persistence user {label} O{opt}")
    data = elf.read_bytes()
    assert_true(64 <= len(data) <= 4096, "persistence ELF outside DBFS1 limit")
    entry = struct.unpack_from("<Q", data, 24)[0]
    assert_true(entry == 0x4000B0, "persistence compact ELF entry drifted")
    return {"label": label, "opt": opt, "elf": str(elf),
            "sha256": sha(elf), "bytes": len(data)}
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
        response = json.loads(stream.readline())
        if response.get("id") == ident:
            if "error" in response:
                raise RuntimeError(f"QMP {name} error: {response['error']}")
            return response.get("return")
def block_stats(stream, ident=20):
    result = qmp_cmd(stream, "query-blockstats", ident=ident)
    stats = []
    def walk(value):
        if isinstance(value, dict):
            if isinstance(value.get("stats"), dict):
                stats.append(value["stats"])
            for child in value.values():
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)
    walk(result)
    return {
        "wr_bytes": max([int(s.get("wr_bytes",0)) for s in stats] or [0]),
        "wr_operations": max([int(s.get("wr_operations",0)) for s in stats] or [0]),
        "flush_operations": max([int(s.get("flush_operations",0)) for s in stats] or [0]),
        "failed_wr_operations": max([int(s.get("failed_wr_operations",0)) for s in stats] or [0]),
    }

def wait_serial(serial, needles, process, timeout=45):
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

def trace_check(trace, entry_resets, label):
    text = read_shared(trace)
    assert_true("Triple fault" not in text, f"{label} triple fault")
    for token in ("v=06", "v=0d", "v=0e"):
        assert_true(token not in text, f"{label} CPU fault observed: {token}")
    final_resets = text.count("CPU Reset")
    assert_true(entry_resets is not None, f"{label} reset baseline missing")
    assert_true(final_resets == entry_resets, f"{label} CPU reset drift")
    return final_resets

def qmp_quit(stream):
    request = {"execute": "quit", "id": 99}
    stream.write((json.dumps(request, separators=(",", ":")) + "\n").encode())
    while True:
        try:
            line = stream.readline()
        except OSError:
            return
        if not line:
            return
        response = json.loads(line)
        if response.get("id") == 99:
            if "error" in response:
                raise RuntimeError(f"QMP quit error: {response['error']}")
            return

def qemu_args(iso, ram, disk, serial, trace, port, cache_mode):
    return [str(QEMU), "-cdrom", str(iso), "-boot", "order=d", "-m", f"{ram}M",
            "-display", "none", "-monitor", "none", "-serial", f"file:{serial}",
            "-d", "int,cpu_reset,guest_errors", "-D", str(trace),
            "-no-reboot", "-no-shutdown",
            "-drive", f"file={disk},format=raw,if=ide,index=0,media=disk,cache={cache_mode}",
            "-qmp", f"tcp:127.0.0.1:{port},server=on,wait=off"]

def run_vm(iso, ram, disk, out, entry_needle, outcome_needles,
           cache_mode="directsync", timeout=60):
    out.mkdir()
    serial = out / "serial.log"
    trace = out / "trace.log"
    port = free_port()
    args = qemu_args(iso, ram, disk, serial, trace, port, cache_mode)
    (out / "qemu-command.txt").write_text(" ".join(args), encoding="utf-8")
    process = subprocess.Popen(args, cwd=REPO, stdout=subprocess.DEVNULL,
                               stderr=subprocess.PIPE, text=True)
    sock = stream = None
    entry_resets = None
    try:
        sock, stream = qmp_connect(port, process)
        qmp_cmd(stream, "qmp_capabilities", ident=10)
        wait_serial(serial, entry_needle, process, timeout=30)
        entry_resets = read_shared(trace).count("CPU Reset")
        text = wait_serial(serial, outcome_needles, process, timeout=timeout)
        stats = block_stats(stream, 20)
        qmp_quit(stream)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        final_resets = trace_check(trace, entry_resets,
                                   f"O? {ram}M {out.name}")
        return {"text": text, "stats": stats,
                "serial_sha256": sha(serial), "trace_sha256": sha(trace),
                "entry_resets": entry_resets, "final_resets": final_resets}
    finally:
        if stream:
            try:
                stream.close()
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

def positive_pair(write_build, read_build, user_build, ram):
    out = ROOT / f"run-o{write_build['opt']}-{ram}m-positive"
    out.mkdir()
    disk = out / "persist.raw"
    make_fixture(disk, Path(user_build["elf"]).read_bytes())
    before = disk.read_bytes()
    before_sha = sha_bytes(before)
    expected = expected_after(before)
    expected_sha = sha_bytes(expected)
    changed = sorted({i // SECTOR for i, (a, b) in enumerate(zip(before, expected)) if a != b})
    assert_true(changed == [69, 85], f"host oracle changed unexpected sectors {changed}")
    write_run = run_vm(write_build["iso"], ram, disk, out / "write-boot",
                       "DBYTE V13 G4 DBFS PERSIST WRITE",
                       ["G4 DBFS PERSIST WRITE COMPLETE", "G4DBFSW ERROR"],
                       cache_mode="writeback")
    text = write_run["text"].replace("\r\n", "\n")
    assert_true("G4DBFSW ERROR" not in text, "G4 writer reported error")
    new_hash = fnv1a(NEW_PAYLOAD)
    assert_true("DBFS MUTATE OK inode=00000002 old=00000019 new=00000080\n" in text,
                "G4 mutation receipt mismatch")
    assert_true("DBFS WRITE OK words=00000200 flush=00000001\n" in text,
                "G4 write/flush receipt mismatch")
    assert_true(f"DBFS READBACK OK bytes=00000080 hash={new_hash:08X}\n" in text,
                "G4 same-boot readback mismatch")
    stats = write_run["stats"]
    assert_true(stats["wr_bytes"] == 1024, "writer did not write exactly 1024 bytes")
    assert_true(stats["wr_operations"] == 2, "writer did not issue exactly two writes")
    assert_true(stats["flush_operations"] == 1, "writer did not issue exactly one flush")
    assert_true(stats["failed_wr_operations"] == 0, "writer recorded failed write")
    observed = disk.read_bytes()
    assert_true(observed == expected, "host exact-byte oracle mismatch after writer")
    assert_true(sha_bytes(observed) == expected_sha, "post-write disk hash mismatch")
    read_run = run_vm(read_build["iso"], ram, disk, out / "fresh-reboot",
                      "DBYTE V13 G5 REBOOT PERSISTENCE",
                      ["G5 REBOOT PERSISTENCE COMPLETE", "G5PERSIST "],
                      cache_mode="directsync")
    read_text = read_run["text"].replace("\r\n", "\n")
    assert_true("G5PERSIST " not in read_text, "G5 persistence reader reported error")
    for token in ("PERSIST USER START\n", "PERSIST USER OK\n",
                  "USER64 EXIT code=00000000\n",
                  "PERSIST STATS req=00000005 open=00000001 read=00000001 close=00000001 write=00000002 cleanup=00000001\n",
                  "G5 REBOOT PERSISTENCE COMPLETE\n"):
        assert_true(token in read_text, f"missing persistence receipt {token!r}")
    rstats = read_run["stats"]
    assert_true(rstats["wr_bytes"] == 0, "fresh G5 reader wrote disk bytes")
    assert_true(rstats["wr_operations"] == 0, "fresh G5 reader issued disk write")
    assert_true(rstats["flush_operations"] == 0, "fresh G5 reader issued disk flush")
    assert_true(rstats["failed_wr_operations"] == 0, "fresh G5 reader recorded failed write")
    assert_true(sha(disk) == expected_sha, "fresh G5 reader mutated persisted image")
    return {
        "opt": write_build["opt"], "ram": ram, "kind": "positive",
        "before_sha256": before_sha, "after_write_sha256": expected_sha,
        "after_fresh_reboot_sha256": sha(disk),
        "payload_sha256": sha_bytes(NEW_PAYLOAD), "payload_hash": f"{new_hash:08X}",
        "changed_sectors": changed, "writer_stats": stats, "reader_stats": rstats,
        "write_serial_sha256": write_run["serial_sha256"],
        "write_trace_sha256": write_run["trace_sha256"],
        "read_serial_sha256": read_run["serial_sha256"],
        "read_trace_sha256": read_run["trace_sha256"]}

def corruption_case(read_build, user_build):
    out = ROOT / f"run-o{read_build['opt']}-64m-corrupt-payload"
    out.mkdir()
    disk = out / "corrupt.raw"
    make_fixture(disk, Path(user_build["elf"]).read_bytes())
    data = expected_after(disk.read_bytes())
    disk.write_bytes(data)
    broken = bytearray(disk.read_bytes())
    broken[85 * SECTOR] ^= 0x01
    disk.write_bytes(broken)
    before = sha(disk)
    run_result = run_vm(read_build["iso"], 64, disk, out / "fresh-reboot",
                        "DBYTE V13 G5 REBOOT PERSISTENCE",
                        ["G5PERSIST EXIT CODE=", "G5 REBOOT PERSISTENCE COMPLETE"],
                        cache_mode="directsync")
    text = run_result["text"].replace("\r\n", "\n")
    assert_true("G5PERSIST EXIT CODE=0000000E\n" in text,
                "corrupted payload was not rejected by userspace oracle")
    assert_true("G5 REBOOT PERSISTENCE COMPLETE" not in text,
                "corrupted payload falsely completed")
    stats = run_result["stats"]
    assert_true(stats["wr_bytes"] == 0 and stats["wr_operations"] == 0,
                "corruption reader wrote disk")
    assert_true(stats["flush_operations"] == 0 and stats["failed_wr_operations"] == 0,
                "corruption reader flush/write failure counters nonzero")
    assert_true(sha(disk) == before, "corruption reader mutated fixture")
    return {"opt": read_build["opt"], "ram": 64, "kind": "corrupt-payload",
            "disk_sha256": before, "stats": stats,
            "serial_sha256": run_result["serial_sha256"],
            "trace_sha256": run_result["trace_sha256"],
            "expected_exit": 14}

def main():
    head = run(["git.exe", "rev-parse", "HEAD"], "HEAD").strip()
    assert_true(head == EXPECTED_HEAD, "unexpected HEAD")
    tag = run(["git.exe", "rev-parse", "v12.4.0^{}"], "frozen tag").strip()
    assert_true(tag == FROZEN_BASE_HEAD, "v12.4.0 tag drifted")
    source_guard()
    assert_true(len(GUARD) == 16, "filesystem guard length drift")
    assert_true(len(OLD_PAYLOAD) == 25, "baseline payload length drift")
    assert_true(len(NEW_PAYLOAD) == 128, "persisted payload length drift")
    run(["cargo.exe", "build", "-q", "-p", "dbyte_cli", "--bin", "dbytec"],
        "fresh compiler")
    epoch = int(run(["git.exe", "show", "-s", "--format=%ct", "HEAD"], "epoch").strip())
    date = time.strftime("%Y%m%d%H%M%S00", time.gmtime(epoch))
    manifest = source_manifest()
    manifest_path = ROOT / "input-manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    builds = []
    users = []
    for opt in ("0", "1"):
        for stem, source in (
            ("dbfs-write", "examples/native_v13_machine/kernel_g4_dbfs_persist_write.dby"),
            ("g5-read", "examples/native_v13_machine/kernel_g5_persistence.dby")):
            a = build_kernel_iso("A", opt, source, stem, date)
            b = build_kernel_iso("B", opt, source, stem, date)
            for key in ("obj_sha256", "elf_sha256", "iso_sha256"):
                assert_true(a[key] == b[key], f"O{opt} {stem} nondeterministic {key}")
            builds.extend([a, b])
        ua = build_user("A", opt)
        ub = build_user("B", opt)
        assert_true(ua["sha256"] == ub["sha256"], f"O{opt} persistence user nondeterministic")
        assert_true(ua["bytes"] == ub["bytes"], f"O{opt} persistence user size drift")
        users.extend([ua, ub])
    (ROOT / "builds.json").write_text(json.dumps(builds, indent=2), encoding="utf-8")
    (ROOT / "users.json").write_text(json.dumps(users, indent=2), encoding="utf-8")
    receipts = []
    negatives = []
    for opt in ("0", "1"):
        writer = next(x for x in builds if x["label"] == "A" and x["opt"] == opt and x["stem"] == "dbfs-write")
        reader = next(x for x in builds if x["label"] == "A" and x["opt"] == opt and x["stem"] == "g5-read")
        user = next(x for x in users if x["label"] == "A" and x["opt"] == opt)
        for ram in (32, 64):
            receipts.append(positive_pair(writer, reader, user, ram))
        negatives.append(corruption_case(reader, user))
    (ROOT / "qemu-receipts.json").write_text(json.dumps(receipts, indent=2), encoding="utf-8")
    (ROOT / "negative-receipts.json").write_text(json.dumps(negatives, indent=2), encoding="utf-8")
    run(["git.exe", "diff", "--check"], "diff check")
    tracked = run(["git.exe", "diff", "--binary"], "tracked diff")
    (ROOT / "tracked.diff").write_text(tracked, encoding="utf-8")
    selected_writer = next(x for x in builds if x["label"] == "A" and x["opt"] == "1" and x["stem"] == "dbfs-write")
    selected_reader = next(x for x in builds if x["label"] == "A" and x["opt"] == "1" and x["stem"] == "g5-read")
    selected_user = next(x for x in users if x["label"] == "A" and x["opt"] == "1")
    result = {
        "status": "PASS",
        "generation": "v13-g5-reboot-persistence-qemu",
        "head": head,
        "compiler_sha256": sha(COMPILER),
        "manifest_sha256": sha(manifest_path),
        "selected_writer_iso_sha256": selected_writer["iso_sha256"],
        "selected_reader_iso_sha256": selected_reader["iso_sha256"],
        "selected_user_elf_sha256": selected_user["sha256"],
        "payload_sha256": sha_bytes(NEW_PAYLOAD),
        "payload_hash": f"{fnv1a(NEW_PAYLOAD):08X}",
        "positive_receipts": len(receipts),
        "negative_receipts": len(negatives),
        "g4_dbfs_write_flush": "PASS_QEMU",
        "host_exact_byte_oracle": "PASS",
        "fresh_qemu_process_reboot": "PASS",
        "ring3_user_readback": "PASS_QEMU",
        "corruption_rejection": "PASS_QEMU",
        "write_cache_mode": "writeback",
        "read_cache_mode": "directsync",
        "interactive_shell_predecessor": "PASS_CURRENT_GENERATION_PREVIOUSLY_SEALED",
        "real_biostar": "PENDING",
        "real_storage": "DISARMED",
        "v13_final": "NOT_CREATED",
    }
    (ROOT / "RESULT.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print("V13 GENERATION 5 REBOOT PERSISTENCE VERIFIER: PASS")
    print("EVIDENCE ROOT:", ROOT)
    print("PAYLOAD HASH:", result["payload_hash"])
    print("POSITIVE RECEIPTS:", len(receipts))
    print("NEGATIVE RECEIPTS:", len(negatives))

if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("V13 GENERATION 5 REBOOT PERSISTENCE VERIFIER: FAIL")
        print("EVIDENCE ROOT:", ROOT)
        raise
