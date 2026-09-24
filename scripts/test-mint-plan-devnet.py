#!/usr/bin/env python3
"""Verify an exact mint-plan ELF on public devnet, including CPI rollback.

Keypairs remain under ignored target/. Only the explicit public devnet endpoint
is used. This spends devnet SOL on mint rent and transaction fees.
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
LEGACY = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
TOKEN_2022 = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
EXTENSIONS = [(1, 108), (3, 32), (9, 0), (12, 32), (14, 64), (18, 64)]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--program", required=True)
    parser.add_argument("--payer", type=Path, required=True)
    parser.add_argument("--hopper", type=Path, required=True)
    parser.add_argument("--elf", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    if not out.is_relative_to(ROOT / "target"):
        raise RuntimeError("output must stay under ignored target/")
    assert rpc("getGenesisHash", []) == GENESIS, "unexpected network"
    assert not run(["git", "status", "--porcelain"]).strip(), "commit source first"
    source = run(["git", "rev-parse", "HEAD"]).strip()
    out.mkdir(parents=True, exist_ok=False)
    (out / "keys").mkdir()
    payer = run(["solana-keygen", "pubkey", str(args.payer)]).strip()
    elf = args.elf.read_bytes()
    records = []

    def write(name, value):
        (out / name).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")

    def deployment(phase):
        dump = out / f"{phase}-onchain.so"
        run(["solana", "program", "dump", args.program, str(dump), "--url", RPC,
             "--keypair", str(args.payer), "--commitment", "finalized"])
        assert dump.read_bytes() == elf, "deployed ELF differs from tested artifact"

    def snapshot(mint, slot=0):
        return rpc("getMultipleAccounts", [[payer, mint], {
            "encoding": "base64", "commitment": "finalized", "minContextSlot": slot
        }])

    def new_mint(name):
        key = out / "keys" / f"{name}.json"
        run(["solana-keygen", "new", "--no-bip39-passphrase", "--silent", "--outfile", str(key)])
        return run(["solana-keygen", "pubkey", str(key)]).strip(), key

    def send(name, program, accounts, data, signers=(), error=None, budget=None):
        argv = [str(args.hopper), "tx", "send", "--program", program,
                "--keypair", str(args.payer), "--rpc", RPC, "--data", data.hex()]
        for account in accounts:
            argv += ["--account", account]
        for signer in signers:
            argv += ["--signer", str(signer)]
        if error is not None:
            argv += ["--allow-failure"]
        if budget is not None:
            argv += ["--compute-limit", str(budget)]
        text = run(argv)
        (out / f"{name}.log").write_text(text, encoding="utf-8")
        match = re.search(r"^signature\s*:\s*(\w+)", text, re.MULTILINE)
        assert match is not None, "sender returned no signature"
        tx = finalize(match[1])
        write(f"{name}.transaction.json", tx)
        expected = None if error is None else {"InstructionError": [int(budget is not None), error]}
        assert tx["meta"]["err"] == expected, (name, tx["meta"]["err"], expected)
        record = {"name": name, "signature": match[1], "slot": tx["slot"],
                  "error": tx["meta"]["err"], "computeUnits": tx["meta"].get("computeUnitsConsumed")}
        records.append(record)
        print(f"{name}: finalized, {record['computeUnits']} CU", flush=True)
        return tx, record

    def exercise(name, mint, key, legacy, mask, operation=0, delta=0, bump=0,
                 error=None, budget=None):
        before = snapshot(mint, records[-1]["slot"] if records else 0)
        token = LEGACY if legacy else TOKEN_2022
        metas = ["payer:sw", mint + (":sw" if key and operation == 0 else ":w"), token, SYSTEM]
        tx, record = send(name, args.program, metas,
                          bytes([int(not legacy), mask, operation, delta, bump]),
                          [key] if key and operation == 0 else [], error, budget)
        after = snapshot(mint, max(tx["slot"], before["context"]["slot"]))
        old_payer, old_mint = before["value"]
        new_payer, new_mint = after["value"]
        fee = tx["meta"]["fee"]
        if error is not None:
            assert old_mint == new_mint, "failed transaction changed the mint snapshot"
            expected_payer = dict(old_payer, lamports=old_payer["lamports"] - fee)
            assert new_payer == expected_payer, "failed transaction spent funds beyond fees"
            if budget is not None:
                logs = tx["meta"]["logMessages"]
                assert f"Program {SYSTEM} success" in logs, "did not reach successful creation CPI"
                assert f"Program {TOKEN_2022} success" in logs, "did not reach successful extension CPI"
                record["rollbackAfterSuccessfulNestedCpis"] = True
        else:
            expected_extensions = [entry for i, entry in enumerate(EXTENSIONS) if mask & (1 << i)]
            size = 82 if not mask else 166 + sum(4 + n for _, n in expected_extensions)
            data = base64.b64decode(new_mint["data"][0])
            assert new_mint["owner"] == token and len(data) == size
            assert struct.unpack_from("<I", data, 0)[0] == 1 and data[4:36] == pubkey_bytes(payer)
            assert struct.unpack_from("<Q", data, 36)[0] == 0 and data[44:46] == bytes([9, 1])
            assert struct.unpack_from("<I", data, 46)[0] == 1 and data[50:82] == pubkey_bytes(payer)
            if mask:
                assert data[82:165] == bytes(83) and data[165] == 1
                pos = 166
                for tag, length in expected_extensions:
                    assert struct.unpack_from("<HH", data, pos) == (tag, length)
                    pos += 4 + length
                assert pos == size
            rent = rpc("getMinimumBalanceForRentExemption", [size, {"commitment": "finalized"}])
            old_balance = old_mint["lamports"] if old_mint else 0
            funding = max(0, rent - old_balance)
            assert new_mint["lamports"] == max(rent, old_balance)
            assert new_payer == dict(old_payer, lamports=old_payer["lamports"] - fee - funding)
            record.update({"mintBytes": size, "rentMinimum": rent, "payerFunding": funding})
        write(f"{name}.snapshots.json", {"addresses": [payer, mint], "before": before, "after": after})
        record["expectedFullSnapshotsVerified"] = True

    deployment("before")
    for name, legacy, mask in [("legacy", True, 0), ("token2022-base", False, 0),
                               ("close-and-pointer", False, 34), ("all-six", False, 63)]:
        mint, key = new_mint(name)
        exercise(name, mint, key, legacy, mask)
        if mask == 63:
            exercise("reinitialize", mint, None, False, mask, operation=1,
                     error="AccountAlreadyInitialized")
    mint, key = new_mint("prefunded")
    prefund = rpc("getMinimumBalanceForRentExemption", [490, {"commitment": "finalized"}]) + 123
    send("prefund", SYSTEM, ["payer:sw", mint + ":w"], struct.pack("<IQ", 2, prefund))
    exercise("prefunded", mint, key, False, 63)
    derived = run([str(args.hopper), "keys", "pda", "mint", "hex:" + pubkey_bytes(payer).hex(),
                   "--program", args.program])
    address = re.search(r"^PDA:\s*(\w+)", derived, re.MULTILINE)
    bump = re.search(r"^bump:\s*(\d+)", derived, re.MULTILINE)
    assert address and bump, "missing PDA derivation"
    exercise("pda", address[1], None, False, 63, operation=3, bump=int(bump[1]))
    for name, delta, error, budget in [
        ("undersized", 255, "InvalidAccountData", None),
        ("oversized", 1, "InvalidAccountData", None),
        ("nested-cpi-rollback", 0, "ComputationalBudgetExceeded", 10_000),
    ]:
        mint, key = new_mint(name)
        exercise(name, mint, key, False, 63, delta=delta, error=error, budget=budget)
    deployment("after")
    assert run(["git", "rev-parse", "HEAD"]).strip() == source
    assert not run(["git", "status", "--porcelain"]).strip(), "source changed during capture"
    write("receipt.json", {"schema": "hopper.mint-plan-devnet.v1", "sourceCommit": source,
                          "rpcEndpoint": RPC, "genesisHash": GENESIS, "commitment": "finalized",
                          "programId": args.program, "elfSha256": hashlib.sha256(elf).hexdigest(),
                          "deployedElfMatchesBeforeAndAfter": True, "transactions": records})
    print("All mint-plan devnet cases and exact state checks passed.", flush=True)


if __name__ == "__main__":
    main()
