#!/usr/bin/env python3
"""Native GNU/Linux host qualification for the DByte v13.2 candidate."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import time
import uuid

from verify_machine_inventory_receipt import validate_receipt


class VerificationError(RuntimeError):
    pass


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def bind_physical_machine(
    accepted: dict[str, object],
    live: dict[str, object],
    expected_fingerprint: str,
) -> dict[str, str]:
    """Bind a final run to one reviewed physical-machine identity."""
    accepted_report = validate_receipt(accepted)
    if accepted_report["status"] != "PASS":
        raise VerificationError("BIOSTAR_ACCEPTED_INVENTORY_INVALID")
    live_report = validate_receipt(live)
    if live_report["status"] != "PASS":
        raise VerificationError("BIOSTAR_LIVE_INVENTORY_INVALID")
    accepted_fingerprint = str(accepted_report["fingerprint_sha256"]).lower()
    live_fingerprint = str(live_report["fingerprint_sha256"]).lower()
    expected = expected_fingerprint.lower()
    if not re.fullmatch(r"[0-9a-f]{64}", expected):
        raise VerificationError("BIOSTAR_EXPECTED_FINGERPRINT_INVALID")
    if accepted_fingerprint != expected:
        raise VerificationError("BIOSTAR_ACCEPTED_FINGERPRINT_MISMATCH")
    if live_fingerprint != expected:
        raise VerificationError("BIOSTAR_LIVE_FINGERPRINT_MISMATCH")
    if accepted.get("fingerprint", {}).get("identity") != live.get("fingerprint", {}).get("identity"):
        raise VerificationError("BIOSTAR_LIVE_IDENTITY_MISMATCH")
    return {
        "accepted_fingerprint_sha256": accepted_fingerprint,
        "live_fingerprint_sha256": live_fingerprint,
    }


class Verifier:
    def __init__(
        self,
        root: Path,
        evidence: Path,
        mode: str,
        final: bool,
        machine_inventory: Path | None,
        expected_inventory_sha256: str | None,
        expected_machine_fingerprint: str | None,
        expected_candidate_identity: str | None,
        source_manifest: Path | None,
    ) -> None:
        self.root = root
        self.evidence = evidence
        self.mode = mode
        self.final = final
        self.machine_inventory = machine_inventory
        self.expected_inventory_sha256 = expected_inventory_sha256
        self.expected_machine_fingerprint = expected_machine_fingerprint
        self.expected_candidate_identity = expected_candidate_identity
        self.source_manifest = source_manifest
        self.commands: list[dict[str, object]] = []
        self.sequence = 0
        self.target = evidence / "cargo-target"
        self.bin = self.target / "release"
        self.builds = evidence / "builds"

    def run(self, name: str, command: list[str], *, cwd: Path | None = None, expected: int = 0, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
        self.sequence += 1
        merged = os.environ.copy()
        merged.update({"CARGO_TARGET_DIR": str(self.target), "CARGO_INCREMENTAL": "0", "SOURCE_DATE_EPOCH": "946684800"})
        if env:
            merged.update(env)
        started = time.monotonic()
        result = subprocess.run(command, cwd=cwd or self.root, env=merged, text=True, capture_output=True)
        elapsed = time.monotonic() - started
        stem = f"{self.sequence:03d}-{name}"
        (self.evidence / f"{stem}.stdout.txt").write_text(result.stdout, encoding="utf-8")
        (self.evidence / f"{stem}.stderr.txt").write_text(result.stderr, encoding="utf-8")
        receipt = {"name": name, "command": command, "cwd": str(cwd or self.root), "exit_code": result.returncode, "expected_exit": expected, "elapsed_seconds": round(elapsed, 6)}
        (self.evidence / f"{stem}.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
        self.commands.append(receipt)
        if result.returncode != expected:
            raise VerificationError(f"COMMAND_FAILED: {name}: expected {expected}, got {result.returncode}")
        return result

    def inspect_generated_elf(
        self, path: Path, name: str, machine: str = "Advanced Micro Devices X86-64"
    ) -> None:
        header = self.run(f"readelf-{name}", ["readelf", "-h", "-l", "-d", "-r", str(path)]).stdout
        if machine not in header:
            raise VerificationError(f"GENERATED_ELF_MACHINE_INVALID: {name}")
        if "INTERP" in header or "There is no dynamic section" not in header:
            raise VerificationError(f"GENERATED_ELF_NOT_STATIC: {name}")
        relocations = self.run(f"readelf-relocs-{name}", ["readelf", "-r", str(path)]).stdout
        if "There are no relocations in this file" not in relocations:
            raise VerificationError(f"GENERATED_ELF_FINAL_RELOCATIONS: {name}")

    def build_case(self, case: dict[str, object], level: int, lane: str) -> tuple[Path, str]:
        name = str(case["name"])
        source = self.root / str(case["source"])
        directory = self.builds / lane / f"O{level}"
        directory.mkdir(parents=True, exist_ok=True)
        output = directory / f"{name}.elf"
        self.run(f"build-{lane}-O{level}-{name}", [str(self.bin / "dbytec"), "build", str(source), "--target", "x86_64-dbyte-linux", "--emit", "elf", "--opt-level", str(level), "-o", str(output)])
        self.inspect_generated_elf(output, f"{lane}-O{level}-{name}")
        return output, sha256(output)

    def execute_case(self, case: dict[str, object], output: Path, level: int) -> None:
        name = str(case["name"])
        kind = str(case["kind"])
        expected = int(case["expected_exit"])
        arguments = [str(value) for value in case.get("arguments", [])]
        cwd = self.evidence / "runtime" / f"O{level}-{name}"
        cwd.mkdir(parents=True, exist_ok=True)
        result = self.run(f"execute-O{level}-{name}", [str(output), *arguments], cwd=cwd, expected=expected)
        if kind == "linux-output" and result.stdout != case["expected_stdout"]:
            raise VerificationError(f"RUNTIME_STDOUT_MISMATCH: O{level}-{name}")
        if kind == "linux-file":
            produced = cwd / "dbyte-v132-file-io.tmp"
            expected_bytes = str(case["expected_file"]).encode()
            if not produced.is_file() or produced.read_bytes() != expected_bytes:
                raise VerificationError(f"RUNTIME_FILE_ORACLE_MISMATCH: O{level}-{name}")

    def execute(self) -> dict[str, object]:
        kernel = platform.release().lower()
        wsl = "microsoft" in kernel or "wsl" in kernel
        if sys.platform != "linux":
            raise VerificationError("LINUX_HOST_REQUIRED")
        if self.final and self.mode != "full":
            raise VerificationError("FINAL_FULL_MODE_REQUIRED")
        if self.final and wsl:
            raise VerificationError("LINUX_HOST_WSL_FORBIDDEN")
        physical_binding: dict[str, str] | None = None
        required = ["cargo", "rustc", "git", "readelf", "objdump", "python3"]
        if self.mode == "full" or self.final:
            required.append("cc")
        missing = [tool for tool in required if shutil.which(tool) is None]
        if missing:
            raise VerificationError(f"LINUX_HOST_DEPENDENCY_UNAVAILABLE: {','.join(missing)}")
        self.evidence.mkdir(parents=True, exist_ok=False)
        if self.final:
            if (
                not self.machine_inventory
                or not self.expected_inventory_sha256
                or not self.expected_machine_fingerprint
                or not self.expected_candidate_identity
            ):
                raise VerificationError("BIOSTAR_INVENTORY_BINDING_REQUIRED")
            accepted_bytes = self.machine_inventory.read_bytes()
            accepted_sha256 = hashlib.sha256(accepted_bytes).hexdigest()
            if accepted_sha256 != self.expected_inventory_sha256.lower():
                raise VerificationError("BIOSTAR_ACCEPTED_RECEIPT_SHA256_MISMATCH")
            try:
                accepted_inventory = json.loads(accepted_bytes.decode("utf-8-sig"))
            except (UnicodeError, json.JSONDecodeError) as error:
                raise VerificationError("BIOSTAR_ACCEPTED_INVENTORY_UNREADABLE") from error
            live_result = self.run(
                "biostar-live-inventory",
                ["python3", str(self.root / "scripts/machine_inventory_linux.py")],
            )
            live_path = self.evidence / "biostar-live-inventory.json"
            live_path.write_text(live_result.stdout, encoding="utf-8")
            try:
                live_inventory = json.loads(live_result.stdout)
            except json.JSONDecodeError as error:
                raise VerificationError("BIOSTAR_LIVE_INVENTORY_UNREADABLE") from error
            physical_binding = bind_physical_machine(
                accepted_inventory,
                live_inventory,
                self.expected_machine_fingerprint,
            )
            physical_binding["accepted_receipt_sha256"] = accepted_sha256
            physical_binding["live_receipt_sha256"] = sha256(live_path)
        if self.source_manifest:
            if not self.expected_candidate_identity:
                raise VerificationError("SOURCE_MANIFEST_EXPECTED_CANDIDATE_REQUIRED")
            source_result = self.run(
                "source-manifest",
                [
                    "python3",
                    str(self.root / "scripts/verify_v13_2_source_manifest.py"),
                    "--root",
                    str(self.root),
                    "--manifest",
                    str(self.source_manifest),
                    "--expected-candidate-identity",
                    self.expected_candidate_identity,
                ],
            )
            candidate = json.loads(source_result.stdout)["candidate_identity"]
        else:
            identity_path = self.evidence / "candidate-identity.json"
            identity_result = self.run("candidate-identity", ["python3", str(self.root / "scripts/v13_2_candidate_identity.py"), "--root", str(self.root), "--output", str(identity_path)])
            candidate = identity_result.stdout.strip()
        if self.final and candidate != self.expected_candidate_identity:
            raise VerificationError("BIOSTAR_CANDIDATE_IDENTITY_MISMATCH")
        machine = {
            "schema": "dbyte-v13.2-linux-host-machine-v1",
            "classification": "FINAL" if self.final else "DEVELOPMENT_ONLY",
            "wsl": wsl,
            "uname": platform.uname()._asdict(),
            "candidate_identity": candidate,
            "real_storage": "DISARMED",
            "physical_binding": physical_binding,
        }
        (self.evidence / "machine.json").write_text(json.dumps(machine, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        rustfmt_probe = subprocess.run(["rustfmt", "--version"], text=True, capture_output=True) if shutil.which("rustfmt") is not None else None
        if rustfmt_probe is not None and rustfmt_probe.returncode == 0:
            self.run("cargo-fmt", ["cargo", "fmt", "--all", "--", "--check"])
        elif self.mode == "full" or self.final:
            raise VerificationError("LINUX_HOST_DEPENDENCY_UNAVAILABLE: rustfmt")
        else:
            machine["rustfmt"] = "UNAVAILABLE_IN_DEVELOPMENT_PROBE"
            (self.evidence / "machine.json").write_text(json.dumps(machine, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        self.run("cargo-build-release-bins", ["cargo", "build", "--release", "--locked", "--bins"])
        if self.mode == "full":
            self.run("cargo-test-workspace", ["cargo", "test", "--workspace", "--locked"])
            self.run("cargo-clippy-workspace", ["cargo", "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])
        else:
            self.run("cargo-test-dbtasm", ["cargo", "test", "-p", "dbyte_native", "dbtasm64"])
        for binary in ("dbyte", "dbytec", "dbtasm"):
            path = self.bin / binary
            if not path.is_file() or not os.access(path, os.X_OK):
                raise VerificationError(f"LINUX_HOST_BINARY_REQUIRED: {binary}")
            elf = self.run(f"host-readelf-{binary}", ["readelf", "-h", "-l", "-d", "--version-info", str(path)]).stdout
            if "Advanced Micro Devices X86-64" not in elf:
                raise VerificationError(f"LINUX_HOST_BINARY_MACHINE_INVALID: {binary}")
        version = self.run("dbyte-version", [str(self.bin / "dbyte"), "--version"]).stdout.strip()
        expected_version = "13.2.0" if self.final else ("13.1.0" if '13.1.0' in version else "13.2.0")
        if version != f"DByte {expected_version}":
            raise VerificationError(f"LINUX_HOST_VERSION_MISMATCH: {version}")
        self.run("dbytec-capability", [str(self.bin / "dbytec")], expected=2)
        self.run("dbtasm-capability", [str(self.bin / "dbtasm")], expected=2)

        manifest = json.loads((self.root / "examples/native_v13_2_linux_host/manifest.json").read_text(encoding="utf-8"))
        portable = self.root / "examples/native_v13_2_linux_host/import_main.dby"
        self.run("check-portable", [str(self.bin / "dbytec"), "check-portable", str(portable)])
        artifacts: dict[str, dict[str, str]] = {}
        for case in manifest["cases"]:
            name = str(case["name"])
            artifacts[name] = {}
            for level in (0, 1):
                first, first_hash = self.build_case(case, level, "A")
                second, second_hash = self.build_case(case, level, "B")
                if first_hash != second_hash:
                    raise VerificationError(f"GENERATED_ELF_NONDETERMINISTIC: O{level}-{name}")
                artifacts[name][f"O{level}"] = first_hash
                self.execute_case(case, first, level)

        object_root = self.builds / "object-link"
        object_root.mkdir(parents=True, exist_ok=True)
        runtime_source = self.root / "examples/native_v12_2_linux/runtime_foundation.dby"
        runtime_object = object_root / "runtime.o"
        runtime_archive = object_root / "runtime.a"
        linked = object_root / "runtime-linked.elf"
        archive_linked = object_root / "runtime-archive-linked.elf"
        compiler = str(self.bin / "dbytec")
        self.run("emit-object", [compiler, "build", str(runtime_source), "--target", "x86_64-dbyte-linux", "--emit", "obj", "-o", str(runtime_object)])
        self.run("emit-archive", [compiler, "build", str(runtime_source), "--target", "x86_64-dbyte-linux", "--emit", "staticlib", "-o", str(runtime_archive)])
        self.run("internal-link-object", [compiler, "link", "--target", "x86_64-dbyte-linux", str(runtime_object), "-o", str(linked)])
        self.run("internal-link-archive", [compiler, "link", "--target", "x86_64-dbyte-linux", str(runtime_archive), "-o", str(archive_linked)])
        self.inspect_generated_elf(linked, "linked-object")
        self.inspect_generated_elf(archive_linked, "linked-archive")
        if sha256(linked) != sha256(archive_linked):
            raise VerificationError("INTERNAL_LINK_ARCHIVE_DIVERGENCE")

        if self.mode == "full" or self.final:
            c_contract = manifest["c_fixture"]
            c_object = object_root / "float-contract-c.o"
            dbyte_object = object_root / "float-contract-dbyte.o"
            c_elf = object_root / "float-contract.elf"
            self.run("c-fixture-object", ["cc", "-O2", "-fno-omit-frame-pointer", "-ffreestanding", "-fno-pic", "-fno-pie", "-fno-stack-protector", "-fno-builtin", "-c", str(self.root / str(c_contract["c"])), "-o", str(c_object)])
            self.run("dbyte-c-abi-object", [compiler, "build", str(self.root / str(c_contract["dbyte"])), "--target", "x86_64-dbyte-linux", "--emit", "obj", "-o", str(dbyte_object)])
            self.run("internal-link-c-abi", [compiler, "link", "--target", "x86_64-dbyte-linux", str(dbyte_object), str(c_object), "-o", str(c_elf)])
            self.inspect_generated_elf(c_elf, "c-abi")
            self.run("execute-c-abi", [str(c_elf)], expected=int(c_contract["expected_exit"]))

        asm_object = object_root / "dbtasm.o"
        asm_elf = object_root / "dbtasm.elf"
        self.run("dbtasm-assemble", [str(self.bin / "dbtasm"), str(self.root / str(manifest["dbtasm"])), "--target", "x86_64-dbyte-none", "-o", str(asm_object)])
        asm_info = self.run("dbtasm-object-inspect", ["readelf", "-h", "-S", "-s", "-r", str(asm_object)]).stdout
        if "kernel_main" not in asm_info or ".dbyte.target" not in asm_info:
            raise VerificationError("DBTASM_OBJECT_CONTRACT_MISMATCH")
        self.run(
            "dbtasm-internal-link",
            [compiler, "link", "--target", "x86_64-dbyte-none", str(asm_object), "-o", str(asm_elf)],
        )
        self.inspect_generated_elf(asm_elf, "dbtasm-linked-none")

        cross = self.builds / "cross-target"
        cross.mkdir(parents=True, exist_ok=True)
        cross_source = self.root / "examples/native_v13_2_linux_host/cross_target.dby"
        cross_artifacts: dict[str, dict[str, str]] = {}
        for target, emit, suffix in (
            ("x86_64-dbyte-none", "elf", "none.elf"),
            ("x86_64-dbyte-windows", "exe", "windows.exe"),
            ("i686-dbyte-none", "elf", "i686.elf"),
        ):
            outputs = []
            hashes = []
            for lane in ("A", "B"):
                output = cross / f"{lane}-{suffix}"
                self.run(
                    f"cross-{lane}-{target}",
                    [
                        compiler,
                        "build",
                        str(cross_source),
                        "--target",
                        target,
                        "--emit",
                        emit,
                        "-o",
                        str(output),
                    ],
                )
                outputs.append(output)
                hashes.append(sha256(output))
            if hashes[0] != hashes[1]:
                raise VerificationError(f"CROSS_TARGET_NONDETERMINISTIC: {target}")
            inspection = self.run(
                f"cross-inspect-{target}",
                ["objdump", "-f", "-h", "-p", str(outputs[0])],
            ).stdout
            if target == "x86_64-dbyte-windows":
                imports = sorted(set(re.findall(r"DLL Name: ([^\r\n]+)", inspection)))
                if any(name.lower() != "kernel32.dll" for name in imports):
                    raise VerificationError("WINDOWS_IMPORT_ALLOWLIST_VIOLATION")
            else:
                machine = "Intel 80386" if target == "i686-dbyte-none" else "Advanced Micro Devices X86-64"
                self.inspect_generated_elf(outputs[0], f"cross-{target}", machine)
                imports = []
            cross_artifacts[target] = {
                "sha256": hashes[0],
                "format": emit,
                "imports": ",".join(imports),
            }
        cross_artifacts["dbtasm-x86_64-dbyte-none"] = {
            "sha256": sha256(asm_elf),
            "format": "elf",
            "imports": "",
        }

        glibc_versions: set[str] = set()
        needed: dict[str, list[str]] = {}
        for binary in ("dbyte", "dbytec", "dbtasm"):
            text = self.run(f"host-version-info-{binary}", ["readelf", "--version-info", "-d", str(self.bin / binary)]).stdout
            glibc_versions.update(re.findall(r"GLIBC_([0-9]+(?:\.[0-9]+)*)", text))
            needed[binary] = sorted(set(re.findall(r"Shared library: \[([^]]+)\]", text)))
        floor = max(glibc_versions, key=lambda value: tuple(int(part) for part in value.split("."))) if glibc_versions else "NONE"
        result = {
            "status": "PASS" if self.final else "DEVELOPMENT_ONLY_PASS",
            "schema": "dbyte-v13.2-linux-host-result-v1",
            "candidate_identity": candidate,
            "version": expected_version,
            "mode": self.mode,
            "wsl": wsl,
            "glibc_floor": floor,
            "dt_needed": needed,
            "artifacts": artifacts,
            "cross_target_artifacts": cross_artifacts,
            "commands": self.commands,
            "real_biostar": "PASS" if physical_binding is not None else "PENDING",
            "real_storage": "DISARMED",
            "physical_binding": physical_binding,
        }
        if self.final and result["real_biostar"] != "PASS":
            raise VerificationError("BIOSTAR_HOST_BINDING_REQUIRED")
        (self.evidence / "RESULT.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--evidence-root", type=Path)
    parser.add_argument("--mode", choices=("focused", "full"), default="focused")
    parser.add_argument("--final", action="store_true")
    parser.add_argument("--machine-inventory", type=Path)
    parser.add_argument("--expected-inventory-sha256")
    parser.add_argument("--expected-machine-fingerprint")
    parser.add_argument("--expected-candidate-identity")
    parser.add_argument("--source-manifest", type=Path)
    args = parser.parse_args()
    evidence = args.evidence_root or args.root / "tmp" / "v13_2_linux_host" / f"linux-{uuid.uuid4().hex}"
    verifier = Verifier(
        args.root.resolve(),
        evidence.resolve(),
        args.mode,
        args.final,
        args.machine_inventory.resolve() if args.machine_inventory else None,
        args.expected_inventory_sha256,
        args.expected_machine_fingerprint,
        args.expected_candidate_identity,
        args.source_manifest.resolve() if args.source_manifest else None,
    )
    try:
        result = verifier.execute()
    except (OSError, VerificationError, subprocess.SubprocessError) as error:
        evidence.mkdir(parents=True, exist_ok=True)
        (evidence / "FAILURE.txt").write_text(f"{error}\n", encoding="utf-8")
        print(f"DBYTE V13.2 LINUX NATIVE HOST PORTABILITY: BLOCKED: {error}")
        print(f"EVIDENCE ROOT: {evidence}")
        return 1
    print(f"DBYTE V13.2 LINUX NATIVE HOST PORTABILITY: {result['status']}")
    print(f"EVIDENCE ROOT: {evidence}")
    print(f"GLIBC FLOOR: {result['glibc_floor']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
