#!/usr/bin/env python3
"""Verify function-lab results, sysvars and alias borrowing on public devnet.

Build after recording a source snapshot with test-borrowed-args-devnet.py
snapshot. The runner verifies unchanged source and deployed ELF bytes before
and after execution. Expected bytes come from function-lab-cases.py, Python
arithmetic and live RPC sysvars; no native host syscall stub is an oracle.
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
H = runpy.run_path(str(ROOT / "scripts/test-borrowed-args-devnet.py"))
run, rpc, finalize, pubkey_bytes, source_snapshot, write = (H[k] for k in (
    "run", "rpc", "finalize", "pubkey_bytes", "source_snapshot", "write"))
RPC, GENESIS, SYSTEM = (H[k] for k in ("RPC", "GENESIS", "SYSTEM"))
cases = runpy.run_path(str(ROOT / "scripts/function-lab-cases.py"))["cases"]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("payer", "hopper", "elf", "source-snapshot", "out"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--program", required=True)
    parser.add_argument("--sha512", action="store_true")
    parser.add_argument("--blake3", action="store_true")
    args = parser.parse_args()
    source = source_snapshot()
    if source != json.loads(args.source_snapshot.read_text(encoding="utf-8")):
        raise RuntimeError("source changed since the pre-build snapshot")
    if rpc("getGenesisHash", []) != GENESIS:
        raise RuntimeError("unexpected network")
    out = H["ignored_output"](args.out)
    out.mkdir(parents=True, exist_ok=False)
    keys = out / "keys"
    keys.mkdir()
    write(out / "source.json", source)
    elf = args.elf.read_bytes()
    records = []

    def deployment(phase):
        dump = out / f"{phase}-onchain.so"
        run(["solana", "program", "dump", args.program, str(dump), "--url", RPC,
             "--keypair", str(args.payer), "--commitment", "finalized"])
        if dump.read_bytes() != elf:
            raise RuntimeError("deployed ELF differs from compiled fixture")

    def send(name, data, expected=None, output=None, accounts=(), signers=(), program=None):
        target = program or args.program
        argv = [str(args.hopper), "tx", "send", "--program", target,
                "--keypair", str(args.payer), "--rpc", RPC, "--data", data.hex()]
        for account in accounts:
            argv += ["--account", account]
        for signer in signers:
            argv += ["--signer", str(signer)]
        if expected is not None:
            argv += ["--allow-failure"]
        sent = run(argv)
        (out / f"{name}.log").write_text(sent, encoding="utf-8")
        match = re.search(r"^signature\s*:\s*(\w+)", sent, re.MULTILINE)
        if match is None:
            raise RuntimeError("no signature; inspect sender log before retrying")
        tx = finalize(match[1])
        write(out / f"{name}.transaction.json", tx)
        wanted_error = None if expected is None else {"InstructionError": [0, expected]}
        if tx["meta"]["err"] != wanted_error:
            raise RuntimeError(f"{name}: expected {wanted_error}, got {tx['meta']['err']}")
        returned = tx["meta"].get("returnData")
        if output is not None:
            # Solana omits returnData when sol_set_return_data receives zero bytes.
            if output:
                if returned is None or returned["programId"] != target or base64.b64decode(returned["data"][0]) != output:
                    raise RuntimeError(f"{name}: incorrect return bytes or producer")
            elif returned is not None and base64.b64decode(returned["data"][0]):
                raise RuntimeError(f"{name}: expected empty return data")
        records.append({"name": name, "signature": match[1], "slot": tx["slot"],
            "error": tx["meta"]["err"], "computeUnits": tx["meta"].get("computeUnitsConsumed")})
        write(out / "progress.json", records)
        print(f"{name}: finalized, {records[-1]['computeUnits']} CU", flush=True)
        return tx

    deployment("before")
    vectors = cases(args.sha512, blake3=args.blake3)
    write(out / "cases.json", vectors)
    for case in vectors:
        tx = send(case["name"], bytes.fromhex(case["data"]), case["error"],
                  bytes.fromhex(case["return"]) if case["error"] is None else None)
        if "logSegments" in case:
            expected_log = "Program data: " + " ".join(base64.b64encode(bytes.fromhex(segment)).decode() for segment in case["logSegments"])
            logs = [line for line in tx["meta"]["logMessages"] if line.startswith("Program data:")]
            if logs != [expected_log]:
                raise RuntimeError(f"{case['name']}: incorrect receipt bytes or segment boundaries")

    tx = send("live-sysvars", bytes([3, 0]))
    returned = tx["meta"].get("returnData", {})
    if returned.get("programId") != args.program:
        raise RuntimeError("wrong sysvar return producer")
    raw = base64.b64decode(returned["data"][0])
    if len(raw) != 113:
        raise RuntimeError("incorrect sysvar response size")
    words = struct.unpack("<14Q", raw[:112])
    schedule = rpc("getEpochSchedule", [])
    rent = [rpc("getMinimumBalanceForRentExemption", [length]) for length in [0, 16, 128]]
    if words[0] != tx["slot"] or list(words[5:8]) != rent:
        raise RuntimeError("clock slot or rent differs from RPC")
    schedule_account = rpc("getMultipleAccounts", [["SysvarEpochSchedu1e111111111111111111111111"],
        {"encoding": "base64", "commitment": "finalized", "minContextSlot": tx["slot"]}])["value"][0]
    if schedule_account is None or schedule_account["owner"] != "Sysvar1111111111111111111111111111111111111":
        raise RuntimeError("missing or foreign-owned EpochSchedule sysvar")
    image = base64.b64decode(schedule_account["data"][0])
    if len(image) != 33 or image[16] not in (0, 1):
        raise RuntimeError("noncanonical EpochSchedule account image")
    slots, offset, warmup, first_epoch, first_slot = struct.unpack("<QQ?QQ", image)
    if slots == 0 or list(words[8:12]) != [slots, offset, first_epoch, first_slot] or raw[112] != int(warmup):
        raise RuntimeError("decoded epoch schedule differs from its actual sysvar bytes")

    def epoch_at(slot):
        if slot >= first_slot:
            return first_epoch + (slot - first_slot) // slots
        epoch, length = 0, 32
        while slot >= length:
            slot -= length
            epoch, length = epoch + 1, length * 2
        return epoch

    def first_slot_at(epoch):
        return 32 * ((1 << epoch) - 1) if epoch <= first_epoch else first_slot + (epoch - first_epoch) * slots

    if words[12:14] != (epoch_at(tx["slot"]), first_slot_at(words[2])):
        raise RuntimeError("epoch arithmetic differs from independently decoded schedule")
    decoded = dict(slotsPerEpoch=slots, leaderScheduleSlotOffset=offset, warmup=warmup,
                   firstNormalEpoch=first_epoch, firstNormalSlot=first_slot)
    consistency_findings = []
    if decoded != schedule:
        consistency_findings.append("EpochSchedule account differs from getEpochSchedule RPC")
    if words[2] != epoch_at(tx["slot"]):
        consistency_findings.append("Clock epoch differs from EpochSchedule arithmetic")
    write(out / "sysvars.json", {"returnedHex": raw.hex(), "rpcEpochSchedule": schedule,
        "epochScheduleAccount": schedule_account, "decodedEpochSchedule": decoded,
        "rent": rent, "clusterConsistencyFindings": consistency_findings})
    for finding in consistency_findings:
        print("CLUSTER CONSISTENCY FINDING: " + finding, flush=True)
    send("sysvar-past-end", bytes([3, 1]), "UnsupportedSysvar")

    key = keys / "state.json"
    run(["solana-keygen", "new", "--no-bip39-passphrase", "--silent", "--outfile", str(key)])
    state = run(["solana-keygen", "pubkey", str(key)]).strip()
    rent = rpc("getMinimumBalanceForRentExemption", [16])
    tx = send("create-state", struct.pack("<IQQ", 0, rent, 16) + pubkey_bytes(args.program),
        accounts=["payer:sw", state + ":sw"], signers=[key], program=SYSTEM)

    def account(slot):
        return rpc("getMultipleAccounts", [[state], {"encoding": "base64", "commitment": "finalized", "minContextSlot": slot}])["value"][0]

    current = account(tx["slot"])
    if current is None or current["owner"] != args.program or current["lamports"] != rent or base64.b64decode(current["data"][0]) != bytes(16):
        raise RuntimeError("unexpected initial state")
    normal = [state + ":w", "payer:s", state + ":w"]
    for name, mode, error, metas in [
        ("state-write-alias", 0, None, normal),
        ("state-held-borrow", 1, "AccountBorrowFailed", normal),
        ("state-lens-past-end", 2, "AccountDataTooSmall", normal),
        ("state-fingerprint-unchanged", 3, None, normal),
        ("state-unsigned", 0, "MissingRequiredSignature", [state + ":w", SYSTEM, state + ":w"]),
        ("state-readonly", 0, "Immutable", [state, "payer:s", state]),
        ("state-foreign-owner", 0, "IncorrectProgramId", [SYSTEM, "payer:s", SYSTEM]),
    ]:
        expected = dict(current)
        if mode == 0 and error is None:
            expected["data"] = [base64.b64encode(bytes([0]) + struct.pack("<Q", 42) + bytes(7)).decode(), "base64"]
        tx = send(name, bytes([6, mode]) + struct.pack("<Q", 42), error,
            base64.b64decode(expected["data"][0]) if error is None else None, metas)
        after = account(tx["slot"])
        if after != expected:
            raise RuntimeError(f"{name}: unexpected state mutation")
        write(out / f"{name}.account.json", after)
        current = after
    deployment("after")
    if source_snapshot() != source:
        raise RuntimeError("source changed during the run")
    write(out / "summary.json", {"schema": "hopper.function-lab.devnet.v1", "network": "devnet", "genesis": GENESIS,
        "programId": args.program, "baseCommit": source["baseCommit"], "elfBytes": len(elf), "elfSha256": hashlib.sha256(elf).hexdigest(),
        "sourceSnapshotSha256": hashlib.sha256((out / "source.json").read_bytes()).hexdigest(),
        "knownAnswerCases": len(vectors), "sha512": args.sha512, "transactions": records,
        "deploymentMatchedBeforeAndAfter": True, "state": state,
        "functionChecksPassed": True, "clusterConsistencyFindings": consistency_findings,
        "allPassed": not consistency_findings})
    if consistency_findings:
        raise SystemExit("Function checks passed, but cluster consistency findings remain; see summary.json")


if __name__ == "__main__":
    main()
