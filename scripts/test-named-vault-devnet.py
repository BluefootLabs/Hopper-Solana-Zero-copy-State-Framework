#!/usr/bin/env python3
"""Verify an exact SOL-vault ELF and complete account state on public devnet."""
from pathlib import Path
import argparse, base64, copy, hashlib, json, re, runpy, struct

ROOT = Path(__file__).resolve().parents[1]
H = runpy.run_path(str(ROOT / "scripts/test-runtime-gate-devnet.py"))
run, rpc, finalize = (H[k] for k in ("run", "rpc", "finalize"))
RPC, GENESIS, SYSTEM = (H[k] for k in ("RPC", "GENESIS", "SYSTEM"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("program", "payer", "hopper", "elf", "out", "header-hex"):
        parser.add_argument("--" + name, required=True)
    args = parser.parse_args()
    header = bytes.fromhex(args.header_hex)
    assert len(header) == 16 and header[:2] == b"\x01\x01"
    out = Path(args.out).resolve()
    assert out.is_relative_to(ROOT / "target")
    assert rpc("getGenesisHash", []) == GENESIS
    assert not run(["git", "status", "--porcelain"]).strip(), "commit source first"
    source = run(["git", "rev-parse", "HEAD"]).strip()
    out.mkdir(parents=True, exist_ok=False)
    (out / "keys").mkdir()
    payer = run(["solana-keygen", "pubkey", args.payer]).strip()
    elf = Path(args.elf).read_bytes()
    records = []

    def write(name, value):
        (out / name).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")

    def deployment(phase):
        dump = out / (phase + "-onchain.so")
        run(["solana", "program", "dump", args.program, str(dump), "--url", RPC,
             "--keypair", args.payer, "--commitment", "finalized"])
        assert dump.read_bytes() == elf, "deployed ELF differs"

    def key(name):
        path = out / "keys" / (name + ".json")
        run(["solana-keygen", "new", "--no-bip39-passphrase", "--silent", "--outfile", str(path)])
        return run(["solana-keygen", "pubkey", str(path)]).strip(), path

    authority, authority_key = key("authority")
    vault, vault_key = key("vault")
    prefunded, prefunded_key = key("prefunded")
    outsider, outsider_key = key("outsider")
    addresses = [payer, authority, vault, prefunded, outsider, SYSTEM]
    assert len(set(addresses)) == len(addresses)

    def snapshot(slot=0):
        return rpc("getMultipleAccounts", [addresses, dict(encoding="base64", commitment="finalized", minContextSlot=slot)])["value"]

    def send(name, program, metas, data, signers=(), error=None):
        command = [args.hopper, "tx", "send", "--program", program, "--keypair", args.payer,
                   "--rpc", RPC, "--data", data.hex()]
        for meta in metas:
            command += ["--account", meta]
        for signer in signers:
            command += ["--signer", str(signer)]
        if error is not None:
            command += ["--allow-failure"]
        output = run(command)
        (out / (name + ".log")).write_text(output, encoding="utf-8")
        match = re.search(r"^signature\s*:\s*(\w+)", output, re.MULTILINE)
        assert match, "no signature; inspect before retrying"
        tx = finalize(match[1])
        write(name + ".transaction.json", tx)
        actual = tx["meta"]["err"]
        assert actual == (None if error is None else {"InstructionError": [0, error]}), (name, actual, error)
        record = dict(name=name, signature=match[1], slot=tx["slot"], error=actual,
                      computeUnits=tx["meta"].get("computeUnitsConsumed"), fee=tx["meta"]["fee"])
        records.append(record)
        return record

    def verify(name, before, expected, record):
        expected[0]["lamports"] -= record["fee"]
        after = snapshot(record["slot"])
        assert after == expected, (name, "unexpected account state")
        write(name + ".snapshots.json", dict(addresses=addresses, before=before, expected=expected, after=after))
        record["expectedStateVerified"] = True
        write("progress.json", records)
        print(f"{name}: finalized, {record['computeUnits']} CU, exact account state verified", flush=True)

    def fresh(owner, lamports, data):
        return dict(owner=owner, lamports=lamports, data=[base64.b64encode(data).decode(), "base64"],
                    executable=False, rentEpoch=(1 << 64) - 1, space=len(data))

    def refusal(name, metas, data, signers=(), error=None):
        assert error is not None
        before = snapshot(records[-1]["slot"])
        record = send(name, args.program, metas, data, signers, error)
        verify(name, before, copy.deepcopy(before), record)

    deployment("before")
    for name, index, amount in [("fund-authority", 1, 10_000_000), ("fund-outsider", 4, 1_000_000), ("prefund-vault", 3, 1_000_000)]:
        before = snapshot(records[-1]["slot"] if records else 0)
        assert before[index] is None
        record = send(name, SYSTEM, ["payer:sw", addresses[index] + ":w"], struct.pack("<IQ", 2, amount))
        expected = copy.deepcopy(before)
        expected[0]["lamports"] -= amount
        expected[index] = fresh(SYSTEM, amount, b"")
        verify(name, before, expected, record)

    rent = rpc("getMinimumBalanceForRentExemption", [57, {"commitment": "finalized"}])
    for name, index, account_key in [("initialize", 2, vault_key), ("initialize-prefunded", 3, prefunded_key)]:
        before = snapshot(records[-1]["slot"])
        record = send(name, args.program, [authority + ":sw", addresses[index] + ":sw", SYSTEM], b"\0", [authority_key, account_key])
        expected = copy.deepcopy(before)
        prefund = before[index]["lamports"] if before[index] else 0
        topup = max(0, rent - prefund)
        expected[1]["lamports"] -= topup
        data = header + H["pubkey_bytes"](authority) + bytes(9)
        expected[index] = fresh(args.program, prefund + topup, data)
        verify(name, before, expected, record)

    init_metas = [authority + ":sw", vault + ":sw", SYSTEM]
    refusal("reinitialize", init_metas, b"\0", [authority_key, vault_key], "AccountAlreadyInitialized")
    for op, name in [(1, "deposit"), (2, "withdraw")]:
        metas = [authority + ":sw", vault + ":w"] + ([SYSTEM] if op == 1 else [])
        zero = bytes([op]) + struct.pack("<Q", 0)
        one = bytes([op]) + struct.pack("<Q", 1)
        refusal(name + "-zero", metas, zero, [authority_key], {"Custom": 6002})
        refusal(name + "-unsigned", [authority + ":w"] + metas[1:], one, [], "MissingRequiredSignature")
        refusal(name + "-readonly", [metas[0], vault] + metas[2:], one, [authority_key], "Immutable")
        refusal(name + "-wrong-authority", [outsider + ":sw"] + metas[1:], one, [outsider_key], "InvalidAccountData")
        if op == 2:
            refusal("withdraw-over-balance", metas, bytes([op]) + struct.pack("<Q", 1001), [authority_key], {"Custom": 6001})
        before = snapshot(records[-1]["slot"])
        record = send(name, args.program, metas, bytes([op]) + struct.pack("<Q", 1000), [authority_key])
        expected = copy.deepcopy(before)
        delta = 1000 if op == 1 else -1000
        expected[1]["lamports"] -= delta
        expected[2]["lamports"] += delta
        data = bytearray(base64.b64decode(expected[2]["data"][0]))
        data[48:56] = struct.pack("<Q", 1000 if op == 1 else 0)
        expected[2]["data"][0] = base64.b64encode(data).decode()
        verify(name, before, expected, record)

    refusal("deposit-insufficient-funds", [authority + ":sw", vault + ":w", SYSTEM], b"\x01" + struct.pack("<Q", 1_000_000_000), [authority_key], {"Custom": 1})
    refusal("deposit-missing-system", [authority + ":sw", vault + ":w"], b"\x01" + struct.pack("<Q", 1), [authority_key], "NotEnoughAccountKeys")
    deployment("after")
    assert run(["git", "rev-parse", "HEAD"]).strip() == source and not run(["git", "status", "--porcelain"]).strip()
    write("receipt.json", dict(schema="hopper.named-vault-devnet.v1", sourceCommit=source,
          genesisHash=GENESIS, programId=args.program, elfSha256=hashlib.sha256(elf).hexdigest(),
          deployedElfMatchesBeforeAndAfter=True, sourceUnchangedAndClean=True,
          addresses=addresses, transactions=records, allPassed=True))


if __name__ == "__main__":
    main()
