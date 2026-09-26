# Hopper program measurements

Measure the complete program you intend to deploy: compute units, executable
size, account space, successful balance changes, refusals, and rollback.
Numbers below are dated fixtures, not guarantees for other applications.

## Funded token escrow: September 26, 2026

The classic SPL Token escrow finalized 33 devnet transactions: setup, funding,
settlement, cancellation, surplus donations, and ten expected refusals. Every
transaction matched its complete expected account state. The deployed ELF
matched the locally tested artifact before and after execution.

| Operation | Compute units |
|---|---:|
| Create and fund first offer | 7,548 |
| Create and fund second offer | 7,873 |
| Take offer, refund surplus, and close accounts | 8,731 |
| Cancel, refund vault, and close accounts | 4,773 |

The v0 executable is 41,656 bytes. These fixture measurements include actual
classic-token CPIs and the example's account/mint restrictions. They do not
cover every token extension or every market design.

[Signatures, snapshots, hashes, and compiled tests](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/token-escrow-governance-2026-09-26)
provide the evidence behind the numbers.

## Named SOL vault: September 25, 2026

The 0.4.0 named-initialization vault finalized 19 devnet transactions with exact
account-state checks. Deposit measured 1,602 CU and withdrawal 240 CU. Read the
[release validation](docs/RELEASE_0_4_VALIDATION.md) for source and artifact scope.

## Reproduce and evaluate

1. Pin program source, dependencies, compiler, SBF target, and cluster features.
2. Build the actual ELF and execute it with realistic account state.
3. Check resulting balances, ownership, data, account closure, and failure rollback.
4. Record successful and rejected instruction costs separately.
5. Hash the deployed artifact and retain finalized transaction receipts.

Optional features and validation choices change cost. Lower compute alone does
not establish correct authorization, token handling, or a safe lifecycle.
Instruction fees, priority fees, and account rent are distinct costs.
