# Checked borrowed arguments: Solana validation

Run date: October 2, 2026 UTC. These are pre-release working-tree results.
The baseline commit is `52b70b8ae009004f951b5db784d99d9bcf284812`;
[`devnet/source.json`](devnet/source.json) records hashes of the Rust, TOML,
Python, and lockfile inputs, including uncommitted changes. It is not a claim
that the baseline commit alone contains the tested implementation.

## Devnet

Program: [`CYybStqY8MAmiV2CyaUBU2WSKGd7CPA2yKPPnVw1jamN`](https://explorer.solana.com/address/CYybStqY8MAmiV2CyaUBU2WSKGd7CPA2yKPPnVw1jamN?cluster=devnet).

- 20 finalized transactions: account creation, five successful updates, and
  14 expected refusals. The final state is count `5`, total `55`.
- Exact and checked-tail parsing accept the expected representations. An absent
  option ignores its payload. Valid tails include zero and 32 bytes.
- Invalid option and enum tags, truncation, surplus exact input, excessive tail
  length, invalid UTF-8, overflow, missing signer, read-only state, and foreign
  ownership return the expected errors. Every refusal preserves the test state.
- Successful calls return the expected bytes with this program as the producer.
- The SBF v3 binary is 3,656 bytes, SHA-256
  `3482129694cfd2ae201fde43d89ee8969953de7b79565d0626fa53f72cfa226d`.
  Both finalized program dumps match it byte for byte.

[`devnet/summary.json`](devnet/summary.json) lists all signatures, slots, errors,
and compute usage. The transaction and account receipts are alongside it.
Successful parser/update instructions used 218–397 CU in this run. This is a
small fixture measurement, not a framework-wide or competitor benchmark.

`fixture-v3.so`, `before-onchain.so`, and `after-onchain.so` retain the compared
bytes; `fixture-v0.so` retains the other locally tested binary. Build tools were
`cargo-build-sbf 4.4.0`, platform-tools v1.57. Deployment used the official Agave
4.3.0 CLI whose archive provenance is recorded in the
[previous tooling receipt](../token-boundaries-2026-09-30/tooling/agave-cli-provenance.json).

## Local checks

The [VM regression](../../bench/framework-comparison/verifier/tests/borrowed_args_sbf.rs)
passes with both SBF v0 and v3 binaries. Each run executes 1,051 instructions:
exhaustive values in four option/enum positions, all prefix truncations, tail
boundaries, authorization and account-shape failures, and count/total overflow.
It also verifies that rejected calls preserve every supplied account.

The native, runtime, derive, macros, and framework library suites passed 671
tests. The CLI readiness regressions passed eight tests. The fixture and runner
commands are in [the fixture README](../../bench/borrowed-args/README.md).

Clippy passed for the changed framework crates, CLI, and fixture with warnings
denied. `cargo check --workspace --all-targets --locked --offline` also passed.
The unsafe map is current at 855 sites, unsafe contracts passed for 324
Rust files across 29 public packages, and 219 documentation citations resolved.

`readiness.json` records `hopper audit-check --strict --json` against the
existing readiness dossier. It correctly exits `1`: old attestations and
changed evidence hashes need renewed review, and independent review has not
started. A duplicate dependency-audit identifier found by the new structural
check was corrected in the dossier. Existing dates and review claims were not
advanced by this run.

## Scope

The fixture asserts that its borrowed prefix and tail point into the original
instruction data and uses no allocator. It still copies scalar state values and
return bytes. Authorization, owner checks, tail semantics, and arithmetic remain
application checks; representation validation does not replace them.

No independent security audit or new registry publication is claimed. This run
does not refresh unrelated release attestations. Private keypairs and private
deployment transcripts are excluded. `SHA256SUMS` binds the archived evidence;
run `python scripts/verify-evidence.py` to check its bytes.
