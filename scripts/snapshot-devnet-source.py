#!/usr/bin/env python3
"""Create an isolated, clean local validation commit from the current source.

The original repository is never staged or committed. Copies only files
reported by git ls-files (tracked plus non-ignored additions), verifies their
bytes, and records the base commit and copied-source hashes outside the new
repository. This is local working-source evidence, not hosted release CI.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args], stderr=subprocess.PIPE)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    if not out.is_relative_to(ROOT / "target") or out == ROOT / "target":
        raise RuntimeError("snapshot must be a fresh subdirectory of ignored target/")
    if out.exists():
        raise RuntimeError("snapshot destination already exists")
    paths = sorted(set(filter(None, git(ROOT, "ls-files", "-z", "--cached", "--others", "--exclude-standard").decode().split("\0"))))
    inventory = {}
    for relative in paths:
        source = ROOT / relative
        if not source.exists():
            continue  # A tracked working-tree deletion remains deleted.
        if source.is_symlink() or not source.resolve().is_relative_to(ROOT):
            raise RuntimeError(f"source is not a regular in-repository path: {relative}")
        inventory[relative] = hashlib.sha256(source.read_bytes()).hexdigest()
    base = git(ROOT, "rev-parse", "HEAD").decode().strip()
    out.mkdir(parents=True)
    checkout = out / "source"
    git(ROOT, "clone", "--shared", "--no-checkout", "--", str(ROOT), str(checkout))
    git(checkout, "read-tree", "HEAD")
    for relative, expected in inventory.items():
        target = checkout / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / relative, target)
        if hashlib.sha256(target.read_bytes()).hexdigest() != expected:
            raise RuntimeError(f"source changed while copying: {relative}")
    git(checkout, "config", "core.autocrlf", "false")
    git(checkout, "config", "core.safecrlf", "false")
    git(checkout, "add", "--update")
    pathspec = out / "paths.txt"
    pathspec.write_bytes(b"\0".join(p.encode() for p in inventory) + b"\0")
    git(checkout, "add", "--force", "--pathspec-file-nul", "--pathspec-from-file=" + str(pathspec))
    git(checkout, "-c", "user.name=Hopper validation", "-c", "user.email=validation@localhost",
        "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "Snapshot source for local devnet validation")
    if git(checkout, "status", "--porcelain").strip():
        raise RuntimeError("snapshot is not clean")
    for relative, expected in inventory.items():
        if hashlib.sha256((ROOT / relative).read_bytes()).hexdigest() != expected:
            raise RuntimeError(f"original source changed: {relative}")
    receipt = {"schema": "hopper.local-source-snapshot.v1", "baseCommit": base,
        "snapshotCommit": git(checkout, "rev-parse", "HEAD").decode().strip(),
        "files": inventory, "sourceUnchangedDuringCopy": True, "snapshotClean": True,
        "scope": "Local validation snapshot of working files; not a published or hosted-CI release commit."}
    (out / "snapshot.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(f"Copied {len(inventory)} files; clean validation commit {receipt['snapshotCommit']}")


if __name__ == "__main__":
    main()
