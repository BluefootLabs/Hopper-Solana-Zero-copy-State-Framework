# Canonical PDA validation, 2026-09-23

Source `3d18085cbf92fa374df9cdd61e96a7891694ffab` passes the compiled
SDK differential matrix on SBF v0 and v3. Every mode accepts the canonical
address and bump, rejects all 255 other bump values and every valid off-curve
noncanonical address, and leaves complete account snapshots unchanged.
Seven additional seed-boundary cases exercise 12 public PDA APIs each.

| Fixture mode | SBF v0 CU | SBF v3 CU |
|---|---:|---:|
| Runtime canonical search | 1,074 | 1,076 |
| Build-time canonical constant | 71 | 71 |
| Generated unchecked binding | 1,061 | 1,064 |
| Explicit validation followed by binding | 2,069 | 2,070 |
| Public per-field validation | 1,080 | 1,082 |
| Typed binding | 1,118 | 1,121 |
| Typed seed helper binding | 2,120 | 2,117 |

The v0 ELF is 8,288 bytes; v3 is 7,784 bytes. All modes are in the same
artifact, so these sizes are not the standalone cost of any individual API.
Mode 3 intentionally repeats validation; it is a synthetic comparison.
The lifetime correction adds 16 v0 ELF bytes and 10 CU to runtime search
against the retained `dc2274c` baseline. The constant path remains 71 CU.
This fixture performs no application writes and does not rank frameworks.

Devnet program `8RJxAyfAMnpb5ghwA4comPDJw6KqbDmZ28LDZHcccaVH` uses
the tested v0 ELF. All 25 transactions finalized: for modes 0 through 4,
canonical success, wrong bump, wrong address, malformed data and missing
account. Live compute matches the local v0 results. The deployed bytes match
the local ELF before and after; full account snapshots, including absent
accounts, remain unchanged. Typed modes 5 and 6 are local SBF tests only.

The archive retains raw public RPC transaction/snapshot responses, sender
logs, receipts and hashes. `program-after.json` is a final observation; it
does not establish unchanged upgrade authority throughout the run. RPC
responses are not independently authenticated ledger proofs. Private keys
and generated build keypairs are excluded. The ELF can be rebuilt from the
source and commands in `bench/canonical-pda/README.md`.

The host suite at this source passed 2,214 tests with zero failures and 225
ignored tests/examples; ignored SBF cases were invoked explicitly above.
Warnings-denied workspace Clippy passed. Later CLI-only release-check fixes
have separate regression evidence and do not change these on-chain binaries.
