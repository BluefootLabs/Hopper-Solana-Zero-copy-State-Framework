# Hopper 0.3.2 on-chain byte-policy evidence — 2026-09-25

This maintainer capture records scoped tests, source observations, and public
devnet execution. It is not an independent audit or a universal ranking.

`gates/` preserves the final `gates-final` capture. Its receipt pins clean source
`2bb9d52221a101b76b88ce935983be5f50066c1e`. Every recorded
gate command exited successfully. Host test summaries report 2235
passed, 0 failed, and 237 ignored. Formatting,
warnings-denied Clippy, unsafe-comment checks, and provenance gates passed.
The separate Cicada semantic adapter reports 698 passed,
0 failed, and 0 skipped cases.

`source-lineage.json` proves the publication descendant against this gated
commit with an explicit changed-path allowlist. Policy source differs only in
full-line comments, whose removal yields identical code bytes; the devnet
harness permits only the exact readonly CLI-meta correction. Documentation
paths are listed individually. Registry consumer receipts separately record
the resulting ELF identities.

| Compiled execution capture | Passed | Ignored |
| --- | ---: | ---: |
| byte-allowance-v0-tests | 3 | 0 |
| byte-allowance-v3-tests | 3 | 0 |
| canonical-pda-v0-tests | 3 | 0 |
| canonical-pda-v3-tests | 3 | 0 |
| cicada-lifecycle | 23 | 0 |
| mint-plan-v0-tests | 5 | 0 |
| mint-plan-v3-tests | 5 | 0 |
| orderbook-v0-tests | 2 | 0 |
| runtime-gate-v0-tests | 1 | 0 |
| runtime-gate-v3-tests | 1 | 0 |

`gates/` includes the 12 exact tested ELFs and command transcripts.
The orderbook suite checks full state changes and requires initialization to
fit within 200,000 CU. Program-specific behavior and limits remain documented
in the corresponding example READMEs at the pinned source revision.

The completed `devnet/` lanes record 62 finalized transactions. Byte allowance has
40, including two distinct delegates, initialization
refusals, full expected snapshots including its payer and System Program, and
selected-cell writes. Runtime gate has 20, including
typed cell and ambient policy cases; its snapshots cover its two fixture accounts.
Both lanes verify exact deployed ELF bytes before and after execution. Each
receipt retains its actual source revision. Orderbook includes 2 finalized transactions: initialization and one bid. It proves the focused bid isolation check, not a complete trading lifecycle or exchange.

`devnet/byte-allowance-devnet-attempt1/` separately discloses 2
earlier successful System Program funding transfers from the aborted first
harness attempt: 2,000,000 lamports of funding plus 10,000 lamports of fees.
They are excluded from the 62-transaction completed-lane total and
publication prerequisite. The completed lanes and prior attempt therefore cover
64 transactions; these are not 64 application test cases.

Surplus account metas may be ignored by the bounded entrypoint. The allowance
suite verifies that this leaves extra accounts unchanged; `sealed` does not
promise exact account-count rejection. Allowance instruction payload lengths are exact.
Byte-range grants operate inside program execution; Solana's writable-account
locks remain account-wide. Allowance units are application quotas, not tokens.

`comparison/` pins clean source `38a247851dacb24c6b7aac1d54523637268a2026` and the recorded
Pina build recipe. Six local ELF measurements and the regression check are
included. Pina, Quasar, and Anchor peer rows remain pinned published values,
not fresh measurements of their newest source heads. See `comparison/RESULTS.md`.

`research/` contains dated source-head metadata, a reviewed comparison summary,
and finalized public RPC observations for devnet, testnet, and mainnet-beta.
Source support and proposal status do not prove cluster activation. These are
endpoint observations, not ledger proofs or a full line-by-line competitor audit.
Raw third-party source, HTML, and full compare patches are excluded.

Publication and registry-only consumer evidence is in
[`../registry-publication-2026-09-25`](../registry-publication-2026-09-25/).
No keypairs, recovery material, registry credentials, or build trees are included.
Website validation is a separate final deployment capture; this archive helper
does not assert visual browser QA or hosted CI success.
