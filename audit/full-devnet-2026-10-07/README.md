# Function and application verification, October 7, 2026

This archive records local checks and finalized public-devnet execution of the
unreleased working tree. It is not an independent security review, a hosted CI
attestation, or a package publication. The original 27-item dossier still has
two external blockers: independent review and required clean-source hosted SBF
jobs. Cluster compatibility and incomplete live coverage remain explicit.

## Implemented corrections

- Runtime hashes use the native implementation and its 20,000-slice ABI bound;
  the former runtime-only 16-slice limit is removed. The bounded function fixture
  uses at most 64 slices. Host SHA-256 computes a digest; raw host hash shims do
  not stand in for Solana execution.
- Modular exponentiation validates the little-endian, 512-byte operand contract,
  exact output size, and odd modulus greater than one before calling the runtime.
  No live success is claimed for this feature-unavailable syscall.
- Generated raw entrypoints export the account-count constant they reference.
  Receipt emission uses the native log-data wrapper, repairing the v3 link path.
- Epoch arithmetic saturates at integer boundaries and handles a zero epoch
  length. It consumes the actual schedule; it never substitutes RPC constants.
- The allowance tests require `PrivilegeEscalation` for an unsigned account at
  the System CPI boundary. The exact error and rollback pass in the VM and devnet.

## Execution scope

| Check | Recorded result |
| --- | --- |
| Workspace tests | 2,500 reported passes, zero failures, 280 ignored |
| Formatting / Clippy | Passed; all-target Clippy denies warnings |
| RustSec | 1,294 advisories checked; zero vulnerabilities, four unmaintained informational advisories |
| Public API locks | All 28 snapshots match |
| Unsafe map | 850 justified sites; 473 mapped to test calls by name |
| Fresh program builds | All 52 deployable workspace packages, SBF v0 and v3 |
| Compiled verifier matrix | 36 tests passed over the named fixture ELFs; native/runtime lifecycle also checked separately |
| Function lab, default | 106 known-answer/refusal cases; 116 finalized transactions including account/sysvar tests |
| Function lab, SHA-512 variant | 120 known-answer/refusal cases; 130 finalized transactions |
| Completed live suites | 677 unique finalized transactions, across 29 packages |
| Earlier identical ELF | Borrowed-argument fixture: 40 October 7 transactions on byte-identical v3 code |
| Website | Typecheck, lint, production build, 61 served pages and 5,402 internal links/anchors pass |

Workspace counts include tests that do not execute a compiled fixture. Building
a program does not test its handlers. The [package matrix](PROGRAM_COVERAGE.md)
and [machine-readable coverage](coverage.json) list the exact scope and receipts.
Twenty-two packages lack current devnet execution in this archive. Named suites
do not establish exhaustive handler, input, or feature-combination coverage.

The application receipts cover System CPI and SOL custody, bounded allowances,
dynamic tails, token escrow and token extensions, confidential transfers and
proof rejection, cross-program reads, nested CPI and return provenance,
instruction introspection, lifecycle operations, treasury/multisig policy,
layout migration and orderbook state. Expected failures count as checked
transactions only when their asserted outcome and state checks complete.

Function results are checked against independent known vectors, Python integer
arithmetic and `hashlib`; selected state cases compare the complete account.
Receipt tests compare exact base64 log segments and return-data tests check
both bytes and producer. Program dumps match the selected ELF before and after
each completed suite. These are recorded RPC observations, not ledger proofs.

## Open runtime findings

Both function runs have `functionChecksPassed: true` and `allPassed: false`.
The EpochSchedule account, generic getter and dedicated getter return 8,192
slots per epoch, while RPC returns 432,000 and Clock agrees with RPC. The
[dedicated probe](diagnostics/epoch-schedule/comparison.json) confirms the
inconsistency independently of Hopper's schedule helpers. Use Clock for the
current epoch; do not mask the discrepancy with hardcoded schedule values.

The [BLAKE3 probe](diagnostics/blake3) preserves both finalized
`ProgramFailedToComplete` results. CLI 4.3 initially rejected that syscall during
feature verification. A diagnostic deployment bypassed only local feature
selection, retaining transaction preflight; both native and runtime calls still
failed on chain. Local VM vectors pass. Accepted ELF deployment is not evidence
that a syscall can execute. The default live fixture excludes BLAKE3, and the
absent modular-exponentiation feature is not included in passing live counts.

## Source and reproducibility

Program builds use clean **local validation snapshot**
`a755515dc0bb33ce5dfa5c3a01159a0ba55c33b0`, based on repository commit
`52b70b8ae009004f951b5db784d99d9bcf284812`. It is not a hosted or released commit.
`source.bundle` contains the incremental Git objects and can be verified in a
clone containing the base with `git bundle verify PATH_TO_SOURCE_BUNDLE`.
Fetch the bundle into a separate review checkout to inspect the recorded
snapshot; do not replace a working tree containing unrelated changes.

`source-snapshot.json` records the copied inputs. The current build/check input
inventory differs by
the two exact-error test corrections retained in
[source-delta](source-delta/inventory.json). All production build inputs remain
identical. The four selected quality receipts cover the corrected current
inventory, while `prior-test-correction` retains the preceding snapshot checks.
Text hashes normalize CRLF to LF; artifacts, logs and archive files are
byte-hashed. The corrected allowance driver and its execution adapter have
separate provenance; the receipt's program-source commit remains the build
snapshot. The accompanying ignore-rule correction keeps public audit logs
eligible for staging, so a later checkout can retain the complete bundles.
No binary rebuild is implied by these test and repository-packaging corrections.

Builds use platform-tools v1.57; the local VM resolves Mollusk 0.15.1 / Agave 4.2.1
in the recorded Cargo.lock (the verifier manifest requests the 0.15 series).
Live commands use the separately verified Solana CLI 4.3.0 against public
devnet, whose observed nodes report 4.4.0-beta.0. The fresh Cargo target per build
prevents reuse of a stale SBF artifact. The original stale artifact, failed
boundary-vector run, clean rebuild and passing run remain in
[diagnostics/cache](diagnostics/cache/comparison.json).

RPC throttling interrupted initial concurrent runs. Read-only recovery checked
the already-submitted transaction and account state before any continuation.
The default function run revalidated its stateless prefix without resubmitting
transactions, then executed the remaining cases. The runtime-gate and allowance
suites were repeated on fresh accounts; their interrupted output and recovery
records remain archived. Completed-suite totals exclude interrupted attempts,
deployment transactions and diagnostic failures. The framework deployment
adapter selects RPC transport only. A token-lab CLI upload exhausted its retries;
verified sequential writes filled only the missing buffer ranges before the
final ELF comparison and deployment. The recovery checks the buffer's loader,
authority, size and complete bytes after every finalized write. Future test
deployments use the same bounded recovery when a CLI buffer upload times out.
Keypairs and private deployment transcripts are excluded from this archive.

Website snippets remain bound to their source hashes, and the revised function
section uses native disclosures, keyboard focus and reduced-motion styles.
Browser interaction and visual QA were unavailable; no website deployment is
claimed. Framework research pins and the October 7 network observation remain
in the [preceding archive](../borrowed-dispatch-2026-10-07/README.md).

Run `python scripts/verify-evidence.py` from the repository root to verify all
retained bundle hashes. Run `hopper audit-check --root . --json --strict` to
check current receipt/source correspondence and the outstanding release gates.
