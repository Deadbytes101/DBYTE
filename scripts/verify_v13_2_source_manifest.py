#!/usr/bin/env python3
"""Verify an extracted v13.2 source transfer before any build runs."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import sys

sys.dont_write_bytecode = True
TAG_OBJECT = "b8dfb2e9c11f5803fc371b0cfb00645527419a12"
TAG_COMMIT = "867bebea05d796855590099d2340de09f2ced4d5"


class ManifestError(RuntimeError):
    pass


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify(root: Path, manifest_path: Path, expected_identity: str) -> dict[str, object]:
    if manifest_path != root / "source-manifest.json":
        raise ManifestError("SOURCE_MANIFEST_LOCATION_INVALID")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("schema") != "dbyte-v13.2-source-transfer-v1":
        raise ManifestError("SOURCE_MANIFEST_SCHEMA_MISMATCH")
    if manifest.get("version") != "13.2.0":
        raise ManifestError("SOURCE_MANIFEST_VERSION_MISMATCH")
    if manifest.get("frozen_v13_1_tag_object") != TAG_OBJECT:
        raise ManifestError("SOURCE_MANIFEST_TAG_OBJECT_MISMATCH")
    if manifest.get("frozen_v13_1_tag_commit") != TAG_COMMIT:
        raise ManifestError("SOURCE_MANIFEST_TAG_COMMIT_MISMATCH")
    if manifest.get("candidate_identity") != expected_identity:
        raise ManifestError("SOURCE_MANIFEST_CANDIDATE_IDENTITY_MISMATCH")
    entries = manifest.get("entries")
    if not isinstance(entries, list):
        raise ManifestError("SOURCE_MANIFEST_ENTRIES_MISSING")
    seen: set[str] = set()
    for entry in entries:
        relative = entry.get("path") if isinstance(entry, dict) else None
        if not isinstance(relative, str):
            raise ManifestError("SOURCE_MANIFEST_PATH_INVALID")
        path = PurePosixPath(relative)
        if path.is_absolute() or ".." in path.parts or relative in seen:
            raise ManifestError("SOURCE_MANIFEST_PATH_INVALID")
        seen.add(relative)
        source = root / relative
        if not source.is_file():
            raise ManifestError(f"SOURCE_MANIFEST_FILE_MISSING:{relative}")
        if source.stat().st_size != entry.get("bytes") or sha256(source) != entry.get("sha256"):
            raise ManifestError(f"SOURCE_MANIFEST_FILE_MISMATCH:{relative}")
    allowed_extras = {"SHA256SUMS", "source-manifest.json"}
    actual = {
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if path.is_file() and ".git" not in path.relative_to(root).parts
    }
    extras = sorted(actual - seen - allowed_extras)
    if extras:
        raise ManifestError(f"SOURCE_MANIFEST_UNEXPECTED_FILE:{extras[0]}")
    expected_sums = "".join(
        f"{entry['sha256']}  {entry['path']}\n" for entry in entries
    ).encode()
    sums_path = root / "SHA256SUMS"
    if not sums_path.is_file() or sums_path.read_bytes() != expected_sums:
        raise ManifestError("SOURCE_MANIFEST_SHA256SUMS_MISMATCH")
    return {
        "status": "PASS",
        "schema": "dbyte-v13.2-source-manifest-result-v1",
        "candidate_identity": expected_identity,
        "source_entries": len(seen),
        "manifest_sha256": sha256(manifest_path),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--expected-candidate-identity", required=True)
    args = parser.parse_args()
    try:
        result = verify(
            args.root.resolve(),
            args.manifest.resolve(),
            args.expected_candidate_identity,
        )
    except (OSError, json.JSONDecodeError, ManifestError) as error:
        print(f"DBYTE V13.2 SOURCE MANIFEST: BLOCKED: {error}")
        return 1
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
