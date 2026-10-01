#!/usr/bin/env python3
"""Drive the hopper-token-lab program on public devnet against SPL Token and
Token-2022 and verify every account it touches byte by byte.

One lane per token program: create a mint, two immutable-owner accounts (sized
by GetAccountDataSize), mint supply, a batched there-and-back transfer, the
UI-amount round trip, an excess-lamport withdrawal, and a multisig. SPL Token
also gets the wrapped-SOL unwrap; Token-2022 also gets the extended mint
(the fixed-size extensions selected by this lab), pause/resume, a scaled-UI
multiplier update, a mint that carries its own metadata (initialize, set and
replace a key, rename, remove, give up the authority, emit), and a token group
with a member. Cases the live program refuses are recorded, not hidden.

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
LEGACY = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
TOKEN_2022 = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
NATIVE_MINT = "So11111111111111111111111111111111111111112"
DECIMALS = 6
SUPPLY = 1_000_000
BATCH_AMOUNT = 250_000
UI_AMOUNT = 1_234_567
PREFUND = 10_000
WRAP = 1_000_000
UNWRAP = 400_000
# (mask bit, TLV type, value length) in the order the program pushes them.
EXTENSIONS = [(1, 6, 1), (2, 26, 33), (4, 25, 56), (8, 10, 52), (16, 20, 64), (32, 28, 32),
              (64, 18, 64), (128, 22, 64)]
EXT_IMMUTABLE_OWNER = 7
EXT_PAUSABLE_ACCOUNT = 27

TAG_CREATE_MINT = 0
TAG_CREATE_EXTENDED_MINT = 1
TAG_IMMUTABLE_ACCOUNT = 2
TAG_MINT_TO = 3
TAG_BATCH_ROUND_TRIP = 4
TAG_UI_AMOUNT_ROUND_TRIP = 5
TAG_WITHDRAW_EXCESS = 6
TAG_INIT_MULTISIG = 7
TAG_WRAP_AND_UNWRAP = 8
TAG_PAUSE_RESUME = 9
TAG_UPDATE_MULTIPLIER = 10
TAG_CREATE_METADATA_MINT = 11
TAG_SET_METADATA_KEY = 12
TAG_FINALIZE_METADATA = 13
TAG_CREATE_GROUP = 14
TAG_CREATE_GROUP_MEMBER = 15
TAG_BATCH_SELF_TRANSFER = 16
TAG_RESOLVE_HOOK_LIST = 17
EXT_METADATA_POINTER, EXT_METADATA = 18, 19
EXT_GROUP_POINTER, EXT_GROUP = 20, 21
EXT_GROUP_MEMBER_POINTER, EXT_GROUP_MEMBER = 22, 23
GROUP_MAX_SIZE = 3


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
    findings = {}

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

    def send(name, program, accounts, data, signers=(), allow_failure=False):
        argv = [str(args.hopper), "tx", "send", "--program", program,
                "--keypair", str(args.payer), "--rpc", RPC, "--data", data.hex()]
        for entry in accounts:
            argv += ["--account", entry]
        for signer in signers:
            argv += ["--signer", str(signer)]
        if allow_failure:
            argv += ["--allow-failure"]
        text = run(argv)
        (out / f"{name}.log").write_text(text, encoding="utf-8")
        match = re.search(r"^signature\s*:\s*(\w+)", text, re.MULTILINE)
        assert match is not None, "sender returned no signature"
        tx = finalize(match[1])
        write(f"{name}.transaction.json", tx)
        record = {"name": name, "signature": match[1], "slot": tx["slot"],
                  "error": tx["meta"]["err"], "computeUnits": tx["meta"].get("computeUnitsConsumed")}
        records.append(record)
        if not allow_failure:
            assert tx["meta"]["err"] is None, (name, tx["meta"]["err"], tx["meta"]["logMessages"])
        status = "ok" if tx["meta"]["err"] is None else f"refused {tx['meta']['err']}"
        print(f"{name}: finalized, {record['computeUnits']} CU, {status}", flush=True)
        return tx, record

    def return_data(tx):
        returned = tx["meta"]["returnData"]
        assert returned["programId"] == args.program, returned
        return base64.b64decode(returned["data"][0])

    def tlv_entries(data, start=166):
        entries = []
        at = start
        while at + 4 <= len(data):
            kind, length = struct.unpack_from("<HH", data, at)
            if kind == 0:
                break
            entries.append((kind, length))
            at += 4 + length
        return entries

    def token_amount(data):
        return struct.unpack_from("<Q", data, 64)[0]

    def create_mint(lane, token):
        mint, key = new_key(f"mint-{lane}")
        tx, _ = send(f"create-mint-{lane}", args.program,
                     ["payer:sw", mint + ":sw", SYSTEM, token],
                     bytes([TAG_CREATE_MINT]), [key])
        data = account(mint, tx["slot"])
        assert data["owner"] == token and len(data["bytes"]) == 82
        b = data["bytes"]
        assert struct.unpack_from("<I", b, 0)[0] == 1 and b[4:36] == pubkey_bytes(payer)
        assert b[44] == DECIMALS and b[45] == 1
        return mint

    def immutable_account(name, lane, token, mint, expected_extensions):
        address, key = new_key(name)
        tx, record = send(name, args.program,
                          ["payer:sw", address + ":sw", mint, payer, SYSTEM, token],
                          bytes([TAG_IMMUTABLE_ACCOUNT]), [key])
        size = struct.unpack("<Q", return_data(tx))[0]
        data = account(address, tx["slot"])
        b = data["bytes"]
        assert data["owner"] == token and len(b) == size, (len(b), size)
        assert b[0:32] == pubkey_bytes(mint) and b[32:64] == pubkey_bytes(payer)
        assert token_amount(b) == 0 and b[108] == 1
        if token == LEGACY:
            assert size == 165, "SPL Token answers the base size"
        else:
            assert b[165] == 2, "Token-2022 account type"
            entries = tlv_entries(b)
            kinds = sorted(kind for kind, _ in entries)
            assert kinds == sorted(expected_extensions), (kinds, expected_extensions)
            assert size == 166 + sum(4 + length for _, length in entries)
        assert data["lamports"] == rent(size)
        record["accountBytes"] = size
        record["extensions"] = tlv_entries(b) if token == TOKEN_2022 else []
        return address, size

    def mint_to(lane, token, mint, target, amount):
        tx, _ = send(f"mint-to-{lane}", args.program,
                     [mint + ":w", target + ":w", "payer:s", token],
                     bytes([TAG_MINT_TO]) + struct.pack("<QB", amount, DECIMALS))
        assert token_amount(account(target, tx["slot"])["bytes"]) == amount

    def batch(lane, token, mint, a, b):
        before = (token_amount(account(a)["bytes"]), token_amount(account(b)["bytes"]))
        tx, record = send(f"batch-{lane}", args.program,
                          [a + ":w", mint, b + ":w", "payer:s", token],
                          bytes([TAG_BATCH_ROUND_TRIP]) + struct.pack("<QB", BATCH_AMOUNT, DECIMALS),
                          allow_failure=(token == TOKEN_2022))
        logs = tx["meta"]["logMessages"]
        invocations = [line for line in logs if line == f"Program {token} invoke [2]"]
        if tx["meta"]["err"] is None:
            assert len(invocations) == 1, "two transfers must cost one token CPI"
            after = (token_amount(account(a, tx["slot"])["bytes"]),
                     token_amount(account(b, tx["slot"])["bytes"]))
            assert after == before, (before, after)
            record["tokenProgramInvocations"] = 1
            record["balancesUnchangedAfterRoundTrip"] = True
            findings[f"batch-{lane}"] = "accepted"
        else:
            findings[f"batch-{lane}"] = f"refused: {tx['meta']['err']}"

    def ui_round_trip(name, token, mint, amount, expected_text):
        tx, record = send(name, args.program, [mint, token],
                          bytes([TAG_UI_AMOUNT_ROUND_TRIP]) + struct.pack("<Q", amount))
        returned = return_data(tx)
        back = struct.unpack_from("<Q", returned, 0)[0]
        text = returned[8:].decode("utf-8")
        assert back == amount, (back, amount)
        assert text == expected_text, (text, expected_text)
        record["uiAmount"] = text
        record["roundTrippedAmount"] = back

    def batch_refusal(lane, token, mint, a):
        before = account(a)
        tx, record = send(f"batch-self-transfer-{lane}", args.program,
                         [mint + ":w", a + ":w", "payer:s", token],
                         bytes([TAG_BATCH_SELF_TRANSFER]) + struct.pack("<QB", BATCH_AMOUNT, DECIMALS),
                         allow_failure=True)
        assert tx["meta"]["err"] == {"InstructionError": [0, "AccountBorrowFailed"]}
        assert not any(line == f"Program {token} invoke [2]" for line in tx["meta"]["logMessages"])
        after = account(a, tx["slot"])
        assert after["bytes"] == before["bytes"] and after["lamports"] == before["lamports"]
        record["tokenProgramInvocations"] = 0
        record["tokenAccountUnchanged"] = True

    def hook_parser(mint):
        discriminator = hashlib.sha256(b"spl-transfer-hook-interface:execute").digest()[:8]
        wire = bytearray(discriminator + struct.pack("<II", 39, 1) + bytes([0]) + pubkey_bytes(mint) + bytes([0, 1]))
        cases = [("literal", wire[:], None)]
        pda = wire[:]; pda[16] = 1; pda[17:49] = bytes([3, 0] + [0] * 30)
        cases.append(("account-key-pda", pda, None))
        derived = json.loads(run(["solana", "find-program-derived-address", TOKEN_2022,
                                  "pubkey:" + mint, "--output", "json"]))["address"]
        wrong = wire[:]; wrong[:8] = bytes([9] * 8)
        cases.append(("wrong-type", wrong, 6700))
        short = wire[:]; short[8:12] = struct.pack("<I", 4)
        cases.append(("count-past-tlv", short, 6702))
        long = wire[:]; long[8:12] = struct.pack("<I", 40)
        cases.append(("truncated-tlv", long, 6701))
        reserved = wire[:]; reserved[16] = 3; reserved[17:49] = bytes(32)
        cases.append(("reserved-kind", reserved, 6703))
        many = wire[:]; many[16] = 1; many[17:49] = bytes([3, 0] * 16)
        cases.append(("too-many-seeds", many, 6704))
        oversized = wire[:]; oversized[16] = 1; oversized[17:49] = bytes([2, 0, 33] + [0] * 29)
        cases.append(("oversized-seed", oversized, 6703))
        for label, data, error in cases:
            tx, record = send(f"hook-parser-{label}", args.program, [mint, TOKEN_2022],
                              bytes([TAG_RESOLVE_HOOK_LIST]) + data, allow_failure=error is not None)
            if error is None:
                expected = derived if label == "account-key-pda" else mint
                assert return_data(tx) == pubkey_bytes(expected)
                record["expectedAddress"] = expected
            else:
                assert tx["meta"]["err"] == {"InstructionError": [0, {"Custom": error}]}
            record["expectedError"] = error

    def withdraw_excess(lane, token, target, size):
        send(f"prefund-{lane}", SYSTEM, ["payer:sw", target + ":w"], struct.pack("<IQ", 2, PREFUND))
        assert account(target)["lamports"] == rent(size) + PREFUND
        tx, record = send(f"withdraw-excess-{lane}", args.program,
                          [target + ":w", "payer:w", "payer:s", token], bytes([TAG_WITHDRAW_EXCESS]))
        after = account(target, tx["slot"])
        assert after["lamports"] == rent(size), (after["lamports"], rent(size))
        assert token_amount(after["bytes"]) == token_amount(account(target)["bytes"])
        record["withdrawnLamports"] = PREFUND

    def multisig(lane, token, member):
        address, key = new_key(f"multisig-{lane}")
        tx, record = send(f"multisig-{lane}", args.program,
                          ["payer:sw", address + ":sw", payer, member, SYSTEM, token],
                          bytes([TAG_INIT_MULTISIG, 1]), [key])
        data = account(address, tx["slot"])
        b = data["bytes"]
        assert data["owner"] == token and len(b) == 355
        assert b[0] == 1 and b[1] == 2 and b[2] == 1, b[:3]
        assert b[3:35] == pubkey_bytes(payer) and b[35:67] == pubkey_bytes(member)
        assert b[67:99] == bytes(32)
        record["multisig"] = {"m": 1, "n": 2}

    def wrap_and_unwrap():
        address, key = new_key("wrapped-sol")
        tx, record = send("wrap-and-unwrap", args.program,
                          ["payer:sw", address + ":sw", NATIVE_MINT, SYSTEM, LEGACY],
                          bytes([TAG_WRAP_AND_UNWRAP]) + struct.pack("<QQ", WRAP, UNWRAP),
                          [key], allow_failure=True)
        if tx["meta"]["err"] is None:
            data = account(address, tx["slot"])
            b = data["bytes"]
            assert token_amount(b) == WRAP - UNWRAP, token_amount(b)
            assert data["lamports"] == rent(165) + WRAP - UNWRAP
            assert struct.unpack_from("<I", b, 109)[0] == 1, "native account"
            record["unwrappedLamports"] = UNWRAP
            findings["unwrap-lamports"] = "accepted"
        else:
            findings["unwrap-lamports"] = f"refused: {tx['meta']['err']}"

    def extended_mint(mask, allow_failure, label=None):
        label = label or f"extended-mint-{mask:02x}"
        mint, key = new_key(label)
        tx, record = send(label, args.program,
                          ["payer:sw", mint + ":sw", SYSTEM, TOKEN_2022],
                          bytes([TAG_CREATE_EXTENDED_MINT, mask]), [key], allow_failure)
        if tx["meta"]["err"] is not None:
            findings[label] = f"refused: {tx['meta']['err']}"
            return None
        expected = [(kind, length) for bit, kind, length in EXTENSIONS if mask & bit]
        size = 166 + sum(4 + length for _, length in expected)
        data = account(mint, tx["slot"])
        b = data["bytes"]
        assert data["owner"] == TOKEN_2022 and len(b) == size, (len(b), size)
        assert b[44] == DECIMALS and b[45] == 1 and b[165] == 1
        assert tlv_entries(b) == expected, (tlv_entries(b), expected)
        assert data["lamports"] == rent(size)
        record["mintBytes"] = size
        record["extensions"] = expected
        return mint

    def tlv_values(data, start=166):
        values = {}
        at = start
        while at + 4 <= len(data):
            kind, length = struct.unpack_from("<HH", data, at)
            if kind == 0:
                break
            values[kind] = data[at + 4:at + 4 + length]
            at += 4 + length
        return values

    def bounded(text):
        raw = text.encode("utf-8")
        return struct.pack("<H", len(raw)) + raw

    def borsh(text):
        raw = text.encode("utf-8")
        return struct.pack("<I", len(raw)) + raw

    def metadata_bytes(authority, mint, name, symbol, uri, extra):
        out = (pubkey_bytes(authority) if authority else bytes(32)) + pubkey_bytes(mint)
        out += borsh(name) + borsh(symbol) + borsh(uri) + struct.pack("<I", len(extra))
        for key, value in extra:
            out += borsh(key) + borsh(value)
        return out

    def emitted(tx):
        returned = tx["meta"]["returnData"]
        assert returned["programId"] == TOKEN_2022, returned
        return base64.b64decode(returned["data"][0])

    def metadata_lane():
        mint, key = new_key("metadata-mint")
        name, symbol, uri = "Hopper Lab", "HOP", "https://hopperzero.dev/lab.json"
        tx, record = send("create-metadata-mint", args.program,
                          ["payer:sw", mint + ":sw", SYSTEM, TOKEN_2022],
                          bytes([TAG_CREATE_METADATA_MINT]) + bounded(name) + bounded(symbol)
                          + bounded(uri), [key])
        data = account(mint, tx["slot"])
        b = data["bytes"]
        values = tlv_values(b)
        assert data["owner"] == TOKEN_2022 and sorted(values) == [EXT_METADATA_POINTER, EXT_METADATA]
        assert values[EXT_METADATA_POINTER] == pubkey_bytes(payer) + pubkey_bytes(mint)
        assert values[EXT_METADATA] == metadata_bytes(payer, mint, name, symbol, uri, [])
        assert data["lamports"] == rent(len(b)), "the mint is funded for exactly its size"
        record["mintBytes"] = len(b)

        update = ["payer:sw", mint + ":w", SYSTEM, TOKEN_2022]
        tx, record = send("set-metadata-key", args.program, update,
                          bytes([TAG_SET_METADATA_KEY]) + bounded("tier") + bounded("gold"))
        data = account(mint, tx["slot"])
        expected = metadata_bytes(payer, mint, name, symbol, uri, [("tier", "gold")])
        assert tlv_values(data["bytes"])[EXT_METADATA] == expected
        assert emitted(tx) == expected, "Emit returns the stored metadata"
        assert data["lamports"] == rent(len(data["bytes"]))
        record["emittedBytes"] = len(expected)

        # The same key again, with a multi-byte value: replaced in place.
        tx, _ = send("replace-metadata-key", args.program, update,
                     bytes([TAG_SET_METADATA_KEY]) + bounded("tier") + bounded("café €"))
        data = account(mint, tx["slot"])
        expected = metadata_bytes(payer, mint, name, symbol, uri, [("tier", "café €")])
        assert tlv_values(data["bytes"])[EXT_METADATA] == expected
        assert emitted(tx) == expected
        assert data["lamports"] >= rent(len(data["bytes"]))

        # Rename, remove the key (then again, idempotently), drop the authority.
        renamed = "Hopper Lab, final"
        tx, _ = send("finalize-metadata", args.program, update,
                     bytes([TAG_FINALIZE_METADATA]) + bounded(renamed) + bounded("tier"))
        data = account(mint, tx["slot"])
        expected = metadata_bytes(None, mint, renamed, symbol, uri, [])
        assert tlv_values(data["bytes"])[EXT_METADATA] == expected
        assert emitted(tx) == expected
        assert data["lamports"] >= rent(len(data["bytes"]))

        # No update authority is left: the next update is refused.
        before = data["bytes"]
        tx, record = send("set-metadata-key-without-authority", args.program, update,
                          bytes([TAG_SET_METADATA_KEY]) + bounded("tier") + bounded("x"),
                          allow_failure=True)
        assert tx["meta"]["err"] is not None, "an update without an authority must be refused"
        assert account(mint, tx["slot"])["bytes"] == before
        record["accountUnchanged"] = True
        findings["metadata"] = "accepted"
        return mint

    def group_lane():
        group, key = new_key("group-mint")
        tx, record = send("create-group", args.program,
                          ["payer:sw", group + ":sw", SYSTEM, TOKEN_2022],
                          bytes([TAG_CREATE_GROUP]) + struct.pack("<Q", GROUP_MAX_SIZE), [key],
                          allow_failure=True)
        if tx["meta"]["err"] is not None:
            findings["token-group"] = f"refused: {tx['meta']['err']}"
            return None, None
        data = account(group, tx["slot"])
        values = tlv_values(data["bytes"])
        assert sorted(values) == [EXT_GROUP_POINTER, EXT_GROUP], sorted(values)
        assert values[EXT_GROUP_POINTER] == pubkey_bytes(payer) + pubkey_bytes(group)
        assert values[EXT_GROUP] == (pubkey_bytes(payer) + pubkey_bytes(group)
                                     + struct.pack("<QQ", 0, GROUP_MAX_SIZE))
        assert data["lamports"] == rent(len(data["bytes"]))
        findings["token-group"] = "accepted"

        member, key = new_key("group-member-mint")
        tx, record = send("create-group-member", args.program,
                          ["payer:sw", member + ":sw", group + ":w", SYSTEM, TOKEN_2022],
                          bytes([TAG_CREATE_GROUP_MEMBER]), [key], allow_failure=True)
        if tx["meta"]["err"] is not None:
            findings["token-group-member"] = f"refused: {tx['meta']['err']}"
            return group, None
        data = account(member, tx["slot"])
        values = tlv_values(data["bytes"])
        assert sorted(values) == [EXT_GROUP_MEMBER_POINTER, EXT_GROUP_MEMBER], sorted(values)
        assert values[EXT_GROUP_MEMBER] == (pubkey_bytes(member) + pubkey_bytes(group)
                                            + struct.pack("<Q", 1))
        assert data["lamports"] == rent(len(data["bytes"]))
        grown = tlv_values(account(group, tx["slot"])["bytes"])[EXT_GROUP]
        assert struct.unpack_from("<Q", grown, 64)[0] == 1, "the group counts its member"
        findings["token-group-member"] = "accepted"
        return group, member

    deployment("before")
    accounts = {}
    for lane, token in [("legacy", LEGACY), ("token-2022", TOKEN_2022)]:
        mint = create_mint(lane, token)
        a, size_a = immutable_account(f"immutable-a-{lane}", lane, token, mint, [EXT_IMMUTABLE_OWNER])
        b, _ = immutable_account(f"immutable-b-{lane}", lane, token, mint, [EXT_IMMUTABLE_OWNER])
        mint_to(lane, token, mint, a, SUPPLY)
        batch(lane, token, mint, a, b)
        batch_refusal(lane, token, mint, a)
        ui_round_trip(f"ui-amount-{lane}", token, mint, UI_AMOUNT, "1.234567")
        withdraw_excess(lane, token, a, size_a)
        multisig(lane, token, a)
        accounts[lane] = {"mint": mint, "a": a, "b": b}
    hook_parser(accounts["token-2022"]["mint"])
    wrap_and_unwrap()

    # The live program decides which extensions coexist (Token-2022 refuses a
    # mint that is both interest bearing and scaled, `Custom(51)`). Every
    # extension is tried alone, then the set is grown greedily in program
    # order; every refusal is a recorded finding, never a hidden skip.
    supported = 0
    for bit, kind, _ in EXTENSIONS:
        if extended_mint(bit, allow_failure=True) is None:
            findings[f"extension-{kind}"] = "refused alone by the live Token-2022 program"
    mask = 0
    for bit, kind, _ in EXTENSIONS:
        if f"extension-{kind}" in findings:
            continue
        candidate = mask | bit
        if extended_mint(candidate, allow_failure=True, label=f"extended-mint-grow-{candidate:02x}") is not None:
            mask = candidate
        else:
            findings[f"extension-{kind}-with-{mask:02x}"] = "refused in combination"
    mint = extended_mint(mask, allow_failure=False, label="extended-mint-final")
    findings["extended-mint-mask"] = mask
    expected_account_extensions = [EXT_IMMUTABLE_OWNER] + ([EXT_PAUSABLE_ACCOUNT] if mask & 2 else [])
    ext_a, _ = immutable_account("immutable-a-extended", "extended", TOKEN_2022, mint,
                                 expected_account_extensions)
    mint_to("extended", TOKEN_2022, mint, ext_a, SUPPLY)
    scaled = bool(mask & 4)
    ui_round_trip("ui-amount-extended", TOKEN_2022, mint, UI_AMOUNT,
                  "2.469134" if scaled else "1.234567")
    if mask & 2:
        send("pause-resume", args.program, [mint + ":w", "payer:s", TOKEN_2022], bytes([TAG_PAUSE_RESUME]))
    if scaled:
        send("update-multiplier", args.program, [mint + ":w", "payer:s", TOKEN_2022],
             bytes([TAG_UPDATE_MULTIPLIER]) + struct.pack("<d", 3.0))
        ui_round_trip("ui-amount-extended-x3", TOKEN_2022, mint, UI_AMOUNT, "3.703701")
    accounts["extended"] = {"mint": mint, "a": ext_a}
    accounts["metadata"] = {"mint": metadata_lane()}
    group, member = group_lane()
    accounts["group"] = {"group": group, "member": member}
    deployment("after")
    assert run(["git", "rev-parse", "HEAD"]).strip() == source
    assert not run(["git", "status", "--porcelain"]).strip(), "source changed during capture"
    write("receipt.json", {"schema": "hopper.token-lab-devnet.v1", "sourceCommit": source,
                          "rpcEndpoint": RPC, "genesisHash": GENESIS, "commitment": "finalized",
                          "programId": args.program, "elfSha256": hashlib.sha256(elf).hexdigest(),
                          "deployedElfMatchesBeforeAndAfter": True, "accounts": accounts,
                          "findings": findings, "transactions": records})
    print("findings:", json.dumps(findings), flush=True)
    print("All token-lab devnet cases and exact state checks passed.", flush=True)


if __name__ == "__main__":
    main()
