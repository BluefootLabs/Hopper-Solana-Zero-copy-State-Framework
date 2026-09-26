# Bounded multisig program

The [bounded multisig example](../examples/hopper-bounded-multisig) combines
bounded member storage, real transaction signatures, System Program deposits,
authorized SOL withdrawals, and threshold-approved payouts with bounded later execution.

Use `#[account]` for a threshold, `String<'a, 32>` label, and
`Vec<'a, Address, 10>` member list. Generated accessors keep the variable tail
within its declared capacity. Typed account bindings validate the state before
the handler examines membership.

A threshold counts distinct configured identities with actual signer privilege.
The example rejects duplicate initialization members and unrelated approval
accounts. Its management instructions authenticate the current threshold before
changing labels or membership. Withdrawal checks the amount and live rent reserve
before moving lamports.

See the example README for instruction payloads, account ordering, and validation
results. For broader proposal and voting programs, see
[governance programs](GOVERNANCE_PROGRAMS.md).

Payouts bind recipient, amount, inclusive Clock window, and policy revision. Any submitter can execute once; membership/threshold changes invalidate outstanding payouts. A threshold can revoke a payout and reclaim its rent to the multisig.

The September 26 devnet payout executed within its stored amount, recipient,
window, and policy revision without member signatures at execution. Replay,
revocation, and stale approvals were rejected. [Validation record](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/audit/native-multisig-2026-09-26).
