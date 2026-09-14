#!/usr/bin/env python3
"""Build and verify the deterministic GNU/Linux DByte release tarball."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import subprocess
import tarfile
import tempfile

FIXED_FILES = (
    "README.md",
    "INSTALL.md",
    "LANGUAGE_SPEC.md",
    "LICENSE",
    "benchmarks/BENCHMARKS.md",
)
TREE_ROOTS = ("docs", "examples", "kernel-lab", "runtime")
BINARIES = ("dbyte", "dbytec", "dbtasm")
EPOCH = 946684800
CANDIDATE_EXTRAS = (
    "docs/LINUX_NATIVE_HOST_PORTABILITY.md",
    "examples/native_v13_2_linux_host/cross_target.dby",
    "examples/native_v13_2_linux_host/dbtasm_probe.dbtasm",
    "examples/native_v13_2_linux_host/file_io.dby",
    "examples/native_v13_2_linux_host/import_main.dby",
    "examples/native_v13_2_linux_host/import_value.dby",
    "examples/native_v13_2_linux_host/manifest.json",
    "runtime/v13.2.0/linux_io.dby",
)
PROTECTED_WORKTREE_PATHS = {
    "examples/.cube-state.bin",
    "examples/hello.dby",
    "examples/native_v13_machine/cpu.dby",
    "examples/patch_file.dby",
    "examples/smoke.dby",
}
FORBIDDEN_SUFFIXES = (".img", ".iso", ".qcow2", ".vhd", ".vhdx", ".pyc", ".pem", ".key")
INSTALL_REQUIRED_MARKERS = (
    "GNU/Linux Binary Tarball",
    "./dbyte --version",
    "--target x86_64-dbyte-linux",
    "glibc 2.39",
)
INSTALL_FORBIDDEN_MARKERS = ("dbyte-v11.6.0-windows-x64.zip", "DByte 11.6.0")
README_REQUIRED_MARKERS = (
    "v13.2.0 Linux-native final-qualification line",
    "v13.1.0 portable-native-float release",
    "Final status is receipt-bound",
)
README_FORBIDDEN_MARKERS = ("REAL_BIOSTAR=PENDING",)


class PackageError(RuntimeError):
    pass


def run(root: Path, *args: str) -> str:
    result = subprocess.run(args, cwd=root, text=True, capture_output=True)
    if result.returncode != 0:
        raise PackageError(f"PACKAGE_COMMAND_FAILED: {' '.join(args)}\n{result.stderr}")
    return result.stdout.strip()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def validate_path(value: str) -> str:
    path = PurePosixPath(value)
    if not value or value.startswith("/") or "\\" in value or ":" in value:
        raise PackageError(f"PACKAGE_PATH_INVALID: {value}")
    if any(part in ("", ".", "..") for part in path.parts):
        raise PackageError(f"PACKAGE_PATH_INVALID: {value}")
    return value


def workspace_version(root: Path) -> str:
    section = False
    for line in (root / "Cargo.toml").read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            section = stripped == "[workspace.package]"
        elif section and stripped.startswith("version"):
            return stripped.split("=", 1)[1].strip().strip('"')
    raise PackageError("PACKAGE_WORKSPACE_VERSION_REQUIRED")


def validate_release_docs(root: Path) -> None:
    checks = (
        ("INSTALL.md", INSTALL_REQUIRED_MARKERS, INSTALL_FORBIDDEN_MARKERS),
        ("README.md", README_REQUIRED_MARKERS, README_FORBIDDEN_MARKERS),
    )
    for relative, required, forbidden in checks:
        path = root / relative
        if not path.is_file():
            raise PackageError(f"PACKAGE_RELEASE_DOC_REQUIRED: {relative}")
        body = path.read_text(encoding="utf-8")
        for marker in required:
            if marker not in body:
                raise PackageError(f"PACKAGE_RELEASE_DOC_STALE: {relative}: missing {marker}")
        for marker in forbidden:
            if marker in body:
                raise PackageError(f"PACKAGE_RELEASE_DOC_STALE: {relative}: forbidden {marker}")


def tracked_package_paths(root: Path, source_manifest: Path | None = None) -> list[str]:
    if source_manifest is None:
        output = run(root, "git", "ls-files", "--", *FIXED_FILES, *TREE_ROOTS)
        paths = {validate_path(line) for line in output.splitlines() if line}
    else:
        manifest = json.loads(source_manifest.read_text(encoding="utf-8"))
        entries = manifest.get("entries")
        if not isinstance(entries, list):
            raise PackageError("PACKAGE_SOURCE_MANIFEST_ENTRIES_MISSING")
        paths = set()
        for entry in entries:
            if not isinstance(entry, dict) or not isinstance(entry.get("path"), str):
                raise PackageError("PACKAGE_SOURCE_MANIFEST_PATH_INVALID")
            relative = validate_path(entry["path"])
            in_scope = relative in FIXED_FILES or any(
                relative.startswith(f"{tree}/") for tree in TREE_ROOTS
            )
            if not in_scope:
                continue
            if entry.get("provenance") == "candidate-untracked" and relative not in CANDIDATE_EXTRAS:
                continue
            paths.add(relative)
    for relative in CANDIDATE_EXTRAS:
        if (root / relative).is_file():
            paths.add(validate_path(relative))
    paths.difference_update(PROTECTED_WORKTREE_PATHS)
    for relative in tuple(paths):
        parts = PurePosixPath(relative).parts
        lowered = relative.lower()
        if "tmp" in parts or "__pycache__" in parts or lowered.endswith(FORBIDDEN_SUFFIXES):
            paths.remove(relative)
    paths = sorted(paths)
    for required in ("README.md", "INSTALL.md", "LICENSE"):
        if required not in paths:
            raise PackageError(f"PACKAGE_REQUIRED_FILE_MISSING: {required}")
    runtime = [path for path in paths if path.startswith("runtime/")]
    if not runtime:
        raise PackageError("PACKAGE_RUNTIME_TREE_REQUIRED")
    return paths


def add_bytes(archive: tarfile.TarFile, name: str, data: bytes, mode: int) -> None:
    info = tarfile.TarInfo(validate_path(name))
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
    binary_root = args.binary_root.resolve()
    validate_release_docs(root)
    version = workspace_version(root)
    if version != args.version:
        raise PackageError(f"PACKAGE_VERSION_MISMATCH: requested {args.version}, source is {version}")
    prefix = f"dbyte-v{version}-linux-x86_64"
    entries: list[tuple[str, Path, int]] = []
    for relative in tracked_package_paths(root, getattr(args, "source_manifest", None)):
        source = root / Path(relative)
        if not source.is_file():
            raise PackageError(f"PACKAGE_TRACKED_FILE_MISSING: {relative}")
        entries.append((f"{prefix}/{relative}", source, 0o644))
    for binary in BINARIES:
        source = binary_root / binary
        if not source.is_file() or not os.access(source, os.X_OK):
            raise PackageError(f"PACKAGE_BINARY_REQUIRED: {binary}")
        entries.append((f"{prefix}/{binary}", source, 0o755))
    version_bytes = f"DByte {version}\n".encode()
    content = []
    for name, source, mode in sorted(entries):
        content.append({"path": name, "bytes": source.stat().st_size, "sha256": sha256(source), "mode": mode})
    content.append({"path": f"{prefix}/version.txt", "bytes": len(version_bytes), "sha256": hashlib.sha256(version_bytes).hexdigest(), "mode": 0o644})
    manifest = {
        "schema": "dbyte-linux-package-v1",
        "version": version,
        "candidate_identity": args.candidate_identity,
        "entries": sorted(content, key=lambda item: str(item["path"])),
    }
    manifest_bytes = (json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode()
    sums = "".join(f"{item['sha256']}  {item['path']}\n" for item in manifest["entries"]).encode()
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=output.parent, delete=False) as temporary:
        temp_path = Path(temporary.name)
    try:
        with temp_path.open("wb") as raw:
            with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
                with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                    for name, source, mode in sorted(entries):
                        add_bytes(archive, name, source.read_bytes(), mode)
                    add_bytes(archive, f"{prefix}/version.txt", version_bytes, 0o644)
                    add_bytes(archive, f"{prefix}/manifest.json", manifest_bytes, 0o644)
                    add_bytes(archive, f"{prefix}/SHA256SUMS", sums, 0o644)
        os.replace(temp_path, output)
    finally:
        temp_path.unlink(missing_ok=True)
    receipt = {"status": "PASS", "path": str(output), "bytes": output.stat().st_size, "sha256": sha256(output), "entries": len(entries) + 3, "manifest": manifest}
    args.receipt.parent.mkdir(parents=True, exist_ok=True)
    args.receipt.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return receipt


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--binary-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--candidate-identity", required=True)
    parser.add_argument("--source-manifest", type=Path)
    args = parser.parse_args()
    try:
        receipt = build(args)
    except (OSError, PackageError, subprocess.SubprocessError) as error:
        print(error)
        return 1
    print(f"DBYTE LINUX PACKAGE: PASS sha256={receipt['sha256']} entries={receipt['entries']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
