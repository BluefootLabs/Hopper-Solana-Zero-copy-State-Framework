#!/usr/bin/env python3
"""Check the deployed canonical-PDA fixture on public devnet only.

Readonly modes 0..4 require no funded PDA account. Typed modes 5/6 and the
exhaustive noncanonical-bump matrix are exercised in the compiled SBF suite.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import runpy
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
# Reuse the existing explicit-devnet RPC, sender and finalization helpers.
helpers = runpy.run_path(str(ROOT / "scripts/test-runtime-gate-devnet.py"))
run, rpc, finalize = (helpers[name] for name in ("run", "rpc", "finalize"))
RPC, GENESIS = helpers["RPC"], helpers["GENESIS"]
PROGRAM = "8RJxAyfAMnpb5ghwA4comPDJw6KqbDmZ28LDZHcccaVH"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--payer", type=Path, required=True)
    parser.add_argument("--hopper", type=Path, required=True)
    parser.add_argument("--elf", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    output = args.out.resolve()
    if not output.is_relative_to(ROOT / "target"):
        raise RuntimeError("output must stay under the ignored target directory")
    if rpc("getGenesisHash", []) != GENESIS:
        raise RuntimeError("unexpected devnet genesis")
    if run(["git", "status", "--porcelain"]).strip():
        raise RuntimeError("commit source before capturing live evidence")
    source = run(["git", "rev-parse", "HEAD"]).strip()
    output.mkdir(parents=True, exist_ok=False)
    elf = args.elf.read_bytes()

    def verify_deployment(phase: str) -> None:
        dump = output / f"{phase}-onchain.so"
        run(["solana", "program", "dump", PROGRAM, str(dump), "--url", RPC,
             "--keypair", str(args.payer), "--commitment", "finalized"])
        if dump.read_bytes() != elf:
            raise RuntimeError("deployed ELF does not match the tested local artifact")

    def derive(*seeds: str) -> tuple[str, int]:
        text = run([str(args.hopper), "keys", "pda", *seeds, "--program", PROGRAM])
        address = re.search(r"^PDA:\s*(\w+)", text, re.MULTILINE)
        bump = re.search(r"^bump:\s*(\d+)", text, re.MULTILINE)
        if address is None or bump is None:
            raise RuntimeError("missing client PDA result")
        return address[1], int(bump[1])

    canonical, bump = derive("config", "v1")
    wrong, _ = derive("wrong-config", "v1")
    addresses = [canonical, wrong]

    def snapshot(min_slot: int) -> dict:
        return rpc("getMultipleAccounts", [addresses, {
            "commitment": "finalized", "encoding": "base64", "minContextSlot": min_slot
        }])

    verify_deployment("before")
    records = []
    for mode in range(5):
        cases = [
            ("canonical", canonical, bytes([mode, bump]), None),
            ("wrong-bump", canonical, bytes([mode, bump ^ 1]), "InvalidSeeds"),
            ("wrong-address", wrong, bytes([mode, bump]), "InvalidSeeds"),
            ("malformed-data", canonical, bytes([mode]), "InvalidInstructionData"),
            ("missing-account", None, bytes([mode, bump]), "NotEnoughAccountKeys"),
        ]
        for case, address, data, error in cases:
            name = f"mode-{mode}-{case}"
            before = snapshot(records[-1]["slot"] if records else 0)
            command = [str(args.hopper), "tx", "send", "--program", PROGRAM,
                       "--keypair", str(args.payer), "--rpc", RPC, "--data", data.hex()]
            if address is not None:
                command += ["--account", address]
            if error is not None:
                command += ["--allow-failure"]
            text = run(command)
            (output / f"{name}.log").write_text(text, encoding="utf-8")
            signature = re.search(r"^signature\s*:\s*(\w+)", text, re.MULTILINE)
            if signature is None:
                raise RuntimeError("sender returned no signature")
            tx = finalize(signature[1])
            expected = None if error is None else {"InstructionError": [0, error]}
            if tx["meta"]["err"] != expected:
                raise RuntimeError(f"{name}: unexpected program result {tx['meta']['err']}")
            after = snapshot(max(tx["slot"], before["context"]["slot"]))
            if before["value"] != after["value"]:
                raise RuntimeError(f"{name}: readonly fixture changed account state")
            (output / f"{name}.transaction.json").write_text(json.dumps(tx, indent=2) + "\n", encoding="utf-8")
            (output / f"{name}.snapshots.json").write_text(json.dumps({
                "addresses": addresses, "before": before, "after": after
            }, indent=2) + "\n", encoding="utf-8")
            record = {"name": name, "signature": signature[1], "slot": tx["slot"],
                      "error": tx["meta"]["err"], "computeUnits": tx["meta"].get("computeUnitsConsumed"),
                      "unchangedFullSnapshots": True}
            records.append(record)
            print(f"{name}: finalized, {record['computeUnits']} CU, expected result and state", flush=True)
    verify_deployment("after")
    if run(["git", "rev-parse", "HEAD"]).strip() != source or run(["git", "status", "--porcelain"]).strip():
        raise RuntimeError("source changed during capture")
    receipt = {"schema": "hopper.canonical-pda-devnet.v1", "sourceCommit": source,
               "rpcEndpoint": RPC, "genesisHash": GENESIS, "commitment": "finalized",
               "programId": PROGRAM, "canonicalAddress": canonical, "canonicalBump": bump,
               "elfSha256": hashlib.sha256(elf).hexdigest(), "elfBytes": len(elf),
               "deployedElfMatchesBeforeAndAfter": True, "transactions": records}
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
