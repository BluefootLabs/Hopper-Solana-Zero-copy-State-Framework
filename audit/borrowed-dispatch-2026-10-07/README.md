# Checked borrowed dispatch: October 7 verification

These local results cover the working tree based on
`52b70b8ae009004f951b5db784d99d9bcf284812`. The source inventory contains 877
inputs, including Markdown, scripts, and configuration. They are not a clean
release attestation or an independent security review.

## Behavior under test

An `#[hopper::args]` layout can be borrowed as `&MyArgs` by a generated
handler. The decoder checks the fixed footprint and nested representations
before account binding or handler execution. Layout aliases preserve their
wire size in metadata. Scalars still decode by value; an explicit final
byte slice consumes a tail whose meaning the application must check.

Generated clients represent these layouts as fixed opaque bytes. TypeScript
builders enforce the exact width, including zero-byte array arguments, so a
short value cannot be silently padded and a long value cannot overlap the
next argument. The retained SDK probe checks encoded bytes, account privileges,
six malformed lengths for a 12-byte layout, and two malformed lengths for a
zero-byte array followed by a scalar. It uses `@solana/web3.js` 1.98.4.

## Compiled Solana execution

The manual and generated paths share the same update function and validation
workload. Each executes 1,054 instructions in the compiled VM tests, including
all byte values in four tag/payload positions, truncation, surplus input,
bounded and malformed tails, account privileges, ownership, account sizes,
and arithmetic overflow. Refusals preserve supplied accounts.

Both v0 and v3 pass: 2,108 instructions per architecture, 4,216 total.
The v0 ELF is 5,240 bytes; v3 is 4,432 bytes. The retained build/test records
bind each artifact to the pre-build source snapshot. The harness uses
Mollusk 0.15.0 and Agave 4.2.1; builds use platform-tools v1.57.

| Sampled workload | v3 manual | v3 generated | v0 manual | v0 generated |
| --- | ---: | ---: | ---: | ---: |
| Exact prefix, checked write and return | 256 CU | 258 CU | 259 CU | 261 CU |
| Prefix and 32-byte tail, checked write and return | 438 CU | 438 CU | 441 CU | 441 CU |
| Refused option tag | 48 CU | 52 CU | 48 CU | 52 CU |

These are fixture measurements, not a comparison with another framework or
a universal performance claim.

## Public devnet

Program `GBjEJhxFrMBjiKV6GerioYqaii7utAiSs2cUU6VpqCo4` ran the v3 fixture.
The final manual and generated suites each finalized 20 transactions, including
14 expected refusals. Each used a fresh state account and finished at count 5,
total 55. The runner checked complete state snapshots, exact instruction errors,
and return-data bytes and producer. Both paths matched their local ELF before
and after execution. The three v3 CU samples above also match devnet results.

The final fixed-byte client correction rebuilt to the identical deployed ELF;
both suites were repeated against the final source inventory. `deployment.json`
records that relationship. Keypairs and private deployment transcripts are not
part of this archive. These RPC observations are not authenticated ledger proofs.
This fixture does not establish coverage of every Hopper program or function.

## Host and website checks

- Workspace: 2,493 reported passes, zero failures, 279 ignored tests. The
  archived 0.5 token-lab v3 ELF was selected with `HOPPER_TOKEN_LAB_SBF`.
  Other tests can skip when their fixture is absent; these counts alone do
  not establish compiled-runtime coverage.
- Formatting and all-target workspace Clippy pass with warnings denied.
- All 28 public API locks match. The unsafe map has 855 justified sites;
  the unsafe-contract check covers 324 Rust files in 29 public packages.
- Documentation citations: 223 checked, zero unresolved. Three Python
  evidence/gate regression tests pass.
- RustSec: 1,294 advisories, zero vulnerabilities, four documented unmaintained
  informational advisories. The clean database export and all 1,313 file hashes
  are retained, with before/after equality checked. The recorded command uses
  the extracted contents of `rustsec-db.tar.gz` at its explicit `--db` path.
- Website source correspondence, TypeScript, ESLint, and production build pass.
  Eight Rust excerpts match their source hashes. HTTP checks cover 59 pages and
  5,113 internal links/anchors with no errors. Source hashes are unchanged across
  those checks. Browser interaction and visual QA were unavailable; the website
  was not deployed by this verification.

The first workspace build encountered a Windows executable lock while the
devnet sender was running. The passing receipts retained here were captured
after separating the sender executable from build output.

## Research and release boundaries

`peer-pins.json` identifies selected Pinocchio, Pina, Quasar, and Anchor v2 source
files examined on October 7. It is a scoped source comparison, not an exhaustive
competitor audit. Pina's current README also describes checked generated clients;
Hopper's opaque-byte client representation does not establish parity with its
structured models or full Codama pipeline.

`network.json` records finalized devnet and mainnet observations at
2026-10-07T15:25:48Z using feature addresses from pinned Agave source. Transaction
v1 is active on both observed clusters. Alpenglow is active on devnet; its feature
account was absent on mainnet. ABI register and CPI representation proposals in
the [October 1 Solana changelog](https://solana.com/news/solana-changelog-october-1-2026)
are not treated as activated runtime contracts.

The strict readiness report verifies 34 evidence entries and four current gates.
Independent review and the required final clean-source hosted SBF lanes remain
blocking. Legacy-only transaction authoring remains tracked. No new package
version or release is published here.

Verify the retained bytes with `python scripts/verify-evidence.py` from the
repository root. Source text normalizes CRLF to LF; artifact and log hashes
operate on exact bytes. The receipts are locally recorded execution evidence.
