# October 2 readiness verification

This archive records local execution receipts and finalized devnet observations
for the unreleased working tree based on commit
`52b70b8ae009004f951b5db784d99d9bcf284812`. It is not a clean release attestation,
an independent audit, or a new publication.

## Dossier result

The starting dossier had 27 blocking findings. The refreshed dossier closes 25:
stale review/evidence entries, missing historical planning documents, expired
local gates, and the obsolete 0.3 publication requirement. Independent review
and required clean-source hosted SBF runs remain open. Both SBF jobs in
[GitHub run 36812193185](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/actions/runs/36812193185)
were prevented from starting by an account billing lock; their annotations are
retained here. Transaction-v1 authoring remains a nonblocking implementation gap.

`readiness.json` is the resulting strict report. `manifest.json` is the exact
readiness manifest checked to produce it. The current repository manifest can
subsequently expire or detect source/evidence drift.

## Local execution

- Four schema 2 gate receipts bind commands, completion dates, exit codes, logs,
  and 733 Rust/configuration/script/diagnostic inputs. Source digests normalize
  CRLF to LF; log and receipt hashes cover exact bytes. These are self-recorded
  local results, not signed CI attestations.
- Full workspace tests report 2,486 passes, zero failures, and 278 ignored
  tests. `HOPPER_TOKEN_LAB_SBF` selected the explicit archived 0.5 token-lab v3
  ELF. Other existing tests can skip when default SBF fixtures are absent;
  this count must not be represented as complete compiled-runtime coverage.
- Full workspace/all-target Clippy and formatting pass. RustSec database commit
  `117edb3bed98e9be112f277b7615eea3252e7c43` reports zero vulnerabilities and four
  unmaintained informational advisories, with no ignored advisories.
- All 28 public library API locks match. The unsafe scan checks 324 Rust files
  across 29 public packages. The map contains 855 sites. Nine readiness tests,
  nine API-planner tests, and three gate/evidence-runner tests pass.
- Cicada's fresh manifest matches its checked-in 698-case plan; all cases pass
  the required business invariant with no skips. This is host semantic evidence.
- Fresh SBPF v0 and v3 argument-fixture binaries each pass the 1,051-instruction
  compiled-VM test. The retained ELF hashes match the earlier fixture archive.
- The 29-package local preflight passes in offline mode: metadata, dependency
  order, packaged README links, and archive closure. Archives use `--no-verify`;
  this is not a registry dry run or upload. The receipt fingerprints the tree
  before this evidence archive was finalized.

## Public devnet

The checked-argument fixture was rebuilt with `cargo-build-sbf 4.4.0`,
platform-tools v1.57, SBPF v3. Its 3,656-byte ELF has SHA-256
`3482129694cfd2ae201fde43d89ee8969953de7b79565d0626fa53f72cfa226d`.
The program was already deployed at
`CYybStqY8MAmiV2CyaUBU2WSKGd7CPA2yKPPnVw1jamN`; the rerun matched its bytes
before and after 20 finalized transactions, including 14 expected refusals.
The new state account is `3fAKZJdkxQweREPXpvkAYU3FMkW4d1mzW9QhThRPxWYK`.
Successful calls end at count 5 and total 55. Refused writes preserve the state;
successful return bytes and producer identity are checked.

`devnet/` contains source fingerprints, public transaction/account observations,
summary, and program dumps. It contains no keys or private deployment logs.
The fixture accepts any signer and does not establish application custody
authorization. RPC observations are not authenticated ledger proofs.

## Other observations

`network.json` records finalized feature accounts and rent reads on devnet and
mainnet-beta. `peer-pins.json` identifies the source revisions behind the
framework-boundary guide. `website-check.json` records served-page and internal
anchor checks; browser interaction and screenshot testing were unavailable.

Verify every file listed in `SHA256SUMS`, then verify that file against
`BUNDLE.SHA256`, or run `python scripts/verify-evidence.py` from the repository.
