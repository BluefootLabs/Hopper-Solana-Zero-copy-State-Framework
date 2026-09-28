# Devnet evidence, 2026-09-28: the token lab, round eight

Public devnet (`https://api.devnet.solana.com`), signer
`4sbBUbY71JFeA4kJckBmNnTADiFu4jtu84Gzev52ZEhn` (the devnet-only payer and
upgrade authority of every lane in this directory). The program is
`examples/hopper-token-lab`, one instruction per SPL Token / Token-2022
builder family, built with `cargo build-sbf` in a clean worktree at commit
`244b72cf36c3e9590ec35f27b73869d249313b32` (local ELF SHA-256
`b90518a68479bbaabed395c783b39c600e8ba49eebe5bffd25202c7e0adbb430`, 53,968
bytes) and deployed fresh as `417akw6B2CcuTZFpePrZaSR5oyX3riBPtHmdePkjrAYJ`
at slot 505,156,238. `before-onchain.so` and `after-onchain.so` are the
`solana program dump` of the deployment before the first transaction and
after the last one; both equal the local ELF byte for byte.

The runner is `scripts/test-token-lab-devnet.py` at the same commit. It
sent 42 transactions (slots 505,156,293 to 505,157,222), re-fetched each at
`finalized`, and compared every touched account with the exact bytes it
expected: mint fields, token account fields, TLV entries and their lengths,
multisig members, and lamport balances against the live rent-exempt
minimum. Per token program (SPL Token, then Token-2022): `create-mint`
(`MintPlan`, no extensions), two `immutable-a/b` accounts sized by
`GetAccountDataSize` (165 bytes on SPL Token; 170 on Token-2022 with the
immutable-owner TLV), `mint-to` (`MintToChecked` through
`invoke_for_owner`), `batch` (two `TransferChecked`s in one `Batch` CPI,
balances unchanged after the round trip, exactly one token-program
invocation in the logs), `ui-amount` (`AmountToUiAmount` then
`UiAmountToAmount` on the returned string, 1,234,567 as `1.234567` and
back), `prefund` and `withdraw-excess` (`WithdrawExcessLamports` returns
the account to its rent-exempt minimum), and `multisig`
(`InitializeMultisig2`, 1 of 2). Then `wrap-and-unwrap` on SPL Token (a
native account created with `InitializeAccount3`, then `UnwrapLamports` of
400,000 of its 1,000,000 lamports) and the Token-2022 extension probes:
each of the eight plannable extensions alone, then the set grown greedily
in program order. `extended-mint-grow-0f` (default account state, pausable,
scaled UI amount, plus interest bearing) is refused by `MintPlan` at plan
time with `InvalidArgument` at 439 CU, before any CPI, because Token-2022
refuses that pair; `extended-mint-final` carries the other seven (mask
`f7`, 508 bytes, 29,294 CU). On that mint: an immutable-owner account whose
size includes the pausable-account TLV (174 bytes), `mint-to`, the UI
amount at multiplier 2 (`2.469134`), `pause-resume`, `update-multiplier`
to 3, and the UI amount again (`3.703701`), each round-tripping to the raw
amount.

Findings the live programs settled (`receipt.json`, `findings`): `Batch`
accepted by both programs; `UnwrapLamports` accepted by SPL Token; every
plannable extension accepted alone; interest bearing with scaled UI amount
the one refused combination.

`receipt.json` lists every signature, slot, error, and compute-unit count
plus the account addresses and findings; `*.transaction.json` are the
finalized transactions as the RPC returned them; `*.log` are
`hopper tx send`'s own output, unedited. `SHA256SUMS` covers every file
here except itself and `BUNDLE.SHA256`, which is its digest.
