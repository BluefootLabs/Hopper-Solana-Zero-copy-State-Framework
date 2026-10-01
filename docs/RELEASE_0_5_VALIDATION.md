# Hopper 0.5 validation

Hopper 0.5 coordinates the framework, native runtime, token builders, and CLI
on one API line. Two borrow-safety fixes change public signatures; upgrade
the Hopper crates together using the [migration guide](MIGRATION_0_5.md).
The support crates `grillo-manifest`, `grillo-verifier`, and `hopper-topology`
are 0.1.1; unchanged `hopper-builtins` remains 0.4.0.

**Published on crates.io**, checked 2026-10-01 UTC: 28 new package versions.
Downloaded archives match the registry checksums and publication commit
`0a8827e177f225574f2285a58875d50e8059ffe7`. A registry-only consumer compiles
the framework macros, token batches, fallible PDA search, hook parser, and
token payout receipts. All 28 versioned docs.rs pages responded successfully.
The unchanged builtins package remains available.

## Programs and token operations

The release includes shared SPL Token and Token-2022 builders, multisig
authorities, bounded token batches, fixed-size mint extension planning,
metadata, groups, and confidential-transfer instruction builders. Programs
can use typed accounts and handlers or call Hopper's own native account,
entrypoint, syscall, and checked CPI APIs directly. Applications supply
authorization, extension policy, custody, and replay protection.

The latest fixes reject writable aliases within an inner token-batch
instruction while retaining account reuse across instructions. A returned
encoder error leaves the batch unchanged. Transfer-hook list parsing now
selects the Execute entry and respects its TLV boundary, rejects reserved
kinds and invalid seed shapes, and preserves output on failure. The runtime
exposes fallible canonical PDA search without allocating a seed list.

Account-data seeds and pubkey-data hook entries still require application
code. The hook fixture tests parsing and resolution, not hook execution or
list-account provenance. This release does not establish complete API parity
with every other framework or a universal performance lead.

## On-chain validation, October 1 UTC / September 30 local

The final token lab completed **59 finalized devnet transactions**,
including **10 expected refusals**, at program
`39rZWxxKvbrdmk21xsLvjWBXpZcgPoqjY83NRNUgATHd`. The deployed v3 ELF matched before and after.

- Both token programs accepted the tested transfers, batches, UI-amount
  conversions, excess-lamport withdrawals, and multisig initialization.
- Batch self-transfers failed before token CPI and preserved the token account
  and mint. Successful batch round trips preserved both token accounts.
- Withdrawals preserved token data and credited the recipient by the expected
  excess after transaction fees.
- The extension, metadata, and group cases check the runner's explicit mint,
  token, TLV, return-data, and lamport invariants. Eight hook probes include
  independent PDA derivation and malformed-input refusals.

The source, binaries, transactions, 57 finalized account
observations, and limitations are in the
[token validation bundle](../audit/token-boundaries-2026-09-30/README.md).
RPC observations are not authenticated ledger proofs. These checks do not
cover every property of every touched account.

Earlier dated runs remain separate evidence: the confidential-transfer lab
completed 60 devnet transactions with real proofs and decrypted checks;
the escrow runner completed 33 transactions; the runtime lab exercised a
requested heap frame, SlotHashes, and panic locations. See the
[changelog](../CHANGELOG.md) and each example's evidence links for source pins.
Those historical fixtures were not all rerun for this token-boundary change.

## Local and release checks

The workspace test command reported 2474 passes and 277 ignored
tests; the token lab used the explicit final v3 binary. Other compiled tests
can skip when their default fixture is absent. Both v0 and v3 token-lab
binaries passed two compiled regressions each. Workspace Clippy passed.
Miri passed six batch tests and six borrow-reader tests. Five fixed-layout
compile cases behaved as expected. Cicada passed 698 host semantic cases.
The API locks, 855-site unsafe map and review ledger, documentation citations,
evidence checksums, and publication metadata checks passed.

The recorded RustSec database found no known vulnerabilities in this lockfile
and reported four unmaintained dependencies. These are targeted tests and
reviews, not an independent security audit. Package checksums, registry-only
consumer results, and docs.rs status are recorded in the release evidence.

GitHub Actions did not start for the publication commit: its annotations report
an account billing lock. Local checks and devnet results are recorded above;
a green hosted CI run is not claimed. The release evidence retains the annotations.

Builds used platform-tools v1.57. Deployment used the official checksum-verified
Agave 4.3.0 CLI with ordinary feature verification and preflight; the older
local 2.3.13 CLI could not parse this v3 ELF. Use a CLI compatible with the
SBPF version selected for your cluster.

[Earlier 0.4 validation](RELEASE_0_4_VALIDATION.md).
