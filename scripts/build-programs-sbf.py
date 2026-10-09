#!/usr/bin/env python3
"""Build every deployable workspace package and record exact SBF artifacts.

Discovers cdylib targets from Cargo metadata, so a newly added program cannot
silently miss the compile gate. Compilation is not execution or devnet coverage.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import runpy
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sources = runpy.run_path(str(ROOT / "scripts/record-quality-gate.py"))["sources"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--arch", choices=["v0", "v1", "v2", "v3"], default="v3")
    parser.add_argument("--builder", default="cargo-build-sbf")
    parser.add_argument("--tools-version")
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    out = args.out.resolve()
    if not out.is_relative_to(ROOT / "target") or out.exists():
        raise RuntimeError("output must be a new directory under ignored target/")
    inventory = sources(ROOT)
    command = ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"]
    if args.offline:
        command.append("--offline")
    metadata = json.loads(subprocess.check_output(command, cwd=ROOT))
    packages = sorted((p for p in metadata["packages"] if p["id"] in metadata["workspace_members"]
                       and any("cdylib" in t["crate_types"] for t in p["targets"])), key=lambda p: p["name"])
    if not packages:
        raise RuntimeError("no deployable workspace packages discovered")
    out.mkdir(parents=True)
    # A fresh Cargo target prevents an earlier checkout's SBF cache from
    # supplying artifacts to this source-bound build matrix.
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(out / "cargo-target")
    report = {"schema": "hopper.program-build-matrix.v1", "arch": args.arch,
              "baseCommit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "sourceFiles": inventory, "freshCargoTarget": True, "programs": [], "allPassed": False}
    for package in packages:
        dest = out / package["name"]
        dest.mkdir()
        command = [args.builder, "--manifest-path", package["manifest_path"], "--arch", args.arch,
                   "--sbf-out-dir", str(dest)]
        if args.tools_version:
            command += ["--tools-version", args.tools_version]
        command += ["--", "--locked", "-j", "1"]
        if args.offline:
            command.append("--offline")
        log = dest / "build.log"
        with log.open("wb") as stream:
            code = subprocess.run(command, cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT).returncode
        text = log.read_text(encoding="utf-8", errors="replace")
        stack_error = any("Stack offset" in line and "exceeded" in line for line in text.splitlines())
        artifacts = [{"path": path.relative_to(ROOT).as_posix(), "bytes": path.stat().st_size,
                      "sha256": hashlib.sha256(path.read_bytes()).hexdigest()} for path in dest.glob("*.so")]
        expected_names = {t["name"] + ".so" for t in package["targets"] if "cdylib" in t["crate_types"]}
        passed = code == 0 and {Path(a["path"]).name for a in artifacts} == expected_names and not stack_error
        report["programs"].append({"package": package["name"], "manifest": Path(package["manifest_path"]).relative_to(ROOT).as_posix(),
            "exitCode": code, "stackFrameDiagnostic": stack_error, "passed": passed, "artifacts": artifacts,
            "log": {"path": log.relative_to(ROOT).as_posix(), "sha256": hashlib.sha256(log.read_bytes()).hexdigest()}, "command": command})
        (out / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(f"{package['name']}: {'passed' if passed else 'FAILED'}", flush=True)
    report["sourceUnchanged"] = inventory == sources(ROOT)
    report["allPassed"] = report["sourceUnchanged"] and all(p["passed"] for p in report["programs"])
    (out / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"{sum(p['passed'] for p in report['programs'])}/{len(packages)} program builds passed")
    return 0 if report["allPassed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
