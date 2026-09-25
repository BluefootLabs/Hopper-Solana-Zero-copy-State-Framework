#!/usr/bin/env python3
"""Build (not cargo check) fixtures to evaluate generic size assertions."""
from pathlib import Path
import argparse, json, os, subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", default="target/hopper/fixed-layout-rejections")
    args = parser.parse_args()
    out = (ROOT / args.out).resolve()
    assert out.is_relative_to(ROOT / "target")
    out.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(CARGO_BUILD_JOBS="1", CARGO_INCREMENTAL="0", CARGO_PROFILE_DEV_DEBUG="0")
    records = []
    for case in ["honest", "pod", "verified", "collection", "event"]:
        command = ["cargo", "+1.96.0", "build", "--manifest-path", "tests/fixtures/fixed-layout-rejections/Cargo.toml",
                   "--target-dir", str(out / "build"), "--locked", "--offline"]
        if case != "honest":
            command += ["--features", case]
        result = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True, encoding="utf-8")
        output = result.stdout + result.stderr
        (out / (case + ".log")).write_text(output, encoding="utf-8")
        if case == "honest":
            assert result.returncode == 0, output
        else:
            assert result.returncode != 0 and "E0080" in output and "FixedLayout::SIZE must equal size_of::<Self>()" in output, output
        records.append(dict(case=case, exitCode=result.returncode, expectedResultVerified=True))
        print(case + ": expected build result verified", flush=True)
    (out / "receipt.json").write_text(json.dumps(dict(allPassed=True, cases=records), indent=2) + "\n")


if __name__ == "__main__":
    main()
