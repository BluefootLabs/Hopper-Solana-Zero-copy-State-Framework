# Hopper 0.5 token boundary validation

The final v3 token lab completed **59 finalized devnet
transactions**, including 10 expected refusals. Both deployed ELF dumps
match the tested binary. Program: `39rZWxxKvbrdmk21xsLvjWBXpZcgPoqjY83NRNUgATHd`.

- Program build commit: `9150e75f9329a7f43a65a1ce2d4c29281177a077`.
- Devnet runner commit: `6042bfc30a581734207b16671be27f1e48386e3c`. Program build inputs are
  unchanged; the later commit strengthens the runner's assertions and capture.
- v3 ELF SHA-256: `e9c65d672d6db5d647a078fbacb6af8d29e646440c52d898d53d9f536fe24488`.
- Toolchain: platform-tools v1.57; deployment CLI solana-cli 4.3.0 (src:825efd18; feat:c9ad34d2, client:Agave).
  Normal feature verification and RPC preflight were enabled. The older local
  2.3.13 CLI refused the v3 ELF before creating any account; its attempts were
  replaced with the official checksum-verified 4.3.0 CLI.

## What was checked

The lab exercises token operations on SPL Token and Token-2022, selected mint
extensions, metadata, groups, wrapped SOL, UI amounts and multisig creation.
Batch round trips preserve both token accounts and use one token CPI. Batched
self-transfers fail before a token CPI and preserve the token account and mint.
Withdrawals preserve token data and credit the recipient by the expected amount
after transaction fees. Eight hook-list probes cover literal/PDA resolution and
malformed data; the valid PDA is compared with the official CLI's derivation.

`devnet/` contains transaction responses, both program dumps, the final receipt,
and 57 finalized account observations. Checks cover the explicit
fields and invariants in the runner, not every property of every touched account.
RPC responses are observations, not authenticated ledger proofs. The hook probe
tests parsing and resolution; it does not invoke a transfer-hook program or
establish list-account provenance.

## Local validation

The workspace command passed (2474 reported passes,
277 ignored tests). The token lab used the explicit final ELF;
other existing compiled tests may skip when their default fixture is absent.
Both final v0/v3 lab binaries passed their two compiled regression tests.
Clippy passed for the workspace and all targets. The API locks match all 28
libraries. Unsafe contracts, the 855-site map/review ledger, documentation
citations, evidence checksums, publication metadata and package closure passed.

Miri passed six batch tests and six borrow-reader tests. The five fixed-layout
code-generation cases produced their expected results. Cicada's host semantic
fuzz workflow passed 698 cases with its required business invariant; this is
host semantic evidence, not SBF execution. The fresh RustSec database found no
known vulnerabilities in the recorded lockfile and reported four unmaintained
upstream packages. The dependency receipt records the exact database and lock
hashes; it was captured during release preparation before the source commit.

`local/checks.json` retains the run history. The initial workspace failure used
an older default token-lab ELF; the successful rerun names the final fixture.
An unsafe scan observed a concurrent source commit and was rerun successfully
against a clean, unchanged checkout. These are targeted checks, not a complete
security audit or proof of parity with every other Solana framework.

## Research scope

`research/` records pinned upstream inventories and 22 feature accounts per
cluster, observed October 1 UTC / September 30 America/Chicago. Files fetched
are not automatically files reviewed. The scoped review is in
[`research/token-boundaries-2026-09-30.txt`](../../research/token-boundaries-2026-09-30.txt).

Keypairs, credentials and private deployment transcripts are excluded.

## Publication and website

On 2026-10-01 UTC, 28 new package versions were published from
`0a8827e177f225574f2285a58875d50e8059ffe7`. Each downloaded archive matches its
registry checksum, VCS commit, source files, and packaged README. The package
train covers 29 packages; unchanged `hopper-builtins` 0.4.0 was retained.
The registry-only consumer passed and all 28 versioned docs.rs pages
returned HTTP 200. `publication/` contains the public verification receipts.

GitHub Actions did not start because of an account billing lock; its public
annotations are retained. No successful hosted CI run is claimed.

Website commit `2d834872d41a859a420a3c89760897407bebc1e5` reached Vercel production.
The live check covered 55 pages and 5061 internal links
with no failures, including headings, current package commands, the token
example, release status, and the network observation. Local build and ESLint
passed. Browser screenshot QA was unavailable. `website/` records the scope
and exact deployment association.
