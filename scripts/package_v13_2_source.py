#!/usr/bin/env python3
"""Create a deterministic, manifest-bound v13.2 BIOSTAR source transfer."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import sys
import tarfile
import tempfile

from v13_2_candidate_identity import PROTECTED, TAG_COMMIT, TAG_OBJECT

EPOCH = 946684800
PREFIX = "dbyte-v13.2.0-source"
FORBIDDEN = {"secret.bin"}
FORBIDDEN_SUFFIXES = (".img", ".iso", ".qcow2", ".vhd", ".vhdx", ".pyc", ".pem", ".key")


class SourcePackageError(RuntimeError):
    pass


def run(root: Path, *arguments: str, binary: bool = False) -> bytes | str:
    result = subprocess.run(arguments, cwd=root, capture_output=True, text=not binary)
    if result.returncode != 0:
        stderr = result.stderr if isinstance(result.stderr, str) else result.stderr.decode(errors="replace")
        raise SourcePackageError(f"SOURCE_PACKAGE_COMMAND_FAILED: {' '.join(arguments)}\n{stderr}")
    return result.stdout


def valid_path(value: str) -> str:
    path = PurePosixPath(value)
    if not value or value.startswith("/") or "\\" in value or ":" in value:
        raise SourcePackageError(f"SOURCE_PACKAGE_PATH_INVALID: {value}")
    if any(part in ("", ".", "..") for part in path.parts):
        raise SourcePackageError(f"SOURCE_PACKAGE_PATH_INVALID: {value}")
    return value


def forbidden_path(relative: str) -> bool:
    path = PurePosixPath(relative)
    lowered = relative.lower()
    return (
        relative in FORBIDDEN
        or "tmp" in path.parts
        or "target" in path.parts
        or "__pycache__" in path.parts
        or lowered.endswith(FORBIDDEN_SUFFIXES)
    )


def untracked_candidate_paths(root: Path) -> set[str]:
    output = str(run(root, "git", "ls-files", "--others", "--exclude-standard"))
    return {
        valid_path(line.replace("\\", "/"))
        for line in output.splitlines()
        if line
    }


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def add_bytes(archive: tarfile.TarFile, name: str, data: bytes, mode: int) -> None:
    info = tarfile.TarInfo(valid_path(name))
    info.size = len(data)
    info.mode = mode
    info.uid = 0
    info.gid = 0
    info.uname = "root"
    info.gname = "root"
    info.mtime = EPOCH
    archive.addfile(info, io.BytesIO(data))


def build(args: argparse.Namespace) -> dict[str, object]:
    root = args.source_root.resolve()
    identity_path = args.identity.resolve()
    run(
        root,
        sys.executable,
        "-B",
        str(root / "scripts/v13_2_candidate_identity.py"),
        "--root",
        str(root),
        "--output",
        str(identity_path),
    )
    identity = json.loads(identity_path.read_text(encoding="utf-8"))
    included_untracked = untracked_candidate_paths(root)
    tracked_lines = str(run(root, "git", "ls-files", "-s")).splitlines()
    files: dict[str, tuple[bytes, int, str]] = {}
    excluded: list[dict[str, str]] = []
    for line in tracked_lines:
        metadata, relative = line.split("\t", 1)
        mode = 0o755 if metadata.split()[0] == "100755" else 0o644
        relative = valid_path(relative.replace("\\", "/"))
        if forbidden_path(relative):
            excluded.append({"path": relative, "reason": "forbidden-transfer-member"})
            continue
        if relative in PROTECTED:
            data = bytes(run(root, "git", "show", f"{TAG_COMMIT}:{relative}", binary=True))
            files[relative] = (data, mode, "frozen-head-protected")
            continue
        source = root / relative
        if source.is_file():
            files[relative] = (source.read_bytes(), mode, "candidate")
    for relative in sorted(included_untracked):
        relative = valid_path(relative)
        if relative in PROTECTED or forbidden_path(relative):
            continue
        source = root / relative
        if source.is_file():
            files[relative] = (source.read_bytes(), 0o755 if relative.endswith(".sh") else 0o644, "candidate-untracked")
    required = ("Cargo.toml", "Cargo.lock", "scripts/verify_v13_2_linux_host.py")
    for relative in required:
        if relative not in files:
            raise SourcePackageError(f"SOURCE_PACKAGE_REQUIRED_FILE_MISSING: {relative}")
    entries = [
        {
            "path": relative,
            "bytes": len(data),
            "sha256": sha256_bytes(data),
            "mode": mode,
            "provenance": provenance,
        }
        for relative, (data, mode, provenance) in sorted(files.items())
    ]
    manifest = {
        "schema": "dbyte-v13.2-source-transfer-v1",
        "version": "13.2.0",
        "candidate_identity": identity["candidate_identity"],
        "base_head": identity["head"],
        "frozen_v13_1_tag_object": TAG_OBJECT,
        "frozen_v13_1_tag_commit": TAG_COMMIT,
        "entries": entries,
        "excluded": sorted(excluded, key=lambda item: item["path"]),
    }
    manifest_bytes = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode()
    sums = "".join(f"{item['sha256']}  {item['path']}\n" for item in entries).encode()
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=output.parent, delete=False) as temporary:
        temporary_path = Path(temporary.name)
    try:
        with temporary_path.open("wb") as raw:
            with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
                with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                    for relative, (data, mode, _) in sorted(files.items()):
                        add_bytes(archive, f"{PREFIX}/{relative}", data, mode)
                    add_bytes(archive, f"{PREFIX}/source-manifest.json", manifest_bytes, 0o644)
                    add_bytes(archive, f"{PREFIX}/SHA256SUMS", sums, 0o644)
        os.replace(temporary_path, output)
    finally:
        temporary_path.unlink(missing_ok=True)
    receipt = {
        "status": "PASS",
        "schema": "dbyte-v13.2-source-package-result-v1",
        "candidate_identity": identity["candidate_identity"],
        "path": str(output),
        "bytes": output.stat().st_size,
        "sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
        "entries": len(entries) + 2,
        "source_entries": len(entries),
    }
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--identity", type=Path, required=True)
    args = parser.parse_args()
    try:
        receipt = build(args)
    except (OSError, SourcePackageError, subprocess.SubprocessError) as error:
        print(error)
        return 1
    print(
        f"DBYTE V13.2 SOURCE PACKAGE: PASS sha256={receipt['sha256']} "
        f"entries={receipt['entries']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
