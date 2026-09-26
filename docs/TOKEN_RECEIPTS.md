# Verify token payouts

An escrow, claim program, or treasury needs to know how many tokens a recipient
actually received. A transfer can succeed while a Token-2022 fee reduces that
amount. `hopper_solana::transfer::TokenTransferSnapshot` checks the result on
chain, using the source and destination accounts already in the instruction.
It needs no indexer or off-chain attestation.

## Set the receipt policy before transferring

Use `hopper-solana` 0.4.1 with `hopper-runtime` 0.4.3. After validating the
instruction's caller, configured mint, destination, and extension policy:

```rust,ignore
use hopper_solana::interface::interface_transfer_checked_with_program;
use hopper_solana::transfer::TokenTransferSnapshot;

let snapshot = TokenTransferSnapshot::capture(
    source, destination, &configured_mint, amount, minimum_received,
)?;
interface_transfer_checked_with_program(
    source, mint, destination, authority, token_program, amount, decimals,
)?;
let received = snapshot.verify()?;
// Credit application accounting with received.credited, in raw mint units.
```

For an exact payout, set `minimum_received` equal to `amount`. For a fee-bearing
token, choose an explicit minimum your application accepts. A transfer of 100
units that credits 99 passes a minimum of 99 and fails a minimum of 100. A zero
minimum explicitly permits a full-fee transfer with no recipient credit.

The snapshot retains the account references, releases its data borrows before
CPI, and re-reads both accounts afterward. It checks their token-program owner,
base account shape, initialized state, expected mint, and unchanged token
authorities. The source must lose exactly the requested amount; the destination
must gain between the minimum and that amount. Self-transfers, zero requested
amounts, and a minimum above the requested amount are rejected before CPI.

## Compose with the transfer your program needs

The snapshot is separate from the CPI builder. Use it around a direct, PDA-signed,
multisig, or hook-aware transfer. The basic interface helper above does not resolve
hook accounts. Supply those through the appropriate Token-2022 builder when a
mint requires them. Classic `hopper_runtime::token` builders always call the
classic SPL Token program.

This check measures net base-balance changes across the operation. It does not
authenticate the caller, select a safe mint, audit hook behavior, prove the
internal source of a credit, or read confidential balances. It also does not
convert raw units into interest-adjusted or scaled UI amounts. Keep application
authorization and extension screening explicit, and verify immediately after
the intended CPI.

## Propagate a rejected outcome

`verify()` returns an error. Propagate it with `?` to the instruction boundary
so Solana rolls back the transaction, including the preceding token CPI.
Catching or ignoring that error leaves the successful CPI's changes in the
current instruction. Host-only CPI no-ops cannot establish transfer behavior;
use a compiled SVM fixture or devnet for that check.

The compiled fixture is [token-outcomes](../bench/token-outcomes/program/src/lib.rs).
Its test runner checks complete account snapshots, including fee withholding and
rollback after a successful nested token CPI.

## Validation

The September 26, 2026 run finalized 34 devnet transactions with complete expected
account snapshots. It includes classic-token receipts, Token-2022 fee withholding,
13 expected refusals, and seven rollbacks after a successful nested token CPI.
The deployed v0 binary matched before and after; compiled v0 and v3 scenarios also
passed. [Inspect the evidence](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/token-outcomes-2026-09-26).
