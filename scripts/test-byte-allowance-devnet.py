#!/usr/bin/env python3
"""Test an exact allowance ELF on public devnet; keys stay under ignored target/."""
from __future__ import annotations
import argparse
import base64
import copy
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
LIMITS, SPENT, REVISIONS, LENGTH = 176, 208, 240, 272
RENT_EXEMPT_EPOCH = (1 << 64) - 1


def fresh_account(owner, lamports, data):
    """Exact public-RPC representation of a new rent-exempt data account."""
    return dict(data=[base64.b64encode(data).decode(), "base64"], executable=False,
                lamports=lamports, owner=owner, rentEpoch=RENT_EXEMPT_EPOCH, space=len(data))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("program", "payer", "hopper", "elf", "out", "header-hex"):
        parser.add_argument("--" + name, required=True)
    args = parser.parse_args()
    assert re.fullmatch(r"[0-9a-fA-F]{32}", args.header_hex), "--header-hex must be the 16-byte header printed by the compiled suite"
    expected_header = bytes.fromhex(args.header_hex)
    assert expected_header[:2] == bytes([92, 1]), "unexpected allowance discriminator/version"
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
        dump = out / f"{phase}-onchain.so"
        run(["solana", "program", "dump", args.program, str(dump), "--url", RPC,
             "--keypair", args.payer, "--commitment", "finalized"])
        assert dump.read_bytes() == elf, "deployed ELF differs"

    def key(name):
        path = out / "keys" / (name + ".json")
        run(["solana-keygen", "new", "--no-bip39-passphrase", "--silent", "--outfile", str(path)])
        return run(["solana-keygen", "pubkey", str(path)]).strip(), path

    def send(name, program, metas, data, signers=(), error=None):
        command = [args.hopper, "tx", "send", "--program", program, "--keypair", args.payer,
                   "--rpc", RPC, "--data", data.hex()]
        for meta in metas:
            command += ["--account", meta]
        for signer in signers:
            command += ["--signer", str(signer)]
        if error is not None:
            command += ["--allow-failure"]
        text = run(command)
        (out / (name + ".log")).write_text(text, encoding="utf-8")
        match = re.search(r"^signature\s*:\s*(\w+)", text, re.MULTILINE)
        assert match, "no signature"
        tx = finalize(match[1])
        write(name + ".transaction.json", tx)
        actual_error = tx["meta"]["err"]
        if error == "any":
            assert actual_error is not None, name
        else:
            assert actual_error == (None if error is None else {"InstructionError": [0, error]}), (name, actual_error)
        record = dict(name=name, signature=match[1], slot=tx["slot"], error=actual_error,
                      computeUnits=tx["meta"].get("computeUnitsConsumed"), fee=tx["meta"]["fee"])
        records.append(record)
        return record

    book, book_key = key("book")
    delegate0, delegate0_key = key("delegate0")
    delegate1, delegate1_key = key("delegate1")
    addresses = [payer, book, delegate0, delegate1, SYSTEM]
    assert len(set(addresses)) == len(addresses)
    delegates = [delegate0, delegate1, delegate0, delegate1]
    signing_keys = {delegate0: delegate0_key, delegate1: delegate1_key, book: book_key}

    def snapshot(slot=0):
        return rpc("getMultipleAccounts", [addresses, {"encoding": "base64", "commitment": "finalized", "minContextSlot": slot}])["value"]

    def verify_snapshot(name, before, expected, record):
        after = snapshot(record["slot"])
        assert after == expected, (name, "unexpected account state")
        write(name + ".snapshots.json", dict(addresses=addresses, before=before, expected=expected, after=after))
        record["expectedStateVerified"] = True
        print(f"{name}: finalized, {record['computeUnits']} CU, complete account state verified", flush=True)

    def refuse(name, metas, data, signers=(), error="any"):
        before = snapshot(records[-1]["slot"])
        record = send(name, args.program, metas, data, signers, error)
        expected = copy.deepcopy(before)
        expected[0]["lamports"] -= record["fee"]
        verify_snapshot(name, before, expected, record)

    deployment("before")
    # Two distinct delegates prove that a signer authorized for a neighboring
    # cell cannot consume this cell. Include System Program in every snapshot.
    for index, delegate in [(2, delegate0), (3, delegate1)]:
        before = snapshot(records[-1]["slot"] if records else 0)
        assert before[index] is None, "new delegate unexpectedly exists"
        record = send(f"fund-delegate{index - 2}", SYSTEM, ["payer:sw", f"{delegate}:w"], struct.pack("<IQ", 2, 1_000_000))
        expected = copy.deepcopy(before)
        expected[0]["lamports"] -= 1_000_000 + record["fee"]
        expected[index] = fresh_account(SYSTEM, 1_000_000, b"")
        verify_snapshot(record["name"], before, expected, record)

    init_data = b"\0" + b"".join(pubkey_bytes(delegate) for delegate in delegates) + struct.pack("<Q", 100)
    init_metas = ["payer:sw", f"{book}:sw", f"{SYSTEM}:r"]
    # All of these must leave the absent book absent and refund any attempted
    # rent debit, with the network transaction fee as the sole state change.
    refuse("init-unsigned-authority", [f"{delegate0}:w", f"{book}:sw", f"{SYSTEM}:r"], init_data, [book_key], "MissingRequiredSignature")
    refuse("init-unsigned-book", ["payer:sw", f"{book}:w", f"{SYSTEM}:r"], init_data, error="MissingRequiredSignature")
    refuse("init-readonly-book", ["payer:sw", f"{book}:s", f"{SYSTEM}:r"], init_data, [book_key])
    refuse("init-wrong-system", ["payer:sw", f"{book}:sw", f"{delegate1}:r"], init_data, [book_key])
    refuse("init-payer-book-alias", ["payer:sw", "payer:sw", f"{SYSTEM}:r"], init_data)
    refuse("init-truncated", init_metas, init_data[:-1], [book_key], "InvalidInstructionData")
    refuse("init-trailing", init_metas, init_data + b"\0", [book_key], "InvalidInstructionData")

    before = snapshot(records[-1]["slot"])
    assert before[1] is None
    rent = rpc("getMinimumBalanceForRentExemption", [LENGTH])
    # The bounded entrypoint ignores surplus metas; sealed does not promise an
    # exact account-count check. Prove the unused account stays unchanged.
    record = send("initialize", args.program, init_metas + [f"{delegate0}:r"], init_data, [book_key])
    payload = (expected_header + pubkey_bytes(payer)
               + b"".join(pubkey_bytes(delegate) for delegate in delegates)
               + struct.pack("<Q", 100) * 4 + bytes(64))
    assert len(payload) == LENGTH
    expected = copy.deepcopy(before)
    expected[0]["lamports"] -= rent + record["fee"]
    expected[1] = fresh_account(args.program, rent, payload)
    verify_snapshot("initialize", before, expected, record)

    cases = []
    for slot in range(4):
        delegate = delegates[slot]
        cases += [(f"consume-{slot}", 1, slot, 0, 40, delegate, None, [(SPENT, 40), (REVISIONS, 1)]),
                  (f"limit-{slot}", 2, slot, 1, 80, payer, None, [(LIMITS, 80), (REVISIONS, 2)]),
                  (f"exhaust-{slot}", 1, slot, 2, 40, delegate, None, [(SPENT, 80), (REVISIONS, 3)])]
    cases += [
        ("wrong-delegate", 1, 0, 3, 1, payer, {"Custom": 7800}, []),
        ("neighbor-delegate-at-zero", 1, 0, 3, 1, delegate1, {"Custom": 7800}, []),
        ("neighbor-delegate-at-one", 1, 1, 3, 1, delegate0, {"Custom": 7800}, []),
        ("stale-revision", 1, 0, 2, 1, delegate0, {"Custom": 7801}, []),
        ("limit-exceeded", 1, 0, 3, 1, delegate0, {"Custom": 7802}, []),
        ("zero-amount", 1, 0, 3, 0, delegate0, {"Custom": 7803}, []),
        ("out-of-bounds", 1, 4, 0, 1, delegate0, "InvalidInstructionData", []),
        ("max-selector", 1, 65535, 0, 1, delegate0, "InvalidInstructionData", []),
        ("wrong-admin", 2, 0, 3, 90, delegate0, "any", []),
        ("stale-admin-revision", 2, 0, 2, 90, payer, {"Custom": 7801}, []),
        ("limit-below-spent", 2, 0, 3, 79, payer, {"Custom": 7804}, []),
    ]
    for name, op, slot, revision, amount, signer, error, edits in cases:
        before = snapshot(records[-1]["slot"])
        record = send(name, args.program, [f"{signer}:s", f"{book}:w"],
                      struct.pack("<B H Q Q", op, slot, revision, amount),
                      [signing_keys[signer]] if signer != payer else [], error)
        expected = copy.deepcopy(before)
        expected[0]["lamports"] -= record["fee"]
        payload = bytearray(base64.b64decode(expected[1]["data"][0]))
        for column, value in edits:
            struct.pack_into("<Q", payload, column + slot * 8, value)
        expected[1]["data"][0] = base64.b64encode(payload).decode()
        verify_snapshot(name, before, expected, record)

    consume = struct.pack("<B H Q Q", 1, 0, 3, 1)
    refuse("consume-unsigned-delegate", [f"{delegate0}:r", f"{book}:w"], consume, error="MissingRequiredSignature")
    refuse("consume-readonly-book", [f"{delegate0}:s", f"{book}:r"], consume, [delegate0_key])
    refuse("consume-delegate-book-alias", [f"{book}:s", f"{book}:w"], consume, [book_key])
    refuse("consume-truncated", [f"{delegate0}:s", f"{book}:w"], consume[:-1], [delegate0_key], "InvalidInstructionData")
    refuse("consume-trailing", [f"{delegate0}:s", f"{book}:w"], consume + b"\0", [delegate0_key], "InvalidInstructionData")
    before = snapshot(records[-1]["slot"])
    record = send("set-limit-extra-account", args.program,
                  ["payer:s", f"{book}:w", f"{delegate1}:r"],
                  struct.pack("<B H Q Q", 2, 0, 3, 100))
    expected = copy.deepcopy(before)
    expected[0]["lamports"] -= record["fee"]
    payload = bytearray(base64.b64decode(expected[1]["data"][0]))
    struct.pack_into("<Q", payload, LIMITS, 100)
    struct.pack_into("<Q", payload, REVISIONS, 4)
    expected[1]["data"][0] = base64.b64encode(payload).decode()
    verify_snapshot("set-limit-extra-account", before, expected, record)
    refuse("reinitialize", init_metas, init_data, [book_key])
    deployment("after")
    assert run(["git", "rev-parse", "HEAD"]).strip() == source
    assert not run(["git", "status", "--porcelain"]).strip()
    write("receipt.json", dict(schema="hopper.byte-allowance-devnet.v1", sourceCommit=source,
        rpcEndpoint=RPC, genesisHash=GENESIS, programId=args.program, commitment="finalized",
        elfSha256=hashlib.sha256(elf).hexdigest(), expectedHeaderHex=expected_header.hex(),
        deployedElfMatchesBeforeAndAfter=True, accounts=addresses, cellDelegates=delegates,
        transactions=records))
    print("allowance lifecycle and refusal cases passed", flush=True)


if __name__ == "__main__":
    main()
