#!/usr/bin/env python3
"""Independent expected outputs for the function-lab SBF and devnet probes.

Python's arbitrary-precision arithmetic supplies the integer oracle. Hash
digests are known-answer vectors; Poseidon uses the Solana SDK's two-input
example. This module never calls Hopper to compute an expected result.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import struct
from pathlib import Path


def cases(sha512: bool = False, big_mod_exp: bool = False, blake3: bool = False) -> list[dict]:
    result = []

    def add(name, data, output=b"", error=None):
        result.append({"name": name, "data": data.hex(), "return": output.hex(), "error": error})

    # Keccak-256 (not SHA3-256), BLAKE3 unkeyed mode.
    digests = {
        0: [hashlib.sha256(b"").digest(), hashlib.sha256(b"abc").digest()],
        1: [bytes.fromhex(s) for s in [
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
            "4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45"]],
        2: [bytes.fromhex(s) for s in [
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"]],
    }
    if sha512:
        digests[3] = [hashlib.sha512(b"").digest(), hashlib.sha512(b"abc").digest()]
    if not blake3:
        del digests[2]
    for api in range(2):
        for algorithm, vectors in digests.items():
            for count in [0, 1, 2, 16, 17, 32, 64]:
                add(f"hash-{api}-{algorithm}-{count}", bytes([0, api, algorithm, count]) + (b"abc" if count else b""), vectors[int(count != 0)])
    for api in range(2):
        add(f"hash-{api}-empty-17", bytes([0, api, 0, 17]), digests[0][0])
    for name, data in [("empty", b""), ("unknown", b"\xff"), ("hash-truncated", b"\0\0"),
                       ("hash-fixture-cap", bytes([0, 0, 0, 65])), ("hash-zero-surplus", bytes([0, 0, 0, 0, 1]))]:
        add(name, data, error="InvalidInstructionData")

    payload = bytes(range(64))
    for api in [0, 1]:
        for dst, length in [(64, 13), (8, 8), (0, 0), (7, 8)]:
            output = payload[:length] + bytes([173]) * (64 - length)
            add(f"copy-{api}-{dst}-{length}", bytes([1, api, dst, length, 0, 173]) + payload,
                output if dst >= length else b"", None if dst >= length else "InvalidArgument")
    for start, length, dest in [(0, 63, 1), (1, 63, 0), (0, 64, 0), (64, 0, 64), (63, 2, 0), (0, 2, 63)]:
        output = bytearray(payload)
        valid = max(start + length, dest + length) <= 64
        if valid:
            output[dest:dest + length] = payload[start:start + length]
        add(f"move-{start}-{length}-{dest}", bytes([1, 2, start, length, dest, 0]) + payload,
            bytes(output) if valid else b"", None if valid else "InvalidArgument")
    for op, byte in [(3, 197), (4, 0), (5, 0)]:
        add(f"fill-{op}", bytes([1, op, 0, 0, 0, byte]) + payload, bytes([byte]) * 64)
    for left_len, start, end in [(8, 0, 8), (8, 1, 9), (8, 0, 7), (0, 64, 64)]:
        left, right = payload[:left_len], payload[start:end]
        add(f"compare-{left_len}-{start}-{end}", bytes([1, 6, left_len, start, end, 0]) + payload,
            bytes([0 if left < right else 2 if left > right else 1, left == right, left == right]))

    limit = (1 << 64) - 1
    # Independent integer arithmetic, including values beyond u64 intermediate products.
    epoch_cases = [(432000, 14, 524256, slot, epoch) for slot, epoch in
        [(0, 0), (31, 1), (32, 2), (95, 14), (96, 15), (524255, limit), (524256, 14), (limit, limit)]]
    epoch_cases += [(8192, 0, 0, 0, 0), (8192, 0, 0, limit, limit),
                    (0, 5, 0, limit, 7), (1, limit - 2, 0, limit, limit)]
    for index, (slots, first_epoch, first_slot, slot, epoch) in enumerate(epoch_cases):
        if slot < first_slot:
            expected_epoch = (slot + 32).bit_length() - 6
        else:
            expected_epoch = min(limit, first_epoch + ((slot - first_slot) // slots if slots else 0))
        if epoch <= first_epoch:
            expected_slot = min(limit, 32 * ((1 << min(epoch, 64)) - 1))
        else:
            expected_slot = min(limit, first_slot + (epoch - first_epoch) * slots)
        image = struct.pack('<QQ?QQ', slots, slots, first_slot != 0, first_epoch, first_slot)
        add(f'epoch-arithmetic-{index}', bytes([3, 2]) + image + struct.pack('<QQ', slot, epoch),
            struct.pack('<QQ', expected_epoch, expected_slot))
    pairs = [(0, 0), (0, limit), (1, 1), (limit, 1), (limit, 2), (limit, limit),
             (1 << 32, 1 << 32), ((1 << 32) - 1, (1 << 32) - 1), (1 << 63, 2),
             (limit // 3, 3), (limit // 3 + 1, 3), (5080, 256), (123456789, 987654321)]
    for index, (a, b) in enumerate(pairs):
        output = b""
        for value in [a + b, a - b, a * b, a // b if b else None, a * b]:
            valid = value is not None and 0 <= value <= limit
            output += bytes([valid]) + struct.pack("<Q", value if valid else 0)
        output += struct.pack("<QQQ", min(a * b, limit), min(a + b, limit), max(a - b, 0))
        add(f"arithmetic-{index}", bytes([2]) + struct.pack("<QQ", a, b), output)
    for length in [0, 1, 1023, 1024, 1025]:
        add(f"return-{length}", bytes([4]) + struct.pack("<HB", length, 165), bytes([165]) * length if length <= 1024 else b"",
            None if length <= 1024 else "InvalidArgument")

    identity = bytes([1]) + bytes(31)
    basepoint = bytes([0x58]) + bytes([0x66]) * 31
    add("edwards-valid", bytes([5, 0]) + basepoint, bytes([1]))
    add("edwards-add-identity", bytes([5, 1]) + basepoint + identity, basepoint)
    add("edwards-multiply-one", bytes([5, 2]) + identity + basepoint, basepoint)
    add("edwards-multiply-zero", bytes([5, 2]) + bytes(32) + basepoint, identity)
    add("ristretto-identity", bytes([5, 3]) + bytes(32), bytes([1]))
    add("ristretto-add-identity", bytes([5, 4]) + bytes(64), bytes(32))
    # https://github.com/anza-xyz/solana-sdk/blob/master/poseidon/src/lib.rs
    add("poseidon-ones-twos", bytes([5, 5]) + bytes([1]) * 32 + bytes([2]) * 32,
        bytes([13, 84, 225, 147, 143, 138, 140, 28, 125, 235, 94, 3, 85, 242, 99, 25,
               32, 123, 132, 254, 156, 162, 206, 27, 38, 231, 53, 200, 41, 130, 25, 144]))
    generator = (1).to_bytes(32, "big") + (2).to_bytes(32, "big")
    add("bn254-add-identity", bytes([5, 6]) + generator + bytes(64), generator)
    add("bn254-multiply-one", bytes([5, 7]) + generator + (1).to_bytes(32, "big"), generator)
    add("bn254-empty-pairing", bytes([5, 8]), (1).to_bytes(32, "big"))
    add("bn254-compress-identity", bytes([5, 9]) + bytes(64), bytes(32))
    add("bn254-decompress-identity", bytes([5, 10]) + bytes(32), bytes(64))
    for op, length in [(6, 129), (7, 97), (8, 191)]:
        add(f"bn254-reject-length-{op}", bytes([5, op]) + bytes(length), error="InvalidArgument")
    for index, (base, exponent, modulus) in enumerate([(2, 10, 17), (17, 0, 19), (limit, 123, 65537)]):
        add(f"modexp-{index}", bytes([5, 11]) + struct.pack("<QQQ", base, exponent, modulus),
            pow(base, exponent, modulus).to_bytes(8, "little"))
    for modulus in [0, 1, 2, 18]:
        add(f"modexp-invalid-modulus-{modulus}", bytes([5, 11]) + struct.pack("<QQQ", 2, 10, modulus), error="InvalidArgument")
    add("stack-top-level", bytes([7]), bytes([1, 1, 0]))
    for mode in [0, 1]:
        for payload in [b"", b"hopper receipt\x00\xff"]:
            add(f"receipt-{mode}-{len(payload)}", bytes([8, mode]) + payload)
            result[-1]["logSegments"] = ([bytes([29]).hex()] if mode else []) + [payload.hex()]
    return result if big_mod_exp else [case for case in result if not case["name"].startswith("modexp-")]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--sha512", action="store_true")
    parser.add_argument("--big-mod-exp", action="store_true", help="requires an implemented, active modular-exponentiation syscall")
    parser.add_argument("--blake3", action="store_true", help="requires an active BLAKE3 syscall")
    args = parser.parse_args()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(cases(args.sha512, args.big_mod_exp, args.blake3), indent=2) + "\n", encoding="utf-8")
    print(f"{len(cases(args.sha512, args.big_mod_exp, args.blake3))} known-answer cases")
