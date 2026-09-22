#!/usr/bin/env python3
"""Exercise an already deployed runtime-gate fixture on public devnet.

Creates two 32-byte test accounts, then submits positive and deliberately
rejected instructions. Keypairs stay in the ignored output directory.
No global Solana configuration is read or changed.
"""
from __future__ import annotations

import argparse
import base64
import copy
import hashlib
import json
import re
import struct
import subprocess
import time
import urllib.request
from pathlib import Path

RPC = "https://api.devnet.solana.com"
GENESIS = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"
SYSTEM = "11111111111111111111111111111111"
REPO = Path(__file__).resolve().parents[1]


def run(argv: list[str]) -> str:
    result = subprocess.run(argv, cwd=REPO, capture_output=True, text=True, encoding="utf-8")
    if result.returncode:
        # Arguments can include keypair paths; do not echo them.
        raise RuntimeError(f"{Path(argv[0]).name} failed: {result.stderr}")
    return result.stdout


def rpc(method: str, params: list[object]) -> object:
    payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    request = urllib.request.Request(RPC, payload, {"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=45) as response:
        result = json.load(response)
    if "error" in result:
        raise RuntimeError(f"{method}: {result['error']}")
    return result["result"]


def finalize(signature: str) -> dict:
    for _ in range(90):
        tx = rpc("getTransaction", [signature, {"commitment": "finalized", "maxSupportedTransactionVersion": 1}])
        if tx is not None:
            return tx
        time.sleep(2)
    raise RuntimeError(f"transaction not finalized: {signature}; inspect before retrying")


def snapshot(addresses: list[str], slot: int) -> list[dict]:
    result = rpc("getMultipleAccounts", [addresses, {"encoding": "base64", "commitment": "finalized", "minContextSlot": slot}])
    values = result["value"]
    if any(value is None for value in values):
        raise RuntimeError("test account missing")
    return values


def digest(value: object) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def pubkey_bytes(address: str) -> bytes:
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    value = 0
    for char in address:
        value = value * 58 + alphabet.index(char)
    return value.to_bytes(32, "big")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--program", required=True)
    parser.add_argument("--payer", type=Path, required=True)
    parser.add_argument("--hopper", type=Path, required=True)
    parser.add_argument("--elf", type=Path, required=True)
    parser.add_argument("--out", type=Path, default=REPO / "target/hopper/runtime-gate-devnet")
    args = parser.parse_args()
    args.out = args.out.resolve()
    if not args.out.is_relative_to(REPO / "target"):
        raise RuntimeError("output must stay under the ignored repository target directory")
    if rpc("getGenesisHash", []) != GENESIS:
        raise RuntimeError("public endpoint did not return the devnet genesis")
    if run(["git", "status", "--porcelain"]).strip():
        raise RuntimeError("commit source before capturing live evidence")
    source_commit = run(["git", "rev-parse", "HEAD"]).strip()
    program = rpc("getAccountInfo", [args.program, {"commitment": "finalized"}])["value"]
    if not program or not program["executable"]:
        raise RuntimeError("fixture is not deployed")
    args.out.mkdir(parents=True, exist_ok=False)
    elf = args.elf.read_bytes()

    def verify_deployment(phase: str) -> None:
        dump = args.out / f"{phase}-onchain.so"
        run(["solana", "program", "dump", args.program, str(dump), "--url", RPC,
             "--keypair", str(args.payer), "--commitment", "finalized"])
        if dump.read_bytes() != elf:
            raise RuntimeError("deployed program does not match the local fixture ELF")

    verify_deployment("before")
    keys = args.out / "keys"
    keys.mkdir()
    transactions = []

    def send(name: str, program_id: str, accounts: list[str], data: bytes,
             signers: list[Path] | None = None, allow_failure: bool = False) -> dict:
        command = [str(args.hopper), "tx", "send", "--program", program_id,
                   "--keypair", str(args.payer), "--rpc", RPC, "--data", data.hex()]
        for account in accounts:
            command += ["--account", account]
        for signer in signers or []:
            command += ["--signer", str(signer)]
        if allow_failure:
            command += ["--allow-failure"]
        output = run(command)
        (args.out / f"{name}.log").write_text(output, encoding="utf-8")
        match = re.search(r"^signature\s*:\s*(\w+)", output, re.MULTILINE)
        if match is None:
            raise RuntimeError("sender returned no signature")
        tx = finalize(match[1])
        (args.out / f"{name}.transaction.json").write_text(json.dumps(tx, indent=2) + "\n", encoding="utf-8")
        record = {"name": name, "signature": match[1], "slot": tx["slot"],
                  "error": tx["meta"]["err"], "computeUnits": tx["meta"].get("computeUnitsConsumed")}
        transactions.append(record)
        return record

    addresses = []
    rent = rpc("getMinimumBalanceForRentExemption", [32])
    for name in ["state", "foreign"]:
        key = keys / f"{name}.json"
        run(["solana-keygen", "new", "--no-bip39-passphrase", "--silent", "--outfile", str(key)])
        address = run(["solana-keygen", "pubkey", str(key)]).strip()
        addresses.append(address)
        created = send(f"create-{name}", SYSTEM, ["payer:sw", f"{address}:sw"],
                       struct.pack("<IQQ", 0, rent, 32) + pubkey_bytes(args.program), [key])
        if created["error"] is not None:
            raise RuntimeError("test account creation failed")
        print(f"created {name}: {address}", flush=True)

    for case in [1, 2, 3, 4, 5, 6, 7, 8, 9, 0]:
        before = snapshot(addresses, transactions[-1]["slot"])
        expected = copy.deepcopy(before)
        code = 0xD000 if case in [2, 6, 7] else 0xD0FF if case in [3, 8, 9] else None
        record = send(f"case-{case}", args.program, [f"{a}:w" for a in addresses], bytes([case]),
                      allow_failure=code is not None)
        expected_error = {"InstructionError": [0, {"Custom": code}]} if code is not None else None
        if record["error"] != expected_error:
            raise RuntimeError(f"case {case}: unexpected program result {record['error']}")
        if code is None:
            edits = {0: [(0, 0, 32)], 1: [(0, 8, 16)], 4: [(0, 8, 16), (1, 0, 32)], 5: [(0, 16, 24)]}[case]
            for index, start, end in edits:
                data = bytearray(base64.b64decode(expected[index]["data"][0]))
                data[start:end] = bytes([7]) * (end - start)
                expected[index]["data"][0] = base64.b64encode(data).decode()
        after = snapshot(addresses, record["slot"])
        if after != expected:
            raise RuntimeError(f"case {case}: unexpected account mutation")
        record.update({"preSnapshotSha256": digest(before), "postSnapshotSha256": digest(after),
                       "expectedStateVerified": True})
        print(f"case {case}: finalized, {record['computeUnits']} CU, exact state verified", flush=True)

    verify_deployment("after")
    if run(["git", "rev-parse", "HEAD"]).strip() != source_commit or run(["git", "status", "--porcelain"]).strip():
        raise RuntimeError("source changed during evidence capture")
    receipt = {"schema": "hopper.runtime-gate-devnet.v1", "sourceCommit": source_commit,
               "rpcEndpoint": RPC, "genesisHash": GENESIS, "programId": args.program,
               "elfSha256": hashlib.sha256(elf).hexdigest(), "deployedElfMatchesBeforeAndAfter": True,
               "accounts": addresses, "commitment": "finalized", "transactions": transactions}
    (args.out / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print("all runtime gate devnet cases passed", flush=True)


if __name__ == "__main__":
    main()
