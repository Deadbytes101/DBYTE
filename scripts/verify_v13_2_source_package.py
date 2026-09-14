#!/usr/bin/env python3
"""Deterministic A/B proof for the v13.2 BIOSTAR source transfer."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import uuid

sys.dont_write_bytecode = True


class VerificationError(RuntimeError):
    pass


def run(command: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(command, cwd=cwd, capture_output=True, text=True)
    if result.returncode != 0:
        raise VerificationError(
            f"SOURCE_PACKAGE_COMMAND_FAILED:{' '.join(command)}:{result.returncode}\n{result.stderr}"
        )
    return result


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
                raise VerificationError(f"SOURCE_PACKAGE_MEMBER_UNSAFE:{member.name}")
        archive.extractall(destination, filter="data")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--evidence-root", type=Path)
    args = parser.parse_args()
    root = args.root.resolve()
    evidence = (
        args.evidence_root
        or root / "tmp/v13_2_linux_host" / f"source-package-{uuid.uuid4().hex}"
    ).resolve()
    try:
        evidence.mkdir(parents=True, exist_ok=False)
        archives = []
        receipts = []
        for lane in ("A", "B"):
            archive = evidence / f"dbyte-v13.2.0-rc-source-{lane}.tar.gz"
            receipt = evidence / f"source-package-{lane}.json"
            identity = evidence / f"candidate-identity-{lane}.json"
            run(
                [
                    sys.executable,
                    "-B",
                    str(root / "scripts/package_v13_2_source.py"),
                    "--source-root",
                    str(root),
                    "--output",
                    str(archive),
                    "--receipt",
                    str(receipt),
                    "--identity",
                    str(identity),
                ],
                root,
            )
            archives.append(archive)
            receipts.append(json.loads(receipt.read_text(encoding="utf-8")))
        hashes = [sha256(archive) for archive in archives]
        if hashes[0] != hashes[1]:
            raise VerificationError("SOURCE_PACKAGE_NONDETERMINISTIC")
        if receipts[0]["candidate_identity"] != receipts[1]["candidate_identity"]:
            raise VerificationError("SOURCE_PACKAGE_IDENTITY_DIVERGENCE")
        extract = evidence / "extract"
        extract.mkdir()
        safe_extract(archives[0], extract)
        source = extract / "dbyte-v13.2.0-source"
        manifest_result = run(
            [
                sys.executable,
                "-B",
                str(source / "scripts/verify_v13_2_source_manifest.py"),
                "--root",
                str(source),
                "--manifest",
                str(source / "source-manifest.json"),
                "--expected-candidate-identity",
                receipts[0]["candidate_identity"],
            ],
            source,
        )
        run(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], source)
        result = {
            "status": "PASS",
            "schema": "dbyte-v13.2-source-package-verifier-v1",
            "candidate_identity": receipts[0]["candidate_identity"],
            "package_sha256": hashes[0],
            "package_bytes": archives[0].stat().st_size,
            "package_entries": receipts[0]["entries"],
            "source_entries": receipts[0]["source_entries"],
            "manifest_result": json.loads(manifest_result.stdout),
            "real_storage": "DISARMED",
        }
        (evidence / "RESULT.json").write_text(
            json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        print("DBYTE V13.2 SOURCE PACKAGE: PASS")
        print(f"EVIDENCE ROOT: {evidence}")
        print(f"PACKAGE SHA256: {hashes[0]}")
        return 0
    except (OSError, json.JSONDecodeError, VerificationError, subprocess.SubprocessError) as error:
        evidence.mkdir(parents=True, exist_ok=True)
        (evidence / "FAILURE.txt").write_text(f"{error}\n", encoding="utf-8")
        print(f"DBYTE V13.2 SOURCE PACKAGE: BLOCKED: {error}")
        print(f"EVIDENCE ROOT: {evidence}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
