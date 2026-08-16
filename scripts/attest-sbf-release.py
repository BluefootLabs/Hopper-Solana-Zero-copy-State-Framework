#!/usr/bin/env python3
"""Create fail-closed provenance for a freshly built Hopper SBF release artifact."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import pathlib
import subprocess
import sys


SCHEMA = "hopper.sbf-release-attestation.v1"


def command(args: list[str], root: pathlib.Path) -> str:
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
    if result.returncode != 0:
        detail = "\n".join(
            part.strip() for part in (result.stdout, result.stderr) if part.strip()
        )
        raise RuntimeError(f"{' '.join(args)} failed ({result.returncode})\n{detail}")
    return (result.stdout or result.stderr).strip()


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def resolve(root: pathlib.Path, value: str) -> pathlib.Path:
    path = pathlib.Path(value)
    return path if path.is_absolute() else root / path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Attest a fresh, ABI-verified SBF artifact. This does not deploy or publish."
    )
    parser.add_argument("--root", default=".")
    parser.add_argument("--package", required=True)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--built-after", required=True, help="marker created before SBF build")
    parser.add_argument("--build-tool", default="cargo-build-sbf")
    parser.add_argument("--hopper", default="hopper", help="Hopper CLI executable")
    parser.add_argument("--require-clean", action="store_true")
    parser.add_argument("--out", required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = pathlib.Path(args.root).resolve()
    manifest = resolve(root, args.manifest)
    binary = resolve(root, args.binary)
    marker = resolve(root, args.built_after)
    output = resolve(root, args.out)
    try:
        for label, path in (
            ("manifest", manifest),
            ("binary", binary),
            ("pre-build marker", marker),
        ):
            if not path.is_file():
                raise RuntimeError(f"{label} not found: {path}")
        if binary.stat().st_mtime_ns < marker.stat().st_mtime_ns:
            raise RuntimeError(
                f"binary predates the build marker: {binary}; rebuild it in this release job"
            )
        if manifest.stat().st_mtime_ns < marker.stat().st_mtime_ns:
            raise RuntimeError(
                f"manifest predates the build marker: {manifest}; regenerate it in this release job"
            )
        marker_value = json.loads(marker.read_text(encoding="utf-8-sig"))
        binary_relative = binary.relative_to(root).as_posix()
        output_directory = binary.parent.relative_to(root).as_posix()
        if (
            marker_value.get("schema") != "hopper.sbf-build-marker.v1"
            or marker_value.get("binary") != binary_relative
            or marker_value.get("outputDirectory") != output_directory
            or marker_value.get("binaryAbsent") is not True
            or marker_value.get("outputDirectoryAbsent") is not True
        ):
            raise RuntimeError(
                "pre-build marker does not prove that the isolated output directory was absent"
            )
        with binary.open("rb") as handle:
            if handle.read(4) != b"\x7fELF":
                raise RuntimeError(f"binary is not an ELF artifact: {binary}")

        manifest_value = json.loads(manifest.read_text(encoding="utf-8-sig"))
        if manifest_value.get("name") != args.package:
            raise RuntimeError(
                f"manifest names {manifest_value.get('name')!r}, expected package {args.package!r}"
            )

        head_before = command(["git", "rev-parse", "HEAD"], root)
        status_before = command(
            ["git", "status", "--porcelain=v1", "--untracked-files=all"], root
        )
        dirty_before = [line for line in status_before.splitlines() if line.strip()]
        if args.require_clean and dirty_before:
            raise RuntimeError(
                "release artifact attestation requires a clean worktree:\n"
                + "\n".join(dirty_before[:30])
            )

        publish_check_output = command(
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

        head_after = command(["git", "rev-parse", "HEAD"], root)
        status_after = command(
            ["git", "status", "--porcelain=v1", "--untracked-files=all"], root
        )
        dirty_after = [line for line in status_after.splitlines() if line.strip()]
        if head_after != head_before:
            raise RuntimeError("source commit changed while publish-check was running")
        if args.require_clean and dirty_after:
            raise RuntimeError(
                "publish-check left the release worktree dirty:\n"
                + "\n".join(dirty_after[:30])
            )

        attestation = {
            "schema": SCHEMA,
            "generatedAt": dt.datetime.now(dt.timezone.utc).isoformat(),
            "package": args.package,
            "program": manifest_value.get("name"),
            "source": {
                "commit": head_after,
                "treeCleanAtStart": not dirty_before,
                "treeCleanAtEnd": not dirty_after,
            },
            "toolchain": {
                "cargoBuildSbf": command([args.build_tool, "--version"], root),
                "rustc": command(["rustc", "--version"], root),
                "cargo": command(["cargo", "--version"], root),
            },
            "freshness": {
                "marker": marker.relative_to(root).as_posix(),
                "markerSha256": sha256(marker),
                "markerModifiedAt": dt.datetime.fromtimestamp(
                    marker.stat().st_mtime, dt.timezone.utc
                ).isoformat(),
                "isolatedOutputDirectory": output_directory,
                "outputDirectoryAbsentBeforeBuild": True,
                "binaryAbsentBeforeBuild": True,
                "binaryBuiltAfterMarker": True,
                "manifestGeneratedAfterMarker": True,
            },
            "publishCheck": {
                "passed": True,
                "mode": "release-with-binary-full",
                "outputSha256": hashlib.sha256(
                    publish_check_output.encode("utf-8")
                ).hexdigest(),
            },
            "manifest": {
                "path": manifest.relative_to(root).as_posix(),
                "bytes": manifest.stat().st_size,
                "sha256": sha256(manifest),
            },
            "binary": {
                "path": binary.relative_to(root).as_posix(),
                "bytes": binary.stat().st_size,
                "sha256": sha256(binary),
                "format": "ELF",
            },
        }
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(attestation, indent=2) + "\n", encoding="utf-8")
        print(f"OK: fresh SBF evidence written to {output}")
        return 0
    except (OSError, RuntimeError, ValueError, json.JSONDecodeError) as error:
        print(f"SBF release attestation: FAIL: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
