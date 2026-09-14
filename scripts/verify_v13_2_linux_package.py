#!/usr/bin/env python3
"""Deterministic A/B and clean-extraction proof for the v13.2 Linux package."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import uuid

from package_release_linux import PROTECTED_WORKTREE_PATHS, validate_release_docs
from verify_v13_2_source_manifest import verify as verify_source_manifest


class Failure(RuntimeError):
    pass


def run(command: list[str], cwd: Path, env: dict[str, str] | None = None, expected: int = 0) -> subprocess.CompletedProcess[str]:
    merged = os.environ.copy()
    if env:
        merged.update(env)
    result = subprocess.run(command, cwd=cwd, env=merged, text=True, capture_output=True)
    if result.returncode != expected:
        raise Failure(f"PACKAGE_COMMAND_FAILED: {' '.join(command)}: expected {expected}, got {result.returncode}\n{result.stderr}")
    return result


def resolve_candidate_identity(
    root: Path,
    evidence: Path,
    final: bool,
    expected_identity: str | None,
) -> str:
    evidence.mkdir(parents=True, exist_ok=True)
    if final:
        if not expected_identity:
            raise Failure("PACKAGE_EXPECTED_CANDIDATE_IDENTITY_REQUIRED")
        manifest_path = root / "source-manifest.json"
        try:
            result = verify_source_manifest(root, manifest_path, expected_identity)
        except Exception as error:
            raise Failure(f"PACKAGE_SOURCE_MANIFEST_FAILED: {error}") from error
        (evidence / "source-manifest-check.json").write_text(
            json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        return str(result["candidate_identity"])
    identity_path = evidence / "candidate-identity.json"
    return run(["python3", str(root / "scripts/v13_2_candidate_identity.py"), "--root", str(root), "--output", str(identity_path)], root).stdout.strip()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def safe_extract(archive_path: Path, destination: Path) -> None:
    with tarfile.open(archive_path, "r:gz") as archive:
        for member in archive.getmembers():
            path = Path(member.name)
            if path.is_absolute() or ".." in path.parts or member.issym() or member.islnk():
                raise Failure(f"PACKAGE_MEMBER_UNSAFE: {member.name}")
        archive.extractall(destination, filter="data")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--evidence-root", type=Path)
    parser.add_argument("--final", action="store_true")
    parser.add_argument("--expected-candidate-identity")
    args = parser.parse_args()
    root = args.root.resolve()
    evidence = (args.evidence_root or root / "tmp/v13_2_linux_host" / f"package-{uuid.uuid4().hex}").resolve()
    try:
        if platform.system() != "Linux":
            raise Failure("LINUX_HOST_REQUIRED")
        wsl = "microsoft" in platform.release().lower() or "wsl" in platform.release().lower()
        if args.final and wsl:
            raise Failure("LINUX_HOST_WSL_FORBIDDEN")
        tools = ["cargo", "python3", "readelf"]
        if not args.final:
            tools.append("git")
        for tool in tools:
            if shutil.which(tool) is None:
                raise Failure(f"LINUX_HOST_DEPENDENCY_UNAVAILABLE: {tool}")
        evidence.mkdir(parents=True, exist_ok=False)
        identity = resolve_candidate_identity(
            root, evidence, args.final, args.expected_candidate_identity
        )
        if args.final and identity != args.expected_candidate_identity:
            raise Failure("PACKAGE_CANDIDATE_IDENTITY_MISMATCH")
        packages = []
        binary_hashes: dict[str, dict[str, str]] = {}
        for lane in ("A", "B"):
            target = evidence / f"target-{lane}"
            env = {
                "CARGO_TARGET_DIR": str(target),
                "CARGO_INCREMENTAL": "0",
                "SOURCE_DATE_EPOCH": "946684800",
                "RUSTFLAGS": f"--remap-path-prefix={root}=.",
            }
            run(["cargo", "build", "--release", "--locked", "--bins"], root, env)
            binary_hashes[lane] = {name: sha256(target / "release" / name) for name in ("dbyte", "dbytec", "dbtasm")}
            package = evidence / f"dbyte-v13.2.0-rc-linux-x86_64-{lane}.tar.gz"
            receipt = evidence / f"package-{lane}.json"
            package_command = [
                "python3", str(root / "scripts/package_release_linux.py"),
                "--source-root", str(root),
                "--binary-root", str(target / "release"),
                "--output", str(package),
                "--receipt", str(receipt),
                "--version", "13.2.0",
                "--candidate-identity", identity,
            ]
            if args.final:
                package_command.extend(["--source-manifest", str(root / "source-manifest.json")])
            run(package_command, root)
            packages.append(package)
        if binary_hashes["A"] != binary_hashes["B"]:
            raise Failure("LINUX_HOST_BINARIES_NONDETERMINISTIC")
        package_hashes = [sha256(path) for path in packages]
        if package_hashes[0] != package_hashes[1]:
            raise Failure("LINUX_PACKAGE_NONDETERMINISTIC")
        extract = evidence / "extract"
        extract.mkdir()
        safe_extract(packages[0], extract)
        package_root = extract / "dbyte-v13.2.0-linux-x86_64"
        validate_release_docs(package_root)
        for relative in PROTECTED_WORKTREE_PATHS:
            if (package_root / relative).exists():
                raise Failure(f"PACKAGE_PROTECTED_WORKTREE_PATH_INCLUDED: {relative}")
        forbidden = [
            path
            for path in package_root.rglob("*")
            if "tmp" in path.relative_to(package_root).parts
            or "__pycache__" in path.relative_to(package_root).parts
            or path.suffix.lower()
            in {".img", ".iso", ".qcow2", ".vhd", ".vhdx", ".pyc", ".pem", ".key"}
        ]
        if forbidden:
            raise Failure(f"PACKAGE_FORBIDDEN_MEMBER: {forbidden[0].relative_to(package_root)}")
        version = run([str(package_root / "dbyte"), "--version"], package_root).stdout.strip()
        if version != "DByte 13.2.0":
            raise Failure(f"PACKAGE_VERSION_MISMATCH: {version}")
        run([str(package_root / "dbytec")], package_root, expected=2)
        run([str(package_root / "dbtasm")], package_root, expected=2)
        smoke_elf = evidence / "package-smoke.elf"
        run([str(package_root / "dbytec"), "build", str(package_root / "examples/native_v13_2_linux_host/import_main.dby"), "--target", "x86_64-dbyte-linux", "--emit", "elf", "-o", str(smoke_elf)], package_root)
        run([str(smoke_elf)], package_root, expected=42)
        result = {
            "status": "PASS" if args.final else "DEVELOPMENT_ONLY_PASS",
            "schema": "dbyte-v13.2-linux-package-result-v1",
            "candidate_identity": identity,
            "wsl": wsl,
            "binary_sha256": binary_hashes["A"],
            "package_sha256": package_hashes[0],
            "package_bytes": packages[0].stat().st_size,
            "real_storage": "DISARMED",
        }
        (evidence / "RESULT.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(f"DBYTE V13.2 LINUX PACKAGE: {result['status']}")
        print(f"EVIDENCE ROOT: {evidence}")
        print(f"PACKAGE SHA256: {package_hashes[0]}")
        return 0
    except (OSError, Failure, subprocess.SubprocessError) as error:
        evidence.mkdir(parents=True, exist_ok=True)
        (evidence / "FAILURE.txt").write_text(f"{error}\n", encoding="utf-8")
        print(f"DBYTE V13.2 LINUX PACKAGE: BLOCKED: {error}")
        print(f"EVIDENCE ROOT: {evidence}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
