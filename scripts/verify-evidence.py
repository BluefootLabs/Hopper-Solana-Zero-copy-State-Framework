#!/usr/bin/env python3
"""Check every evidence bundle under audit/ against its SHA256SUMS.

A bundle is a directory holding a SHA256SUMS file that lists
`<sha256>  <path>` for each file in it; most also carry BUNDLE.SHA256, the
hash of SHA256SUMS itself. This script hashes every listed file and checks
every BUNDLE.SHA256. It fails when a file's bytes differ from the listed
hash, when a listed file is missing and the gap is not one recorded below,
and when a bundle hash does not match.

`.gitattributes` stores the devnet bundles byte for byte (no end-of-line
conversion), so a checkout on any system holds the bytes that were hashed.

    python scripts/verify-evidence.py           check the files in the checkout
    python scripts/verify-evidence.py --index   check the staged blobs instead
"""
from __future__ import annotations

import argparse
import hashlib
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Files some early bundles list but never archived: the program dumps taken
# before and after a run and the local release ELF. Their SHA-256s stay in
# the bundle's SHA256SUMS, and the deployed programs can be dumped again from
# devnet and compared with them. Nothing may be added to this list; a bundle
# written today archives every file it lists.
_DUMPS = ["after-onchain.so", "before-onchain.so", "release-local.so"]
NOT_ARCHIVED = {
    "audit/devnet-evidence-2026-09-19/compact-vault": _DUMPS,
    "audit/devnet-evidence-2026-09-19/devnet-audit": _DUMPS,
    "audit/devnet-evidence-2026-09-19/escrow": _DUMPS,
    "audit/devnet-evidence-2026-09-19/migration": _DUMPS,
    "audit/devnet-evidence-2026-09-19/orderbook": _DUMPS,
    "audit/devnet-evidence-2026-09-19/token-2022-vault": _DUMPS,
    "audit/devnet-evidence-2026-09-19/sentinel-authority-gate": [
        "target/hopper/devnet-release/sentinel-v1.so",
        "target/hopper/devnet-release/sentinel-v2.so",
    ],
    "audit/devnet-evidence-2026-09-21/devnet-audit": _DUMPS,
    "audit/devnet-evidence-2026-09-21/devnet-audit-round4": _DUMPS,
    "audit/devnet-evidence-2026-09-21/escrow": _DUMPS,
    "audit/devnet-evidence-2026-09-21/escrow-round4": _DUMPS,
    "audit/devnet-evidence-2026-09-22/round4/devnet-audit": _DUMPS,
    "audit/devnet-evidence-2026-09-22/round4/escrow": _DUMPS,
    "audit/devnet-evidence-2026-09-27/devnet-audit-round5": _DUMPS,
    "audit/devnet-evidence-2026-09-28/devnet-audit-round7": _DUMPS,
}


def git(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *args], cwd=ROOT, capture_output=True, check=False)


def reader(index: bool):
    if index:
        def read(path: str) -> bytes | None:
            result = git("cat-file", "blob", f":{path}")
            return result.stdout if result.returncode == 0 else None
    else:
        def read(path: str) -> bytes | None:
            file = ROOT / path
            return file.read_bytes() if file.is_file() else None
    return read


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--index", action="store_true", help="check the staged blobs instead of the files")
    args = parser.parse_args()
    read = reader(args.index)
    listed = git("ls-files", "audit").stdout.decode().splitlines()
    sums_paths = sorted(path for path in listed if path.endswith("/SHA256SUMS"))
    problems: list[str] = []
    files = 0
    gaps = 0
    for sums_path in sums_paths:
        bundle = sums_path.rsplit("/", 1)[0]
        sums = read(sums_path)
        if sums is None:
            problems.append(f"{bundle}: SHA256SUMS unreadable")
            continue
        allowed = set(NOT_ARCHIVED.get(bundle, []))
        for line in sums.decode("utf-8").splitlines():
            if not line.strip():
                continue
            digest, name = line.split(None, 1)
            name = name.strip().lstrip("*")
            data = read(f"{bundle}/{name}")
            if data is None:
                if name in allowed:
                    gaps += 1
                else:
                    problems.append(f"{bundle}/{name}: listed but missing")
                continue
            files += 1
            if hashlib.sha256(data).hexdigest() != digest:
                problems.append(f"{bundle}/{name}: bytes differ from SHA256SUMS")
        bundle_hash = read(f"{bundle}/BUNDLE.SHA256")
        if bundle_hash is not None:
            expected = bundle_hash.decode("utf-8").split()[0]
            if hashlib.sha256(sums).hexdigest() != expected:
                problems.append(f"{bundle}: BUNDLE.SHA256 does not match SHA256SUMS")
    for problem in problems:
        print(problem)
    where = "staged blobs" if args.index else "files"
    print(f"{len(sums_paths)} bundles, {files} {where} verified, {gaps} recorded as never archived, "
          f"{len(problems)} problems")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
