#!/usr/bin/env python3
"""Reject unsafe blocks without nearby SAFETY comments and pub unsafe fns without docs.

This intentionally uses a lightweight textual scan rather than a Rust parser so
it can run in CI without installing extra tooling. It is scoped to the Hopper
crates that host audited unsafe boundaries.
"""
from __future__ import annotations

from pathlib import Path
import re
import sys

ROOTS = [
    Path("crates/hopper-native/src"),
    Path("crates/hopper-runtime/src"),
    Path("crates/hopper-solana/src"),
    Path("crates/hopper-core/src"),
]

UNSAFE_BLOCK = re.compile(r"\bunsafe\s*\{")
PUB_UNSAFE_FN = re.compile(r"\bpub\s+unsafe\s+fn\b")


def is_doc_or_comment(line: str) -> bool:
    stripped = line.lstrip()
    return stripped.startswith("///") or stripped.startswith("//!")


def nearby_safety(lines: list[str], index: int, radius: int = 3) -> bool:
    start = max(0, index - radius)
    end = min(len(lines), index + radius + 1)
    return any("SAFETY:" in lines[i] for i in range(start, end))


def doc_block_has_safety(lines: list[str], index: int) -> bool:
    seen: list[str] = []
    cursor = index - 1
    while cursor >= 0:
        stripped = lines[cursor].lstrip()
        if (
            stripped.startswith("///")
            or stripped.startswith("#[")
            or stripped == ""
            or stripped.startswith("//")
        ):
            seen.append(lines[cursor])
            cursor -= 1
            continue
        break
    return "# Safety" in "\n".join(reversed(seen))


def main() -> int:
    failures: list[str] = []
    files = sorted({path for root in ROOTS for path in root.rglob("*.rs")})
    for path in files:
        lines = path.read_text(encoding="utf-8").splitlines()
        for idx, line in enumerate(lines):
            if is_doc_or_comment(line):
                continue
            if UNSAFE_BLOCK.search(line) and not nearby_safety(lines, idx):
                failures.append(f"{path}:{idx + 1}: unsafe block lacks nearby SAFETY comment")
            if PUB_UNSAFE_FN.search(line) and not doc_block_has_safety(lines, idx):
                failures.append(f"{path}:{idx + 1}: pub unsafe fn lacks rustdoc # Safety section")

    if failures:
        print("Unsafe safety-comment check failed:", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1
    print(f"checked {len(files)} Rust files for unsafe SAFETY comments")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

    