# Governance, multisig, and treasury programs

Use Hopper to authenticate members, hold funds, and execute authorized payments
on Solana. Governance needs an executable permission policy as well as vote data.
The repository now contains two concrete SOL custody examples.

## Member-approved custody

[Bounded multisig](../examples/hopper-bounded-multisig) stores a threshold, a
bounded label, and up to ten unique member keys. Its instructions create and
fund the account, rename it, add members, and withdraw SOL.

Management and withdrawal authenticate real transaction signers against the
stored member set. Duplicate or unrelated signers cannot inflate approval counts.
The signers approve the exact instruction and destination in the transaction.
Deposits use the System Program; withdrawals debit the program-owned account
and preserve its live rent reserve.

## Threshold approval with bounded later execution

Members can approve a single-use SOL payout containing the exact destination,
amount, policy revision, and execution window. Once approved, any transaction
submitter can execute it without a fresh round of member signatures. The program
checks every stored term and preserves the multisig's rent reserve.

Successful execution records completion and rejects replay. A threshold can
revoke the payout. Membership and threshold changes invalidate outstanding
payouts by advancing one policy revision; no scan through proposal accounts is
needed. Revocation and execution obey transaction order, and approvals do not
reserve funds. Anyone submitting execution still needs to pay its transaction fee.

This offers a concrete on-chain authorization pattern for scheduled payments.
It is not yet an arbitrary trading or perpetuals executor: protocol-specific
adapters must enforce markets, target programs, spending caps, price/oracle bounds,
and post-execution outcomes. Members sign each approval transaction together;
this example does not accumulate votes across separate transactions.

## Delegated treasury spending

[Treasury](../examples/hopper-treasury) separates authority, permissions, and
budget state into validated segments. The administrator sets an operator,
freezes spending, changes the per-withdrawal limit, and advances budget periods.
The operator can withdraw only within the single-payment limit, remaining period
budget, available funds, and live-Clock cooldown.

Deposits transfer real wallet SOL through a checked System CPI. Withdrawals pay
the requested destination and retain rent. A budget period is an administrator-
controlled revision; it is not an automatically resetting Solana epoch.

## Token treasuries and voting programs

The same execution stack includes checked token transfers, PDA signing, mint and
authority constraints, and account lifecycle helpers. The [funded token escrow](https://hopperzero.dev/docs/token-escrow) demonstrates actual token custody,
atomic exchange, cancellation, surplus refunds, and account closure.
A token treasury must connect its approvals or allowances to those token CPIs
and bind the destination and supported mint policy explicitly.

For broader DAO programs, define the voting-weight source, quorum, proposal
commitment, execution window, replay prevention, and membership-change policy.
Deposits/locks can establish weight on chain. Snapshot proofs require a trusted
commitment policy and explicit data assumptions. Removing a voter and revoking
a spending delegate should be intentional rules in the application.

## State boundaries and scaling

Keep proposal terms, approvals, and usage accounting separately identifiable.
Optional tracked-write policies can restrict program-side updates to the
intended fields. A downstream CPI still needs exact target, account, privilege,
instruction-data, and PDA-authority checks.

Solana locks whole writable accounts. Separate approval accounts may reduce
contention but add rent and account metas. Byte ranges inside one account do
not provide independent transaction locks.

See each example's README and validation receipts for its exact supported ABI,
executed tests, and remaining product features.
