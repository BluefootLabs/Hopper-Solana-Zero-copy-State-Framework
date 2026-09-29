#!/usr/bin/env python3
"""Build the audit map of Hopper's own `unsafe` code, and keep a review
ledger against it.

An auditor's first week on a zero-copy framework goes to finding the
`unsafe` code, reading what each site claims, and working out which claims
anything tests. This script does that part and writes it down:

    audit/unsafe-map.json   every site, machine readable
    audit/UNSAFE_MAP.md     totals per crate and the reading order

For every `unsafe` block, function, impl, and trait outside test code it
records the file, the enclosing function, the justification written next
to it (`// SAFETY:` or a `# Safety` doc section), whether that text is
specific or the shared boilerplate sentence, the tests that call the
enclosing function, and a SHA-256 of the site's code.

A site is identified by `path::function#n`, not by line number, so an edit
elsewhere in the file does not move it.

The review ledger (`audit/unsafe-review.json`) records who reviewed which
site at which hash. `--check` reports every reviewed site whose code has
changed since, so a sign-off cannot silently outlive the code it covered.

    python scripts/audit-map.py                 write the map
    python scripts/audit-map.py --verify        fail if the committed map is stale,
                                                if a site has no reasoning of its
                                                own, or if no test reaches a site
                                                that runs on the host
    python scripts/audit-map.py --check         report review drift (exit 2 on drift)
    python scripts/audit-map.py --sign ID --reviewer NAME [--note TEXT]
    python scripts/audit-map.py --show ID       print one site with its code
"""
from __future__ import annotations

import argparse
from collections import deque
import hashlib
import json
import re
import subprocess
import sys
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MAP_JSON = ROOT / "audit/unsafe-map.json"
MAP_MD = ROOT / "audit/UNSAFE_MAP.md"
LEDGER = ROOT / "audit/unsafe-review.json"
SCHEMA = "hopper.unsafe-map.v1"
LEDGER_SCHEMA = "hopper.unsafe-review.v1"

# The sentence used where no site-specific reasoning was written.
BOILERPLATE = "reviewed zero-copy/backend boundary"
# Names too common to attribute a test call to one function.
COMMON = {
    "new", "get", "set", "len", "from", "into", "read", "write", "load", "next", "push",
    "pop", "iter", "as_ref", "as_mut", "default", "clone", "drop", "deref", "deref_mut",
    "fmt", "eq", "hash", "index", "is_empty", "as_slice", "as_ptr", "as_mut_ptr", "init",
    "close", "check", "parse", "emit", "invoke", "owner", "data", "key", "address",
}


def tracked(pattern: str) -> list[str]:
    out = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "--cached", "--others", "--exclude-standard", pattern],
        capture_output=True, text=True, check=True,
    ).stdout.split()
    return sorted(out)


def mask(text: str) -> str:
    """The text with comments, strings, and char literals blanked, same length."""
    out = list(text)
    i, n = 0, len(text)

    def blank(a: int, b: int) -> None:
        for k in range(a, b):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = text[i]
        two = text[i:i + 2]
        if two == "//":
            j = text.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif two == "/*":
            depth, j = 1, i + 2
            while j < n and depth:
                if text[j:j + 2] == "/*":
                    depth, j = depth + 1, j + 2
                elif text[j:j + 2] == "*/":
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif c == '"' or (c in "br" and re.match(r'(?:b|r|br)#*"', text[i:i + 8])):
            match = re.match(r'(b?r)(#*)"', text[i:])
            if match:
                close = '"' + match.group(2)
                j = text.find(close, i + match.end())
                j = n if j < 0 else j + len(close)
            else:
                j = i + (2 if c == "b" else 1)
                while j < n and text[j] != '"':
                    j += 2 if text[j] == "\\" else 1
                j = min(n, j + 1)
            blank(i, j)
            i = j
        elif c == "'":
            match = re.match(r"'(?:\\.[^']*|[^'\\])'", text[i:])
            if match:
                blank(i, i + match.end())
                i += match.end()
            else:
                i += 1  # a lifetime
        else:
            i += 1
    return "".join(out)


def matching_brace(masked: str, open_at: int) -> int:
    depth = 0
    for k in range(open_at, len(masked)):
        if masked[k] == "{":
            depth += 1
        elif masked[k] == "}":
            depth -= 1
            if depth == 0:
                return k
    return len(masked) - 1


def spans(masked: str, pattern: str) -> list[tuple[int, int, str]]:
    """(start, end, name) of every item the pattern opens, body included."""
    found = []
    for match in re.finditer(pattern, masked):
        brace = body_open(masked, match.end())
        if brace is None:
            continue
        found.append((match.start(), matching_brace(masked, brace), match.group(1)))
    return found


TEST_ITEM = re.compile(
    r"#\[cfg\((?:all\(\s*)?(test|kani)\b[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*"
    r"(?:pub(?:\([a-z]+\))?\s+)?(?:const\s+)?(?:unsafe\s+)?(?:mod|fn)\s+\w+"
)


def test_items(masked: str) -> list[tuple[int, int, str]]:
    """Every module or function compiled only for tests or proofs:
    `#[cfg(test)]`, `#[cfg(kani)]`, and `#[cfg(all(test, ...))]`."""
    found = []
    for match in TEST_ITEM.finditer(masked):
        brace = body_open(masked, match.end())
        if brace is None:
            continue
        found.append((match.start(), matching_brace(masked, brace), match.group(1)))
    return found


def body_open(masked: str, start: int) -> int | None:
    """The `{` that opens the body of the item whose header starts at
    `start`, or `None` for a declaration that ends in `;`. A `;` inside
    parentheses or brackets (an array type in a signature) ends nothing."""
    depth = 0
    for k in range(start, min(len(masked), start + 4000)):
        c = masked[k]
        if c in "([":
            depth += 1
        elif c in ")]":
            depth -= 1
        elif depth == 0 and c == "{":
            return k
        elif depth == 0 and c == ";":
            return None
    return None


def line_of(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def comment_text(lines: list[str]) -> str:
    return " ".join(" ".join(line.strip().lstrip("/").strip() for line in lines).split())


def from_label(text: str) -> str:
    """The part of a comment from its `SAFETY` label on."""
    return re.sub(r"^.*?SAFETY\b[:.]?\s*", "", text, count=1).strip()


ONE_LINE_IMPL = re.compile(r"^\s*unsafe\s+impl\b.*\{\s*\}\s*$")


def justification(lines: list[str], line: int, kind: str) -> tuple[str, str]:
    """The reasoning written for the site at 1-based `line`, and how it was
    found:

    - `own`: a `SAFETY` comment directly above the site's statement, or the
      `# Safety` section of the item's doc comment;
    - `shared`: a `SAFETY` comment within three lines that belongs to a
      neighbouring site (a group of one-line impls under one comment, two
      blocks under one comment);
    - `unlabelled`: a comment directly above with no `SAFETY` label;
    - `none`.
    """
    index = line - 1
    if kind in ("fn", "trait"):
        k = index - 1
        doc = []
        while k >= 0 and (lines[k].lstrip().startswith(("///", "#[", "//")) or not lines[k].strip()):
            if not lines[k].strip() and not doc:
                break
            doc.append(lines[k].strip())
            k -= 1
        doc.reverse()
        joined = "\n".join(doc)
        section = re.search(r"///\s*# Safety\s*\n((?:///.*\n?)*)", joined)
        if section:
            body = re.sub(r"^///\s?", "", section.group(1), flags=re.M).strip()
            body = re.split(r"\n#+ ", body)[0].strip()
            if body:
                return " ".join(body.split()), "own"

    # Walk up to the first line of the statement the site sits in.
    top = index
    while top > 0:
        previous = lines[top - 1].strip()
        if not previous or previous.startswith(("//", "#[")):
            break
        if previous.endswith((";", "{", "}", ",")) and not previous.endswith(("= {", "({")):
            break
        top -= 1
    # The contiguous comment block directly above it.
    block = []
    k = top - 1
    while k >= 0 and lines[k].strip().startswith("#["):
        k -= 1
    while k >= 0 and lines[k].strip().startswith("//") and not lines[k].strip().startswith("///"):
        block.append(lines[k])
        k -= 1
    block.reverse()
    text = comment_text(block)
    if "SAFETY" in text:
        return from_label(text), "own"
    # A trailing comment on the site's own line.
    same = re.search(r"//.*SAFETY.*$", lines[index])
    if same:
        return from_label(comment_text([same.group(0)])), "own"

    # A paragraph made only of `unsafe impl` items, attributes, and
    # comments shares the comment it carries.
    if kind == "impl":
        a = index
        while a > 0 and lines[a - 1].strip():
            a -= 1
        b = index
        while b + 1 < len(lines) and lines[b + 1].strip():
            b += 1
        paragraph = lines[a:b + 1]
        comments = [l for l in paragraph if l.strip().startswith("//") and not l.strip().startswith("///")]
        code = " ".join(
            l.strip() for l in paragraph
            # A closing brace ends the module the group sits in.
            if not l.strip().startswith("//") and l.strip() != "}"
        )
        code = re.sub(r"#\[(?:[^\[\]]|\[[^\]]*\])*\]", " ", code)
        only_impls = re.fullmatch(r"(?:\s*unsafe\s+impl[<\s][^{}]*\{\s*\})+\s*", code) is not None
        if only_impls and comments:
            group_text = comment_text(comments)
            if "SAFETY" in group_text:
                return from_label(group_text), "shared"
            return group_text, "unlabelled"

    # The gate's rule: a `SAFETY` comment within three lines, either side.
    for k in list(range(index - 1, max(-1, index - 4), -1)) + list(range(index + 1, min(len(lines), index + 4))):
        if "SAFETY" in lines[k] and "//" in lines[k]:
            chunk = [lines[k][lines[k].index("//"):]]
            k2 = k + 1
            while k2 < len(lines) and lines[k2].strip().startswith("//"):
                chunk.append(lines[k2])
                k2 += 1
            return from_label(comment_text(chunk)), "shared"
    if text:
        return text, "unlabelled"
    return "", "none"


def on_chain_only(text: str, masked: str, at: int) -> bool:
    """Whether the site sits under `#[cfg(target_os = "solana")]`: on the
    attribute's own item or statement, or inside a block it gates. Such a
    site cannot run in a host test; the VM suites and devnet cover it."""
    gate = re.compile(r'#\[cfg\((?:all\()?\s*target_os\s*=\s*"solana"')
    # `masked` blanks the string, so match on the text.
    for match in gate.finditer(text, max(0, at - 6000), at):
        close = text.find("]", match.end())
        if close < 0:
            continue
        rest = masked[close + 1:]
        item = re.match(r"\s*(?:#\[[^\]]*\]\s*)*", rest)
        begin = close + 1 + (item.end() if item else 0)
        brace = masked.find("{", begin)
        semi = masked.find(";", begin)
        if brace >= 0 and (semi < 0 or brace < semi):
            end = matching_brace(masked, brace)
        else:
            end = semi if semi >= 0 else begin
        if begin <= at <= end:
            return True
    return False


def build() -> dict:
    sources = [
        name for name in tracked("*.rs")
        if (name.startswith(("crates/", "src/", "tools/")))
        and "/tests/" not in name and "/benches/" not in name and "/examples/" not in name
        and "/fuzz/" not in name and not name.endswith("_tests.rs")
    ]
    test_files = [
        name for name in tracked("*.rs")
        if "/tests/" in name or name.startswith(("tests/", "fuzz/", "bench/"))
        # A `*_tests.rs` module is declared under `#[cfg(test)]` by its parent.
        or name.endswith("_tests.rs")
    ]

    # Test regions: whole test files, and `#[cfg(test)]` / `#[cfg(kani)]`
    # modules inside source files.
    regions: list[tuple[str, str]] = []
    for name in test_files:
        regions.append((name, mask((ROOT / name).read_text(encoding="utf-8", errors="replace"))))

    sites = []
    test_site_count = 0
    parsed = {}
    for name in sources:
        text = (ROOT / name).read_text(encoding="utf-8", errors="replace").replace("\r\n", "\n")
        masked = mask(text)
        parsed[name] = (text, masked)
        for a, b, kind in test_items(masked):
            label = f"{name} ({'proof' if kind == 'kani' else 'test'} module)"
            regions.append((label, masked[a:b]))
    region_index = [(label, body) for label, body in regions]

    # Which functions do the tests reach? A test calls some by name; those
    # call others. The graph is by name, within the audited sources, and
    # skips names too common to mean one function.
    call = re.compile(r"\b([a-z_][a-z0-9_]*)\s*(?:::<[^>()]*>)?\s*\(")
    by_path = re.compile(r"::([a-z_][a-z0-9_]*)\s*[,)]")
    defined: dict[str, set[str]] = {}
    for name in sources:
        text, masked = parsed[name]
        in_test = [(a, b) for a, b, _ in test_items(masked)]
        for start, end, function in spans(masked, r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)"):
            if any(a <= start <= b for a, b in in_test):
                continue
            if function in COMMON or len(function) <= 3:
                continue
            body = masked[start:end]
            callees = {c for c in call.findall(body) if c != function}
            callees.update(by_path.findall(body))
            defined.setdefault(function, set()).update(callees)
    called_by_tests: set[str] = set()
    for _, body in region_index:
        called_by_tests.update(c for c in call.findall(body) if c in defined)
    # `reached_from[f]` is a function a test calls by name that leads to `f`.
    # Breadth first and in sorted order, so every run of the script names
    # the same (and the nearest) tested function for each site.
    reached_from: dict[str, str] = {f: f for f in sorted(called_by_tests)}
    frontier = deque(sorted(called_by_tests))
    while frontier:
        function = frontier.popleft()
        for callee in sorted(defined.get(function, ())):
            if callee in defined and callee not in reached_from:
                reached_from[callee] = reached_from[function]
                frontier.append(callee)

    for name in sources:
        text, masked = parsed[name]
        lines = text.split("\n")
        fns = spans(masked, r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")
        impls = spans(masked, r"\bimpl\b[^{;]*?\b([A-Z][A-Za-z0-9_]*)\s*(?:<[^{;]*>)?\s*(?:where[^{]*)?(?=\{)")
        test_spans = [(a, b) for a, b, _ in test_items(masked)]
        macros = spans(masked, r"\bmacro_rules!\s*([A-Za-z_][A-Za-z0-9_]*)")
        ordinals: dict[str, int] = {}
        for match in re.finditer(r"\bunsafe\b", masked):
            at = match.start()
            after = text[match.end():match.end() + 80].lstrip()
            if after.startswith("fn") or re.match(r'extern\s+"[A-Za-z\-]+"\s+fn\b', after):
                kind = "fn"
            elif after.startswith("impl"):
                kind = "impl"
            elif after.startswith("trait"):
                kind = "trait"
            elif after.startswith("extern"):
                kind = "extern"
            elif after.startswith("{"):
                kind = "block"
            else:
                continue
            if any(a <= at <= b for a, b in test_spans):
                test_site_count += 1
                continue
            if kind == "block":
                open_at = masked.find("{", match.end())
                end = matching_brace(masked, open_at)
            else:
                brace = body_open(masked, match.end())
                if brace is None:
                    semi = masked.find(";", match.end())
                    end = semi if semi >= 0 else match.end()
                else:
                    end = matching_brace(masked, brace)
            enclosing = [f for f in fns if f[0] <= at <= f[1]]
            if kind == "fn":
                own = re.search(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)", masked[match.end():match.end() + 120])
                function = own.group(1) if own else "?"
            elif enclosing:
                function = max(enclosing, key=lambda f: f[0])[2]
            elif any(m[0] <= at <= m[1] for m in macros):
                # Code a macro expands at its call site: a test reaches it
                # by invoking the macro.
                function = max((m for m in macros if m[0] <= at <= m[1]), key=lambda m: m[0])[2] + "!"
            else:
                named = re.match(r"\s*(?:impl|trait)\b[^{;]*?([A-Z][A-Za-z0-9_]*)\s*(?:for\s+([A-Za-z_][A-Za-z0-9_:<>, ]*))?", masked[match.end():])
                function = (named.group(2) or named.group(1)).strip() if named else "(module)"
                function = re.sub(r"\s+", "", function)
            owner = [i for i in impls if i[0] <= at <= i[1]]
            owner_name = max(owner, key=lambda i: i[0])[2] if owner else None
            line = line_of(text, at)
            prefix = masked[max(0, at - 40):at]
            code = text[at:end + 1]
            normalized = " ".join(code.split())
            key = f"{name}::{function}"
            ordinals[key] = ordinals.get(key, 0) + 1
            reason, found = justification(lines, line, kind)
            calls = []
            attributable = function not in COMMON and len(function) > 3 and kind in ("fn", "block")
            if attributable:
                if function.endswith("!"):
                    needle = re.compile(r"\b" + re.escape(function) + r"\s*[\(\[\{]")
                else:
                    needle = re.compile(r"\b" + re.escape(function) + r"\s*(?:::<[^>]*>)?\s*\(")
                for label, body in region_index:
                    if needle.search(body):
                        calls.append(label)
            sites.append({
                "id": f"{key}#{ordinals[key]}",
                "file": name,
                "line": line,
                "kind": kind,
                "function": function,
                "type": owner_name,
                "public": bool(re.search(r"\bpub\b(?:\([a-z]+\))?\s*(?:const\s+)?$", prefix)) if kind != "block" else None,
                "justification": reason,
                "justification_class": (
                    "none" if not reason
                    else "boilerplate" if BOILERPLATE in reason
                    else "specific" if found == "own"
                    else found
                ),
                "tests": sorted(set(calls))[:12],
                "reached_through": (
                    reached_from.get(function)
                    if attributable and not calls and reached_from.get(function) != function
                    else None
                ),
                "on_chain_only": on_chain_only(text, masked, at),
                "test_attribution": "by name" if attributable else "not attributable",
                "lines_of_code": code.count("\n") + 1,
                "sha256": hashlib.sha256(normalized.encode()).hexdigest(),
            })

    sites.sort(key=lambda s: (s["file"], s["line"]))
    return {
        "schema": SCHEMA,
        "scope": "crates/, src/, tools/ outside tests, benches, examples, and cfg(test) modules",
        "sites": sites,
        "test_code_sites": test_site_count,
    }


def priority(site: dict) -> int:
    """Reading order: what is least explained and least exercised first."""
    score = 0
    if site["justification_class"] == "none":
        score += 6
    elif site["justification_class"] == "boilerplate":
        score += 3
    elif site["justification_class"] in ("shared", "unlabelled"):
        score += 1
    if not site["tests"]:
        if site.get("reached_through") or site.get("on_chain_only"):
            score += 1
        else:
            score += 3 if site["test_attribution"] == "by name" else 1
    if site["public"]:
        score += 2
    if site["kind"] in ("impl", "extern"):
        score -= 1
    if site["lines_of_code"] > 10:
        score += 1
    return score


def crate_of(path: str) -> str:
    parts = path.split("/")
    if parts[0] == "src":
        return "hopper-lang (facade)"
    if parts[1] == "hopper-spl":
        return parts[2]
    return parts[1]


def render(data: dict) -> str:
    sites = data["sites"]
    crates: dict[str, list[dict]] = {}
    for site in sites:
        crates.setdefault(crate_of(site["file"]), []).append(site)
    out = []
    add = out.append
    add("# Unsafe map")
    add("")
    add("Generated by `scripts/audit-map.py` from the source tree. Do not edit by")
    add("hand: run the script, and the gate (`--verify`) fails when this file and")
    add("`unsafe-map.json` no longer match the code.")
    add("")
    add("Every `unsafe` block, function, impl, and trait outside test code is a")
    add("site. A site is named `path::function#n`, so it keeps its name when the")
    add("lines around it move. `unsafe-map.json` holds the full record of each")
    add("one, with a SHA-256 of its code.")
    add("")
    total = len(sites)
    by_class = {k: sum(1 for s in sites if s["justification_class"] == k) for k in ("specific", "shared", "unlabelled", "boilerplate", "none")}
    tested = sum(1 for s in sites if s["tests"])
    attributable = sum(1 for s in sites if s["test_attribution"] == "by name")
    add("## Totals")
    add("")
    add(f"- Sites: {total:,} ({data['test_code_sites']:,} more inside test and proof modules are not listed)")
    add(f"- `specific`: a `SAFETY` comment or `# Safety` section written for the site: {by_class['specific']:,}")
    add(f"- `shared`: the `SAFETY` comment of a neighbouring site within three lines: {by_class['shared']:,}")
    add(f"- `unlabelled`: a comment directly above with no `SAFETY` label: {by_class['unlabelled']:,}")
    add(f"- `boilerplate`: the sentence every unreasoned site carries: {by_class['boilerplate']:,}")
    add(f"- `none`: no comment next to the site: {by_class['none']:,}")
    indirect = sum(1 for s in sites if not s["tests"] and s.get("reached_through"))
    vm_only = sum(
        1 for s in sites
        if not s["tests"] and not s.get("reached_through") and s.get("on_chain_only")
    )
    unreached = sum(
        1 for s in sites
        if s["test_attribution"] == "by name" and not s["tests"]
        and not s.get("reached_through") and not s.get("on_chain_only")
    )
    add(f"- Enclosing function called by name from a test, proof, or fuzz target: {tested:,} of {attributable:,} attributable")
    add(f"- Not called by name, reached through a function a test calls: {indirect:,}")
    add(f"- Compiled for the VM only, so covered by the compiled-program suites and devnet, not by host tests: {vm_only:,}")
    add(f"- Attributable, on the host, and not reached from any test: {unreached:,}")
    add("")
    add("\"Reached through\" follows calls by name inside the audited sources,")
    add("starting from the functions the tests call. It is a text match, so it")
    add("over-counts a little where two functions share a name and under-counts")
    add("calls made through a trait or a macro.")
    add("")
    add("Reading the numbers: \"boilerplate\" means the site carries the sentence")
    add("\"part of Hopper's reviewed zero-copy/backend boundary\", which says a")
    add("person looked and does not say why the code is sound. Those sites are")
    add("where a reviewer's time goes first. \"Called by name\" is a text match on")
    add("the enclosing function's name. It finds direct calls. It does not see a")
    add("test that reaches the function through a safe wrapper, and it skips")
    add("names too common to attribute (`new`, `get`, `load`).")
    add("")
    add("## Per crate")
    add("")
    add("| Crate | Sites | Specific | Shared | Unlabelled | Boilerplate | None | Public unsafe fn | Called from a test |")
    add("|---|---:|---:|---:|---:|---:|---:|---:|---:|")
    for crate in sorted(crates, key=lambda c: -len(crates[c])):
        rows = crates[crate]
        count = lambda pred: sum(1 for s in rows if pred(s))
        add(
            f"| `{crate}` | {len(rows)} | {count(lambda s: s['justification_class'] == 'specific')} "
            f"| {count(lambda s: s['justification_class'] == 'shared')} "
            f"| {count(lambda s: s['justification_class'] == 'unlabelled')} "
            f"| {count(lambda s: s['justification_class'] == 'boilerplate')} "
            f"| {count(lambda s: s['justification_class'] == 'none')} "
            f"| {count(lambda s: s['kind'] == 'fn' and s['public'])} "
            f"| {count(lambda s: bool(s['tests']))} |"
        )
    add("")
    add("## Per file")
    add("")
    add("| File | Sites | Boilerplate or none | Not called from a test |")
    add("|---|---:|---:|---:|")
    files: dict[str, list[dict]] = {}
    for site in sites:
        files.setdefault(site["file"], []).append(site)
    for name in sorted(files, key=lambda f: -len(files[f])):
        rows = files[name]
        weak = sum(1 for s in rows if s["justification_class"] in ("boilerplate", "none"))
        untested = sum(1 for s in rows if not s["tests"])
        add(f"| `{name}` | {len(rows)} | {weak} | {untested} |")
    add("")
    add("## Reading order")
    add("")
    add("The sixty sites to read first: public, thinly justified, and not reached")
    add("from any test. `python scripts/audit-map.py --show <id>` prints a site")
    add("with its code.")
    add("")
    add("| Site | Line | Kind | Justification | Tests |")
    add("|---|---:|---|---|---|")
    ranked = sorted(sites, key=lambda s: (-priority(s), s["file"], s["line"]))
    for site in ranked[:60]:
        kind = ("pub " if site["public"] else "") + ("unsafe " + site["kind"])
        if site["tests"]:
            tests = str(len(site["tests"]))
        elif site.get("reached_through"):
            tests = f"through `{site['reached_through']}`"
        elif site.get("on_chain_only"):
            tests = "VM only"
        else:
            tests = "none"
        add(f"| `{site['id']}` | {site['line']} | {kind} | {site['justification_class']} | {tests} |")
    add("")
    add("## Review ledger")
    add("")
    add("`audit/unsafe-review.json` records a reviewer's sign-off per site with")
    add("the hash of the code that was read:")
    add("")
    add("```text")
    add("python scripts/audit-map.py --sign <id> --reviewer <name> --note <text>")
    add("python scripts/audit-map.py --check")
    add("```")
    add("")
    add("`--check` lists every site that was signed and has changed since, every")
    add("signed site that no longer exists, and how many sites have no sign-off.")
    add("It exits 2 when a signed site changed, so a release cannot carry a")
    add("sign-off for code nobody re-read.")
    add("")
    return "\n".join(out)


def dump(data: dict) -> str:
    return json.dumps(data, indent=1, ensure_ascii=False) + "\n"


def load_ledger() -> dict:
    if LEDGER.exists():
        return json.loads(LEDGER.read_text(encoding="utf-8"))
    return {"schema": LEDGER_SCHEMA, "reviews": {}}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--sign")
    parser.add_argument("--reviewer")
    parser.add_argument("--note", default="")
    parser.add_argument("--show")
    args = parser.parse_args()
    sys.stdout.reconfigure(encoding="utf-8")

    data = build()
    by_id = {site["id"]: site for site in data["sites"]}

    if args.show:
        site = by_id.get(args.show)
        if site is None:
            print(f"no site `{args.show}`")
            return 1
        print(json.dumps(site, indent=2, ensure_ascii=False))
        lines = (ROOT / site["file"]).read_text(encoding="utf-8", errors="replace").split("\n")
        first = max(0, site["line"] - 8)
        last = min(len(lines), site["line"] + site["lines_of_code"] + 1)
        for number in range(first, last):
            print(f"{number + 1:>6}  {lines[number].rstrip()}")
        return 0

    if args.sign:
        if not args.reviewer:
            print("--sign needs --reviewer")
            return 1
        site = by_id.get(args.sign)
        if site is None:
            print(f"no site `{args.sign}`")
            return 1
        ledger = load_ledger()
        ledger["reviews"][site["id"]] = {
            "sha256": site["sha256"],
            "reviewer": args.reviewer,
            "date": date.today().isoformat(),
            "note": args.note,
        }
        ledger["reviews"] = dict(sorted(ledger["reviews"].items()))
        LEDGER.write_text(dump(ledger), encoding="utf-8", newline="\n")
        print(f"signed {site['id']} at {site['sha256'][:16]}")
        return 0

    if args.check:
        ledger = load_ledger()
        reviews = ledger.get("reviews", {})
        changed = [(i, r) for i, r in reviews.items() if i in by_id and by_id[i]["sha256"] != r["sha256"]]
        gone = [i for i in reviews if i not in by_id]
        current = [i for i, r in reviews.items() if i in by_id and by_id[i]["sha256"] == r["sha256"]]
        for site_id, review in changed:
            site = by_id[site_id]
            print(f"CHANGED  {site_id}  {site['file']}:{site['line']}  "
                  f"signed by {review['reviewer']} on {review['date']}")
        for site_id in gone:
            print(f"GONE     {site_id}  signed by {reviews[site_id]['reviewer']}")
        print(f"{len(data['sites'])} sites: {len(current)} signed and unchanged, "
              f"{len(changed)} changed since sign-off, {len(gone)} signed and gone, "
              f"{len(data['sites']) - len(current) - len(changed)} never signed")
        return 2 if changed else 0

    text_json, text_md = dump(data), render(data)
    if args.verify:
        stale = []
        for path, text in ((MAP_JSON, text_json), (MAP_MD, text_md)):
            have = path.read_text(encoding="utf-8").replace("\r\n", "\n") if path.exists() else None
            if have != text:
                stale.append(path.relative_to(ROOT).as_posix())
        if stale:
            print("stale: " + ", ".join(stale) + " (run scripts/audit-map.py)")
            return 1
        # The ratchet: a new site arrives with its reasoning written down
        # and with a test that reaches it, or the gate fails.
        bare = [
            s for s in data["sites"]
            if s["justification_class"] in ("none", "unlabelled", "boilerplate")
        ]
        unreached = [
            s for s in data["sites"]
            if s["test_attribution"] == "by name" and not s["tests"]
            and not s.get("reached_through") and not s.get("on_chain_only")
        ]
        for site in bare:
            print(f"UNJUSTIFIED  {site['id']}  {site['file']}:{site['line']}")
        for site in unreached:
            print(f"UNREACHED    {site['id']}  {site['file']}:{site['line']}")
        if bare or unreached:
            print(f"{len(bare)} sites without their own reasoning, "
                  f"{len(unreached)} host sites no test reaches")
            return 1
        print(f"unsafe map is current: {len(data['sites'])} sites, "
              "each justified, each host site reached by a test")
        return 0

    MAP_JSON.write_text(text_json, encoding="utf-8", newline="\n")
    MAP_MD.write_text(text_md, encoding="utf-8", newline="\n")
    classes = {k: sum(1 for s in data["sites"] if s["justification_class"] == k) for k in ("specific", "shared", "unlabelled", "boilerplate", "none")}
    print(f"{len(data['sites'])} sites written; justification {classes}; "
          f"{sum(1 for s in data['sites'] if s['tests'])} called from a test by name")
    return 0


if __name__ == "__main__":
    sys.exit(main())
