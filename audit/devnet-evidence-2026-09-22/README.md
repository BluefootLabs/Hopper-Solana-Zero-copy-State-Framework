# Devnet evidence, 2026-09-22

[`runtime-gate-ad1e209/`](runtime-gate-ad1e209/README.md) repeats the 12-case
transaction run from clean `ad1e209`, with a freshly rebuilt identical ELF
and full before/after snapshot JSON for every policy case.

[`runtime-gate/`](runtime-gate/README.md) records the later policy fixture at
`fefc94b`: 12 finalized transactions, six exact refusals with unchanged
snapshots, and matching local/before/after deployed ELF bytes.

`round4/` preserves the completed round-4 lanes started from the previous
handoff. Both bind to source commit
`03adc6d615a1ceb1bcb989acb8a01835328453e7`, before the installation-registered
gate change. They are evidence for that commit, not the newer optimization.

| Lane | Program | Finalized transactions | Result |
| --- | --- | ---: | --- |
| escrow | `ADjgLVeqM4t64bFuR5JKYhYw2JdsNcBWuQ3judJn7s4N` | 5 | pass |
| devnet-audit | `F42uSNm8WgnMoKWuKKtqEiKmeeayF9iVSD7NExzoDuJ8` | 15 | pass |

The original bundle checksum lists and their hashes are retained. Every
source bundle file, including its before/after on-chain and local ELF, was
checked against its SHA-256 before archival. ELF files are omitted here;
their hashes remain in `SHA256SUMS` and `provenance.json`. JSON records
contain public account data and receipts. No signing keypairs are included.
