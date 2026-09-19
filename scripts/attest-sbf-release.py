#!/usr/bin/env python3
"""Build and attest an isolated Hopper SBF release artifact without publishing."""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import hashlib
import json
import os
import pathlib
import shlex
import subprocess
import sys
import tomllib
from typing import Any


SCHEMA = "hopper.sbf-release-attestation.v2"


@dataclasses.dataclass(frozen=True)
class CommandResult:
    argv: list[str]
    exit_code: int
    stdout: str
    stderr: str


def run_command(args: list[str], root: pathlib.Path) -> CommandResult:
    result = subprocess.run(
        args,
        cwd=root,
        check=False,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    command_result = CommandResult(
        argv=args,
        exit_code=result.returncode,
        stdout=result.stdout or "",
        stderr=result.stderr or "",
    )
    if result.returncode != 0:
        detail = "\n".join(
            part.strip()
            for part in (command_result.stdout, command_result.stderr)
            if part.strip()
        )
        rendered = shlex.join(args)
        raise RuntimeError(f"{rendered} failed ({result.returncode})\n{detail}")
    return command_result


def command_text(result: CommandResult) -> str:
    return (result.stdout + result.stderr).strip()


def command_transcript(result: CommandResult) -> bytes:
    return json.dumps(
        {
            "argv": result.argv,
            "exitCode": result.exit_code,
            "stderr": result.stderr,
            "stdout": result.stdout,
        },
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("utf-8")


def command_evidence(result: CommandResult) -> dict[str, Any]:
    transcript = command_transcript(result)
    return {
        "argv": result.argv,
        "exitCode": result.exit_code,
        "stdoutSha256": hashlib.sha256(result.stdout.encode("utf-8")).hexdigest(),
        "stderrSha256": hashlib.sha256(result.stderr.encode("utf-8")).hexdigest(),
        "transcriptSha256": hashlib.sha256(transcript).hexdigest(),
    }


def write_command_log(
    root: pathlib.Path, directory: pathlib.Path, name: str, result: CommandResult
) -> dict[str, Any]:
    path = directory / f"{name}.json"
    path.write_bytes(command_transcript(result))
    return {
        "path": display_path(root, path),
        "bytes": path.stat().st_size,
        "sha256": sha256(path),
    }


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def resolve(root: pathlib.Path, value: str) -> pathlib.Path:
    path = pathlib.Path(value)
    return path.resolve() if path.is_absolute() else (root / path).resolve()


def display_path(root: pathlib.Path, path: pathlib.Path) -> str:
    try:
        return path.relative_to(root).as_posix()
    except ValueError:
        return str(path)


def source_state(root: pathlib.Path) -> tuple[str, list[str]]:
    head = command_text(run_command(["git", "rev-parse", "HEAD"], root))
    status = command_text(
        run_command(
            ["git", "status", "--porcelain=v1", "--untracked-files=all"], root
        )
    )
    return head, [line for line in status.splitlines() if line.strip()]


def is_git_ignored(root: pathlib.Path, path: pathlib.Path) -> bool:
    try:
        relative = path.relative_to(root).as_posix()
    except ValueError:
        return True
    result = subprocess.run(
        ["git", "check-ignore", "--quiet", "--no-index", "--", relative],
        cwd=root,
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )
    if result.returncode not in (0, 1):
        raise RuntimeError(
            f"git check-ignore failed for {relative} with exit code {result.returncode}"
        )
    return result.returncode == 0


def build_command(
    build_tool: str,
    program_manifest: pathlib.Path,
    output_directory: pathlib.Path,
    arch: str | None,
) -> list[str]:
    args = [
        build_tool,
        "--manifest-path",
        str(program_manifest),
        "--sbf-out-dir",
        str(output_directory),
    ]
    if arch:
        args.extend(["--arch", arch])
    args.extend(["--", "--locked"])
    return args


def require_absent(path: pathlib.Path, label: str) -> None:
    if path.exists():
        raise RuntimeError(
            f"{label} already exists: {path}; use a new isolated release output path"
        )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Build, verify, and attest an isolated SBF artifact. "
            "This does not deploy or publish."
        )
    )
    parser.add_argument("--root", default=".")
    parser.add_argument("--package", required=True)
    parser.add_argument("--program-manifest", required=True, help="program Cargo.toml")
    parser.add_argument("--manifest", required=True, help="generated Hopper manifest path")
    parser.add_argument("--binary", required=True, help="new SBF binary output path")
    parser.add_argument("--build-tool", default="cargo-build-sbf")
    parser.add_argument("--arch", choices=("v0", "v3"))
    parser.add_argument("--hopper", default="hopper", help="Hopper CLI executable")
    parser.add_argument(
        "--require-clean",
        action="store_true",
        help="accepted for compatibility; release attestation always requires a clean tree",
    )
    parser.add_argument("--out", required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = pathlib.Path(args.root).resolve()
    program_manifest = resolve(root, args.program_manifest)
    manifest = resolve(root, args.manifest)
    binary = resolve(root, args.binary)
    output = resolve(root, args.out)
    log_directory = output.with_suffix(".logs")
    try:
        if not program_manifest.is_file():
            raise RuntimeError(f"program manifest not found: {program_manifest}")
        package_value = tomllib.loads(program_manifest.read_text(encoding="utf-8-sig"))
        declared_package = package_value.get("package", {}).get("name")
        if declared_package != args.package:
            raise RuntimeError(
                f"program manifest names {declared_package!r}, expected {args.package!r}"
            )
        expected_binary_name = f"{args.package.replace('-', '_')}.so"
        if binary.name != expected_binary_name:
            raise RuntimeError(
                f"binary path must end in {expected_binary_name}, got {binary.name}"
            )

        head_before, dirty_before = source_state(root)
        if dirty_before:
            raise RuntimeError(
                "release artifact attestation requires a clean worktree:\n"
                + "\n".join(dirty_before[:30])
            )
        github_sha = os.environ.get("GITHUB_SHA")
        if github_sha and github_sha != head_before:
            raise RuntimeError(
                f"GITHUB_SHA {github_sha} does not match checked-out HEAD {head_before}"
            )

        for label, path in (
            ("isolated SBF output directory", binary.parent),
            ("generated manifest", manifest),
            ("attestation output", output),
            ("attestation command-log directory", log_directory),
        ):
            require_absent(path, label)
            if not is_git_ignored(root, path):
                raise RuntimeError(
                    f"{label} must be outside the repository or matched by .gitignore: {path}"
                )

        build_result = run_command(
            build_command(
                args.build_tool,
                program_manifest,
                binary.parent,
                args.arch,
            ),
            root,
        )
        if not binary.is_file():
            raise RuntimeError(f"build succeeded but did not create {binary}")
        with binary.open("rb") as handle:
            if handle.read(4) != b"\x7fELF":
                raise RuntimeError(f"binary is not an ELF artifact: {binary}")

        manifest_result = run_command(
            [
                args.hopper,
                "compile",
                "--emit",
                "manifest",
                "--package",
                args.package,
                "--out",
                str(manifest),
                "--force",
            ],
            root,
        )
        if not manifest.is_file():
            raise RuntimeError(f"manifest generation succeeded but did not create {manifest}")
        manifest_value = json.loads(manifest.read_text(encoding="utf-8-sig"))
        if manifest_value.get("name") != args.package:
            raise RuntimeError(
                f"generated manifest names {manifest_value.get('name')!r}, "
                f"expected {args.package!r}"
            )

        publish_check_result = run_command(
            [
                args.hopper,
                "publish-check",
                "--manifest",
                str(manifest),
                "--so",
                str(binary),
                "--full",
            ],
            root,
        )
        build_tool_version = run_command([args.build_tool, "--version"], root)
        rustc_version = run_command(["rustc", "--version", "--verbose"], root)
        cargo_version = run_command(["cargo", "--version", "--verbose"], root)

        log_directory.mkdir(parents=True)
        command_logs = {
            "build": write_command_log(
                root, log_directory, "build", build_result
            ),
            "manifestGeneration": write_command_log(
                root, log_directory, "manifest-generation", manifest_result
            ),
            "publishCheck": write_command_log(
                root, log_directory, "publish-check", publish_check_result
            ),
        }

        head_after, dirty_after = source_state(root)
        if head_after != head_before:
            raise RuntimeError("source commit changed during release build or verification")
        if dirty_after != dirty_before:
            raise RuntimeError(
                "release build or verification changed the source worktree:\n"
                f"before={dirty_before!r}\nafter={dirty_after!r}"
            )

        attestation = {
            "schema": SCHEMA,
            "generatedAt": dt.datetime.now(dt.timezone.utc).isoformat(),
            "package": args.package,
            "program": manifest_value.get("name"),
            "source": {
                "commitAtStart": head_before,
                "commitAtEnd": head_after,
                "treeCleanAtStart": True,
                "treeCleanAtEnd": True,
            },
            "ci": {
                "githubRunId": os.environ.get("GITHUB_RUN_ID"),
                "githubRunAttempt": os.environ.get("GITHUB_RUN_ATTEMPT"),
                "githubWorkflowRef": os.environ.get("GITHUB_WORKFLOW_REF"),
                "githubJob": os.environ.get("GITHUB_JOB"),
            },
            "toolchain": {
                "cargoBuildSbf": command_text(build_tool_version),
                "rustc": command_text(rustc_version),
                "cargo": command_text(cargo_version),
            },
            "build": {
                "isolatedOutputDirectory": display_path(root, binary.parent),
                "outputDirectoryAbsentBeforeBuild": True,
                "binaryAbsentBeforeBuild": True,
                "command": command_evidence(build_result),
            },
            "manifestGeneration": command_evidence(manifest_result),
            "publishCheck": {
                "passed": True,
                "mode": "release-with-binary-full",
                "command": command_evidence(publish_check_result),
            },
            "commandLogs": command_logs,
            "programManifest": {
                "path": display_path(root, program_manifest),
                "bytes": program_manifest.stat().st_size,
                "sha256": sha256(program_manifest),
            },
            "manifest": {
                "path": display_path(root, manifest),
                "bytes": manifest.stat().st_size,
                "sha256": sha256(manifest),
            },
            "binary": {
                "path": display_path(root, binary),
                "bytes": binary.stat().st_size,
                "sha256": sha256(binary),
                "format": "ELF",
            },
        }
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(attestation, indent=2) + "\n", encoding="utf-8")

        final_head, final_dirty = source_state(root)
        if final_head != head_before or final_dirty != dirty_before:
            raise RuntimeError(
                "writing the attestation changed the source checkout; output must remain ignored"
            )
        print(f"OK: isolated SBF evidence written to {output}")
        return 0
    except (OSError, RuntimeError, ValueError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        print(f"SBF release attestation: FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
