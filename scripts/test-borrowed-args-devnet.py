#!/usr/bin/env python3
"""Check borrowed argument validation and refused-write invariants on public devnet.

The snapshot subcommand records the working source before compilation. The run
subcommand refuses changed source and checks the deployed ELF before and after
the transactions. This is working-tree evidence, not a clean release attestation.
Keys and raw receipts stay under ignored target/. No global config is changed.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import runpy
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
H = runpy.run_path(str(ROOT / "scripts/test-runtime-gate-devnet.py"))
run, rpc, finalize, pubkey_bytes = (H[k] for k in ("run", "rpc", "finalize", "pubkey_bytes"))
RPC, GENESIS, SYSTEM = (H[k] for k in ("RPC", "GENESIS", "SYSTEM"))
gate_sources = runpy.run_path(str(ROOT / "scripts/record-quality-gate.py"))["sources"]


def source_snapshot() -> dict:
    return {"baseCommit": run(["git", "rev-parse", "HEAD"]).strip(),
            "normalizeLineEndings": True, "files": gate_sources(ROOT)}


def write(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def ignored_output(path: Path) -> Path:
    path = path.resolve()
    if not path.is_relative_to(ROOT / "target"):
        raise RuntimeError("output must stay under ignored target/")
    return path


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    snapshot = commands.add_parser("snapshot")
    snapshot.add_argument("--out", required=True, type=Path)
    live = commands.add_parser("run")
    for name in ("payer", "hopper", "elf", "source-snapshot", "out"):
        live.add_argument("--" + name, required=True, type=Path)
    live.add_argument("--program", required=True)
    live.add_argument("--generated-dispatch", action="store_true",
                      help="exercise the generated &Update handlers (tags 2 and 3)")
    args = parser.parse_args()
    out = ignored_output(args.out)
    source = source_snapshot()
    if args.command == "snapshot":
        out.parent.mkdir(parents=True, exist_ok=True)
        if out.exists():
            raise RuntimeError("source snapshot already exists; choose a fresh path")
        write(out, source)
        return
    if source != json.loads(args.source_snapshot.read_text(encoding="utf-8")):
        raise RuntimeError("source changed since the pre-build snapshot; rebuild first")
    if rpc("getGenesisHash", []) != GENESIS:
        raise RuntimeError("unexpected network")
    elf = args.elf.read_bytes()
    out.mkdir(parents=True, exist_ok=False)
    keys = out / "keys"
    keys.mkdir()
    write(out / "source.json", source)
    records = []

    def deployment(phase):
        dump = out / f"{phase}-onchain.so"
        run(["solana", "program", "dump", args.program, str(dump), "--url", RPC,
             "--keypair", str(args.payer), "--commitment", "finalized"])
        if dump.read_bytes() != elf:
            raise RuntimeError("deployed ELF differs from compiled fixture")

    def send(name, accounts, data, expected=None, signers=()):
        argv = [str(args.hopper), "tx", "send", "--program", args.program,
                "--keypair", str(args.payer), "--rpc", RPC, "--data", data.hex()]
        for account in accounts:
            argv += ["--account", account]
        for signer in signers:
            argv += ["--signer", str(signer)]
        if expected is not None:
            argv += ["--allow-failure"]
        output = run(argv)
        (out / f"{name}.log").write_text(output, encoding="utf-8")
        match = re.search(r"^signature\s*:\s*(\w+)", output, re.MULTILINE)
        if match is None:
            raise RuntimeError("sender returned no signature; inspect before retrying")
        tx = finalize(match[1])
        write(out / f"{name}.transaction.json", tx)
        error = tx["meta"]["err"]
        if error != (None if expected is None else {"InstructionError": [0, expected]}):
            raise RuntimeError(f"{name}: unexpected error {error}")
        records.append({"name": name, "signature": match[1], "slot": tx["slot"],
                        "error": error, "computeUnits": tx["meta"].get("computeUnitsConsumed")})
        print(f"{name}: finalized, {records[-1]['computeUnits']} CU, {error or 'ok'}", flush=True)
        return tx

    deployment("before")
    key = keys / "state.json"
    run(["solana-keygen", "new", "--no-bip39-passphrase", "--silent", "--outfile", str(key)])
    state = run(["solana-keygen", "pubkey", str(key)]).strip()
    rent = rpc("getMinimumBalanceForRentExemption", [16])
    fixture = args.program
    args.program = SYSTEM
    tx = send("create-state", ["payer:sw", state + ":sw"],
              struct.pack("<IQQ", 0, rent, 16) + pubkey_bytes(fixture), signers=[key])
    args.program = fixture

    def account(slot):
        return rpc("getMultipleAccounts", [[state], {"encoding": "base64",
                   "commitment": "finalized", "minContextSlot": slot}])["value"][0]

    current = account(tx["slot"])
    if current is None or current["owner"] != fixture or current["lamports"] != rent:
        raise RuntimeError("state creation does not match expected owner/rent")
    if base64.b64decode(current["data"][0]) != bytes(16):
        raise RuntimeError("state did not start empty")
    count, total = 0, 0
    normal = [state + ":w", "payer:s"]
    invalid = "InvalidInstructionData"
    cases = [
        ("exact", 0, [1, 1, 0, 255], 42, b"", None, normal),
        ("absent-payload", 0, [0, 255, 1, 7], 7, b"", None, normal),
        ("tail-memo", 1, [1, 7, 1, 1], 5, b"memo", None, normal),
        ("tail-empty", 1, [0, 255, 0, 255], 0, b"", None, normal),
        ("tail-bound", 1, [1, 1, 0, 255], 1, b"x" * 32, None, normal),
        ("bad-option", 0, [2, 1, 0, 255], 42, b"", invalid, normal),
        ("bad-first-enum", 0, [1, 9, 0, 255], 42, b"", invalid, normal),
        ("bad-second-enum", 0, [1, 1, 1, 9], 42, b"", invalid, normal),
        ("bad-second-option", 1, [1, 1, 255, 7], 42, b"memo", invalid, normal),
        ("exact-surplus", 0, [1, 1, 0, 255], 42, b"x", invalid, normal),
        ("tail-too-long", 1, [1, 1, 0, 255], 42, b"x" * 33, invalid, normal),
        ("tail-invalid-utf8", 1, [1, 1, 0, 255], 42, b"\xff", invalid, normal),
        ("overflow", 0, [1, 1, 0, 255], (1 << 64) - 1, b"", "ArithmeticOverflow", normal),
        ("unsigned", 0, [1, 1, 0, 255], 42, b"", "MissingRequiredSignature", [state + ":w", SYSTEM]),
        ("readonly", 0, [1, 1, 0, 255], 42, b"", "InvalidAccountData", [state, "payer:s"]),
        ("foreign-owner", 0, [1, 1, 0, 255], 42, b"", "IllegalOwner", [SYSTEM, "payer:s"]),
    ]
    for name, mode, tags, amount, tail, error, accounts in cases:
        if args.generated_dispatch:
            mode += 2
        data = bytes([mode, *tags]) + struct.pack("<Q", amount) + tail
        tx = send(name, accounts, data, expected=error)
        after = account(tx["slot"])
        if error is None:
            count, total = count + 1, total + amount
            expected = dict(current)
            expected["data"] = [base64.b64encode(struct.pack("<QQ", count, total)).decode(), "base64"]
            if after != expected:
                raise RuntimeError(f"{name}: unexpected account mutation")
            returned = tx["meta"].get("returnData", {})
            if returned.get("programId") != fixture or base64.b64decode(returned["data"][0]) != struct.pack("<QQ", count, total):
                raise RuntimeError(f"{name}: unexpected return producer or bytes")
        elif after != current:
            raise RuntimeError(f"{name}: refused instruction mutated state")
        write(out / f"{name}.account.json", after)
        current = after
    for length in (0, 1, 12):
        mode = 2 if args.generated_dispatch else 0
        data = (bytes([mode, 1, 1, 0, 255]) + struct.pack("<Q", 42))[:length]
        tx = send(f"truncated-{length}", normal, data, expected=invalid)
        if account(tx["slot"]) != current:
            raise RuntimeError("truncated instruction mutated state")
    deployment("after")
    if source_snapshot() != source:
        raise RuntimeError("source changed during the run")
    write(out / "summary.json", {"schema": "hopper.borrowed-args.devnet.v1",
        "network": "devnet", "genesis": GENESIS, "programId": fixture,
        "baseCommit": source["baseCommit"], "sourceSnapshotSha256": hashlib.sha256((out / "source.json").read_bytes()).hexdigest(),
        "elfSha256": hashlib.sha256(elf).hexdigest(), "elfBytes": len(elf),
        "state": state, "finalCount": count, "finalTotal": total,
        "dispatch": "generated" if args.generated_dispatch else "manual",
        "transactions": records, "deploymentMatchedBeforeAndAfter": True})


if __name__ == "__main__":
    main()
