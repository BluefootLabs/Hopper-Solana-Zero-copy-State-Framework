#!/usr/bin/env python3
"""Fail-closed, non-publishing validation for Hopper's crates.io train."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import tomllib
import unittest
import urllib.parse
from typing import Any
from unittest import mock


SCHEMA = "hopper.publish-train-attestation.v2"
EXPECTED_AUTHORS = ["QuarksBlueFoot <quark@bluefoot.tech>"]
EXPECTED_PUBLISH_REGISTRIES = ["crates-io"]
REQUIRED_METADATA = (
    "description",
    "license",
    "repository",
    "homepage",
    "documentation",
    "readme",
)
INLINE_LINK_RE = re.compile(
    r"!?\[[^\]\n]*\]\(\s*(?P<target><[^>\n]+>|[^\s)\n]+)"
)
REFERENCE_LINK_RE = re.compile(
    r"^\s{0,3}\[[^\]\n]+\]:\s*(?P<target><[^>\n]+>|\S+)", re.MULTILINE
)
INCLUDE_MACRO_RE = re.compile(r"\binclude(?:_str|_bytes)?!\s*\(")
INCLUDE_LITERAL_ARG_RE = re.compile(
    r'\s*(?:"(?P<normal>[^"\\\r\n]*)"|'
    r'r(?P<hashes>#{0,16})"(?P<raw>.*?)"(?P=hashes))\s*\)',
    re.DOTALL,
)


def markdown_link_targets(markdown: str) -> list[tuple[int, str]]:
    """Return inline/image and reference-definition targets with line numbers."""
    targets: list[tuple[int, str]] = []
    for pattern in (INLINE_LINK_RE, REFERENCE_LINK_RE):
        for match in pattern.finditer(markdown):
            target = match.group("target")
            if target.startswith("<") and target.endswith(">"):
                target = target[1:-1]
            line = markdown.count("\n", 0, match.start()) + 1
            targets.append((line, target))
    return sorted(targets)


def validate_packaged_readme_links(
    package_name: str,
    readme_path: pathlib.Path,
    package_root: pathlib.Path,
    packaged_files: set[str],
) -> None:
    markdown = readme_path.read_text(encoding="utf-8")
    package_root = package_root.resolve()
    for line, target in markdown_link_targets(markdown):
        parsed = urllib.parse.urlsplit(target)
        if parsed.scheme or parsed.netloc or target.startswith("#"):
            continue
        local_path = urllib.parse.unquote(parsed.path).replace("\\", "/")
        if not local_path:
            continue
        resolved = (readme_path.parent / local_path).resolve()
        try:
            package_relative = resolved.relative_to(package_root).as_posix()
        except ValueError as error:
            raise RuntimeError(
                f"{package_name} README line {line} links outside its crate package: "
                f"{target}"
            ) from error
        prefix = package_relative.rstrip("/") + "/"
        if package_relative not in packaged_files and not any(
            file.startswith(prefix) for file in packaged_files
        ):
            raise RuntimeError(
                f"{package_name} README line {line} links to unpackaged local target: "
                f"{target}"
            )


def validate_packaged_source_includes(
    package_name: str,
    package_root: pathlib.Path,
    packaged_files: set[str],
) -> None:
    package_root = package_root.resolve()
    for packaged_file in sorted(packaged_files):
        if not packaged_file.endswith(".rs"):
            continue
        source_path = package_root / packaged_file
        if not source_path.is_file():
            continue
        source = source_path.read_text(encoding="utf-8")
        for match in INCLUDE_MACRO_RE.finditer(source):
            line = source.count("\n", 0, match.start()) + 1
            argument = INCLUDE_LITERAL_ARG_RE.match(source, match.end())
            if argument is None:
                raise RuntimeError(
                    f"{package_name} {packaged_file}:{line} uses a dynamic or "
                    "unsupported include macro argument; package closure cannot be "
                    "proven"
                )
            target = argument.group("normal")
            if target is None:
                target = argument.group("raw")
            resolved = (source_path.parent / target).resolve()
            try:
                package_relative = resolved.relative_to(package_root).as_posix()
            except ValueError as error:
                raise RuntimeError(
                    f"{package_name} {packaged_file}:{line} includes a file outside "
                    f"its crate package: {target}"
                ) from error
            if package_relative not in packaged_files:
                raise RuntimeError(
                    f"{package_name} {packaged_file}:{line} includes an unpackaged "
                    f"file: {target}"
                )


def run(
    args: list[str], root: pathlib.Path, *, capture: bool = True
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        cwd=root,
        check=False,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
    )


def require_success(result: subprocess.CompletedProcess[str], label: str) -> str:
    if result.returncode != 0:
        detail = "\n".join(
            part.strip() for part in (result.stdout, result.stderr) if part and part.strip()
        )
        raise RuntimeError(f"{label} failed with exit code {result.returncode}\n{detail}")
    return result.stdout or ""


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def relative_or_absolute(root: pathlib.Path, path: pathlib.Path) -> str:
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        return str(path.resolve())


def command_evidence(
    args: list[str], result: subprocess.CompletedProcess[str]
) -> dict[str, Any]:
    stdout = result.stdout or ""
    stderr = result.stderr or ""
    transcript = json.dumps(
        {
            "argv": args,
            "exitCode": result.returncode,
            "stderr": stderr,
            "stdout": stdout,
        },
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")
    return {
        "argv": args,
        "exitCode": result.returncode,
        "stdoutSha256": hashlib.sha256(stdout.encode("utf-8")).hexdigest(),
        "stderrSha256": hashlib.sha256(stderr.encode("utf-8")).hexdigest(),
        "transcriptSha256": hashlib.sha256(transcript).hexdigest(),
    }


def git_state(root: pathlib.Path) -> tuple[str, list[str]]:
    head = require_success(
        run(["git", "rev-parse", "HEAD"], root), "git rev-parse HEAD"
    ).strip()
    status = require_success(
        run(["git", "status", "--porcelain=v1", "--untracked-files=all"], root),
        "git status",
    )
    return head, [line for line in status.splitlines() if line.strip()]


def git_worktree_fingerprint(root: pathlib.Path) -> str:
    """Hash every tracked change plus every non-ignored untracked file.

    A porcelain-status snapshot only records path/state codes. In diagnostic
    `--allow-dirty` mode, the contents of an already modified file could change
    without changing those codes. This fingerprint binds the exact dirty tree
    at both ends of the check while naturally excluding ignored build output.
    """
    tracked_diff = require_success(
        run(
            ["git", "diff", "--no-ext-diff", "--binary", "HEAD", "--"],
            root,
        ),
        "git diff HEAD",
    ).encode("utf-8")
    untracked_output = require_success(
        run(
            ["git", "ls-files", "--others", "--exclude-standard", "-z"],
            root,
        ),
        "git ls-files --others",
    )
    untracked = sorted(path for path in untracked_output.split("\0") if path)

    digest = hashlib.sha256()
    digest.update(b"tracked-diff\0")
    digest.update(len(tracked_diff).to_bytes(8, "big"))
    digest.update(tracked_diff)
    for relative in untracked:
        encoded_path = relative.encode("utf-8")
        path = root / relative
        digest.update(b"untracked\0")
        digest.update(len(encoded_path).to_bytes(8, "big"))
        digest.update(encoded_path)
        if path.is_symlink():
            payload = os.readlink(path).encode("utf-8")
            digest.update(b"symlink\0")
            digest.update(len(payload).to_bytes(8, "big"))
            digest.update(payload)
        elif path.is_file():
            digest.update(b"file\0")
            digest.update(path.stat().st_size.to_bytes(8, "big"))
            with path.open("rb") as handle:
                for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                    digest.update(chunk)
        else:
            raise RuntimeError(f"untracked path is not a file or symlink: {relative}")
    return digest.hexdigest()


def load_config(path: pathlib.Path) -> tuple[str, list[str], dict[str, str]]:
    with path.open("rb") as handle:
        value = tomllib.load(handle)
    if value.get("schema") != "hopper.publish-train.v1":
        raise RuntimeError(f"unsupported publish-train schema in {path}")
    version = value.get("default_version")
    overrides = value.get("version_overrides", {})
    packages = value.get("packages")
    if not isinstance(version, str) or not version:
        raise RuntimeError("default_version must be a non-empty string")
    if not isinstance(overrides, dict) or not all(
        isinstance(name, str)
        and isinstance(package_version, str)
        and package_version
        for name, package_version in overrides.items()
    ):
        raise RuntimeError("version_overrides must map package names to versions")
    if not isinstance(packages, list) or not all(
        isinstance(package, str) and package for package in packages
    ):
        raise RuntimeError("packages must be a list of non-empty strings")
    if len(packages) != len(set(packages)):
        raise RuntimeError("publish train contains a duplicate package")
    unknown_overrides = sorted(set(overrides) - set(packages))
    if unknown_overrides:
        raise RuntimeError(f"version overrides name unknown packages: {unknown_overrides}")
    expected_versions = {name: overrides.get(name, version) for name in packages}
    return version, packages, expected_versions


def cargo_metadata(root: pathlib.Path) -> dict[str, Any]:
    output = require_success(
        run(
            [
                "cargo",
                "metadata",
                "--format-version",
                "1",
                "--locked",
                "--no-deps",
            ],
            root,
        ),
        "cargo metadata",
    )
    return json.loads(output)


def is_public(package: dict[str, Any]) -> bool:
    # Cargo reports `publish = false` as an empty registry list.  `null` means
    # unrestricted publication and a non-empty list names allowed registries.
    return package.get("publish") != []


def validate_train(
    metadata: dict[str, Any], expected_versions: dict[str, str], order: list[str]
) -> list[dict[str, Any]]:
    workspace_ids = set(metadata["workspace_members"])
    workspace_packages = {
        package["name"]: package
        for package in metadata["packages"]
        if package["id"] in workspace_ids
    }
    public_names = {name for name, package in workspace_packages.items() if is_public(package)}
    configured = set(order)
    missing = sorted(public_names - configured)
    extra = sorted(configured - public_names)
    if missing or extra:
        raise RuntimeError(
            "publish train does not exactly match public workspace packages: "
            f"missing={missing}, extra_or_private={extra}"
        )

    position = {name: index for index, name in enumerate(order)}
    workspace_root = pathlib.Path(metadata["workspace_root"])
    records: list[dict[str, Any]] = []
    for name in order:
        package = workspace_packages[name]
        expected_version = expected_versions[name]
        if package["version"] != expected_version:
            raise RuntimeError(
                f"{name} is {package['version']}, expected release {expected_version}"
            )
        if package.get("authors") != EXPECTED_AUTHORS:
            raise RuntimeError(
                f"{name} authors are {package.get('authors')!r}; expected "
                f"{EXPECTED_AUTHORS!r}"
            )
        if package.get("publish") != EXPECTED_PUBLISH_REGISTRIES:
            raise RuntimeError(
                f"{name} publish registries are {package.get('publish')!r}; expected "
                f"{EXPECTED_PUBLISH_REGISTRIES!r}"
            )
        missing_metadata = [
            field for field in REQUIRED_METADATA if not package.get(field)
        ]
        if not package.get("keywords"):
            missing_metadata.append("keywords")
        if missing_metadata:
            raise RuntimeError(f"{name} is missing crates.io metadata: {missing_metadata}")

        internal_dependencies: list[dict[str, str]] = []
        for dependency in package.get("dependencies", []):
            dependency_name = dependency["name"]
            if dependency.get("path") is None or dependency_name not in workspace_packages:
                continue
            if dependency.get("kind") == "dev":
                if dependency_name not in public_names:
                    raise RuntimeError(
                        f"{name} has a dev path dependency on private workspace package "
                        f"{dependency_name}; its published crate cannot resolve that test graph"
                    )
                continue
            if dependency_name not in public_names:
                raise RuntimeError(
                    f"{name} depends on private workspace package {dependency_name}"
                )
            if position[dependency_name] >= position[name]:
                raise RuntimeError(
                    f"publication order violation: {name} depends on later package "
                    f"{dependency_name}"
                )
            requirement = dependency.get("req")
            dependency_version = expected_versions[dependency_name]
            accepted_requirements = {
                dependency_version,
                f"^{dependency_version}",
                f"={dependency_version}",
            }
            if requirement not in accepted_requirements:
                raise RuntimeError(
                    f"{name} -> {dependency_name} uses {requirement!r}; expected "
                    f"the {dependency_version} release line"
                )
            internal_dependencies.append(
                {"name": dependency_name, "requirement": requirement}
            )

        manifest_path = pathlib.Path(package["manifest_path"])
        readme_path = manifest_path.parent / package["readme"]
        records.append(
            {
                "position": position[name] + 1,
                "name": name,
                "version": expected_version,
                "manifest": manifest_path.relative_to(workspace_root).as_posix(),
                "readme": readme_path.relative_to(workspace_root).as_posix(),
                "internalDependencies": sorted(
                    internal_dependencies, key=lambda item: item["name"]
                ),
            }
        )
    return records


def package_archive_path(
    target_directory: pathlib.Path, record: dict[str, Any], *, publish: bool = False
) -> pathlib.Path:
    directory = target_directory / "package"
    # Pinned Cargo 1.96 stages publish archives separately from cargo package.
    # The latter can retain an earlier generated dependency lockfile.
    if publish:
        directory /= "tmp-crate"
    return directory / f"{record['name']}-{record['version']}.crate"


def package_args(package: str, allow_dirty: bool, *, list_only: bool) -> list[str]:
    args = ["cargo", "package", "--locked"]
    if allow_dirty:
        args.append("--allow-dirty")
    args.append("--list" if list_only else "--no-verify")
    args.extend(["-p", package])
    return args


def package_train_args(packages: list[str], allow_dirty: bool) -> list[str]:
    """Package the selected train as one Cargo operation.

    Cargo can resolve unpublished path dependencies when all of them are
    selected in the same `cargo package` invocation. Packaging each member in
    isolation would make a valid new release train fail until every earlier
    package had already reached the registry.
    """
    args = ["cargo", "package", "--locked"]
    if allow_dirty:
        args.append("--allow-dirty")
    args.append("--no-verify")
    for package in packages:
        args.extend(["-p", package])
    return args


def require_archive(
    root: pathlib.Path,
    target_directory: pathlib.Path,
    record: dict[str, Any],
    *,
    publish: bool = False,
) -> dict[str, Any]:
    archive = package_archive_path(target_directory, record, publish=publish)
    if not archive.is_file():
        raise RuntimeError(
            f"cargo did not produce the expected archive for {record['name']}: {archive}"
        )
    return {
        "path": relative_or_absolute(root, archive),
        "bytes": archive.stat().st_size,
        "sha256": sha256_file(archive),
    }


def package_lists(
    root: pathlib.Path,
    target_directory: pathlib.Path,
    records: list[dict[str, Any]],
    allow_dirty: bool,
) -> dict[str, Any]:
    for record in records:
        args = package_args(record["name"], allow_dirty, list_only=True)
        list_result = run(args, root)
        output = require_success(
            list_result, f"cargo package --list -p {record['name']}"
        )
        files = sorted(line.strip().replace("\\", "/") for line in output.splitlines() if line.strip())
        required = {"Cargo.toml", "Cargo.toml.orig"}
        package_root = (root / record["manifest"]).parent.resolve()
        readme_path = (root / record["readme"]).resolve()
        packaged_readme = readme_path.relative_to(package_root).as_posix()
        if not required.issubset(files) or packaged_readme not in files:
            raise RuntimeError(
                f"{record['name']} package list is missing Cargo.toml/Cargo.toml.orig/"
                f"{packaged_readme}"
            )
        validate_packaged_readme_links(
            record["name"], readme_path, package_root, set(files)
        )
        validate_packaged_source_includes(record["name"], package_root, set(files))
        normalized = ("\n".join(files) + "\n").encode("utf-8")
        record["packagedFileCount"] = len(files)
        record["packageListSha256"] = hashlib.sha256(normalized).hexdigest()
        record["packageListCommand"] = command_evidence(args, list_result)
        record["registryDryRun"] = {"status": "not-requested"}

    expected_archives = [
        package_archive_path(target_directory, record) for record in records
    ]
    for archive in expected_archives:
        if archive.exists():
            if not archive.is_file():
                raise RuntimeError(f"expected archive path is not a file: {archive}")
            archive.unlink()

    archive_args = package_train_args(
        [record["name"] for record in records], allow_dirty
    )
    archive_result = run(archive_args, root)
    require_success(archive_result, "cargo package for the 29-package train")
    archive_command = command_evidence(archive_args, archive_result)

    for record in records:
        record["packageArchive"] = require_archive(root, target_directory, record)
        record["packageCommand"] = {
            "mode": "single-train-command",
            "selectedPackage": record["name"],
            "trainCommandTranscriptSha256": archive_command["transcriptSha256"],
        }
        print(
            f"[{record['position']:02d}/{len(records):02d}] {record['name']}: "
            f"{record['packagedFileCount']} packaged files, archive "
            f"{record['packageArchive']['sha256']}"
        )
    return archive_command


def registry_dry_run_args(package: str, allow_dirty: bool) -> list[str]:
    args = [
        "cargo",
        "publish",
        "--dry-run",
        "--locked",
        "--registry",
        "crates-io",
        "-p",
        package,
    ]
    if allow_dirty:
        args.insert(2, "--allow-dirty")
    return args


def registry_dry_run(
    root: pathlib.Path,
    target_directory: pathlib.Path,
    records: list[dict[str, Any]],
    allow_dirty: bool,
    start_at: str | None,
) -> dict[str, Any]:
    selected = records
    start_index = 0
    if start_at:
        names = [record["name"] for record in records]
        if start_at not in names:
            raise RuntimeError(f"--start-at package is not in the train: {start_at}")
        start_index = names.index(start_at)
        selected = records[start_index:]
    for record in records[:start_index]:
        record["registryDryRun"] = {
            "status": "skipped-before-start-at",
            "reason": f"resume started at {start_at}",
        }
    for record in selected:
        args = registry_dry_run_args(record["name"], allow_dirty)
        archive = package_archive_path(target_directory, record, publish=True)
        if archive.exists():
            if not archive.is_file():
                raise RuntimeError(f"expected publish archive path is not a file: {archive}")
            archive.unlink()
        print(f"registry dry-run: {record['name']}")
        result = run(args, root)
        if result.stdout:
            print(result.stdout, end="" if result.stdout.endswith("\n") else "\n")
        if result.stderr:
            print(
                result.stderr,
                end="" if result.stderr.endswith("\n") else "\n",
                file=sys.stderr,
            )
        if result.returncode != 0:
            raise RuntimeError(
                f"registry dry-run stopped at {record['name']}; publish and wait for "
                "all earlier release dependencies to index, then resume with "
                f"--start-at {record['name']}"
            )
        record["registryDryRun"] = {
            "status": "passed",
            "command": command_evidence(args, result),
            "archive": require_archive(root, target_directory, record, publish=True),
        }
    return {
        "requested": True,
        "registry": "crates-io",
        "startAt": start_at,
        "completeTrain": start_index == 0,
        "passedPackageCount": len(selected),
        "skippedPackageCount": start_index,
    }


def run_self_tests() -> None:
    class PackagedReadmeLinkTests(unittest.TestCase):
        def setUp(self) -> None:
            self.temp = tempfile.TemporaryDirectory()
            self.addCleanup(self.temp.cleanup)
            self.package_root = pathlib.Path(self.temp.name) / "crate"
            self.package_root.mkdir()
            (self.package_root / "assets").mkdir()
            self.readme = self.package_root / "README.md"
            self.files = {"README.md", "guide.md", "assets/logo.png"}

        def validate(self, markdown: str) -> None:
            self.readme.write_text(markdown, encoding="utf-8")
            validate_packaged_readme_links(
                "fixture", self.readme, self.package_root, self.files
            )

        def test_allows_external_anchor_and_packaged_targets(self) -> None:
            self.validate(
                "[web](https://example.com) [section](#local) "
                "[guide](<guide.md>) ![logo](assets/logo.png)\n"
                "[reference][ref]\n[ref]: guide.md\n"
            )

        def test_rejects_parent_relative_inline_image_angle_and_reference(self) -> None:
            hostile = (
                "[inline](../outside.md)",
                "![image](../outside.png)",
                "[angle](<../outside.md>)",
                "[reference][ref]\n[ref]: ../outside.md\n",
            )
            for markdown in hostile:
                with self.subTest(markdown=markdown):
                    with self.assertRaisesRegex(RuntimeError, "outside its crate package"):
                        self.validate(markdown)

        def test_rejects_unpacked_local_target(self) -> None:
            with self.assertRaisesRegex(RuntimeError, "unpackaged local target"):
                self.validate("[missing](missing.md)")

        def test_source_includes_must_stay_inside_the_package(self) -> None:
            source_dir = self.package_root / "src"
            source_dir.mkdir()
            source = source_dir / "lib.rs"
            generated = source_dir / "generated.rs"
            generated.write_text("pub const VALUE: u8 = 1;\n", encoding="utf-8")
            binary = source_dir / "fixture.bin"
            binary.write_bytes(b"fixture")
            source.write_text(
                'include!("generated.rs");\n'
                'const TEXT: &str = include_str!(r#"../guide.md"#);\n'
                'const BYTES: &[u8] = include_bytes!("fixture.bin");\n',
                encoding="utf-8",
            )
            files = self.files | {
                "src/lib.rs",
                "src/generated.rs",
                "src/fixture.bin",
            }
            validate_packaged_source_includes("fixture", self.package_root, files)
            source.write_text(
                'const BAD: &str = include_str!("../../outside.md");\n',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(RuntimeError, "outside its crate package"):
                validate_packaged_source_includes("fixture", self.package_root, files)

        def test_source_includes_reject_computed_or_unparsed_arguments(self) -> None:
            source_dir = self.package_root / "src"
            source_dir.mkdir()
            source = source_dir / "lib.rs"
            files = self.files | {"src/lib.rs"}
            hostile = (
                'include!(concat!("generated", ".rs"));\n',
                'const TEXT: &str = include_str!(env!("FIXTURE"));\n',
                "const BYTES: &[u8] = include_bytes!(FIXTURE);\n",
            )
            for contents in hostile:
                with self.subTest(contents=contents):
                    source.write_text(contents, encoding="utf-8")
                    with self.assertRaisesRegex(
                        RuntimeError, "dynamic or unsupported include macro argument"
                    ):
                        validate_packaged_source_includes(
                            "fixture", self.package_root, files
                        )

        def test_registry_dry_run_pins_crates_io_in_exact_argv(self) -> None:
            self.assertEqual(
                registry_dry_run_args("hopper-runtime", False),
                [
                    "cargo",
                    "publish",
                    "--dry-run",
                    "--locked",
                    "--registry",
                    "crates-io",
                    "-p",
                    "hopper-runtime",
                ],
            )
            self.assertEqual(
                registry_dry_run_args("hopper-runtime", True),
                [
                    "cargo",
                    "publish",
                    "--allow-dirty",
                    "--dry-run",
                    "--locked",
                    "--registry",
                    "crates-io",
                    "-p",
                    "hopper-runtime",
                ],
            )

        def test_package_train_selects_every_package_in_one_locked_command(self) -> None:
            self.assertEqual(
                package_train_args(["first", "second"], False),
                [
                    "cargo",
                    "package",
                    "--locked",
                    "--no-verify",
                    "-p",
                    "first",
                    "-p",
                    "second",
                ],
            )

        def test_worktree_fingerprint_binds_dirty_file_contents(self) -> None:
            untracked = pathlib.Path(self.temp.name) / "new.txt"
            untracked.write_text("first", encoding="utf-8")
            tracked_diff = ["diff --git a/file b/file\n-old\n+first\n"]

            def fake_run(
                args: list[str], root: pathlib.Path, *, capture: bool = True
            ) -> subprocess.CompletedProcess[str]:
                del root, capture
                if args[1] == "diff":
                    stdout = tracked_diff[0]
                elif args[1] == "ls-files":
                    stdout = "new.txt\0"
                else:
                    raise AssertionError(args)
                return subprocess.CompletedProcess(args, 0, stdout, "")

            module = sys.modules[__name__]
            with mock.patch.object(module, "run", side_effect=fake_run):
                first = git_worktree_fingerprint(pathlib.Path(self.temp.name))
                tracked_diff[0] = "diff --git a/file b/file\n-old\n+other\n"
                second = git_worktree_fingerprint(pathlib.Path(self.temp.name))
                self.assertNotEqual(first, second)
                tracked_diff[0] = "diff --git a/file b/file\n-old\n+first\n"
                untracked.write_text("later", encoding="utf-8")
                third = git_worktree_fingerprint(pathlib.Path(self.temp.name))
                self.assertNotEqual(first, third)

        def test_package_archive_evidence_hashes_exact_bytes(self) -> None:
            target = pathlib.Path(self.temp.name) / "target"
            archive = target / "package" / "hopper-runtime-1.2.3.crate"
            archive.parent.mkdir(parents=True)
            archive.write_bytes(b"crate archive bytes")
            record = {"name": "hopper-runtime", "version": "1.2.3"}
            evidence = require_archive(pathlib.Path(self.temp.name), target, record)
            self.assertEqual(evidence["path"], "target/package/hopper-runtime-1.2.3.crate")
            self.assertEqual(evidence["bytes"], 19)
            self.assertEqual(
                evidence["sha256"], hashlib.sha256(b"crate archive bytes").hexdigest()
            )

        def test_registry_dry_run_requires_fresh_publish_archive(self) -> None:
            root = pathlib.Path(self.temp.name)
            target = root / "target"
            record = {"name": "hopper-runtime", "version": "1.2.3"}
            packaged = package_archive_path(target, record)
            packaged.parent.mkdir(parents=True)
            packaged.write_bytes(b"older package lockfile")
            staged = package_archive_path(target, record, publish=True)
            staged.parent.mkdir(parents=True)
            staged.write_bytes(b"stale publish archive")

            def fake_run(args: list[str], cwd: pathlib.Path) -> subprocess.CompletedProcess[str]:
                self.assertFalse(staged.exists())
                self.assertEqual(packaged.read_bytes(), b"older package lockfile")
                staged.write_bytes(b"fresh registry lockfile")
                return subprocess.CompletedProcess(args, 0, "", "")

            with mock.patch.object(sys.modules[__name__], "run", side_effect=fake_run):
                registry_dry_run(root, target, [record], False, None)
            evidence = record["registryDryRun"]["archive"]
            self.assertEqual(evidence["path"], "target/package/tmp-crate/hopper-runtime-1.2.3.crate")
            self.assertEqual(evidence["sha256"], hashlib.sha256(b"fresh registry lockfile").hexdigest())

        def test_registry_dry_run_refuses_old_package_when_publish_output_missing(self) -> None:
            root = pathlib.Path(self.temp.name)
            target = root / "target"
            record = {"name": "hopper-runtime", "version": "1.2.3"}
            packaged = package_archive_path(target, record)
            packaged.parent.mkdir(parents=True)
            packaged.write_bytes(b"old package output must not satisfy this gate")
            result = subprocess.CompletedProcess(["cargo"], 0, "", "")
            with mock.patch.object(sys.modules[__name__], "run", return_value=result):
                with self.assertRaisesRegex(RuntimeError, "did not produce the expected archive"):
                    registry_dry_run(root, target, [record], False, None)

        def test_command_evidence_binds_both_streams_and_exit_code(self) -> None:
            result = subprocess.CompletedProcess(
                ["cargo"], 0, stdout="out\n", stderr="warning\n"
            )
            first = command_evidence(["cargo", "publish"], result)
            changed_stderr = command_evidence(
                ["cargo", "publish"],
                subprocess.CompletedProcess(
                    ["cargo"], 0, stdout="out\n", stderr="different\n"
                ),
            )
            self.assertNotEqual(
                first["transcriptSha256"], changed_stderr["transcriptSha256"]
            )
            self.assertEqual(first["exitCode"], 0)

    result = unittest.TextTestRunner(verbosity=2).run(
        unittest.defaultTestLoader.loadTestsFromTestCase(PackagedReadmeLinkTests)
    )
    if not result.wasSuccessful():
        raise RuntimeError("publish-train self-tests failed")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Validate all public Hopper packages and their topological crates.io order. "
            "This command never publishes."
        )
    )
    parser.add_argument("--root", default=".", help="workspace root")
    parser.add_argument(
        "--config", default="release/publish-order.toml", help="publish-train TOML"
    )
    parser.add_argument(
        "--allow-dirty",
        action="store_true",
        help="permit a dirty source tree for diagnostics (CI/release default is clean)",
    )
    parser.add_argument(
        "--registry-dry-run",
        action="store_true",
        help="also run cargo publish --dry-run in order; no crate is uploaded",
    )
    parser.add_argument(
        "--start-at", help="resume registry dry-runs at this package after indexing"
    )
    parser.add_argument("--out", help="write a JSON attestation")
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="run package-README boundary regression tests and exit",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = pathlib.Path(args.root).resolve()
    config = pathlib.Path(args.config)
    if not config.is_absolute():
        config = root / config
    try:
        if args.self_test:
            run_self_tests()
            print("OK: publish-train self-tests passed")
            return 0
        if args.start_at and not args.registry_dry_run:
            raise RuntimeError("--start-at requires --registry-dry-run")
        head, dirty = git_state(root)
        worktree_fingerprint = git_worktree_fingerprint(root)
        if dirty and not args.allow_dirty:
            raise RuntimeError(
                "release train requires a clean worktree; use --allow-dirty only for diagnostics"
            )
        default_version, order, expected_versions = load_config(config)
        metadata = cargo_metadata(root)
        records = validate_train(metadata, expected_versions, order)
        if len(records) != 29:
            raise RuntimeError(f"expected 29 public packages, found {len(records)}")
        target_directory = pathlib.Path(metadata["target_directory"])
        package_archive_build = package_lists(
            root, target_directory, records, args.allow_dirty
        )
        registry_summary = {
            "requested": False,
            "registry": "crates-io",
            "startAt": None,
            "completeTrain": False,
            "passedPackageCount": 0,
            "skippedPackageCount": 0,
        }
        if args.registry_dry_run:
            registry_summary = registry_dry_run(
                root,
                target_directory,
                records,
                args.allow_dirty,
                args.start_at,
            )

        head_after, dirty_after = git_state(root)
        worktree_fingerprint_after = git_worktree_fingerprint(root)
        if head_after != head:
            raise RuntimeError("source commit changed while validating the publish train")
        if (
            dirty_after != dirty
            or worktree_fingerprint_after != worktree_fingerprint
        ):
            raise RuntimeError(
                "source worktree changed while validating the publish train:\n"
                f"status before={dirty!r}\nstatus after={dirty_after!r}\n"
                f"fingerprint before={worktree_fingerprint}\n"
                f"fingerprint after={worktree_fingerprint_after}"
            )

        attestation = {
            "schema": SCHEMA,
            "generatedAt": dt.datetime.now(dt.timezone.utc).isoformat(),
            "source": {
                "commitAtStart": head,
                "commitAtEnd": head_after,
                "treeCleanAtStart": not dirty,
                "treeCleanAtEnd": not dirty_after,
                "statusUnchanged": dirty_after == dirty,
                "worktreeFingerprintAtStart": worktree_fingerprint,
                "worktreeFingerprintAtEnd": worktree_fingerprint_after,
                "worktreeUnchanged": (
                    worktree_fingerprint_after == worktree_fingerprint
                ),
            },
            "ci": {
                "githubRunId": os.environ.get("GITHUB_RUN_ID"),
                "githubRunAttempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
                "githubWorkflowRef": os.environ.get("GITHUB_WORKFLOW_REF"),
                "githubJob": os.environ.get("GITHUB_JOB"),
            },
            "publishOrder": {
                "path": relative_or_absolute(root, config),
                "sha256": sha256_file(config),
            },
            "defaultVersion": default_version,
            "packageVersions": expected_versions,
            "packageArchiveBuild": package_archive_build,
            "registryDryRun": registry_summary,
            "packages": records,
        }
        if args.out:
            output = pathlib.Path(args.out)
            if not output.is_absolute():
                output = root / output
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(json.dumps(attestation, indent=2) + "\n", encoding="utf-8")
            final_head, final_dirty = git_state(root)
            final_worktree_fingerprint = git_worktree_fingerprint(root)
            if (
                final_head != head
                or final_dirty != dirty
                or final_worktree_fingerprint != worktree_fingerprint
            ):
                raise RuntimeError(
                    "attestation output changed the source checkout; write --out below an "
                    "ignored target directory or outside the repository"
                )
            print(f"wrote {output}")
        print(
            f"OK: {len(records)} public packages match the declared "
            "topological publication train; no crate was published"
        )
        return 0
    except (OSError, RuntimeError, ValueError, json.JSONDecodeError) as error:
        print(f"publish train: FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
