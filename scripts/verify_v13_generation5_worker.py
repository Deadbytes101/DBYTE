from pathlib import Path
import hashlib
import json
import re
import shutil
import struct
import subprocess
import time
import uuid

from v13_release_identity import FROZEN_BASE_HEAD, expected_head

REPO = Path(__file__).resolve().parent.parent
COMPILER = REPO / "target/debug/dbytec.exe"
QEMU = Path(r"C:\Program Files\qemu\qemu-system-x86_64.exe")
EXPECTED_HEAD = expected_head()
ROOT = REPO / "tmp/v13_g5" / ("worker-" + uuid.uuid4().hex)
ROOT.mkdir(parents=True)
COMMAND_INDEX = 0
KERNEL = "examples/native_v13_machine/kernel_g5_worker.dby"
USER = "examples/native_v13_machine/programs64/worker_probe.dby"
LEGACY_STR = "examples/native_v13_machine/legacy_str64_regression.dby"


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest().upper()


def assert_true(value, message):
    if not value:
        raise RuntimeError(message)
def run(args, description, expect=0):
    global COMMAND_INDEX
    COMMAND_INDEX += 1
    proc = subprocess.run([str(x) for x in args], cwd=REPO, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = ROOT / f"command-{COMMAND_INDEX:03d}.txt"
    log.write_text(
        f"COMMAND={' '.join(map(str, args))}\nEXIT={proc.returncode}\n{proc.stdout}",
        encoding="utf-8")
    if proc.returncode != expect:
        raise RuntimeError(
            f"{description} exit {proc.returncode}, expected {expect}: {log}")
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
    user = (REPO / "examples/native_v13_machine/user_space64.dby").read_text(encoding="utf-8")
    worker = (REPO / "examples/native_v13_machine/syscall_worker64.dby").read_text(encoding="utf-8")
    handles = (REPO / "examples/native_v13_machine/owner_qualified_fds64.dby").read_text(encoding="utf-8")
    vfs = (REPO / "examples/native_v13_machine/vfs_ro.dby").read_text(encoding="utf-8")
    hir = (REPO / "crates/dbyte_hir/src/lib.rs").read_text(encoding="utf-8")
    assert_true("REQUEST_RESUME: array[u64, 20]" in user, "saved worker resume image missing")
    assert_true("fn capture_request" in user, "worker request capture missing")
    assert_true("pub fn complete_request" in user, "worker completion resume missing")
    assert_true("native.cpu_write_cr3(KERNEL_CR3)" in user, "kernel CR3 switch missing")
    assert_true("import \"vfs_ro.dby\" as vfs" in worker, "worker does not use VFS")
    assert_true("vfs_rw_test" not in worker and "storage_ide_write_test" not in worker,
                "worker escaped read-only storage boundary")
    assert_true("WRONG_OWNER" in handles and "STALE_HANDLE" in handles,
                "owner-qualified handle rejection missing")
    assert_true("pub fn read_inode_at" in vfs, "FD inode read seam missing")
    assert_true("Profile::X86_64None" in hir and "Profile::X86_64User" in hir,
                "x86_64 legacy string profile support missing")


def predecessor_gate():
    output = run(["python.exe", str(REPO / "scripts/verify_v13_generation5_fault_containment.py")],
                 "fresh hostile-fault predecessor")
    assert_true("VERIFIER: PASS" in output, "fault containment predecessor not PASS")
    (ROOT / "fault-predecessor.txt").write_text(output, encoding="utf-8")
def fnv1a(data):
    value = 0x811C9DC5
    for byte in data:
        value = ((value ^ byte) * 0x01000193) & 0xFFFFFFFF
    return value


def put_u32(buffer, offset, value):
    struct.pack_into("<I", buffer, offset, value)


def make_fixture():
    disk = bytearray(8 * 1024 * 1024)
    super_off = 64 * 512
    disk[super_off:super_off + 5] = b"DBFS1"
    fields = [(8, 1), (12, 512), (16, 64), (20, 16320), (24, 65), (28, 4),
              (32, 69), (36, 64), (40, 8), (44, 77), (48, 16307),
              (52, 1), (56, 64)]
    for offset, value in fields:
        put_u32(disk, super_off + offset, value)
    put_u32(disk, super_off + 60, fnv1a(disk[super_off:super_off + 60]))
    bitmap = 65 * 512
    disk[bitmap] = 0xFF
    disk[bitmap + 1] = 0x01
    inode_base = 69 * 512
    root_inode = inode_base + 64
    put_u32(disk, root_inode, 1)
    put_u32(disk, root_inode + 4, 2)
    put_u32(disk, root_inode + 8, 4096)
    put_u32(disk, root_inode + 12, 8)
    for index in range(8):
        put_u32(disk, root_inode + 16 + (4 * index), index)
    file_inode = inode_base + 128
    put_u32(disk, file_inode, 1)
    put_u32(disk, file_inode + 4, 1)
    put_u32(disk, file_inode + 8, 128)
    put_u32(disk, file_inode + 12, 1)
    put_u32(disk, file_inode + 16, 8)
    for index in range(1, 8):
        put_u32(disk, file_inode + 16 + (4 * index), 0xFFFFFFFF)
    directory = 77 * 512
    put_u32(disk, directory, 2)
    put_u32(disk, directory + 4, 1)
    put_u32(disk, directory + 8, 11)
    disk[directory + 12:directory + 23] = b"PERSIST.TXT"
    payload = bytearray(128)
    payload[:8] = b"DBYTEFS1"
    payload[8] = 10
    payload[9:11] = b"42"
    payload[11] = 10
    for index in range(12, 128):
        payload[index] = (index * 31 + 0x5D) & 0xFF
    disk[85 * 512:(85 * 512) + 128] = payload
    path = ROOT / "worker-fixture.raw"
    path.write_bytes(disk)
    assert_true(sha(path) == "5AB4AB07E31C8ECBADA874185E13D27EC23A7E43CFD0E0C330F1613AFE9627CB",
                "worker DBFS fixture hash drift")
    assert_true(hashlib.sha256(payload).hexdigest().upper() ==
                "BEC0C8787425F40F28A46208C995FCD0B3B800AC23820B50B309F3135D745C70",
                "worker payload hash drift")
    return path


def legacy_string_gate():
    run(["cargo.exe", "test", "-q", "-p", "dbyte_hir",
         "legacy_str_usize_casts_follow_native_pointer_width_profiles"],
        "focused x86_64 legacy string HIR test")
    receipts = []
    for target in ("x86_64-dbyte-none", "x86_64-dbyte-user"):
        for opt in ("0", "1"):
            output = ROOT / f"legacy-{target}-{opt}.o"
            run([COMPILER, "build", LEGACY_STR, "--target", target,
                 "--opt-level", opt, "--emit", "obj", "-o", output],
                f"legacy string {target} O{opt}")
            receipts.append({"target": target, "opt": opt, "sha256": sha(output)})
    return receipts


def build_iso(label, opt, date, disk_backed=False):
    out = ROOT / f"{label}-o{opt}"
    out.mkdir()
    kernel = out / "kernel.elf"
    user = out / "worker.elf"
    run([COMPILER, "build", KERNEL, "--target", "x86_64-dbyte-none",
         "--opt-level", opt, "--emit", "elf", "-o", kernel],
        f"worker kernel {label} O{opt}")
    run([COMPILER, "build", USER, "--target", "x86_64-dbyte-user",
         "--opt-level", opt, "--emit", "elf", "-o", user],
        f"worker user {label} O{opt}")
    tree = out / "iso"
    grub = tree / "boot/grub"
    grub.mkdir(parents=True)
    (tree / "boot/kernel.elf").write_bytes(kernel.read_bytes())
    if not disk_backed:
        (tree / "boot/worker.elf").write_bytes(user.read_bytes())
    module_line = "" if disk_backed else " module2 /boot/worker.elf worker64\n"
    (grub / "grub.cfg").write_text(
        "set timeout=0\nset default=0\nmenuentry 'DByte G5 worker' {\n"
        " multiboot2 /boot/kernel.elf\n"
        + module_line + " boot\n}\n", encoding="ascii")
    iso = out / "g5-worker.iso"
    run(["wsl.exe", "-d", "Debian", "--", "grub-mkrescue", "-o", wslpath(iso),
         f"--modification-date={date}", "--set_all_file_dates", date, wslpath(tree)],
        f"deterministic worker ISO {label} O{opt}")
    disassembly = run(["objdump.exe", "-d", "-Mintel", kernel],
                      f"worker kernel disassembly {label} O{opt}")
    (out / "disassembly.txt").write_text(disassembly, encoding="utf-8")
    user_bytes = user.read_bytes()
    entry, phoff, shoff = struct.unpack_from("<QQQ", user_bytes, 24)
    phentsize, phnum, shentsize, shnum = struct.unpack_from("<HHHH", user_bytes, 54)
    assert_true(len(user_bytes) <= 4096, f"worker user O{opt} exceeds DBFS1 file limit")
    assert_true(entry == 0x4000B0, f"worker user O{opt} compact entry drifted")
    assert_true(phentsize == 56 and phnum == 2, f"worker user O{opt} PT_LOAD layout drifted")
    assert_true(shoff == 0 and shentsize == 0 and shnum == 0,
                f"worker user O{opt} unexpectedly has section headers")
    rx_offset, rx_vaddr, _, rx_filesz, _, rx_align = struct.unpack_from(
        "<QQQQQQ", user_bytes, phoff + 8)
    assert_true(rx_offset == 0xB0 and rx_vaddr == 0x4000B0 and rx_align == 4096,
                f"worker user O{opt} compact RX placement drifted")
    rx_bytes = user_bytes[rx_offset:rx_offset + rx_filesz]
    assert_true(rx_bytes.count(bytes([0xCD, 0x80])) >= 2,
                f"worker user O{opt} missing linked INT80 instructions")
    return {"label": label, "opt": opt, "kernel": str(kernel), "user": str(user),
            "iso": str(iso), "kernel_sha256": sha(kernel), "user_sha256": sha(user),
            "iso_sha256": sha(iso), "iso_bytes": iso.stat().st_size}


def install_fixture_elf(base, build):
    # Verifier-owned read-only fixture provisioning, not guest filesystem writes.
    disk = bytearray(Path(base).read_bytes())
    image = Path(build["user"]).read_bytes()
    assert_true(64 <= len(image) <= 4096, "ELF outside bounded file size")
    blocks = (len(image) + 511) // 512
    inode = 69 * 512 + 3 * 64
    for offset, value in ((0, 1), (4, 1), (8, len(image)), (12, blocks)):
        put_u32(disk, inode + offset, value)
    for index in range(8):
        put_u32(disk, inode + 16 + index * 4, 9 + index if index < blocks else 0xFFFFFFFF)
    for block in range(9, 9 + blocks):
        disk[65 * 512 + block // 8] |= 1 << (block % 8)
    entry = 77 * 512 + 64
    for offset, value in ((0, 3), (4, 1), (8, 10)):
        put_u32(disk, entry + offset, value)
    disk[entry + 12:entry + 22] = b"WORKER.ELF"
    disk[86 * 512:86 * 512 + len(image)] = image
    output = Path(build["user"]).parent / "disk-elf.raw"
    output.write_bytes(disk)
    assert_true(bytes(disk[86 * 512:86 * 512 + len(image)]) == image,
                "installed ELF differs from compiler output")
    return output


def wait_serial(serial, trace, process, timeout=45):
    deadline = time.time() + timeout
    entry_resets = None
    text = ""
    while time.time() < deadline:
        text = read_shared(serial)
        if text and entry_resets is None and trace.exists():
            entry_resets = read_shared(trace).count("CPU Reset")
        if "G5 WORKER SYSCALL COMPLETE" in text:
            return text, entry_resets
        if re.search(r"G5WORKER [^\n]*=[0-9A-F]{8}\n", text) or "G5 KERNEL #" in text:
            return text, entry_resets
        if process.poll() is not None:
            break
        time.sleep(0.05)
    raise RuntimeError(f"QEMU timeout/exit={process.poll()}; serial={text}")


def trace_gate(trace, entry_resets, label):
    text = read_shared(trace)
    assert_true("Triple fault" not in text, f"{label} triple fault")
    for token in ("v=06", "v=0d", "v=0e"):
        assert_true(token not in text, f"{label} CPU fault observed: {token}")
    final_resets = text.count("CPU Reset")
    assert_true(entry_resets is not None, f"{label} no reset baseline sampled")
    assert_true(final_resets == entry_resets, f"{label} CPU reset after kernel entry")
    return final_resets


def boot(build, fixture, ram, expected_error=None, case="positive"):
    out = ROOT / f"run-o{build['opt']}-{ram}m-{case}"
    out.mkdir()
    disk = out / "worker.raw"
    shutil.copyfile(fixture, disk)
    before = sha(disk)
    serial = out / "serial.log"
    trace = out / "trace.log"
    args = [str(QEMU), "-cdrom", build["iso"], "-boot", "order=d",
            "-m", f"{ram}M", "-display", "none", "-monitor", "none",
            "-serial", f"file:{serial}", "-d", "int,cpu_reset,guest_errors",
            "-D", str(trace), "-no-reboot", "-no-shutdown", "-drive",
            f"file={disk},format=raw,if=ide,index=0,media=disk,cache=directsync"]
    (out / "qemu-command.txt").write_text(" ".join(args), encoding="utf-8")
    process = subprocess.Popen(args, cwd=REPO, stdout=subprocess.DEVNULL,
                               stderr=subprocess.PIPE, text=True)
    try:
        text, entry_resets = wait_serial(serial, trace, process)
        assert_true("G5WORKER ERROR" not in text,
                    f"worker failure O{build['opt']} {ram}M")
        assert_true("G5 KERNEL #" not in text,
                    f"kernel fault O{build['opt']} {ram}M")
        expected = [
            "DBYTE V13 G5 WORKER",
            "WORKER READY owner=00000001 entry=00000000004000B0",
            "WORKER USER START",
            "WORKER USER OK",
            "USER64 EXIT code=00000000",
            "WORKER STATS req=00000005 open=00000001 read=00000001 close=00000001 write=00000002 cleanup=00000001",
            "OWNER FD SELFTEST wrong=00000001 stale=00000001",
            "G5 WORKER SYSCALL COMPLETE",
        ]
        if expected_error is None:
            for token in expected:
                assert_true(token in text, f"missing worker receipt {token!r}")
        else:
            assert_true(serial.read_bytes().endswith((expected_error + "\r\n").encode("ascii")),
                        "wrong disk-loader rejection")
            assert_true("WORKER READY" not in text and "G5 WORKER SYSCALL COMPLETE" not in text,
                        "invalid disk ELF entered userspace")
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
    after = sha(disk)
    assert_true(before == after,
                f"worker read-only path mutated disk O{build['opt']} {ram}M")
    final_resets = trace_gate(trace, entry_resets,
                              f"worker O{build['opt']} {ram}M")
    if expected_error is not None:
        return {"opt": build["opt"], "case": case, "disk_before": before,
                "disk_after": after, "serial_sha256": sha(serial),
                "trace_sha256": sha(trace), "rejection": expected_error,
                "entry_resets": entry_resets, "final_resets": final_resets}
    return {
        "opt": build["opt"], "ram": ram,
        "disk_before": before, "disk_after": after,
        "serial_sha256": sha(serial), "trace_sha256": sha(trace),
        "entry_resets": entry_resets, "final_resets": final_resets,
        "requests": 5, "open": 1, "read": 1, "close": 1,
        "writes": 2, "owner_cleanup": 1,
        "wrong_owner_rejects": 1, "stale_handle_rejects": 1,
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


def main(disk_backed=False, focused=False):
    head = run(["git.exe", "rev-parse", "HEAD"], "HEAD").strip()
    assert_true(head == EXPECTED_HEAD, "unexpected HEAD")
    tag = run(["git.exe", "rev-parse", "v12.4.0^{}"], "frozen tag").strip()
    assert_true(tag == FROZEN_BASE_HEAD, "v12.4.0 tag drifted")
    source_guard()
    if not focused:
        predecessor_gate()
    run(["cargo.exe", "build", "-q", "-p", "dbyte_cli", "--bin", "dbytec"],
        "fresh compiler")
    legacy = legacy_string_gate()
    (ROOT / "legacy-string-receipts.json").write_text(
        json.dumps(legacy, indent=2), encoding="utf-8")
    fixture = make_fixture()
    epoch = int(run(["git.exe", "show", "-s", "--format=%ct", "HEAD"], "epoch").strip())
    date = time.strftime("%Y%m%d%H%M%S00", time.gmtime(epoch))
    builds = []
    for opt in ("0", "1"):
        a = build_iso("A", opt, date, disk_backed)
        b = build_iso("B", opt, date, disk_backed)
        for key in ("kernel_sha256", "user_sha256", "iso_sha256"):
            assert_true(a[key] == b[key], f"O{opt} nondeterministic {key}")
        if disk_backed:
            a["disk"] = str(install_fixture_elf(fixture, a))
            b["disk"] = str(install_fixture_elf(fixture, b))
            assert_true(sha(a["disk"]) == sha(b["disk"]), "installed disk nondeterministic")
        builds.extend([a, b])
    (ROOT / "builds.json").write_text(json.dumps(builds, indent=2), encoding="utf-8")
    receipts = []
    for opt in ("0", "1"):
        selected = next(x for x in builds if x["label"] == "A" and x["opt"] == opt)
        for ram in (32, 64):
            receipts.append(boot(selected, selected["disk"] if disk_backed else fixture, ram))
    (ROOT / "qemu-receipts.json").write_text(
        json.dumps(receipts, indent=2), encoding="utf-8")
    negatives = []
    if disk_backed:
        for opt in ("0", "1"):
            selected = next(x for x in builds if x["label"] == "A" and x["opt"] == opt)
            for case, expected in (("bad-disk-elf", "0000002B"), ("missing-file", "00000065")):
                data = bytearray(Path(selected["disk"]).read_bytes())
                if case == "bad-disk-elf":
                    data[86 * 512] = 0
                else:
                    data[77 * 512 + 64 + 12:77 * 512 + 64 + 22] = b"ABSENT.ELF"
                fixture_path = ROOT / f"{case}-o{opt}.raw"
                fixture_path.write_bytes(data)
                negatives.append(boot(selected, fixture_path, 64,
                                      "G5WORKER ELF LOAD=" + expected, case))
    (ROOT / "disk-negative-receipts.json").write_text(json.dumps(negatives, indent=2), encoding="utf-8")
    run(["git.exe", "diff", "--check"], "diff check")
    tracked = run(["git.exe", "diff", "--binary"], "tracked diff")
    (ROOT / "tracked.diff").write_text(tracked, encoding="utf-8")
    manifest = source_manifest()
    manifest_path = ROOT / "input-manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    selected = next(x for x in builds if x["label"] == "A" and x["opt"] == "1")
    result = {
        "status": "FOCUSED_PASS" if focused else "PASS",
        "generation": "v13-g5-worker-syscall-owner-fd-qemu",
        "head": head,
        "hostile_fault_predecessor": "NOT_RUN" if focused else "PASS",
        "legacy_str_pointer_cast": "PASS_FOCUSED",
        "worker_syscall_boundary": "PASS_QEMU",
        "owner_qualified_fds": "PASS_QEMU",
        "validated_copyin_copyout": "PASS_QEMU",
        "vfs_dbfs_user_read": "PASS_QEMU",
        "disk_mutation": "NONE_OBSERVED",
        "selected_kernel_sha256": selected["kernel_sha256"],
        "selected_user_sha256": selected["user_sha256"],
        "selected_iso_sha256": selected["iso_sha256"],
        "selected_iso_bytes": selected["iso_bytes"],
        "fixture_sha256": sha(fixture),
        "manifest_sha256": sha(manifest_path),
        "qemu_receipts": len(receipts),
        "disk_negative_receipts": len(negatives),
        "interactive_shell": "PENDING",
        "disk_backed_elf_loading": "PASS_QEMU" if disk_backed else "PENDING",
        "boot_transport": "DBFS1_FILE" if disk_backed else "MULTIBOOT_MODULE",
        "reboot_persistence": "PENDING",
        "real_biostar": "PENDING",
        "real_storage": "DISARMED",
        "v13_final": "NOT_CREATED",
    }
    (ROOT / "RESULT.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print("V13 GENERATION 5 WORKER SYSCALL VERIFIER:", result["status"])
    print("EVIDENCE ROOT:", ROOT)
    print("SELECTED ISO SHA256:", result["selected_iso_sha256"])
    print("QEMU RECEIPTS:", len(receipts))


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--disk-backed", action="store_true")
    parser.add_argument("--focused", action="store_true",
                        help="Non-master evidence only; never emits status PASS")
    args = parser.parse_args()
    try:
        main(args.disk_backed, args.focused)
    except Exception:
        print("V13 GENERATION 5 WORKER SYSCALL VERIFIER: FAIL")
        print("EVIDENCE ROOT:", ROOT)
        raise
