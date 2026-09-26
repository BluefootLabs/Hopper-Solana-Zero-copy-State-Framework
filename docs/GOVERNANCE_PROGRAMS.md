# Governance, DAOs, and multisig treasuries

Hopper is a Solana program framework. Governance programs can use its typed
accounts, zero-copy collections, signer/PDA checks, token CPIs, and declared
write boundaries. A complete multisig still needs proposal, voting, execution,
membership, and recovery rules implemented by the application.

## What Squads teaches us

This September 26 review inspected Squads v4 at
[af94153](https://github.com/Squads-Protocol/v4/tree/af94153ff77a28b6effe46b9c94baaa93742b48c),
including proposal votes, vault execution, spending-limit use, and their state.
It did not reproduce Squads tests, deployments, audits, or performance results.

| Application requirement | Observed Squads source behavior | Consequence for Hopper |
|---|---|---|
| Approvals from distinct members | Proposal approval rejects a repeated vote | Reject duplicate member configuration and count authenticated identities once. A list of public keys alone is not signature evidence. |
| Delayed execution | Vault execution requires an approved proposal, executor permission, and elapsed time lock | State layout cannot substitute for proposal and Clock checks. Test before, at, and after the boundary. |
| Policy changes | Previously approved vault proposals may execute after becoming stale | Make grandfathering versus revocation an explicit product rule. Neither behavior is universally correct. |
| Delegated spending | Limits bind the vault, mint, allowed members, and optionally destinations; usage transfers real assets | A quota counter is only one part of a treasury allowance. Validate destinations and move funds in the same atomic instruction. |
| Membership changes | Spending-limit members are deliberately independent of multisig members | Removing a voter does not automatically revoke a spending delegate. Document the relationship or explicitly couple the policies. |

These observations come from
[proposal state](https://github.com/Squads-Protocol/v4/blob/af94153ff77a28b6effe46b9c94baaa93742b48c/programs/squads_multisig_program/src/state/proposal.rs),
[vault execution](https://github.com/Squads-Protocol/v4/blob/af94153ff77a28b6effe46b9c94baaa93742b48c/programs/squads_multisig_program/src/instructions/vault_transaction_execute.rs),
[spending-limit state](https://github.com/Squads-Protocol/v4/blob/af94153ff77a28b6effe46b9c94baaa93742b48c/programs/squads_multisig_program/src/state/spending_limit.rs),
and [spending-limit execution](https://github.com/Squads-Protocol/v4/blob/af94153ff77a28b6effe46b9c94baaa93742b48c/programs/squads_multisig_program/src/instructions/spending_limit_use.rs).

## Available Hopper building blocks

- The [bounded multisig data example](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/quasar-port-20-min)
  stores member and label tails. This review fixes duplicate members counting
  toward a threshold, rejects malformed duplicate storage, and prevents its
  removal helper from reducing membership below the configured threshold.
  Its approval helper consumes caller-supplied keys; it is not an authenticated
  proposal executor or a replacement for Squads.
- The [byte-allowance program](https://hopperzero.dev/docs/byte-allowance)
  demonstrates delegated limits, stale-request rejection, and scoped usage
  writes. It accounts for application credits, not treasury token custody.
- [Funded token escrow](https://github.com/BluefootLabs/Hopper-Solana-Zero-copy-State-Framework/tree/main/examples/hopper-escrow)
  demonstrates PDA custody, checked transfers, and atomic settlement. Its
  classic-token policy does not automatically support every Token-2022 extension.
- Hopper's CPI and state tools let an application compose with existing programs.
  There is no shipped Squads adapter or complete DAO governance template claimed
  by this guide.

## A useful on-chain direction to test

Keep immutable proposal terms, mutable approval records, and treasury allowance
usage in clearly separated fields or accounts. A vote instruction should only
change that voter's approval state; an allowance spend should only change its
usage accounting and the explicitly authorized token balances. Hopper's tracked
write policies can help enforce the state part inside the program.

For a product that wants immediate revocation, bind proposals or spending grants
to a configuration revision and check it during execution. For a product that
honors earlier approvals, store the approved policy snapshot and define how
changes affect it. Both approaches require actual signer checks, proposal
commitment checks, replay prevention, and destination validation.

Validate the exact target program, accounts, privileges, instruction data, and
PDA signer scope before executing an approved CPI. Hopper's tracked byte
policies are not a general sandbox for arbitrary downstream programs. No CPI
should receive governance authority merely because a proposal exists.

This is an implementation direction, not a new shipped governance engine or
a novelty claim. Measure a bounded prototype against an equivalent workload
before claiming lower compute, smaller accounts, or a safety advantage. Solana
still locks whole writable accounts; multiple approval cells in one account do
not create parallel transaction execution. Separate approval accounts may reduce
contention but add rent, account metas, and lifecycle complexity.

## Required proof before promoting a governance example

Exercise real treasury transfers plus duplicate-vote/member rejection, stale
configuration behavior, altered proposal data/accounts, early execution,
repeated execution, delegate revocation policy, allowance exhaustion/reset, and
rollback after a failed downstream CPI. Capture full expected state and token
balances on compiled SBF and devnet. Broader DAO requirements such as token-weighted
voting, delegation, quorum, and vote escrow need their own rules and tests.
