# Token receipt and host-runtime validation — September 26, 2026

Published hopper-runtime 0.4.3 and hopper-solana 0.4.1. Native stays 0.4.2;
framework/CLI stay 0.4.0. Package checksums and 74 packaged Rust/README files
match publication commit 05afb389d2e4bcbbc6d6a21a16de8c9b5b0b2717. A registry-only
payout consumer compiles with framework 0.4.0 and no Pinocchio dependency.

The same clean source commit produced 34 finalized devnet transactions with
complete expected account snapshots: classic and Token-2022 transfers, 1% fee
withholding, 13 expected refusals, and seven rollbacks after successful nested
token CPIs. A reordered System CPI leaves its unrelated extra account unchanged.
The deployed v0 ELF matches the local binary before and after the run.

Local compiled v0/v3 tests, changed-crate and framework/core tests, Clippy, API
docs and the 29-public-package unsafe scan passed. Two host-only regressions
are recorded: positional emulation of deduplicated infos and mutable-guard
provenance. Both fail before correction and pass afterward, including Miri.
The snapshot checks net base-token balance changes and explicit receipt policy;
it does not supply authorization, extension screening, hooks or confidential
balance support. No universal speed or whole-framework security claim is made.

Use scripts/test-token-outcomes-devnet.py with the compiled fixture in
bench/token-outcomes/program. It spends devnet SOL and requires a clean source
checkout, an existing deployed matching ELF, and an authorized payer.

Source review was targeted. research/sources.json records refreshed upstream
heads and fetched-file hashes; downloaded files are not automatically audited.
Private keys, deployment recovery logs, crate archives and HTML are excluded.
The website checks cover rendered HTML, story order, routes and heading links;
they do not establish visual browser QA. Hosted-CI status, when captured, is
reported separately from the local results.

All four checked API pages, including the prior 0.4.2 pages, return HTTP 200.

Hosted GitHub workflows did not start: their annotations report an account
billing lock. The local test passes above are independent of those blocked jobs.

Production website c194f3699f383981487bede5e4b1fdc2fa1b8e4d passed all 51 pages
and 4439 internal links/heading targets.
