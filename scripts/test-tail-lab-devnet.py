#!/usr/bin/env python3
"""Drive the hopper-tail-lab program on public devnet and verify every account
it writes byte by byte against a model kept here.

The note lane creates a note, fills its bounded reviewer list in place, tries
a fifth reviewer and a duplicate, rewrites the label and the body shorter and
longer, and tries a rewrite signed by someone else. The blob lane creates a
blob, rewrites it, and tries a tag the layout's `#[check]` rule refuses, both
on a write and at creation. Refusals are asserted with their exact error and
with the account unchanged.

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

HEADER = 16
NOTE_DISC, BLOB_DISC = 21, 22
NOTE_PREFIX = HEADER + 32 + 8
BLOB_PREFIX = HEADER + 32 + 8 + 1
NOTE_BODY_MAX = 160
BLOB_BYTES_MAX = 96
BLOB_TAG_MAX = 15
LABEL_MAX = 32
REVIEWERS_MAX = 4

TAG_INIT_NOTE, TAG_REWRITE_NOTE, TAG_ADD_REVIEWER, TAG_INIT_BLOB, TAG_WRITE_BLOB = range(5)
ERR_EMPTY_BODY, ERR_EMPTY_PAYLOAD, ERR_TAG_OUT_OF_RANGE = 6700, 6701, 6702
ANY_ERROR = object()


def bounded(payload: bytes) -> bytes:
    return struct.pack("<H", len(payload)) + payload


def note_tail(label: str, reviewers: list[str], body: str) -> bytes:
    fields = bounded(label.encode())
    fields += struct.pack("<H", len(reviewers)) + b"".join(pubkey_bytes(r) for r in reviewers)
    return fields + body.encode()


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

    def rent(size):
        return rpc("getMinimumBalanceForRentExemption", [size, {"commitment": "finalized"}])

    def send(name, accounts, data, signers=(), refused=None):
        argv = [str(args.hopper), "tx", "send", "--program", args.program,
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
        elif refused is ANY_ERROR:
            # The chain names the error; it is recorded, not predicted.
            assert error is not None, f"{name} must be refused"
        else:
            assert error == {"InstructionError": [0, refused]}, (name, error, refused)
        status = "ok" if error is None else f"refused {json.dumps(error)}"
        print(f"{name}: finalized, {record['computeUnits']} CU, {status}", flush=True)
        return tx, record

    def check_fixed(data, disc, authority, revision):
        b = data["bytes"]
        assert data["owner"] == args.program
        assert b[0] == disc and b[1] == 1, (b[0], b[1])
        assert b[HEADER:HEADER + 32] == pubkey_bytes(authority)
        assert struct.unpack_from("<Q", b, HEADER + 32)[0] == revision, revision
        return b

    def check_note(address, slot, revision, label, reviewers, body):
        data = account(address, slot)
        b = check_fixed(data, NOTE_DISC, payer, revision)
        expected = note_tail(label, reviewers, body)
        length = struct.unpack_from("<I", b, NOTE_PREFIX)[0]
        assert length == len(expected), (length, len(expected))
        tail = b[NOTE_PREFIX + 4:NOTE_PREFIX + 4 + length]
        assert tail == expected, "note tail differs from the model"
        return data

    def check_blob(address, slot, revision, tag, payload):
        data = account(address, slot)
        b = check_fixed(data, BLOB_DISC, payer, revision)
        assert b[HEADER + 40] == tag, (b[HEADER + 40], tag)
        length = struct.unpack_from("<I", b, BLOB_PREFIX)[0]
        assert length == len(payload), (length, len(payload))
        assert b[BLOB_PREFIX + 4:BLOB_PREFIX + 4 + length] == payload
        return data

    deployment("before")

    # ── Note lane ────────────────────────────────────────────────────
    note, note_key = new_key("note")
    label, body = "audit", "x" * NOTE_BODY_MAX
    reviewers = [payer]
    revision = 0
    tx, record = send("init-note", ["payer:sw", note + ":sw", SYSTEM],
                      bytes([TAG_INIT_NOTE]) + bounded(label.encode()) + bounded(body.encode()),
                      [note_key])
    created = check_note(note, tx["slot"], revision, label, reviewers, body)
    size = len(created["bytes"])
    assert created["lamports"] == rent(size), (created["lamports"], rent(size))
    record["accountBytes"] = size
    layout_id = created["bytes"][4:12].hex()

    note_accounts = ["payer:s", note + ":w"]
    added = []
    for index in range(REVIEWERS_MAX - 1):
        reviewer, _ = new_key(f"reviewer-{index}")
        tx, _ = send(f"add-reviewer-{index}", note_accounts,
                     bytes([TAG_ADD_REVIEWER]) + pubkey_bytes(reviewer))
        reviewers.append(reviewer)
        added.append(reviewer)
        revision += 1
        check_note(note, tx["slot"], revision, label, reviewers, body)

    # The list is full: a fifth reviewer is refused and nothing moves.
    before = account(note)["bytes"]
    extra, _ = new_key("reviewer-extra")
    tx, record = send("add-reviewer-when-full", note_accounts,
                      bytes([TAG_ADD_REVIEWER]) + pubkey_bytes(extra), refused=ANY_ERROR)
    assert account(note, tx["slot"])["bytes"] == before, "a refused edit changed the account"
    record["accountUnchanged"] = True

    # Shorter label, shorter body: the suffix moves down.
    label, body = "ops", "y" * (NOTE_BODY_MAX - 40)
    tx, _ = send("rewrite-note-shorter", note_accounts,
                 bytes([TAG_REWRITE_NOTE]) + bounded(label.encode()) + bounded(body.encode()))
    revision += 1
    check_note(note, tx["slot"], revision, label, reviewers, body)

    # Longest label, longest body: the suffix moves up to the account's end.
    label, body = "L" * LABEL_MAX, "z" * NOTE_BODY_MAX
    tx, _ = send("rewrite-note-longest", note_accounts,
                 bytes([TAG_REWRITE_NOTE]) + bounded(label.encode()) + bounded(body.encode()))
    revision += 1
    check_note(note, tx["slot"], revision, label, reviewers, body)

    # Multi-byte text survives in-place moves.
    label, body = "café €", "\U0001f980 tail " * 8
    tx, _ = send("rewrite-note-utf8", note_accounts,
                 bytes([TAG_REWRITE_NOTE]) + bounded(label.encode()) + bounded(body.encode()))
    revision += 1
    check_note(note, tx["slot"], revision, label, reviewers, body)

    before = account(note)["bytes"]
    tx, record = send("rewrite-note-empty-body", note_accounts,
                      bytes([TAG_REWRITE_NOTE]) + bounded(b"x") + bounded(b""),
                      refused={"Custom": ERR_EMPTY_BODY})
    assert account(note, tx["slot"])["bytes"] == before
    record["accountUnchanged"] = True

    # Someone else signs: `has_one = authority` refuses.
    stranger, stranger_key = new_key("stranger")
    tx, record = send("rewrite-note-by-stranger", [stranger + ":s", note + ":w"],
                      bytes([TAG_REWRITE_NOTE]) + bounded(b"mine") + bounded(b"taken"),
                      [stranger_key], refused=ANY_ERROR)
    assert account(note, tx["slot"])["bytes"] == before
    record["accountUnchanged"] = True

    # ── Blob lane ────────────────────────────────────────────────────
    blob, blob_key = new_key("blob")
    payload = bytes([0, 1, 2, 0xFF])
    tx, record = send("init-blob", ["payer:sw", blob + ":sw", SYSTEM],
                      bytes([TAG_INIT_BLOB, 7]) + bounded(payload), [blob_key])
    created = check_blob(blob, tx["slot"], 0, 7, payload)
    assert created["lamports"] == rent(len(created["bytes"]))
    record["accountBytes"] = len(created["bytes"])

    blob_accounts = ["payer:s", blob + ":w"]
    payload = bytes(range(BLOB_BYTES_MAX))
    tx, _ = send("write-blob-longest", blob_accounts,
                 bytes([TAG_WRITE_BLOB, BLOB_TAG_MAX]) + bounded(payload))
    check_blob(blob, tx["slot"], 1, BLOB_TAG_MAX, payload)

    # The layout's rule refuses the tag; revision, tag and payload stay.
    before = account(blob)["bytes"]
    tx, record = send("write-blob-tag-out-of-range", blob_accounts,
                      bytes([TAG_WRITE_BLOB, BLOB_TAG_MAX + 1]) + bounded(b"\x01"),
                      refused={"Custom": ERR_TAG_OUT_OF_RANGE})
    assert account(blob, tx["slot"])["bytes"] == before
    record["accountUnchanged"] = True

    tx, record = send("write-blob-empty", blob_accounts,
                      bytes([TAG_WRITE_BLOB, 1]) + bounded(b""),
                      refused={"Custom": ERR_EMPTY_PAYLOAD})
    assert account(blob, tx["slot"])["bytes"] == before
    record["accountUnchanged"] = True

    # A blob cannot be created with a tag the rule refuses.
    bad, bad_key = new_key("blob-bad-tag")
    tx, record = send("init-blob-tag-out-of-range", ["payer:sw", bad + ":sw", SYSTEM],
                      bytes([TAG_INIT_BLOB, 200]) + bounded(b"\x01"), [bad_key],
                      refused={"Custom": ERR_TAG_OUT_OF_RANGE})
    assert account(bad, tx["slot"]) is None, "a refused creation left an account"
    record["accountCreated"] = False

    deployment("after")
    assert run(["git", "rev-parse", "HEAD"]).strip() == source
    assert not run(["git", "status", "--porcelain"]).strip(), "source changed during capture"
    write("receipt.json", {"schema": "hopper.tail-lab-devnet.v1", "sourceCommit": source,
                          "rpcEndpoint": RPC, "genesisHash": GENESIS, "commitment": "finalized",
                          "programId": args.program, "elfSha256": hashlib.sha256(elf).hexdigest(),
                          "deployedElfMatchesBeforeAndAfter": True,
                          "accounts": {"note": note, "blob": blob},
                          "noteLayoutId": layout_id, "transactions": records})
    print("All tail-lab devnet cases and exact state checks passed.", flush=True)


if __name__ == "__main__":
    main()
