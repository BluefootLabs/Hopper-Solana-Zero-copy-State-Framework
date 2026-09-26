# Native execution and multisig validation — September 26, 2026

This record contains **81 finalized devnet transactions**, including 30 expected
refusals: 44 treasury/multisig/native transactions, 33 classic SPL Token escrow
transactions, and four CPI return-data transactions. Each checks complete expected
state for the accounts observed by its runner, including fee-payer changes.
Treasury timestamps are bounded by independently sampled finalized Clock values;
all other checked bytes and balances match exactly.

The first two suites use clean source `0ef6f2a694a2dfe51cc13c1425e606d1a8b6746f`.
The return-data suite and published native/runtime 0.4.1 patches use
`85eb40a378ed71adaeb7034164ebd8daf1b47ae3`. Rebuilding the four earlier programs
at the latter source produced byte-identical v0 ELFs, recorded under `lineage/`.
Every tested deployment matches its local ELF before and after the run.

Local compiled SBF v0/v3 tests include exact payout window boundaries, replay,
revocation, stale membership/threshold policy, rent protection, token settlement
and rollback. Devnet covers the cases named in its receipts, not every local test.
The return-data baseline accepts a nested producer and fails the new regression;
the patched v0/v3 binaries reject it. Direct valid, short, and empty results are
tested separately. Native panic and allocation failures terminate immediately.

The bounded multisig implements fixed SOL payouts and same-transaction approvals.
It does not implement asynchronous voting, arbitrary CPI proposals, token payouts,
recurring allowances, or a perpetuals adapter. Any submitter can execute an approved
payout within its on-chain bounds; a transaction still has to be submitted.

Registry archives match their API checksums and all packaged Rust source/README
files match the publishing checkout. A registry-only consumer compiles both patches.
The framework crate remains 0.4.0. This is an internal validation record, not an
independent security audit or a universal performance comparison.

Reproduction runners are `scripts/test-governance-devnet.py`,
`scripts/test-token-escrow-devnet.py`, and `scripts/test-return-provenance-devnet.py`.
Source research is separate from product documentation. The dated Alpenglow
observation records cluster activation rather than inferring it from a proposal.
Private keys, raw deployment logs, and third-party source copies are excluded.
