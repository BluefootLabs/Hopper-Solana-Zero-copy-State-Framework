#!/usr/bin/env python3
"""Gate unsafe contracts across every package in Hopper's public release train."""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib


UNSAFE_BLOCK = re.compile(r"\bunsafe\s*\{")
FUNCTION_ITEM = re.compile(
    r"\b(?P<visibility>pub(?:\s*\([^)]*\))?\s+)?"
    r"(?P<qualifiers>(?:(?:const|async|unsafe)\s+|extern(?:\s+\"[^\"]*\")?\s+)*)"
    r"fn\b",
    re.MULTILINE,
)
UNSAFE_TRAIT = re.compile(
    r"\b(?P<visibility>pub(?:\s*\([^)]*\))?\s+)?unsafe\s+(?:auto\s+)?trait\b",
    re.MULTILINE,
)
UNSAFE_IMPL = re.compile(r"\bunsafe\s+impl\b", re.MULTILINE)
UNSAFE_EXTERN_BLOCK = re.compile(
    r"\bunsafe\s+extern(?:\s+\"[^\"]*\")?\s*\{", re.MULTILINE
)
SKIPPED_COMPONENTS = {".git", "target"}


@dataclasses.dataclass
class FileInventory:
    path: Path
    unsafe_blocks: int = 0
    unsafe_functions: int = 0
    public_unsafe_functions: int = 0
    unsafe_traits: int = 0
    unsafe_impls: int = 0
    unsafe_extern_blocks: int = 0

    def has_unsafe(self) -> bool:
        return any(
            (
                self.unsafe_blocks,
                self.unsafe_functions,
                self.unsafe_traits,
                self.unsafe_impls,
                self.unsafe_extern_blocks,
            )
        )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Check unsafe safety comments across every public package and optionally "
            "emit an inventory."
        )
    )
    parser.add_argument("--root", type=Path, default=Path("."))
    parser.add_argument(
        "--publish-order",
        type=Path,
        default=Path("release/publish-order.toml"),
        help="public package train used to discover the complete scan surface",
    )
    parser.add_argument(
        "--inventory-out",
        type=Path,
        help="Write a markdown inventory of all detected unsafe constructs.",
    )
    parser.add_argument(
        "--require-clean",
        action="store_true",
        help="require a clean checkout and prove the scan did not change it",
    )
    return parser.parse_args()


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git_state(root: Path) -> tuple[str, list[str]]:
    head = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=root,
        check=False,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    status = subprocess.run(
        ["git", "status", "--porcelain=v1", "--untracked-files=all"],
        cwd=root,
        check=False,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if head.returncode != 0 or status.returncode != 0:
        raise RuntimeError("unsafe inventory requires a readable Git source checkout")
    return head.stdout.strip(), [
        line for line in status.stdout.splitlines() if line.strip()
    ]


def load_public_package_roots(root: Path, config: Path) -> tuple[list[Path], list[str]]:
    with config.open("rb") as handle:
        value = tomllib.load(handle)
    if value.get("schema") != "hopper.publish-train.v1":
        raise RuntimeError(f"unsupported publish-order schema in {config}")
    packages = value.get("packages")
    if not isinstance(packages, list) or not packages or not all(
        isinstance(package, str) and package for package in packages
    ):
        raise RuntimeError("publish-order packages must be a non-empty string list")
    if len(packages) != len(set(packages)):
        raise RuntimeError("publish-order contains duplicate package names")

    manifests: dict[str, list[Path]] = {}
    for current, directories, filenames in os.walk(root):
        directories[:] = [
            directory
            for directory in directories
            if directory not in SKIPPED_COMPONENTS
            and directory not in {"__pycache__", "node_modules"}
            and not directory.startswith(".")
        ]
        if "Cargo.toml" not in filenames:
            continue
        manifest = Path(current) / "Cargo.toml"
        try:
            with manifest.open("rb") as handle:
                cargo = tomllib.load(handle)
        except tomllib.TOMLDecodeError as error:
            raise RuntimeError(f"cannot parse {manifest}: {error}") from error
        name = cargo.get("package", {}).get("name")
        if isinstance(name, str):
            manifests.setdefault(name, []).append(manifest)

    roots: list[Path] = []
    for package in packages:
        matches = manifests.get(package, [])
        if len(matches) != 1:
            raise RuntimeError(
                f"public package {package!r} resolved to {len(matches)} manifests: {matches}"
            )
        roots.append(matches[0].parent)
    return roots, packages


def rust_files_for_package(package_root: Path) -> set[Path]:
    files: set[Path] = set()
    # The gate covers code shipped in each public crate. Test fixtures and
    # standalone nested example packages are not part of that crate's public
    # unsafe surface and have separate execution gates.
    for directory in ("src",):
        candidate = package_root / directory
        if candidate.is_dir():
            files.update(
                path
                for path in candidate.rglob("*.rs")
                if not any(component in SKIPPED_COMPONENTS for component in path.parts)
            )
    build_script = package_root / "build.rs"
    if build_script.is_file():
        files.add(build_script)
    return files


def mask_non_code(source: str) -> str:
    """Mask comments and string literals while preserving offsets and newlines."""
    chars = list(source)
    masked = list(source)
    index = 0
    block_depth = 0
    state = "code"
    raw_hashes = 0
    while index < len(chars):
        current = chars[index]
        following = chars[index + 1] if index + 1 < len(chars) else ""
        if state == "line-comment":
            if current == "\n":
                state = "code"
            else:
                masked[index] = " "
            index += 1
            continue
        if state == "block-comment":
            if current == "/" and following == "*":
                masked[index] = masked[index + 1] = " "
                block_depth += 1
                index += 2
            elif current == "*" and following == "/":
                masked[index] = masked[index + 1] = " "
                block_depth -= 1
                index += 2
                if block_depth == 0:
                    state = "code"
            else:
                if current != "\n":
                    masked[index] = " "
                index += 1
            continue
        if state == "string":
            if current == "\\":
                masked[index] = " "
                if index + 1 < len(chars):
                    if chars[index + 1] != "\n":
                        masked[index + 1] = " "
                    index += 2
                else:
                    index += 1
            else:
                if current != "\n":
                    masked[index] = " "
                index += 1
                if current == '"':
                    state = "code"
            continue
        if state == "raw-string":
            terminator = '"' + ("#" * raw_hashes)
            if source.startswith(terminator, index):
                for offset in range(len(terminator)):
                    masked[index + offset] = " "
                index += len(terminator)
                state = "code"
            else:
                if current != "\n":
                    masked[index] = " "
                index += 1
            continue

        if current == "/" and following == "/":
            masked[index] = masked[index + 1] = " "
            state = "line-comment"
            index += 2
        elif current == "/" and following == "*":
            masked[index] = masked[index + 1] = " "
            state = "block-comment"
            block_depth = 1
            index += 2
        elif current == "'":
            character = re.match(
                r"'(?:\\(?:x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f_]+\}|.)|[^'\\\n])'",
                source[index:],
            )
            if character:
                for offset in range(len(character.group(0))):
                    masked[index + offset] = " "
                index += len(character.group(0))
            else:
                # A lifetime such as `'info`, not a character literal.
                index += 1
        elif current == '"':
            masked[index] = " "
            state = "string"
            index += 1
        else:
            raw = re.match(r"(?:br|r)(?P<hashes>#{0,255})\"", source[index:])
            if raw:
                raw_hashes = len(raw.group("hashes"))
                for offset in range(len(raw.group(0))):
                    masked[index + offset] = " "
                index += len(raw.group(0))
                state = "raw-string"
            else:
                index += 1
    if state == "block-comment":
        raise RuntimeError("unterminated block comment")
    if state in ("string", "raw-string"):
        raise RuntimeError("unterminated string literal")
    return "".join(masked)


def nearby_safety(lines: list[str], index: int, radius: int = 3) -> bool:
    start = max(0, index - radius)
    end = min(len(lines), index + radius + 1)
    if any("SAFETY:" in lines[line] for line in range(start, end)):
        return True
    cursor = index - 1
    while cursor >= 0:
        stripped = lines[cursor].rstrip()
        leading = stripped.lstrip()
        if leading.startswith("//") or leading.startswith("#[") or not leading:
            if "SAFETY:" in lines[cursor]:
                return True
            cursor -= 1
            continue
        if stripped.endswith("="):
            cursor -= 1
            continue
        break
    return False


def doc_block_has_safety(lines: list[str], index: int) -> bool:
    seen: list[str] = []
    cursor = index - 1
    while cursor >= 0:
        stripped = lines[cursor].lstrip()
        if (
            stripped.startswith("///")
            or stripped.startswith("#[")
            or not stripped
            or stripped.startswith("//")
        ):
            seen.append(lines[cursor])
            cursor -= 1
            continue
        break
    return "# Safety" in "\n".join(reversed(seen))


def line_index(source: str, offset: int) -> int:
    return source.count("\n", 0, offset)


def scan_source(path: Path, source: str) -> tuple[FileInventory, list[str]]:
    masked = mask_non_code(source)
    lines = source.splitlines()
    inventory = FileInventory(path=path)
    failures: list[str] = []

    for match in UNSAFE_BLOCK.finditer(masked):
        index = line_index(masked, match.start())
        inventory.unsafe_blocks += 1
        if not nearby_safety(lines, index):
            failures.append(f"{path}:{index + 1}: unsafe block lacks nearby SAFETY comment")

    for match in FUNCTION_ITEM.finditer(masked):
        qualifiers = match.group("qualifiers").split()
        if "unsafe" not in qualifiers:
            continue
        index = line_index(masked, match.start())
        inventory.unsafe_functions += 1
        if match.group("visibility"):
            inventory.public_unsafe_functions += 1
            if not doc_block_has_safety(lines, index):
                failures.append(
                    f"{path}:{index + 1}: public unsafe fn lacks rustdoc # Safety section"
                )

    for match in UNSAFE_TRAIT.finditer(masked):
        index = line_index(masked, match.start())
        inventory.unsafe_traits += 1
        if match.group("visibility") and not doc_block_has_safety(lines, index):
            failures.append(
                f"{path}:{index + 1}: public unsafe trait lacks rustdoc # Safety section"
            )

    inventory.unsafe_impls = len(list(UNSAFE_IMPL.finditer(masked)))
    for match in UNSAFE_EXTERN_BLOCK.finditer(masked):
        index = line_index(masked, match.start())
        inventory.unsafe_extern_blocks += 1
        if not nearby_safety(lines, index):
            failures.append(
                f"{path}:{index + 1}: unsafe extern block lacks nearby SAFETY comment"
            )
    return inventory, failures


def write_inventory(
    path: Path,
    inventory: list[FileInventory],
    checked_files: int,
    packages: list[str],
    publish_order: Path,
    source_commit: str,
    source_tree_clean: bool,
) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = [
        "# Unsafe inventory",
        "",
        "Generated by `scripts/check-unsafe-safety-comments.py`.",
        "",
        f"Publish-order SHA-256: `{sha256(publish_order)}`",
        f"Source commit: `{source_commit}`",
        f"Source tree clean: `{str(source_tree_clean).lower()}`",
        f"Public packages: {len(packages)}",
        f"Checked Rust files: {checked_files}",
        "",
        "| Source file | Blocks | Unsafe fns | Public unsafe fns | Unsafe traits | Unsafe impls | Unsafe extern blocks |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    if inventory:
        for item in inventory:
            lines.append(
                f"| `{item.path.as_posix()}` | {item.unsafe_blocks} | "
                f"{item.unsafe_functions} | {item.public_unsafe_functions} | "
                f"{item.unsafe_traits} | {item.unsafe_impls} | "
                f"{item.unsafe_extern_blocks} |"
            )
    else:
        lines.append("| _none_ | 0 | 0 | 0 | 0 | 0 | 0 |")
    lines.append("")
    path.write_text("\n".join(lines), encoding="utf-8")


def main() -> int:
    args = parse_args()
    root = args.root.resolve()
    publish_order = args.publish_order
    if not publish_order.is_absolute():
        publish_order = root / publish_order
    try:
        head_before, dirty_before = git_state(root)
        if args.require_clean and dirty_before:
            raise RuntimeError(
                "unsafe inventory requires a clean worktree:\n"
                + "\n".join(dirty_before[:30])
            )
        package_roots, packages = load_public_package_roots(root, publish_order)
        files = sorted(
            {path for package_root in package_roots for path in rust_files_for_package(package_root)}
        )
        if not files:
            raise RuntimeError("public-package scan resolved no Rust source files")
        failures: list[str] = []
        inventory: list[FileInventory] = []
        for path in files:
            relative = path.relative_to(root)
            item, source_failures = scan_source(
                relative, path.read_text(encoding="utf-8")
            )
            failures.extend(source_failures)
            if item.has_unsafe():
                inventory.append(item)

        if args.inventory_out:
            output = args.inventory_out
            if not output.is_absolute():
                output = root / output
            write_inventory(
                output,
                inventory,
                len(files),
                packages,
                publish_order,
                head_before,
                not dirty_before,
            )

        head_after, dirty_after = git_state(root)
        if head_after != head_before or dirty_after != dirty_before:
            raise RuntimeError(
                "unsafe scan changed the source checkout; write the inventory to an "
                "ignored target path or outside the repository"
            )

        if failures:
            print("Unsafe safety-comment check failed:", file=sys.stderr)
            for failure in failures:
                print(f"  {failure}", file=sys.stderr)
            return 1
        print(
            f"checked {len(files)} Rust files across {len(packages)} public packages "
            "for unsafe contracts"
        )
        if args.inventory_out:
            print(f"wrote unsafe inventory to {args.inventory_out}")
        return 0
    except (OSError, RuntimeError, tomllib.TOMLDecodeError) as error:
        print(f"Unsafe safety-comment check failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
