#!/usr/bin/env python3
"""Run the framework-comparison fixtures on public devnet.

Deploys the four Hopper programs `scripts/bench-framework-comparison.py`
measures (hello world and the PDA counter, each on the raw layer and with
`#[program]`) as fresh programs, checks that each deployed program equals
the local ELF, and runs them: hello once, the counter's create once and its
increment twice, with every byte of the counter account checked after each
step. Records the compute units the cluster charged, and checks the deployed
programs again after the last transaction.

Build the ELFs first with `scripts/bench-framework-comparison.py` (they land
in `bench/framework-comparison/results/sbf`). Keypairs and output stay under
ignored target/. Only the explicit public devnet endpoint is used. This
spends devnet SOL on deployments and fees.
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

HELLO_LOG = "Program log: Hello, Solana!"
# name, ELF stem, kind, and for the counters the account layout the
# benchmark verifier checks: total size, discriminator at byte 0, the bump
# and the little-endian count at their offsets.
FIXTURES = [
    ("hello-raw", "hello_hopper_substrate", "hello", None),
    ("hello-framework", "hello_hopper", "hello", None),
    ("counter-raw", "counter_hopper_substrate", "counter", {"size": 10, "bump": 1, "count": 2}),
    ("counter-framework", "counter_hopper", "counter", {"size": 25, "bump": 16, "count": 17}),
]


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--payer", type=Path, required=True)
    parser.add_argument("--hopper", type=Path, required=True)
    parser.add_argument("--sbf-dir", type=Path, default=ROOT / "bench/framework-comparison/results/sbf")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    if not out.is_relative_to(ROOT / "target"):
        raise RuntimeError("output must stay under ignored target/")
    if rpc("getGenesisHash", []) != GENESIS:
        raise RuntimeError("public endpoint did not return the devnet genesis")
    if run(["git", "status", "--porcelain"]).strip():
        raise RuntimeError("commit source before capturing live evidence")
    source = run(["git", "rev-parse", "HEAD"]).strip()
    out.mkdir(parents=True, exist_ok=False)
    keys = out / "keys"
    keys.mkdir()
    payer = run(["solana-keygen", "pubkey", str(args.payer)]).strip()
    transactions: list[dict] = []
    checks: list[str] = []

    def write(name: str, value: object) -> None:
        (out / name).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")

    def send(name: str, program: str, accounts: list[str], data: bytes) -> dict:
        command = [str(args.hopper), "tx", "send", "--program", program,
                   "--keypair", str(args.payer), "--rpc", RPC, "--data", data.hex()]
        for account in accounts:
            command += ["--account", account]
        output = run(command)
        (out / f"{name}.log").write_text(output, encoding="utf-8")
        match = re.search(r"^signature\s*:\s*(\w+)", output, re.MULTILINE)
        if match is None:
            raise RuntimeError(f"{name}: the sender returned no signature")
        tx = finalize(match[1])
        write(f"{name}.transaction.json", tx)
        record = {"name": name, "signature": match[1], "slot": tx["slot"],
                  "error": tx["meta"]["err"], "computeUnits": tx["meta"].get("computeUnitsConsumed")}
        transactions.append(record)
        if record["error"] is not None:
            raise RuntimeError(f"{name} failed: {record['error']}")
        print(f"{name}: finalized, {record['computeUnits']} CU", flush=True)
        return tx

    # Deploy every fixture fresh and check the bytes on chain.
    programs: dict[str, dict] = {}
    for name, stem, _, _ in FIXTURES:
        elf_path = args.sbf_dir / f"{stem}.so"
        elf = elf_path.read_bytes()
        keypair = keys / f"{name}.json"
        run(["solana-keygen", "new", "--no-bip39-passphrase", "--silent", "--outfile", str(keypair)])
        program_id = run(["solana-keygen", "pubkey", str(keypair)]).strip()
        deploy = json.loads(run(["solana", "program", "deploy", str(elf_path), "--program-id", str(keypair),
                                 "--url", RPC, "--keypair", str(args.payer), "--commitment", "finalized",
                                 "--output", "json"]))
        write(f"deploy-{name}.json", deploy)
        programs[name] = {"id": program_id, "elf": stem + ".so", "bytes": len(elf), "elfSha256": sha256(elf),
                          "deploySignature": deploy.get("signature")}
        print(f"deployed {name}: {program_id}", flush=True)

    def verify_deployments(phase: str) -> None:
        for name, stem, _, _ in FIXTURES:
            program_id = programs[name]["id"]
            dump = out / f"{phase}-{name}-onchain.so"
            run(["solana", "program", "dump", program_id, str(dump), "--url", RPC,
                 "--keypair", str(args.payer), "--commitment", "finalized"])
            if dump.read_bytes() != (args.sbf_dir / f"{stem}.so").read_bytes():
                raise RuntimeError(f"{name}: the deployed program does not equal the local ELF")
        checks.append(f"{phase} the run: every deployed program equals its local ELF byte for byte")

    verify_deployments("before")

    for name, _, kind, layout in FIXTURES:
        program_id = programs[name]["id"]
        if kind == "hello":
            # One readonly signer, as the benchmark verifier passes. The raw
            # program ignores its data; the framework program dispatches on
            # discriminator 0.
            tx = send(name, program_id, ["payer:s"], b"\x00")
            if HELLO_LOG not in tx["meta"]["logMessages"]:
                raise RuntimeError(f"{name}: no hello log line")
            checks.append(f"{name}: logged `Hello, Solana!`")
            continue

        pda = json.loads(run(["solana", "find-program-derived-address", program_id, "string:counter",
                              f"pubkey:{payer}", "--url", RPC, "--output", "json"]))
        counter, bump = pda["address"], pda["bumpSeed"]
        programs[name]["counter"] = counter

        def state(slot: int) -> bytes:
            value = rpc("getMultipleAccounts", [[counter], {"encoding": "base64", "commitment": "finalized",
                                                           "minContextSlot": slot}])["value"][0]
            if value is None or value["owner"] != program_id:
                raise RuntimeError(f"{name}: the counter is missing or not owned by the program")
            return base64.b64decode(value["data"][0])

        def expect(data: bytes, count: int, step: str) -> None:
            if len(data) != layout["size"] or data[0] != 1:
                raise RuntimeError(f"{name} {step}: wrong size or discriminator")
            if data[layout["bump"]] != bump:
                raise RuntimeError(f"{name} {step}: wrong bump")
            if int.from_bytes(data[layout["count"]:layout["count"] + 8], "little") != count:
                raise RuntimeError(f"{name} {step}: wrong count")
            checks.append(f"{name} {step}: {layout['size']} bytes, discriminator 1, bump {bump}, count {count}")

        tx = send(f"{name}-create", program_id, ["payer:sw", f"{counter}:w", SYSTEM], bytes([0, bump]))
        expect(state(tx["slot"]), 0, "create")
        for n in (1, 2):
            tx = send(f"{name}-increment-{n}", program_id, ["payer:s", f"{counter}:w"], b"\x01")
            expect(state(tx["slot"]), n, f"increment {n}")

    verify_deployments("after")
    write("receipt.json", {
        "schema": "hopper.framework-fixtures-devnet.v1",
        "rpc": RPC,
        "sourceCommit": source,
        "payer": payer,
        "programs": programs,
        "transactions": transactions,
        "checks": checks,
    })
    print(f"{len(transactions)} transactions, every check passed", flush=True)


if __name__ == "__main__":
    main()
