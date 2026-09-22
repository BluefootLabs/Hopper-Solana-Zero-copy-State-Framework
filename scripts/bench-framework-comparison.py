#!/usr/bin/env python3
"""Build and measure the framework-comparison fixtures the way pina does.

pina (https://github.com/pina-rs/pina) publishes a like-for-like table of
hello-world and PDA-counter programs written in Pina, Pinocchio, Quasar and
Anchor v2, each built with one release recipe and measured once in Mollusk.
This driver reproduces that recipe for the Hopper fixtures under
`bench/framework-comparison/programs/` so a Hopper row can sit next to the
published numbers on equal terms:

* release profile injected through the same environment variables pina's
  `scripts/benchmark-frameworks.ts` sets (`lto = "fat"`, `codegen-units = 1`,
  `opt-level = 3`, `overflow-checks = false`), plus `cargo build-sbf --lto`;
* the same fixed program ids, instruction bytes, and account layouts handed
  to the same verifier contract (`bench/framework-comparison/verifier`);
* size is the whole `.so` in bytes, compute units are one Mollusk run per
  instruction, no warmup, no averaging.

Usage:

    py -3.12 scripts/bench-framework-comparison.py [--out bench/framework-comparison/results]
                                                   [--reference-dir <dir>] [--skip-build]

`--reference-dir` points at a directory holding pina's own pinocchio fixtures
(`hello/` and `counter/`, each a detached crate) and builds and measures them
with the identical recipe. That cross-check is how the verifier is proven to
reproduce pina's published pinocchio numbers on this toolchain before any
Hopper number is trusted.

The script writes `<out>/results.json` and `<out>/RESULTS.md`. It exits
non-zero if any fixture fails to build or fails its functional checks.
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
FIXTURES = REPO / "bench" / "framework-comparison" / "programs"
VERIFIER_PACKAGE = "hopper-framework-verifier"
VERIFIER_BIN = "framework-verifier"

# pina's per-fixture program ids (scripts/benchmark-frameworks.ts). Shared by
# every framework so the PDA derivation, and therefore the bump search, is
# identical across rows.
PROGRAM_IDS = {
    "hello": "DCF5KBmtQ9ryDC7mQezKLwuJHem6coVUCmKkw37M9J4A",
    "counter": "GJQcuWrT2f3f4KNuJcXhhwUa1ZQTYbxzzJ1hotzKu8hS",
}

# pina's RELEASE_PROFILE, verbatim.
RELEASE_PROFILE = {
    "CARGO_PROFILE_RELEASE_LTO": "fat",
    "CARGO_PROFILE_RELEASE_CODEGEN_UNITS": "1",
    "CARGO_PROFILE_RELEASE_OPT_LEVEL": "3",
    "CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS": "false",
}

# `[disc:u8][bump:u8][count:u64 LE]`, the layout pina calls BUMP_THEN_COUNT.
BUMP_THEN_COUNT = {"size": 10, "discriminator": "01", "bump_offset": 1, "count_offset": 2}
# Hopper's headered account: 16-byte universal header (discriminator at byte
# 0) then the same `[bump][count]` body.
HOPPER_HEADERED = {"size": 25, "discriminator": "01", "bump_offset": 16, "count_offset": 17}

HELLO_LOG = "Hello, Solana!"


def fixture(directory: str, label: str, crate: str, **extra: object) -> dict[str, object]:
    row = {"directory": directory, "label": label, "crate": crate}
    row.update(extra)
    return row


# One entry per framework row, in table order. `crate` is the artifact stem
# (`<crate>.so`). `data` holds the instruction bytes as hex, per instruction.
HOPPER_FIXTURES: dict[str, list[dict[str, object]]] = {
    "hello": [
        fixture(
            "hopper-substrate",
            "Hopper (substrate)",
            "hello_hopper_substrate",
            data={"hello": ""},
        ),
        fixture("hopper", "Hopper (macro)", "hello_hopper", data={"hello": "00"}),
    ],
    "counter": [
        fixture(
            "hopper-substrate",
            "Hopper (substrate)",
            "counter_hopper_substrate",
            data={"initialize": "00", "increment": "01"},
            initialize_takes_bump=True,
            counter_account=BUMP_THEN_COUNT,
        ),
        fixture(
            "hopper",
            "Hopper (macro)",
            "counter_hopper",
            data={"initialize": "00", "increment": "01"},
            initialize_takes_bump=True,
            counter_account=HOPPER_HEADERED,
        ),
    ],
}

# pina's pinocchio reference fixtures, when a `--reference-dir` is supplied.
REFERENCE_FIXTURES: dict[str, dict[str, object]] = {
    "hello": fixture(
        "hello", "Pinocchio (pina reference, rebuilt here)", "hello_pinocchio", data={"hello": ""}
    ),
    "counter": fixture(
        "counter",
        "Pinocchio (pina reference, rebuilt here)",
        "counter_pinocchio",
        data={"initialize": "00", "increment": "01"},
        initialize_takes_bump=True,
        counter_account=BUMP_THEN_COUNT,
    ),
}

# pina's published table, generated at pina commit 625d03476052 (PR #411,
# 2026-09-14) with cargo-build-sbf from Agave 4.2.2 and platform-tools v1.54,
# copied from docs/src/framework-comparison.md at commit
# aa81c8d1d5816bb932832adce1c5550cd74b6782. Reproduced here so the rendered
# table is complete; every number in this block is pina's, not ours.
PINA_PUBLISHED = {
    "hello": [
        {"label": "Pina", "bytes": 4680, "hello": 145},
        {"label": "Pinocchio (hand-written)", "bytes": 3160, "hello": 111},
        {"label": "Quasar", "bytes": 2520, "hello": 115},
        {"label": "Anchor v2 (lang-v2, rc.1)", "bytes": 1880, "hello": 127},
    ],
    "counter": [
        {"label": "Pina", "bytes": 13024, "initialize": 3301, "increment": 1753},
        {"label": "Pinocchio (hand-written)", "bytes": 6512, "initialize": 1490, "increment": 1721},
        {"label": "Quasar", "bytes": 7808, "initialize": 3488, "increment": 330},
        {"label": "Anchor v2 (lang-v2, rc.1)", "bytes": 8696, "initialize": 3458, "increment": 2117},
    ],
}
PINA_PINOCCHIO_BYTES = {"hello": 3160, "counter": 6512}


def run(cmd: list[str], *, env: dict[str, str] | None = None, cwd: Path | None = None) -> str:
    print("$", " ".join(cmd), flush=True)
    completed = subprocess.run(
        cmd, cwd=cwd, env=env, text=True, capture_output=True, encoding="utf-8", errors="replace"
    )
    if completed.returncode != 0:
        sys.stderr.write(completed.stdout)
        sys.stderr.write(completed.stderr)
        raise SystemExit(f"command failed ({completed.returncode}): {' '.join(cmd)}")
    return completed.stdout


def tool_versions() -> dict[str, str]:
    versions = {}
    for name, cmd in {
        "cargo-build-sbf": ["cargo", "build-sbf", "--version"],
        "rustc": ["rustc", "--version"],
    }.items():
        try:
            out = subprocess.run(cmd, text=True, capture_output=True, encoding="utf-8").stdout
            versions[name] = " ".join(out.split())
        except OSError as error:
            versions[name] = f"unavailable ({error})"
    return versions


def build_fixture(manifest: Path, out_dir: Path) -> None:
    env = dict(os.environ)
    env.update(RELEASE_PROFILE)
    run(
        [
            "cargo",
            "build-sbf",
            "--lto",
            "--manifest-path",
            str(manifest),
            "--sbf-out-dir",
            str(out_dir),
        ],
        env=env,
    )


def build_verifier() -> Path:
    run(["cargo", "build", "--release", "-p", VERIFIER_PACKAGE], cwd=REPO)
    exe = ".exe" if platform.system() == "Windows" else ""
    path = REPO / "target" / "release" / f"{VERIFIER_BIN}{exe}"
    if not path.exists():
        raise SystemExit(f"verifier binary missing after build: {path}")
    return path


def measure(verifier: Path, case: str, so: Path, row: dict[str, object]) -> dict[str, object]:
    data = row["data"]  # type: ignore[index]
    cmd = [str(verifier), "--so", str(so), "--program-id", PROGRAM_IDS[case], "--case", case]
    if case == "hello":
        cmd += ["--hello-data", data["hello"], "--expect-log", HELLO_LOG]  # type: ignore[index]
    else:
        layout = row["counter_account"]  # type: ignore[index]
        cmd += [
            "--initialize-data",
            data["initialize"],  # type: ignore[index]
            "--increment-data",
            data["increment"],  # type: ignore[index]
            "--account-size",
            str(layout["size"]),  # type: ignore[index]
            "--account-discriminator",
            layout["discriminator"],  # type: ignore[index]
            "--bump-offset",
            str(layout["bump_offset"]),  # type: ignore[index]
            "--count-offset",
            str(layout["count_offset"]),  # type: ignore[index]
        ]
        if row.get("initialize_takes_bump"):
            cmd.append("--initialize-takes-bump")
    report = json.loads(run(cmd))
    result = {"label": row["label"], "bytes": report["bytes"], "so": so.name}
    for step in report["instructions"]:
        if not step["ok"]:
            raise SystemExit(f"{row['label']} {case}: step {step['name']} failed")
        result[step["name"]] = step["compute_units"]
    if case == "counter":
        result["account_size"] = row["counter_account"]["size"]  # type: ignore[index]
    return result


def pct(bytes_: int, base: int) -> str:
    delta = (bytes_ - base) / base * 100
    sign = "+" if delta >= 0 else "-"
    return f"{sign}{abs(delta):.0f}%"


def render_markdown(results: dict[str, object]) -> str:
    versions = results["toolchain"]  # type: ignore[index]
    lines = [
        "# Framework comparison: Hopper rows against pina's fixtures",
        "",
        f"Generated {results['generated']} by `scripts/bench-framework-comparison.py`.",
        "",
        "Same fixtures, same verifier contract, same release recipe as pina's",
        "`benchmarks/framework-comparison` (pinned at pina commit",
        "`aa81c8d1d5816bb932832adce1c5550cd74b6782`). Size is the whole `.so`;",
        "compute units are one Mollusk run per instruction.",
        "",
        "Rows marked `measured here` were built and run on this machine:",
        "",
        f"- `{versions['cargo-build-sbf']}`",
        f"- `{versions['rustc']}`",
        f"- Mollusk {results['mollusk']}",
        "",
        "Rows marked `pina published` are copied from pina's",
        "`docs/src/framework-comparison.md` (generated at pina commit",
        "`625d03476052`, cargo-build-sbf from Agave 4.2.2, platform-tools v1.54,",
        "Mollusk 0.14). The rebuilt pinocchio reference row, when present, is the",
        "cross-check: it is pina's own fixture source built and measured here, so",
        "the gap between it and the published pinocchio row is the toolchain",
        "delta to keep in mind when reading the Hopper rows.",
        "",
    ]

    hello = results["hello"]  # type: ignore[index]
    lines += [
        "## Hello world",
        "",
        "| Framework | Source | Size (bytes) | `hello` CU | vs Pinocchio size |",
        "| --- | --- | ---: | ---: | ---: |",
    ]
    for row in hello:  # type: ignore[union-attr]
        lines.append(
            f"| {row['label']} | {row['source']} | {row['bytes']:,} | {row['hello']:,} | "
            f"{pct(row['bytes'], PINA_PINOCCHIO_BYTES['hello'])} |"
        )

    counter = results["counter"]  # type: ignore[index]
    lines += [
        "",
        "## Counter",
        "",
        "| Framework | Source | Size (bytes) | `initialize` CU | `increment` CU | Account bytes | vs Pinocchio size |",
        "| --- | --- | ---: | ---: | ---: | ---: | ---: |",
    ]
    for row in counter:  # type: ignore[union-attr]
        lines.append(
            f"| {row['label']} | {row['source']} | {row['bytes']:,} | {row['initialize']:,} | "
            f"{row['increment']:,} | {row.get('account_size', '')} | "
            f"{pct(row['bytes'], PINA_PINOCCHIO_BYTES['counter'])} |"
        )

    lines += [
        "",
        "## Reading the counter rows",
        "",
        "- `Hopper (substrate)` is the raw `program_entrypoint!` path with a",
        "  `#[hopper::state(compact, disc = 1)]` account: the same 10-byte",
        "  `[disc][bump][count]` layout, the same plain `CreateAccount` CPI, and",
        "  the same `create_program_address` re-derivation on `increment` as the",
        "  pinocchio and Pina fixtures. It is the like-for-like row.",
        "- `Hopper (macro)` is `#[derive(Accounts)]` plus `#[program]`: `init`,",
        "  `payer`, `seeds`, `bump = <arg>` on `initialize`, `bump = stored` on",
        "  `increment`. The account carries Hopper's 16-byte universal header, so",
        "  it is 25 bytes; Anchor's row has the same caveat at 24 bytes. The",
        "  verifier is told the offsets and checks the same post-state.",
        "- Quasar and the Hopper macro row verify the PDA with SHA-256 without",
        "  a curve check after account validation. The Hopper substrate,",
        "  Pinocchio, and Pina rows use the full PDA derivation syscall.",
        "- The pinned Anchor fixture disables default features and enables",
        "  `alloc`; its row does not include the default `guardrails` feature.",
        "- Pinocchio and Pina take the bump from instruction data; Quasar and",
        "  Anchor search for it on chain inside `initialize`. Both Hopper rows",
        "  take it from instruction data.",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", default=str(REPO / "bench" / "framework-comparison" / "results"))
    parser.add_argument(
        "--reference-dir",
        default=str(REPO / "bench" / "framework-comparison" / "reference"),
        help="pina's pinocchio fixtures (hello/, counter/) to rebuild as the cross-check",
    )
    parser.add_argument("--no-reference", action="store_true", help="skip the pinocchio cross-check rows")
    parser.add_argument("--skip-build", action="store_true", help="reuse the .so files already in --out")
    args = parser.parse_args()
    if args.no_reference:
        args.reference_dir = None

    out = Path(args.out).resolve()
    artifacts = out / "sbf"
    artifacts.mkdir(parents=True, exist_ok=True)

    if not args.skip_build:
        for case, rows in HOPPER_FIXTURES.items():
            for row in rows:
                manifest = FIXTURES / case / str(row["directory"]) / "Cargo.toml"
                build_fixture(manifest, artifacts)
        if args.reference_dir:
            for case, row in REFERENCE_FIXTURES.items():
                manifest = Path(args.reference_dir) / str(row["directory"]) / "Cargo.toml"
                build_fixture(manifest, artifacts)

    verifier = build_verifier()

    mollusk = "unknown"
    lock = (REPO / "Cargo.lock").read_text(encoding="utf-8").splitlines()
    for index, line in enumerate(lock):
        if line.strip() == 'name = "mollusk-svm"' and index + 1 < len(lock):
            mollusk = lock[index + 1].split('"')[1]
            break

    results: dict[str, object] = {
        "generated": datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M UTC"),
        "toolchain": tool_versions(),
        "mollusk": mollusk,
        "release_profile": RELEASE_PROFILE,
        "program_ids": PROGRAM_IDS,
    }

    for case, rows in HOPPER_FIXTURES.items():
        table: list[dict[str, object]] = []
        for row in rows:
            so = artifacts / f"{row['crate']}.so"
            measured = measure(verifier, case, so, row)
            measured["source"] = "measured here"
            table.append(measured)
        if args.reference_dir:
            row = REFERENCE_FIXTURES[case]
            so = artifacts / f"{row['crate']}.so"
            measured = measure(verifier, case, so, row)
            measured["source"] = "measured here (cross-check)"
            table.append(measured)
        for published in PINA_PUBLISHED[case]:
            table.append({**published, "source": "pina published"})
        results[case] = table

    (out / "results.json").write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")
    (out / "RESULTS.md").write_text(render_markdown(results), encoding="utf-8")
    print(render_markdown(results))
    return 0


if __name__ == "__main__":
    sys.exit(main())
