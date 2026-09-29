#!/usr/bin/env python3
"""Drive the hopper-confidential-lab program on public devnet as far as a
public cluster allows.

A confidential balance moves only against a proof the ZK ElGamal proof
program verified, and that program is disabled on every public cluster. So
this runs the instructions that need no proof, through Hopper's builders,
against the Token-2022 devnet runs: create a confidential mint that needs
approval and has an auditor, then switch it to auto-approve. It checks the
mint byte for byte after each step. Then it records what the cluster does
with a call to the proof program, and asserts that configuring an account
without a verified proof reaches Token-2022's `ConfigureAccount` and is
refused, leaving no account behind.

The full flow, with proofs, runs under Mollusk in
`examples/hopper-confidential-lab/tests/flow.rs` against the Token-2022
mainnet runs.

Keypairs stay under ignored target/. Only the explicit public devnet endpoint
is used. This spends devnet SOL on rent and fees.
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

TOKEN_2022 = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
ZK_PROOF_PROGRAM = "ZkE1Gama1Proof11111111111111111111111111111"
# The Ristretto base point: a well-formed ElGamal public key for the auditor.
AUDITOR = bytes.fromhex("e2f2ae0a6abc4e71a884a961c500515f58e30b6aa582dd8db6a65945e08d2d76")
MINT_LEN = 165 + 1 + 4 + 65
DECIMALS = 2
TAG_CREATE_MINT, TAG_UPDATE_MINT, TAG_OPEN_ACCOUNT = 0, 1, 2
VERIFY_PUBKEY_VALIDITY = 4
ANY_ERROR = object()


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

    def account(address, slot=0):
        value = rpc("getMultipleAccounts", [[address], {
            "encoding": "base64", "commitment": "finalized", "minContextSlot": slot
        }])["value"][0]
        if value is None:
            return None
        value = dict(value)
        value["bytes"] = base64.b64decode(value["data"][0])
        return value

    def new_key(name):
        key = out / "keys" / f"{name}.json"
        run(["solana-keygen", "new", "--no-bip39-passphrase", "--silent", "--outfile", str(key)])
        return run(["solana-keygen", "pubkey", str(key)]).strip(), key

    def send(name, accounts, data, signers=(), program=None, refused=None):
        argv = [str(args.hopper), "tx", "send", "--program", program or args.program,
                "--keypair", str(args.payer), "--rpc", RPC, "--data", data.hex()]
        for entry in accounts:
            argv += ["--account", entry]
        for signer in signers:
            argv += ["--signer", str(signer)]
        if refused is not None:
            argv += ["--allow-failure"]
        text = run(argv)
        (out / f"{name}.log").write_text(text, encoding="utf-8")
        match = re.search(r"^signature\s*:\s*(\w+)", text, re.MULTILINE)
        assert match is not None, "sender returned no signature"
        tx = finalize(match[1])
        write(f"{name}.transaction.json", tx)
        error = tx["meta"]["err"]
        record = {"name": name, "signature": match[1], "slot": tx["slot"], "error": error,
                  "computeUnits": tx["meta"].get("computeUnitsConsumed")}
        records.append(record)
        if refused is None:
            assert error is None, (name, error, tx["meta"]["logMessages"])
        else:
            assert error is not None, f"{name} must be refused"
        status = "ok" if error is None else f"refused {json.dumps(error)}"
        print(f"{name}: finalized, {record['computeUnits']} CU, {status}", flush=True)
        return tx, record

    def check_mint(address, slot, auto_approve):
        data = account(address, slot)
        b = data["bytes"]
        assert data["owner"] == TOKEN_2022, data["owner"]
        assert len(b) == MINT_LEN, len(b)
        assert struct.unpack_from("<I", b, 0)[0] == 1 and b[4:36] == pubkey_bytes(payer)
        assert b[44] == DECIMALS and b[45] == 1
        assert b[82:165] == bytes(83), "padding"
        assert b[165] == 1, "account type: mint"
        assert struct.unpack_from("<HH", b, 166) == (4, 65), "confidential-transfer mint TLV"
        assert b[170:202] == pubkey_bytes(payer), "confidential-transfer authority"
        assert b[202] == auto_approve, b[202]
        assert b[203:235] == AUDITOR, "auditor key"
        return data

    token_dump = out / "token-2022-devnet.so"
    run(["solana", "program", "dump", TOKEN_2022, str(token_dump), "--url", RPC,
         "--keypair", str(args.payer), "--commitment", "finalized"])
    token_elf = token_dump.read_bytes()
    release = re.search(rb"program@v[0-9.]+", token_elf)
    token_2022 = {"sha256": hashlib.sha256(token_elf).hexdigest(), "bytes": len(token_elf),
                  "sourceRelease": release[0].decode() if release else None}
    token_dump.unlink()

    deployment("before")

    mint, mint_key = new_key("mint")
    tx, record = send("create-mint", ["payer:sw", mint + ":sw", SYSTEM, TOKEN_2022],
                      bytes([TAG_CREATE_MINT, 0]) + AUDITOR, [mint_key])
    check_mint(mint, tx["slot"], 0)
    record["mintBytes"] = MINT_LEN

    tx, record = send("update-mint-auto-approve", [mint + ":w", "payer:s", TOKEN_2022],
                      bytes([TAG_UPDATE_MINT, 1]) + AUDITOR)
    check_mint(mint, tx["slot"], 1)

    # The proof program itself: what the cluster answers is recorded.
    tx, record = send("proof-program-call", [],
                      bytes([VERIFY_PUBKEY_VALIDITY]) + bytes(96),
                      program=ZK_PROOF_PROGRAM, refused=ANY_ERROR)
    record["logs"] = tx["meta"]["logMessages"]

    # Configuring without a verified proof: the context account is not the
    # proof program's, so Token-2022 refuses and the whole transaction,
    # account creation included, is rolled back.
    token_account, token_key = new_key("token-account")
    fake_context, _ = new_key("not-a-proof-context")
    data = (bytes([TAG_OPEN_ACCOUNT]) + struct.pack("<Q", 65_536) + bytes(36)
            + bytes([0]) + struct.pack("<H", 0))
    tx, record = send("configure-without-proof",
                      ["payer:sw", token_account + ":sw", mint, "payer:s", fake_context,
                       SYSTEM, TOKEN_2022],
                      data, [token_key], refused=ANY_ERROR)
    logs = tx["meta"]["logMessages"]
    assert "Program log: ConfidentialTransferInstruction::ConfigureAccount" in logs, logs
    assert account(token_account, tx["slot"]) is None, "a refused configure left an account"
    record.update(logs=logs, accountCreated=False)

    deployment("after")
    assert run(["git", "rev-parse", "HEAD"]).strip() == source
    assert not run(["git", "status", "--porcelain"]).strip(), "source changed during capture"
    write("receipt.json", {
        "schema": "hopper.confidential-lab-devnet.v1", "sourceCommit": source,
        "rpcEndpoint": RPC, "genesisHash": GENESIS, "commitment": "finalized",
        "programId": args.program, "elfSha256": hashlib.sha256(elf).hexdigest(),
        "deployedElfMatchesBeforeAndAfter": True, "token2022": token_2022,
        "accounts": {"mint": mint}, "transactions": records,
    })
    print("All confidential-lab devnet cases passed.", flush=True)


if __name__ == "__main__":
    main()
