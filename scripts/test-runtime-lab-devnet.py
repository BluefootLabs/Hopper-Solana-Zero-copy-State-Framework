#!/usr/bin/env python3
"""Drive the hopper-runtime-lab program on public devnet.

The heap lane allocates inside the default 32 KiB heap, allocates 200 KiB
with a requested 256 KiB heap frame and without one, asks for one byte past
the declared heap, grows a vector in place, and runs a checkpointed loop.
The SlotHashes lane looks up recent, older, skipped, too-old, and future
slots and checks every returned hash against the live sysvar. The panic
lane panics in the silent build and in the reporting build (a second
program id) and checks what each one logs.

Keypairs stay under ignored target/. Only the explicit public devnet endpoint
is used. This spends devnet SOL on fees.
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
run, rpc, finalize = (H[k] for k in ("run", "rpc", "finalize"))
RPC, GENESIS = (H[k] for k in ("RPC", "GENESIS"))

SLOT_HASHES = "SysvarS1otHashes111111111111111111111111111"
KIB = 1024
HEAP_FRAME = 256 * KIB
SCRATCH = 8 + 20 * KIB
TAG_ALLOCATE, TAG_GROW, TAG_CHECKPOINT, TAG_PANIC, TAG_BACK, TAG_AT = range(6)
ERR_ALLOCATION_REFUSED = 6800
FOUND, SKIPPED, TOO_OLD, AHEAD = range(4)
ANY_ERROR = object()


def slot_hashes(slot: int) -> dict[int, bytes]:
    value = rpc("getMultipleAccounts", [[SLOT_HASHES], {
        "encoding": "base64", "commitment": "finalized", "minContextSlot": slot
    }])["value"][0]
    data = base64.b64decode(value["data"][0])
    count = struct.unpack_from("<Q", data, 0)[0]
    entries = {}
    for i in range(count):
        at = 8 + 40 * i
        entries[struct.unpack_from("<Q", data, at)[0]] = data[at + 8:at + 40]
    return entries


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--program", required=True)
    parser.add_argument("--elf", type=Path, required=True)
    parser.add_argument("--reporting-program", required=True)
    parser.add_argument("--reporting-elf", type=Path, required=True)
    parser.add_argument("--payer", type=Path, required=True)
    parser.add_argument("--hopper", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    if not out.is_relative_to(ROOT / "target"):
        raise RuntimeError("output must stay under ignored target/")
    assert rpc("getGenesisHash", []) == GENESIS, "unexpected network"
    assert not run(["git", "status", "--porcelain"]).strip(), "commit source first"
    source = run(["git", "rev-parse", "HEAD"]).strip()
    out.mkdir(parents=True, exist_ok=False)
    records = []

    def write(name, value):
        (out / name).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")

    def deployment(phase):
        for program, elf in ((args.program, args.elf), (args.reporting_program, args.reporting_elf)):
            dump = out / f"{phase}-{program[:8]}-onchain.so"
            run(["solana", "program", "dump", program, str(dump), "--url", RPC,
                 "--keypair", str(args.payer), "--commitment", "finalized"])
            assert dump.read_bytes() == elf.read_bytes(), f"{program}: deployed ELF differs"

    def send(name, data, program=None, heap_frame=None, refused=None):
        program = program or args.program
        argv = [str(args.hopper), "tx", "send", "--program", program,
                "--keypair", str(args.payer), "--rpc", RPC, "--data", data.hex(),
                "--account", "payer:s"]
        if heap_frame is not None:
            argv += ["--heap-frame", str(heap_frame)]
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
                  "computeUnits": tx["meta"].get("computeUnitsConsumed"),
                  "heapFrame": heap_frame}
        records.append(record)
        if refused is None:
            assert error is None, (name, error, tx["meta"]["logMessages"])
        elif refused is ANY_ERROR:
            assert error is not None, f"{name} must be refused"
        else:
            # The heap-frame instruction comes first when one is requested.
            index = 1 if heap_frame is not None else 0
            assert error == {"InstructionError": [index, refused]}, (name, error, refused)
        status = "ok" if error is None else f"refused {json.dumps(error)}"
        print(f"{name}: finalized, {record['computeUnits']} CU, {status}", flush=True)
        return tx, record

    def returned(tx) -> bytes:
        data = tx["meta"].get("returnData") or {}
        return base64.b64decode(data["data"][0]) if data else b""

    deployment("before")

    # ── Heap ─────────────────────────────────────────────────────────
    room = 32 * KIB - SCRATCH
    tx, record = send("allocate-default-heap", bytes([TAG_ALLOCATE]) + struct.pack("<I", room))
    used, pages = struct.unpack("<QQ", returned(tx))
    assert used == room, (used, room)
    record.update(bytes=room, heapUsed=used, pages=pages)

    tx, record = send("allocate-past-default-heap",
                      bytes([TAG_ALLOCATE]) + struct.pack("<I", room + 1), refused=ANY_ERROR)
    record["bytes"] = room + 1

    big = 200 * KIB
    tx, record = send("allocate-200k-with-frame", bytes([TAG_ALLOCATE]) + struct.pack("<I", big),
                      heap_frame=HEAP_FRAME)
    used, pages = struct.unpack("<QQ", returned(tx))
    assert used == big and pages == 50, (used, pages)
    record.update(bytes=big, heapUsed=used, pages=pages)

    tx, record = send("allocate-200k-without-frame",
                      bytes([TAG_ALLOCATE]) + struct.pack("<I", big), refused=ANY_ERROR)
    record["bytes"] = big

    most = HEAP_FRAME - SCRATCH
    tx, record = send("allocate-whole-frame", bytes([TAG_ALLOCATE]) + struct.pack("<I", most),
                      heap_frame=HEAP_FRAME)
    assert struct.unpack("<QQ", returned(tx))[0] == most
    record["bytes"] = most
    tx, record = send("allocate-past-declared-heap",
                      bytes([TAG_ALLOCATE]) + struct.pack("<I", most + 1),
                      heap_frame=HEAP_FRAME, refused={"Custom": ERR_ALLOCATION_REFUSED})
    record["bytes"] = most + 1

    tx, record = send("grow-to-200k", bytes([TAG_GROW]) + struct.pack("<H", 200),
                      heap_frame=HEAP_FRAME)
    used, capacity = struct.unpack("<QQ", returned(tx))
    assert used == capacity == big, (used, capacity)
    record.update(heapUsed=used, capacity=capacity)

    tx, record = send("checkpoint-loop", bytes([TAG_CHECKPOINT]) + struct.pack("<IH", 8 * KIB, 100))
    after, peak = struct.unpack("<QQ", returned(tx))
    assert after == 0 and peak == 8 * KIB, (after, peak)
    record.update(heapUsedAfter=after, peak=peak, rounds=100)

    # ── SlotHashes ───────────────────────────────────────────────────
    def lookup(name, tag, value):
        tx, record = send(name, bytes([tag]) + struct.pack("<Q", value))
        newest, target = struct.unpack_from("<QQ", returned(tx))
        status, reads = returned(tx)[16], returned(tx)[17]
        digest = returned(tx)[18:50]
        record.update(newest=newest, target=target, status=status, reads=reads)
        return tx, record, newest, target, status, digest

    for back in (1, 100, 300):
        tx, record, newest, target, status, digest = lookup(f"slot-hash-{back}-back", TAG_BACK, back)
        live = slot_hashes(tx["slot"])
        if target in live:
            assert status == FOUND, (back, status)
            assert digest == live[target], f"{back} back: hash differs from the sysvar"
            record["matchesSysvar"] = True
        else:
            assert status == SKIPPED, (back, status)
        record["hash"] = digest.hex()

    # A slot the cluster skipped, if the window has one.
    live = slot_hashes(0)
    present = sorted(live)
    gaps = [s for s in range(present[-1] - 200, present[-1]) if s not in live and s > present[0]]
    if gaps:
        tx, record, *_, status, _ = lookup("slot-hash-skipped", TAG_AT, gaps[-1])
        assert status == SKIPPED, status
        record["skippedSlot"] = gaps[-1]
    else:
        records.append({"name": "slot-hash-skipped", "note": "no skipped slot in the last 200"})

    tx, record, newest, _, status, _ = lookup("slot-hash-too-old", TAG_AT, present[0] - 1000)
    assert status == TOO_OLD, status
    tx, record, newest, _, status, _ = lookup("slot-hash-ahead", TAG_AT, present[-1] + 100_000)
    assert status == AHEAD, status

    # ── Panics ───────────────────────────────────────────────────────
    tx, record = send("panic-silent", bytes([TAG_PANIC, 7]), refused=ANY_ERROR)
    logs = tx["meta"]["logMessages"]
    assert not any(line.startswith("Program log:") for line in logs), logs
    assert not any("Panicked in" in line for line in logs), logs
    record["logs"] = logs

    tx, record = send("panic-reported", bytes([TAG_PANIC, 7]),
                      program=args.reporting_program, refused=ANY_ERROR)
    logs = tx["meta"]["logMessages"]
    assert "Program log: runtime lab panic, code 7" in logs, logs
    located = [line for line in logs if "Panicked in" in line]
    assert located and re.search(r"lib\.rs at \d+:\d+", located[0]), logs
    record["logs"] = logs

    deployment("after")
    assert run(["git", "rev-parse", "HEAD"]).strip() == source
    assert not run(["git", "status", "--porcelain"]).strip(), "source changed during capture"
    write("receipt.json", {
        "schema": "hopper.runtime-lab-devnet.v1", "sourceCommit": source, "rpcEndpoint": RPC,
        "genesisHash": GENESIS, "commitment": "finalized",
        "programs": {
            "silent": {"id": args.program, "elfSha256": hashlib.sha256(args.elf.read_bytes()).hexdigest()},
            "reporting": {"id": args.reporting_program,
                          "elfSha256": hashlib.sha256(args.reporting_elf.read_bytes()).hexdigest()},
        },
        "deployedElfMatchesBeforeAndAfter": True, "transactions": records,
    })
    print("All runtime-lab devnet cases passed.", flush=True)


if __name__ == "__main__":
    main()
