# Native instruction inspection — September 27, 2026

Published native 0.4.4 and runtime 0.4.5; framework/CLI remain 0.4.0 and Solana
integration 0.4.1. Registry checksums and 87 packaged Rust/README files match
publication commit f642abad62034fd688c493991c9d123e05f03388. A registry-only consumer compiles
the new inspection API, existing token payout API, and framework macros together.

The sibling syscall returns one for found and copies only at exact queried data
and account lengths. The old wrappers used the opposite result convention and
incorrect buffer lengths. Published native 0.4.2/runtime 0.4.3 reproduce four
regressions in compiled SBF. The correction adds a no-allocation caller-buffer
reader exposing account identities/privileges and distinct capacity errors.

All 11 scenarios pass on SBF v0 and v3 and in finalized devnet transactions,
including two intentional capacity refusals. Complete payer/program snapshots
match, and the deployed v0 ELF matches before and after. Other cases cover all
three convenience readers, missing and empty instructions, reverse sibling
order, child exclusion and 1,300-byte CPI data. This is a trace-read fixture;
it does not claim token transfers or full precompile authorization coverage.

Implementation source: 228e24225cd858c78ba4d9d11b460a3785bbdbaa.
Devnet program: 4b9CU3PwSDiAj7TZHAJfBLoJ2vPumfQ8tkNYZhe7NZpT.
The first publication was native 0.4.3/runtime 0.4.4. Final versions correct
packaged README wording only; the Rust implementation is unchanged and rebuilt
v0/v3 ELFs are byte-identical. documentation-refresh/lineage.json records this.

Host tests, Miri contract tests, framework/core tests, Clippy, API docs and a
clean-source 29-public-package unsafe-comment scan passed. The inventory counts
review surfaces; it is not a security proof. Hosted GitHub workflows did not
start because their annotations report an account billing lock.
The current 29-package metadata and package-boundary gate also passed; it did
not republish the unchanged framework or request a registry dry run of the train.

Use scripts/test-sibling-introspection-devnet.py with a deployed matching ELF,
a clean source checkout and an authorized devnet payer. It spends devnet SOL.
bench/sibling-introspection describes the fixture and compiled-SVM lane.

Research coverage is bounded and explicit in research/review-scope.txt. Captured
source heads and file hashes do not mean every downloaded line was reviewed.
The finalized network snapshots check genesis, Feature ownership and activation
encoding for 22 gates per cluster. Alpenglow is active on devnet/testnet and its
mainnet gate is absent in this capture; this is not a latency measurement.

Production website 36a52b4a813eebaf6ae4f14a29efa30c7450cba9 passed 52 pages and
4580 internal links/heading targets. Checks cover rendered
HTML, story order and release/network facts, not visual browser inspection.

Private keys, raw deployment logs, crate archives, third-party source files,
and rendered HTML are excluded. SHA256SUMS covers every archived artifact.

All four API documentation pages returned HTTP 200.
