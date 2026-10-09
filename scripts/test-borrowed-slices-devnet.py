#!/usr/bin/env python3
"""Check bounded batch validation and refused-write invariants on public devnet.

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
    modes = live.add_mutually_exclusive_group()
    modes.add_argument("--generated-dispatch", action="store_true",
                      help="exercise the generated batch handler (tag 1)")
    modes.add_argument("--owned-aliases", action="store_true",
                      help="exercise aliased owned vector, UTF-8 string, and scalar arguments (tag 2)")
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
    mode = 1 if args.generated_dispatch else 0

    def payload(rows, count=None, nonce=513):
        return bytes([mode]) + struct.pack("<H", len(rows) if count is None else count) + b"".join(
            bytes([tag, side]) + struct.pack("<Q", amount) for tag, side, amount in rows
        ) + struct.pack("<H", nonce)

    valid = [(1, 1, 42), (0, 255, 99)]
    cases = [
        ("batch-two", payload(valid), None, normal, 2, 141),
        ("empty", payload([]), None, normal, 0, 0),
        ("maximum", payload([(1, 7, 1)] * 32), None, normal, 32, 32),
        ("last-invalid-option", payload([(1, 1, 42), (2, 7, 99)]), invalid, normal, 0, 0),
        ("last-invalid-enum", payload([(1, 1, 42), (1, 9, 99)]), invalid, normal, 0, 0),
        ("over-capacity", payload([(1, 1, 1)] * 33), invalid, normal, 0, 0),
        ("short-elements", payload(valid, count=3), invalid, normal, 0, 0),
        ("count-u16-max", payload([], count=65535), invalid, normal, 0, 0),
        ("surplus", payload(valid) + b"x", invalid, normal, 0, 0),
        ("nonce", payload(valid, nonce=514), invalid, normal, 0, 0),
        ("sum-overflow", payload([(1, 1, (1 << 64) - 1), (1, 7, 1)]), "ArithmeticOverflow", normal, 0, 0),
        ("state-overflow", payload([(1, 1, (1 << 64) - 1)]), "ArithmeticOverflow", normal, 0, 0),
        ("unsigned", payload(valid), "MissingRequiredSignature", [state + ":w", SYSTEM], 0, 0),
        ("readonly", payload(valid), "InvalidAccountData", [state, "payer:s"], 0, 0),
        ("foreign-owner", payload(valid), "IllegalOwner", [SYSTEM, "payer:s"], 0, 0),
    ]
    if args.owned_aliases:
        def payload(rows, count=None, nonce=513, label=b"\xc3\xa9"):
            return bytes([2]) + struct.pack("<H", len(rows) if count is None else count) + b"".join(
                struct.pack("<Q", amount) for amount in rows
            ) + struct.pack("<H", len(label)) + label + struct.pack("<H", nonce)
        valid = [42, 99]
        cases = [
            ("batch-two", payload(valid), None, normal, 2, 141),
            ("empty", payload([]), None, normal, 0, 0),
            ("maximum", payload([1] * 32), None, normal, 32, 32),
            ("over-capacity", payload([1] * 33), invalid, normal, 0, 0),
            ("short-elements", payload(valid, count=3), invalid, normal, 0, 0),
            ("count-u16-max", payload([], count=65535), invalid, normal, 0, 0),
            ("invalid-utf8", payload(valid, label=b"\xff"), invalid, normal, 0, 0),
            ("string-capacity", payload(valid, label=b"123456789"), invalid, normal, 0, 0),
            ("wrong-label", payload(valid, label=b"ok"), invalid, normal, 0, 0),
            ("surplus", payload(valid) + b"x", invalid, normal, 0, 0),
            ("nonce", payload(valid, nonce=514), invalid, normal, 0, 0),
            ("sum-overflow", payload([(1 << 64) - 1, 1]), "ArithmeticOverflow", normal, 0, 0),
            ("state-overflow", payload([(1 << 64) - 1]), "ArithmeticOverflow", normal, 0, 0),
            ("unsigned", payload(valid), "MissingRequiredSignature", [state + ":w", SYSTEM], 0, 0),
            ("readonly", payload(valid), "InvalidAccountData", [state, "payer:s"], 0, 0),
            ("foreign-owner", payload(valid), "IllegalOwner", [SYSTEM, "payer:s"], 0, 0),
        ]
    for name, data, error, accounts, added_count, added_total in cases:
        tx = send(name, accounts, data, expected=error)
        after = account(tx["slot"])
        if error is None:
            count, total = count + added_count, total + added_total
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
    for length in (0, 1, 2, 3, 22, 24):
        tx = send(f"truncated-{length}", normal, payload(valid)[:length], expected=invalid)
        after = account(tx["slot"])
        if after != current:
            raise RuntimeError("truncated instruction mutated state")
        write(out / f"truncated-{length}.account.json", after)
    deployment("after")
    if source_snapshot() != source:
        raise RuntimeError("source changed during the run")
    write(out / "summary.json", {"schema": "hopper.borrowed-slices.devnet.v1",
        "network": "devnet", "genesis": GENESIS, "programId": fixture,
        "baseCommit": source["baseCommit"], "sourceSnapshotSha256": hashlib.sha256((out / "source.json").read_bytes()).hexdigest(),
        "elfSha256": hashlib.sha256(elf).hexdigest(), "elfBytes": len(elf),
        "state": state, "finalCount": count, "finalTotal": total,
        "dispatch": "owned-aliases" if args.owned_aliases else "generated" if args.generated_dispatch else "manual",
        "transactions": records, "deploymentMatchedBeforeAndAfter": True})


if __name__ == "__main__":
    main()
