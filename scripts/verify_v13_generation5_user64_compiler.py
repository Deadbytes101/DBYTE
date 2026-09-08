from pathlib import Path
import hashlib, json, struct, subprocess, uuid

from v13_release_identity import FROZEN_BASE_HEAD, expected_head

REPO = Path(__file__).resolve().parent.parent
COMPILER = REPO / "target" / "debug" / "dbytec.exe"
EXPECTED_HEAD = expected_head()
ROOT = REPO / "tmp" / "v13_g5" / ("user64-compiler-" + uuid.uuid4().hex)
ROOT.mkdir(parents=True)
COMMAND_INDEX = 0
HELLO = "examples/native_v13_machine/programs64/hello.dby"
FORBIDDEN = "examples/native_v13_machine/programs64/forbidden_native.dby"

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

def source_guard():
    target = (REPO / "crates/dbyte_native/src/target.rs").read_text(encoding="utf-8")
    cli = (REPO / "crates/dbyte_native/src/cli.rs").read_text(encoding="utf-8")
    selector = (REPO / "crates/dbyte_native/src/native_select_x86_64.rs").read_text(encoding="utf-8")
    modern = (REPO / "crates/dbyte_native/src/modern_native.rs").read_text(encoding="utf-8")
    hir = (REPO / "crates/dbyte_hir/src/lib.rs").read_text(encoding="utf-8")
    assert_true('triple: "x86_64-dbyte-user"' in target, "X86_64 user target missing")
    assert_true("Profile::X86_64User => 7" in target, "target metadata profile id drifted")
    assert_true('"x86_64-dbyte-user" => BuildTarget::X86_64User' in cli, "CLI target missing")
    assert_true("x86_64-dbyte-user does not support --emit staticlib" in cli, "user staticlib gate missing")
    assert_true("software_interrupt(0x80)" in selector, "user INT80 lowering missing")
    assert_true('"user_main"' in modern and "user_start_object" in modern, "user ELF64 entry seam missing")
    assert_true("link_user_sections" in modern and "write_user_executable" in modern,
                "compact user ELF64 link/writer seam missing")
    assert_true("Profile::I686User | Profile::X86_64User => namespace == \"user\"" in hir,
                "canonical user intrinsic profile gate missing")

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

def elf64_info(path):
    data = Path(path).read_bytes()
    assert_true(data[:4] == b"\x7fELF", "ELF magic missing")
    assert_true(data[4:7] == bytes([2, 1, 1]), "ELF64 little-endian ident mismatch")
    etype, machine = struct.unpack_from("<HH", data, 16)
    entry, phoff, shoff = struct.unpack_from("<QQQ", data, 24)
    phentsize, phnum, shentsize, shnum = struct.unpack_from("<HHHH", data, 54)
    assert_true(etype == 2 and machine == 62, "expected x86_64 ET_EXEC")
    assert_true(phentsize == 56 and phnum == 2, "unexpected program header layout")
    assert_true(shoff == 0 and shentsize == 0 and shnum == 0,
                "user ELF must omit section headers")
    assert_true(len(data) <= 4096, "user ELF exceeds DBFS1 file-size limit")
    loads = []
    for index in range(phnum):
        off = phoff + index * phentsize
        ptype, flags = struct.unpack_from("<II", data, off)
        poff, vaddr, paddr, filesz, memsz, align = struct.unpack_from("<QQQQQQ", data, off + 8)
        assert_true(ptype == 1, "non-PT_LOAD program header present")
        assert_true(filesz <= memsz, "PT_LOAD filesz exceeds memsz")
        assert_true(align == 4096, "PT_LOAD alignment must be one page")
        assert_true((poff % align) == (vaddr % align), "PT_LOAD offset/VA incongruent")
        assert_true(paddr == vaddr, "PT_LOAD physical/virtual address mismatch")
        assert_true(poff + filesz <= len(data), "PT_LOAD exceeds compact ELF bytes")
        loads.append({"flags": flags, "offset": poff, "vaddr": vaddr,
                      "filesz": filesz, "memsz": memsz, "align": align})
    assert_true(loads[0]["flags"] == 5, "first PT_LOAD must be RX")
    assert_true(loads[1]["flags"] == 6, "second PT_LOAD must be RW")
    assert_true(loads[0]["offset"] == 0xB0 and loads[0]["vaddr"] == 0x4000B0,
                "compact user text placement drifted")
    assert_true(entry == 0x4000B0, "user entry address drifted")
    return data, {"entry": f"0x{entry:016X}", "program_headers": loads}

def assert_user_start(data, info):
    start_offset = info["program_headers"][0]["offset"]
    start = data[start_offset:start_offset + 22]
    assert_true(len(start) == 22 and start[0] == 0xE8, "user _start call missing")
    assert_true(start[5:8] == bytes([0x48, 0x89, 0xC2]), "user exit status move missing")
    assert_true(start[8:10] == bytes([0x48, 0xB8]) and start[10:18] == bytes(8),
                "user exit syscall number setup mismatch")
    assert_true(start[18:22] == bytes([0xCD, 0x80, 0x0F, 0x0B]), "user _start INT80/UD2 mismatch")

def build_set(label, opt):
    out = ROOT / f"{label}-o{opt}"
    out.mkdir()
    obj = out / "hello.o"
    elf = out / "hello.elf"
    base = [COMPILER, "build", HELLO, "--target", "x86_64-dbyte-user", "--opt-level", opt]
    run(base + ["--emit", "obj", "-o", obj], f"build {label} O{opt} object")
    run(base + ["--emit", "elf", "-o", elf], f"build {label} O{opt} ELF")
    dis = run(["objdump.exe", "-d", "-Mintel", obj], f"disassemble object {label} O{opt}")
    (out / "object-disassembly.txt").write_text(dis, encoding="utf-8")
    data, info = elf64_info(elf)
    assert_user_start(data, info)
    normalized = " ".join(dis.replace("\t", " ").split())
    assert_true(normalized.count("int 0x80") >= 1, "user object is missing INT80")
    assert_true("mov rdx" in normalized and "mov rcx" in normalized and "mov r8" in normalized,
                "user syscall argument register setup missing")
    rx = info["program_headers"][0]
    rx_bytes = data[rx["offset"]:rx["offset"] + rx["filesz"]]
    int80_count = rx_bytes.count(bytes([0xCD, 0x80]))
    assert_true(int80_count >= 2, "linked compact ELF is missing exit/user INT80 sites")
    return {"label": label, "opt": opt, "object": str(obj), "elf": str(elf),
            "object_sha256": sha(obj), "elf_sha256": sha(elf), "elf_bytes": elf.stat().st_size,
            "elf_info": info, "int80_count": int80_count}

def negative_compile(source, emit, expected_text, name):
    output = ROOT / f"negative-{name}.{emit}"
    args = [COMPILER, "build", source, "--target", "x86_64-dbyte-user",
            "--opt-level", "1", "--emit", emit, "-o", output]
    global COMMAND_INDEX
    COMMAND_INDEX += 1
    p = subprocess.run([str(x) for x in args], cwd=REPO, text=True,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = ROOT / f"command-{COMMAND_INDEX:03d}.txt"
    log.write_text(f"COMMAND={' '.join(map(str,args))}\nEXIT={p.returncode}\n{p.stdout}", encoding="utf-8")
    assert_true(p.returncode != 0, f"negative {name} unexpectedly succeeded")
    assert_true(expected_text in p.stdout, f"negative {name} diagnostic mismatch")
    assert_true(not output.exists(), f"negative {name} left output artifact")
    return {"name": name, "exit": p.returncode, "diagnostic": expected_text}

def main():
    head = run(["git.exe", "rev-parse", "HEAD"], "HEAD").strip()
    assert_true(head == EXPECTED_HEAD, "unexpected HEAD")
    tag = run(["git.exe", "rev-parse", "v12.4.0^{}"], "frozen tag").strip()
    assert_true(tag == FROZEN_BASE_HEAD, "v12.4.0 tag drifted")
    source_guard()
    run(["cargo.exe", "build", "-q", "-p", "dbyte_cli", "--bin", "dbytec"], "fresh compiler")
    tests = [
        ("dbyte_hir", "x86_64_user_profile_accepts_user_syscalls_and_rejects_native_namespace"),
        ("dbyte_native", "target::tests::x86_64_target_metadata_round_trips_and_is_strict"),
        ("dbyte_native", "x86_64::tests::encodes_software_interrupt_exactly"),
        ("dbyte_native", "elf64::tests::writes_compact_user_elf64_for_dbfs1"),
    ]
    for package, test in tests:
        run(["cargo.exe", "test", "-q", "-p", package, test, "--", "--exact"], f"focused test {test}")
    manifest = source_manifest()
    (ROOT / "input-manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    builds = []
    for opt in ("0", "1"):
        a = build_set("A", opt)
        b = build_set("B", opt)
        assert_true(a["object_sha256"] == b["object_sha256"], f"O{opt} object nondeterministic")
        assert_true(a["elf_sha256"] == b["elf_sha256"], f"O{opt} ELF nondeterministic")
        builds.extend([a, b])
    (ROOT / "builds.json").write_text(json.dumps(builds, indent=2), encoding="utf-8")
    negatives = []
    negatives.append(negative_compile(
        FORBIDDEN, "obj", "E_PROFILE_CAPABILITY", "forbidden-native"))
    negatives.append(negative_compile(
        HELLO, "staticlib", "x86_64-dbyte-user does not support --emit staticlib", "staticlib"))
    negatives.append(negative_compile(
        HELLO, "bin", "DBYTE64 BIN emission is supported only for x86_64-dbyte-none", "bin"))
    (ROOT / "negative-receipts.json").write_text(json.dumps(negatives, indent=2), encoding="utf-8")
    run(["git.exe", "diff", "--check"], "diff check")
    tracked = run(["git.exe", "diff", "--binary"], "tracked diff")
    (ROOT / "tracked.diff").write_text(tracked, encoding="utf-8")
    selected = next(x for x in builds if x["label"] == "A" and x["opt"] == "1")
    result = {
        "status": "PASS",
        "generation": "v13-g5-user64-compiler",
        "head": head,
        "compiler_sha256": sha(COMPILER),
        "manifest_sha256": sha(ROOT / "input-manifest.json"),
        "selected_object_sha256": selected["object_sha256"],
        "selected_elf_sha256": selected["elf_sha256"],
        "selected_elf_bytes": selected["elf_bytes"],
        "entry": selected["elf_info"]["entry"],
        "int80_count": selected["int80_count"],
        "deterministic_builds": 4,
        "negative_receipts": len(negatives),
        "ring3_runtime": "PENDING",
        "private_address_spaces": "PENDING",
        "copy_boundary": "PENDING",
        "shell": "PENDING",
        "real_biostar": "PENDING",
        "real_storage": "DISARMED",
        "v13_final": "NOT_CREATED",
    }
    (ROOT / "RESULT.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print("V13 GENERATION 5 USER64 COMPILER VERIFIER: PASS")
    print("EVIDENCE ROOT:", ROOT)
    print("SELECTED ELF SHA256:", result["selected_elf_sha256"])
    print("NEGATIVE RECEIPTS:", len(negatives))

if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("V13 GENERATION 5 USER64 COMPILER VERIFIER: FAIL")
        print("EVIDENCE ROOT:", ROOT)
        raise
