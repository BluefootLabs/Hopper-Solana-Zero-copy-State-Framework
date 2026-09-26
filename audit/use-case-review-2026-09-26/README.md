# Hopper use-case and DX review — September 26, 2026

The homepage now leads with vaults, quotas, and trading examples, followed by
Rust authoring and the controls the application can use. The benchmark tables
remain separately dated. See [positioning and claim boundaries](../../docs/MARKETING.md).

## Code and documentation corrections

The quick-start deposit previously changed only a balance field. It now shows
the actual tested System transfer, required account, authority constraint, and
error propagation. Install guides use 0.4.0 and the homepage starts a new project
in the correct order: install CLI, scaffold, build. Repository crate READMEs
were corrected; immutable published 0.4.0 archives were not replaced.

A public RPC rate limit stopped the first test attempt before any test
transaction was sent. The program deployment had completed. Bounded retries
were added for an explicit read-method allowlist; submissions, unknown methods,
permanent HTTP failures, and JSON-RPC errors are not retried. Four tests pass.
The existing deployed program was reused for the successful run.

## Fresh devnet results

Test source: 7b9739f6bde5b56af4dc0d6fa1af9968f100f5e2.
Program: 3ygR7zvgaSSH6PEuEg643eoVQahGSBHX17qSHwxVxbct.
Artifact SHA-256: 6d18fea5d19d805dacc279c416346a570f33e7f8d0e43512bb249cb62b86a8cc.

All 40 transactions finalized with exact expected account snapshots, including
all four quota slots, wrong delegates, stale revisions, insufficient quota,
invalid privileges, malformed input, aliases, and reinitialization refusal.
Consumption: 889 CU. Limit update: 831 CU. Before/after program dumps match the
published-registry consumer artifact and the 0.4 release gate artifact.
The deployment receipt proves executable source was unchanged; the retry fix
changes the Python harness, not the on-chain program.

## First-build check

An isolated project was created with the 0.4 CLI and no Git initialization.
Its generated dependency uses crates.io Hopper 0.4.0. The actual `hopper build`
path completed with the installed SBF tools (v1.54, v0), producing a
5,768-byte artifact. The test environment explicitly selected its
installed host Rust toolchain so it could coexist with the separate SBF
toolchain home; two setup attempts preceded the successful capture.
This verifies scaffolding and compilation, not execution of the minimal scaffold.

## Scope

Competitor README and selected authoring sources were pinned September 25.
Their manifests record exact URLs and hashes. This is not a whole-source audit
or a new cross-framework benchmark. Existing Cicada, Grillo, and placement
compiler limitations remain unchanged. Public receipts exclude private keys.
Browser inventory was empty, so visual QA is not claimed; website build,
lint, and rendered-link checks are recorded separately.
