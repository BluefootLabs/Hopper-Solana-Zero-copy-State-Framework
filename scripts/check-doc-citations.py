#!/usr/bin/env python3
"""Fail when a document cites a source file, test, or proof that does not exist.

A safety document that names the test covering an `unsafe` function is only
worth reading while that test exists. This gate reads the documents listed
below, takes every citation of the forms

    path/to/file.rs            a tracked source file (full path or basename)
    file.rs::function_name     a function in that file
    module::tests::function    a test function, anywhere in the workspace
    Kani `harness_name`        a proof harness

and checks each one against the tracked sources. Run it from anywhere:

    python scripts/check-doc-citations.py [--list]
"""
from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DOCUMENTS = ["docs/UNSAFE_INVARIANTS.md"]

FILE_CITATION = re.compile(r"`((?:[A-Za-z0-9_\-./]+/)?[A-Za-z0-9_\-]+\.rs)(?:::([A-Za-z0-9_]+))?`")
TEST_PATH = re.compile(r"`((?:[a-z_0-9]+::)+)([a-z_0-9]+)`")
WILDCARD = re.compile(r"[{*]")


def tracked_sources() -> dict[str, str]:
    names = subprocess.run(
        # Tracked files plus new ones not yet added, never ignored ones.
        ["git", "-C", str(ROOT), "ls-files", "--cached", "--others", "--exclude-standard", "*.rs"],
        capture_output=True, text=True, check=True,
    ).stdout.split()
    return {name: (ROOT / name).read_text(encoding="utf-8", errors="replace") for name in names}


def main() -> int:
    sources = tracked_sources()
    by_basename: dict[str, list[str]] = {}
    for name in sources:
        by_basename.setdefault(name.rsplit("/", 1)[-1], []).append(name)
    functions = {
        name: set(re.findall(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)", text))
        for name, text in sources.items()
    }
    every_function = set().union(*functions.values())
    modules = set()
    for text in sources.values():
        modules.update(re.findall(r"\bmod\s+([a-z_][a-z0-9_]*)", text))
    modules.update(name.rsplit("/", 1)[-1][:-3] for name in sources)

    listing = "--list" in sys.argv
    problems = []
    checked = 0
    for document in DOCUMENTS:
        text = (ROOT / document).read_text(encoding="utf-8")
        for number, line in enumerate(text.splitlines(), 1):
            for path, function in FILE_CITATION.findall(line):
                checked += 1
                candidates = [path] if path in sources else [
                    name for name in by_basename.get(path.rsplit("/", 1)[-1], [])
                    if name.endswith(path)
                ]
                if not candidates:
                    problems.append(f"{document}:{number}: no tracked file `{path}`")
                    continue
                if function and not any(function in functions[c] for c in candidates):
                    problems.append(
                        f"{document}:{number}: `{path}` has no function `{function}`"
                    )
                elif listing:
                    print(f"ok  {document}:{number}: {path}" + (f"::{function}" if function else ""))
            for prefix, function in TEST_PATH.findall(line):
                segments = [s for s in prefix.split("::") if s]
                # Only paths that name a test or proof module are citations;
                # `hopper::token::X` style API paths are not.
                if not ({"tests", "kani_proofs"} & set(segments)) and not any(
                    s.endswith("_tests") for s in segments
                ):
                    continue
                checked += 1
                missing_module = [
                    s for s in segments if s not in modules and s not in ("tests",)
                ]
                if missing_module:
                    problems.append(
                        f"{document}:{number}: no module `{missing_module[0]}` "
                        f"(cited as `{prefix}{function}`)"
                    )
                elif function not in every_function:
                    problems.append(
                        f"{document}:{number}: no function `{function}` "
                        f"(cited as `{prefix}{function}`)"
                    )
                elif listing:
                    print(f"ok  {document}:{number}: {prefix}{function}")
            for harness in re.findall(r"Kani `([A-Za-z0-9_{},*]+)`", line):
                if WILDCARD.search(harness):
                    stem = WILDCARD.split(harness)[0]
                    checked += 1
                    if not any(f.startswith(stem) for f in every_function):
                        problems.append(
                            f"{document}:{number}: no proof harness starts with `{stem}`"
                        )
                    continue
                checked += 1
                if harness not in every_function and harness not in modules:
                    problems.append(f"{document}:{number}: no proof harness `{harness}`")

    for problem in problems:
        print(problem)
    print(f"{checked} citations checked, {len(problems)} unresolved")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
