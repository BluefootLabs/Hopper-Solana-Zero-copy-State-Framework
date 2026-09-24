#!/usr/bin/env python3
"""Test canonical bump persistence and typed reads on public devnet.

Requires the updated canonical-PDA fixture and a previously absent config PDA.
Readonly mode coverage remains in test-canonical-pda-devnet.py.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import runpy
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
H = runpy.run_path(str(ROOT / "scripts/test-runtime-gate-devnet.py"))
run, rpc, finalize = (H[k] for k in ("run", "rpc", "finalize"))
RPC, GENESIS, SYSTEM = (H[k] for k in ("RPC", "GENESIS", "SYSTEM"))
PROGRAM = "8RJxAyfAMnpb5ghwA4comPDJw6KqbDmZ28LDZHcccaVH"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--payer", type=Path, required=True)
    parser.add_argument("--hopper", type=Path, required=True)
    parser.add_argument("--elf", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    assert out.is_relative_to(ROOT / "target"), "output must stay under ignored target/"
    assert rpc("getGenesisHash", []) == GENESIS, "unexpected network"
    assert not run(["git", "status", "--porcelain"]).strip(), "commit source first"
    source = run(["git", "rev-parse", "HEAD"]).strip()
    out.mkdir(parents=True, exist_ok=False)
    payer = run(["solana-keygen", "pubkey", str(args.payer)]).strip()
    derived = run([str(args.hopper), "keys", "pda", "config", "v1", "--program", PROGRAM])
    address = re.search(r"^PDA:\s*(\w+)", derived, re.MULTILINE)
    match = re.search(r"^bump:\s*(\d+)", derived, re.MULTILINE)
    assert address and match, "missing client PDA derivation"
    config, bump = address[1], int(match[1])
    elf = args.elf.read_bytes()
    records = []

    def write(name, value):
        (out / name).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")

    def deployment(phase):
        dump = out / f"{phase}-onchain.so"
        run(["solana", "program", "dump", PROGRAM, str(dump), "--url", RPC,
             "--keypair", str(args.payer), "--commitment", "finalized"])
        assert dump.read_bytes() == elf, "deployed ELF does not match tested binary"

    def snapshot(slot=0):
        return rpc("getMultipleAccounts", [[config, payer], {
            "encoding": "base64", "commitment": "finalized", "minContextSlot": slot
        }])

    deployment("before")
    assert snapshot()["value"][0] is None, "config already exists; inspect before retrying initialization"
    cases = [("wrong-init-bump", 8, bump ^ 1, False), ("initialize", 8, bump, True),
             ("reinitialize", 8, bump, False)]
    for mode in [5, 6, 7]:
        cases.extend([(f"typed-{mode}", mode, bump, True),
                      (f"typed-{mode}-wrong-argument", mode, bump ^ 1, False)])
    for name, mode, supplied, success in cases:
        before = snapshot(records[-1]["slot"] if records else 0)
        argv = [str(args.hopper), "tx", "send", "--program", PROGRAM,
                "--keypair", str(args.payer), "--rpc", RPC,
                "--data", bytes([mode, supplied]).hex(), "--account", config + (":w" if mode == 8 else "")]
        if mode == 8:
            argv += ["--account", "payer:sw", "--account", SYSTEM]
        if not success:
            argv += ["--allow-failure"]
        text = run(argv)
        (out / f"{name}.log").write_text(text, encoding="utf-8")
        signature = re.search(r"^signature\s*:\s*(\w+)", text, re.MULTILINE)
        assert signature is not None, "sender returned no signature"
        tx = finalize(signature[1])
        write(f"{name}.transaction.json", tx)
        error = tx["meta"]["err"]
        assert (error is None) == success, (name, error)
        if not success and name != "reinitialize":
            assert error == {"InstructionError": [0, "InvalidSeeds"]}, (name, error)
        after = snapshot(max(tx["slot"], before["context"]["slot"]))
        old_config, old_payer = before["value"]
        new_config, new_payer = after["value"]
        funding = 0
        if name == "initialize":
            data = base64.b64decode(new_config["data"][0])
            assert len(data) == 17 and data[0:2] == bytes([1, 1]) and data[16] == bump
            assert new_config["owner"] == PROGRAM
            funding = rpc("getMinimumBalanceForRentExemption", [17, {"commitment": "finalized"}])
            assert new_config["lamports"] == funding
        else:
            assert old_config == new_config, "read or refusal changed full config snapshot"
        assert new_payer == dict(old_payer, lamports=old_payer["lamports"] - tx["meta"]["fee"] - funding)
        write(f"{name}.snapshots.json", {"addresses": [config, payer], "before": before, "after": after})
        record = {"name": name, "signature": signature[1], "slot": tx["slot"], "error": error,
                  "computeUnits": tx["meta"].get("computeUnitsConsumed"), "expectedFullSnapshotsVerified": True}
        records.append(record)
        print(f"{name}: finalized, {record['computeUnits']} CU", flush=True)
    deployment("after")
    assert run(["git", "rev-parse", "HEAD"]).strip() == source
    assert not run(["git", "status", "--porcelain"]).strip(), "source changed during capture"
    write("receipt.json", {"schema": "hopper.pda-initialization-devnet.v1", "sourceCommit": source,
                          "rpcEndpoint": RPC, "genesisHash": GENESIS, "commitment": "finalized",
                          "programId": PROGRAM, "config": config, "canonicalBump": bump,
                          "elfSha256": hashlib.sha256(elf).hexdigest(),
                          "deployedElfMatchesBeforeAndAfter": True, "transactions": records})
    print("All PDA initialization and typed-read devnet cases passed.", flush=True)


if __name__ == "__main__":
    main()
