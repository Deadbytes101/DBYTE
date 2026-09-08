from pathlib import Path
import hashlib, json, subprocess, uuid

from v13_release_identity import FROZEN_BASE_HEAD, expected_head

REPO = Path(__file__).resolve().parent.parent
EXPECTED_HEAD = expected_head()
ROOT = REPO / "tmp/v13_g5" / ("aggregate-" + uuid.uuid4().hex)
ROOT.mkdir(parents=True)
COMMAND_INDEX = 0

def assert_true(value, message):
    if not value:
        raise RuntimeError(message)

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest().upper()

def run(args, description):
    global COMMAND_INDEX
    COMMAND_INDEX += 1
    proc = subprocess.run([str(x) for x in args], cwd=REPO, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    log = ROOT / f"command-{COMMAND_INDEX:03d}.txt"
    log.write_text(f"COMMAND={' '.join(map(str,args))}\nEXIT={proc.returncode}\n{proc.stdout}",
                   encoding="utf-8")
    if proc.returncode != 0:
        raise RuntimeError(f"{description} failed ({proc.returncode}): {log}")
    return proc.stdout

def result_from_output(output, label):
    marker = "EVIDENCE ROOT:"
    line = next((x for x in output.splitlines() if x.startswith(marker)), None)
    assert_true(line is not None, f"{label} evidence root missing")
    evidence = Path(line[len(marker):].strip())
    result_path = evidence / "RESULT.json"
    assert_true(result_path.is_file(), f"{label} RESULT.json missing")
    result = json.loads(result_path.read_text(encoding="utf-8"))
    assert_true(result.get("status") == "PASS", f"{label} did not seal PASS")
    assert_true(result.get("head") == EXPECTED_HEAD, f"{label} head drifted")
    assert_true(result.get("real_storage") == "DISARMED", f"{label} armed real storage")
    return evidence, result, sha(result_path)

def main():
    head = run(["git.exe", "rev-parse", "HEAD"], "HEAD").strip()
    assert_true(head == EXPECTED_HEAD, "unexpected HEAD")
    tag = run(["git.exe", "rev-parse", "v12.4.0^{}"], "frozen tag").strip()
    assert_true(tag == FROZEN_BASE_HEAD, "v12.4.0 tag drifted")
    shell_output = run(["python.exe", "scripts/verify_v13_generation5_shell.py"],
                       "fresh G5 interactive shell master")
    shell_root, shell, shell_result_sha = result_from_output(shell_output, "shell")
    assert_true(shell.get("generation") == "v13-g5-interactive-ps2-disk-shell-qemu",
                "wrong shell generation")
    for key in ("disk_programs", "interactive_ps2_shell",
                "hostile_user_survival_in_shell", "line_editing", "line_overflow_bound"):
        assert_true(shell.get(key) == "PASS_QEMU", f"shell gate not PASS: {key}")
    assert_true(shell.get("worker_disk_predecessor") == "PASS",
                "worker predecessor missing from shell master")
    persist_output = run(["python.exe", "scripts/verify_v13_generation5_reboot_persistence.py"],
                         "fresh G5 reboot persistence master")
    persist_root, persist, persist_result_sha = result_from_output(persist_output, "persistence")
    assert_true(persist.get("generation") == "v13-g5-reboot-persistence-qemu",
                "wrong persistence generation")
    for key, expected in (
        ("g4_dbfs_write_flush", "PASS_QEMU"),
        ("host_exact_byte_oracle", "PASS"),
        ("fresh_qemu_process_reboot", "PASS"),
        ("ring3_user_readback", "PASS_QEMU"),
        ("corruption_rejection", "PASS_QEMU")):
        assert_true(persist.get(key) == expected, f"persistence gate not PASS: {key}")
    run(["git.exe", "diff", "--check"], "diff check")
    result = {
        "status": "PASS", "generation": "v13-g5-aggregate-qemu", "head": head,
        "shell_root": str(shell_root), "shell_result_sha256": shell_result_sha,
        "persistence_root": str(persist_root), "persistence_result_sha256": persist_result_sha,
        "user64_compiler": "PASS_TRANSITIVE", "module_bootstrap": "PASS_TRANSITIVE",
        "ring3_private_cr3": "PASS_TRANSITIVE", "hostile_fault_containment": "PASS_TRANSITIVE",
        "disk_backed_worker": "PASS_TRANSITIVE", "interactive_shell": "PASS_QEMU",
        "reboot_persistence": "PASS_QEMU", "real_biostar": "PENDING",
        "real_storage": "DISARMED", "v13_final": "NOT_CREATED"
    }
    result_path = ROOT / "RESULT.json"
    result_path.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print("V13 GENERATION 5 AGGREGATE VERIFIER: PASS")
    print("EVIDENCE ROOT:", ROOT)
    print("SHELL RESULT SHA256:", shell_result_sha)
    print("PERSISTENCE RESULT SHA256:", persist_result_sha)

if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("V13 GENERATION 5 AGGREGATE VERIFIER: FAIL")
        print("EVIDENCE ROOT:", ROOT)
        raise
