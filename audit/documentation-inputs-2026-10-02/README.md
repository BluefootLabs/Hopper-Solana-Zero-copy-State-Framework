# Documentation and execution-model verification

This archive records local checks completed on October 3, 2026 UTC for the
working tree based on commit `52b70b8ae009004f951b5db784d99d9bcf284812`.
The directory uses the October 2 local work date. These results do not attest
a clean release, an independent security review, or a new publication.

## Scope

The execution guide, root and selected crate READMEs, and website now describe
Hopper as a Rust framework compiled to Solana SBF. They distinguish supported
zero-copy state views from decoded arguments, optional policies and receipts
from ordinary handlers, and local segment borrows from Solana account locks.
CPI borrow compatibility, account authorization, host simulation limits, and
published versus unreleased APIs are explicit.

The gate recorder and CLI verifier now inventory Markdown. Documentation can
be compiled by Rust through `include_str!` and documentation attributes, so
adding, removing, or changing it must invalidate an earlier check. The
recorder checks the inventory before and after each command. Source text
normalizes CRLF to LF; execution logs and receipts are hashed byte for byte.
Other file formats and the execution environment require their own evidence.

## Local checks

- Four receipt files use receipt schema 1 and are consumed by readiness
  manifest schema 2. Each binds the same 876 source inputs, including 143
  Markdown files, to its command, date, exit code, and log.
- Workspace tests report 2,486 passes, zero failures, and 278 ignored tests.
  The archived 0.5 token-lab v3 ELF was selected with `HOPPER_TOKEN_LAB_SBF`.
  Existing tests can skip when other default SBF fixtures are absent; these
  totals do not establish complete compiled-runtime coverage.
- The receipt-verifier Rust regression passes. Three Python gate/evidence
  tests pass, including documentation edits, additions, and removals during
  execution, line-ending normalization, and evidence exclusions.
- Formatting and workspace/all-target Clippy pass with warnings denied.
- The dependency audit reports zero vulnerabilities and four unmaintained
  informational advisories. It uses a clean export of RustSec commit
  `f8dee89e1b2f2f1eaf548312df7655fe5202a302` without fetching.
  `dependency-database.json` records all 1,307 database file hashes and checks
  that they remain unchanged during the command. The exact source export is
  `rustsec-db.tar.gz`; extract it to the `--db` path in the recorded command
  to reproduce the advisory set. `dependency-audit.log` contains the result.
- Documentation source citations resolve: 223 checked, zero unresolved.

`inventory-change.json` compares the new input set with the preceding
readiness archive. The non-Markdown changes in this pass are the gate recorder,
its tests, and the CLI verifier. No on-chain library Rust source changed in
this pass. No new SBF build, deployment, or devnet transaction is claimed here;
the preceding dated runtime and devnet evidence remains in its original archive.

## Website checks

TypeScript, ESLint, and the production build pass. HTTP checks cover 59 served
pages and 5,116 internal links and anchors with zero errors. Six displayed Rust
excerpts match their framework source and file hashes. A changed source fixture
correctly makes that check fail. The landing page uses those excerpts in its
state/accounts/handler walkthrough and labels the separate borrowed-input
illustration as unreleased.

`website-source.json` is a post-check source snapshot, not a before/after
execution attestation. Browser interaction and screenshot verification were
unavailable. The website was not deployed by this pass.

## Readiness

`manifest.json` records the selected evidence and gate receipts. `readiness.json`
is the strict report from the rebuilt CLI; `checks.json` records its exit code
and test-group counts. Independent review and the required clean-source hosted
SBF lane artifacts remain release blockers. Transaction-v1 authoring remains a
nonblocking implementation gap. The previous 27-finding report and subsequent
25 closures remain archived; these checks do not close either external gate.

Run `python scripts/verify-evidence.py` from the repository to verify the listed
files and bundle hash. These are local, self-recorded results, not signed CI
attestations.
