# Bounded multisig with SOL custody

A transaction multisig with threshold-approved, single-use SOL payouts. It creates a program-owned
account, accepts real SOL deposits, and requires the configured member threshold
for label changes, membership changes, thresholds, withdrawals, and payout approval.

Members sign the same transaction. Signer privileges are checked on chain and
matched to stored members; duplicate or unrelated approvals are rejected.
Deposits invoke the System Program. Withdrawals debit the program-owned account
and retain its live rent reserve. A receiving member can also approve through
the destination account role without repeating that account in the tail.

## Instructions

All tags are one byte. Integers are little-endian. Bounded strings and member
vectors use a **u16** length/count prefix in instruction data. The account's
outer dynamic-tail payload uses its separate **u32** length prefix.

| Tag | Payload after tag | Declared accounts, in order |
|---|---|---|
| 0 Rename | u16 UTF-8 length + label bytes, at most 32 | writable multisig; then member signer approvals |
| 1 Add member | 32-byte new member | writable multisig; then member signer approvals |
| 2 Initialize | u64 threshold + bounded label + u16 member count + member keys | writable payer signer, writable new multisig signer, System Program |
| 3 Withdraw | u64 lamports | writable multisig, writable destination; then member signer approvals |
| 4 Deposit | u64 lamports | writable depositor signer, writable multisig, System Program |
| 5 Approve payout | u64 amount, not-before timestamp, expiry timestamp | writable multisig, writable new payout signer, writable payer signer, destination, System Program; then member signer approvals |
| 6 Execute payout | empty | writable multisig, writable payout, writable destination |
| 7 Revoke/reclaim payout | empty | writable multisig, writable payout; then member signer approvals |
| 8 Invalidate all payouts | empty | writable multisig; then member signer approvals |
| 9 Remove member | 32-byte member key | writable multisig; then member signer approvals |
| 10 Change threshold | u64 new threshold | writable multisig; then member signer approvals |

Initialize permits one to ten distinct, nonzero member addresses, with a threshold
between one and the member count. The new account requires its own signature.
Management and withdrawal require the current threshold. Remaining approval
accounts must be distinct signers and members. A signing destination counts only
if it is a configured member. Extra unrelated remaining accounts are rejected.

The **392-byte v2** account holds a headered threshold and policy revision, plus a bounded tail containing the
label and member list. Setters update that tail within the allocated capacity.
The framework validates the account before handlers access its state.

## Approve once, execute within the approved limits

Members sign the payout-approval transaction together. It creates a **113-byte**
payout account binding this multisig, an exact destination, lamport amount, policy
revision, and inclusive `not_before <= Clock.unix_timestamp <= expires` window.
A configured member paying the payout rent counts as an approval without being
repeated in the remaining-account tail. The payer must be distinct from the
multisig, payout, and destination roles.

Anyone may submit execution and pay the transaction fee. Execution takes no
amount or destination override; the program checks the stored terms, available
funds above rent, and single-use flag before transferring SOL. Successful
execution retains a receipt and refuses replay. A threshold can revoke an unused
payout or reclaim a used receipt; all payout-account rent returns to the multisig.

Adding/removing a member, changing the threshold, or explicitly invalidating
payouts increments the policy revision. Outstanding payouts from older revisions
then fail without iterating over them. Removal cannot leave fewer members than
the threshold. Revocation takes effect in chain execution order: it cannot undo
a payment that executed first. Rename does not invalidate payout permissions.

Payout approval does not reserve funds. Multiple approvals can exceed the current
balance; an execution lacking funds fails atomically. Clock uses whole seconds,
and a transaction submitter is still needed to trigger execution. The authority,
payment limits, and replay checks run on chain; no approval service is required.

## Scope

This example holds SOL and executes fixed payments. For checked token custody
and exchange, see [funded token escrow](../hopper-escrow). Payout windows are not a
global governance timelock. Members do not accumulate votes across separate
transactions. The program has no arbitrary-CPI proposal executor, recurring
allowance, token payout instruction, oracle/slippage policy, or perpetuals adapter.
Those require their own explicit target, account, authority, and outcome checks.

Version 2 changes the state layout and ABI. Create a fresh account; no in-place
v1 migration is included.

## Validation

Actual SBF v0 and v3 tests cover initialization, funding, authenticated changes,
withdrawal, receiving-member approval, duplicate/unknown/unsigned approvals,
rent protection, overflow, payout time boundaries, replay, revocation, membership/threshold invalidation, and complete expected account state after refusals.
Run the compiled suite with `HOPPER_MULTISIG_SBF` and `HOPPER_TREASURY_SBF` set:

```sh
cargo test -p hopper-framework-verifier --test governance_sbf -- --ignored --nocapture
```
