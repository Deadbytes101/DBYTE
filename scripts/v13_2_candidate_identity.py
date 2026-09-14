#!/usr/bin/env python3
"""Create a content identity for the current reviewed v13.2 candidate."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess

PROTECTED = {
    "examples/.cube-state.bin",
    "examples/hello.dby",
    "examples/native_v13_machine/cpu.dby",
    "examples/patch_file.dby",
    "examples/smoke.dby",
}
TAG_OBJECT = "b8dfb2e9c11f5803fc371b0cfb00645527419a12"
TAG_COMMIT = "867bebea05d796855590099d2340de09f2ced4d5"


def git(root: Path, *arguments: str) -> str:
    result = subprocess.run(
        ("git", "-c", "core.autocrlf=true", "-c", "core.filemode=false", *arguments),
        cwd=root,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"IDENTITY_GIT_FAILED: {' '.join(arguments)}\n{result.stderr}")
    return result.stdout.strip()


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def compute_candidate_identity(payload: dict[str, object]) -> str:
    authority = {
        "schema": payload["schema"],
        "head": payload["head"],
        "frozen_v13_1_tag_object": payload["frozen_v13_1_tag_object"],
        "frozen_v13_1_tag_commit": payload["frozen_v13_1_tag_commit"],
        "included_dirty_inputs": payload["included_dirty_inputs"],
    }
    canonical = json.dumps(authority, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(canonical).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = args.root.resolve()
    head = git(root, "rev-parse", "HEAD")
    tag_line = git(root, "for-each-ref", "refs/tags/v13.1.0", "--format=%(objectname) %(*objectname)")
    fields = tag_line.split()
    if fields != [TAG_OBJECT, TAG_COMMIT]:
        raise RuntimeError(f"V13_1_TAG_IDENTITY_MISMATCH: {tag_line}")
    modified = set(filter(None, git(root, "diff", "--name-only").splitlines()))
    untracked = set(filter(None, git(root, "ls-files", "--others", "--exclude-standard").splitlines()))
    dirty = sorted(modified | untracked)
    included = []
    excluded = []
    for relative in dirty:
        normalized = relative.replace("\\", "/")
        if normalized in PROTECTED:
            excluded.append({"path": normalized, "reason": "user-owned-protected"})
            continue
        path = root / normalized
        if not path.is_file():
            included.append({"path": normalized, "state": "deleted"})
        else:
            included.append({"path": normalized, "bytes": path.stat().st_size, "sha256": digest(path)})
    payload = {
        "schema": "dbyte-v13.2-candidate-identity-v1",
        "head": head,
        "frozen_v13_1_tag_object": TAG_OBJECT,
        "frozen_v13_1_tag_commit": TAG_COMMIT,
        "included_dirty_inputs": included,
        "excluded_dirty_inputs": excluded,
    }
    payload["candidate_identity"] = compute_candidate_identity(payload)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(payload["candidate_identity"])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
