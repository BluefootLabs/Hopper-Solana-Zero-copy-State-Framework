# SOL treasury with delegated spending

This program receives wallet SOL through a System Program CPI and pays authorized
withdrawals from its program-owned treasury account. An administrator controls
the operator, freeze status, per-withdrawal limit, and budget periods. The operator
can spend only within those rules, the live-Clock cooldown, and the available
balance above rent.

## Account layout v2

The account contains **169 bytes** with three independently validated headered
segments: core (56 bytes), permissions (57), and budget (56). The core stores the
administrator and cumulative deposits. Permissions store the operator, freeze
flag, and maximum withdrawal. Budget stores the limit, spent amount, period
number, cooldown seconds, and last withdrawal timestamp.

Version 2 is incompatible with the earlier treasury sketch. Create a fresh
account; this example does not include an in-place v1 migration.

## Instructions

Tags are one byte; all integers are little-endian u64. Payload lengths and
account counts are exact. `s` means signer and `w` means writable.

| Tag | Payload after tag | Accounts |
|---|---|---|
| 0 Initialize | budget, maximum withdrawal, cooldown seconds | payer (sw), new treasury (sw), System Program |
| 1 Deposit | lamports | depositor (sw), treasury (w), System Program |
| 2 Withdraw | lamports | operator (s), treasury (w), destination (w) |
| 3 Update permissions | action 0 + new operator key; action 1 alone toggles freeze; action 2 + new maximum | administrator (s), treasury (w) |
| 4 Advance period | new period number; optionally followed by new budget | administrator (s), treasury (w) |

A budget period is an administrator-controlled revision, not a Solana epoch.
Advancing it resets spent units and must strictly increase the number. It does
not reset the withdrawal cooldown. Initial administrator and operator are the
payer. The first withdrawal is allowed immediately; later withdrawals use the
live timestamp and configured cooldown. A zero cooldown disables that delay.

The operator chooses the recipient. This example has no recipient allowlist,
multisig administrator, automatic budget timer, token vault, or close instruction.
Use the [bounded multisig](../hopper-bounded-multisig) for member-approved SOL
custody and [token escrow](../hopper-escrow) for token transfers.

## Validation

The actual v0 and v3 ELFs pass compiled tests for wallet deposits, real
withdrawals, exact state and balance changes, authorization, every segment's
identity, cooldown boundaries, exhausted budgets, freezing, rent protection,
arithmetic overflow, and rollback on refusal. The same SBF suite exercises the
bounded multisig; see its README for the command.
