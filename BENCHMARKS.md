# Hopper program measurements

Measure the complete program you intend to deploy: compute units, executable
size, account space, successful balance changes, refusals, and rollback.
Numbers below are dated fixtures, not guarantees for other applications.

## Hello world and PDA counter: September 30, 2026

pina's framework-comparison fixtures, built with its release recipe and run
once per instruction in Mollusk. The Pinocchio column is pina's own
hand-written fixture, rebuilt and measured by the same script,
`scripts/bench-framework-comparison.py`.

| Program | Hand-written Pinocchio | Hopper raw | Hopper framework |
|---|---:|---:|---:|
| Hello world | 111 CU, 3,160 B | 111 CU, 1,456 B | 127 CU, 1,824 B |
| PDA counter, create | 1,490 CU | 1,514 CU | 1,471 CU |
| PDA counter, update | 1,721 CU | 1,722 CU | 325 CU |
| PDA counter, program size | 6,512 B | 6,608 B | 8,744 B |

The raw counter is the same program as Pinocchio's: a 10-byte account, one
account-creation CPI, and a full PDA derivation on every update. The framework
counter checks the signer, the owner, the layout, and the PDA before its
handler runs, checks the PDA with one hash from the bump stored in the
account, and keeps a 16-byte header, so its account is 25 bytes.
[Coming from Pinocchio](docs/FROM_PINOCCHIO.md) lists the checks the raw
layer makes that Pinocchio leaves out.

## Funded token escrow: September 26, 2026

The classic SPL Token escrow finalized 33 devnet transactions: setup, funding,
settlement, cancellation, surplus donations, and ten expected refusals. Every
transaction matched its complete expected account state. The deployed ELF
matched the locally tested artifact before and after execution.

| Operation | Compute units |
|---|---:|
| Create and fund first offer | 7,222 |
| Create and fund second offer | 7,222 |
| Take offer, refund surplus, and close accounts | 8,403 |
| Cancel, refund vault, and close accounts | 4,118 |

The v0 executable is 41,704 bytes. These fixture measurements include actual
classic-token CPIs and the example's account/mint restrictions. They do not
cover every token extension or every market design.

[Signatures, snapshots, hashes, and compiled tests](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/native-multisig-2026-09-26)
provide the evidence behind the numbers.

## SOL governance and treasury: September 26, 2026

The shared governance/treasury/native run finalized 44 transactions, including
17 expected refusals. Multisig deposit measured 1,537 CU, direct withdrawal
1,301 CU, approved payout execution 679 CU, and treasury withdrawal 551 CU.
Payout execution needs no member signature after approval; its recipient,
amount, time window, revision, and unused state are checked on chain.
These costs apply to the recorded fixture's account and approval counts.

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
