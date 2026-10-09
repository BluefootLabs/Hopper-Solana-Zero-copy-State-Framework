#!/usr/bin/env python3
"""Run a check and bind its exit code and log to the current source inventory.

Example: python scripts/record-quality-gate.py --out target/fmt.json -- cargo fmt --all -- --check
The command is executed directly, without a shell. Receipts are local evidence,
not authenticated CI results. Source changes during execution fail the check.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def digest(path: Path, *, normalize_line_endings: bool = False) -> str:
    data = path.read_bytes()
    if normalize_line_endings:
        data = data.replace(b"\r\n", b"\n")
    return hashlib.sha256(data).hexdigest()


def sources(root: Path) -> dict[str, str]:
    result = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=root, check=True, capture_output=True,
    )
    paths = sorted(set(p for p in result.stdout.decode("utf-8").split("\0")
        if not p.startswith("audit/") and (p == "Cargo.lock"
        or Path(p).suffix in (".rs", ".toml", ".py", ".yml", ".yaml", ".json", ".stderr", ".md"))))
    if not paths:
        raise RuntimeError("no gate sources found")
    for path in paths:
        if not (root / path).resolve().is_relative_to(root.resolve()):
            raise RuntimeError(f"source escapes repository: {path}")
    return {path: digest(root / path, normalize_line_endings=True) for path in paths}


def record(root: Path, out: Path, argv: list[str]) -> int:
    out = out.resolve()
    if not out.is_relative_to(root.resolve()):
        raise RuntimeError("receipt must stay inside the repository")
    relative = out.relative_to(root.resolve())
    ignored = subprocess.run(["git", "check-ignore", "--quiet", "--no-index", "--", relative.as_posix()], cwd=root, check=False).returncode == 0
    if relative.parts[0] != "audit" and not ignored:
        raise RuntimeError("receipt must be under audit/ or ignored build storage to avoid hashing itself")
    log = out.with_suffix(".log")
    if out.exists() or log.exists():
        raise RuntimeError("receipt or log already exists; choose a fresh output")
    if not argv or not argv[0].strip():
        raise RuntimeError("a command is required after --")
    before = sources(root)
    out.parent.mkdir(parents=True, exist_ok=True)
    with log.open("xb") as stream:
        result = subprocess.run(argv, cwd=root, stdout=stream, stderr=subprocess.STDOUT, check=False)
    completed = dt.datetime.now(dt.timezone.utc).date().isoformat()
    after = sources(root)
    code = result.returncode if before == after else 125
    receipt = {
        "schemaVersion": 1, "command": " ".join(argv), "argv": argv,
        "completedOn": completed, "exitCode": code, "sourceFiles": before,
        "log": {"path": log.relative_to(root.resolve()).as_posix(), "sha256": digest(log)},
    }
    out.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8", newline="\n")
    print(f"exit={code} receipt={out.relative_to(root.resolve()).as_posix()}", flush=True)
    if before != after:
        print("source changed during execution; receipt cannot attest a passing gate", file=sys.stderr)
    return code


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    return record(ROOT, args.out, command)


if __name__ == "__main__":
    sys.exit(main())
