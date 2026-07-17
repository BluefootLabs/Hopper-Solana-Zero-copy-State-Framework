# Cicada architecture and implementation status

This document describes the first executable Cicada vertical slice included in
Hopper 0.3.0. It records the security model separately from future product
ambitions so later work does not silently weaken the original constraints.

## Product boundary

Cicada is an on-chain protected-execution envelope. A user commits custody,
route, expiry, executor, maximum-input, and minimum-output constraints. A solver
may deliver the transaction through normal RPC, direct TPU, a Jito bundle, BAM,
or a future Cicada scheduler. Settlement depends only on the on-chain result.

The transport controls ordering and landing. It does not control the user's
settlement policy.

## Current account model

### `CicadaConfig`

Global administration and emergency pause. It is not a custody authority.

### `IntentShard`

A 9,080-byte, directly initializable Hopper account containing twenty intents
in a column-oriented layout. Immutable user columns and executor lifecycle
columns are separate byte ranges. `cells(slot; ...)` compiles runtime-selected
cells into exact parametric `strict_writes` rules, and
`ScopedContext::segment_mut` enforces them through the instruction borrow
ledger.

### `SourceLease`

A PDA at `[b"cicada-source", source_token]`. It prevents the same funded source
account from backing live records in multiple shards. The marker binds the
source account to the owner, shard, slot, and sequence and is closed only when
the final record is reclaimed.

### Vault authority

Each source account is controlled by:

```text
[b"cicada-vault", owner, source_token]
```

The owner creates or prepares a token account, derives this PDA, transfers the
token-account authority to it, funds the account, and creates the intent.
Binding the PDA to both owner and source prevents a later user from adopting an
abandoned or refilled custody account.

## Lifecycle

```text
EMPTY
  -> OPEN
  -> CLAIMED (allowlisted executor only)
  -> OPEN    (expired reservation release)
  -> SETTLED
  -> CANCELLED
  -> EMPTY   (owner reclaim)
```

Permissionless intents do not enter `CLAIMED`; they execute atomically from
`OPEN`, preventing reservation griefing.

## Route policy

### Exact mode

Commits to the target program, route bytes, account order, duplicates, and
writable/signer flags.

### Program mode

Commits to one executable program while allowing the solver to select the call
envelope. The route commitment must be zero to keep the representation
canonical.

Neither mode binds the target program's deployed bytecode. A production version
should optionally require a Grillo deployment commitment for upgradeable route
programs.

## CPI signer containment

The route may receive only the owner-bound source-vault PDA as an escalated PDA
signer. It may not write the PDA account itself. Cicada also refuses writable
delegation of:

- the config or intent shard;
- any Cicada-owned account;
- the refund token account;
- another token account controlled by the same vault PDA.

The dynamic CPI call runs in a separate non-inlined stack frame so its bounded
meta and account-info scratch buffers do not share one SBF frame with the intent
snapshot and policy commitments.

## Settlement invariants

Before CPI, Cicada snapshots source and destination amounts and hashes token
account policy excluding only the amount bytes. It hashes mint policy excluding
only supply. After CPI:

```text
source_policy_after      == source_policy_before
destination_policy_after == destination_policy_before
input_mint_policy_after  == input_mint_policy_before
output_mint_policy_after == output_mint_policy_before
spent                    = source_before - source_after
received                 = destination_after - destination_before
0 < spent <= max_input
received >= min_output
```

Every remaining source token is refunded before the record becomes final.

## Hopper-specific guarantees

For Cicada-owned state:

- user constraints are never in execute/claim write ranges;
- the selected lifecycle cell is derived from the decoded slot and neighboring
  cells in the same column are refused;
- the segment borrow ledger prevents overlapping mutable leases;
- touch maps record the acquired shared-state ranges;
- CI denies Hopper raw policy escapes;
- CI additionally denies safe whole-account mutation wrappers in Cicada.

The last rule is deliberately Cicada-specific until Hopper's planned ambient
data-write gate covers every safe wrapper path framework-wide.

## Explicit non-guarantees

Cicada does not prove that a route program is bug-free or that an upgradeable
program retains the same behavior. Program-trust mode intentionally delegates
broad authority to the chosen route over the accounts the caller supplies.
Cicada protects its custody capability, its own state, token/mint policy, and
the user's economic result.

The Hopper mutation manifest describes Cicada-owned writes and declared fixed
accounts. Dynamic downstream route effects require validator/RPC account-delta
capture and Grillo attribution.

## Highest-value next work

1. **Expand compiled-SBF adversarial coverage.** The current fixture proves
   token-policy mutation and output-without-input spoofing roll back. Add mint
   mutation, source inflation, account closure, signer escalation, and
   duplicate-meta edge cases against canonical SPL deployments.
2. **Immutable final receipts.** Persist settlement/cancellation evidence in a
   sequence-derived receipt PDA so shard slots can be garbage-collected without
   losing history.
3. **One-transaction custody setup.** Add a helper that creates an owner-bound
   vault token account and transfers funds atomically rather than requiring
   manual preparation.
4. **Grillo deployment binding.** Let exact/program intents require a verified
   binary or deployment revision, protecting users from same-address upgrades.
5. **Solver compensation.** Add bounded executor fees and optional tip ceilings
   measured independently from swap output.
6. **Partial fills and recurring schedules.** Model remaining quantity as its
   own executor-governed column without opening immutable user constraints.
7. **BAM/Cicada scheduling hints.** Publish an off-chain scheduling envelope for
   just-in-force execution, cancellation priority, and application-controlled
   batches while preserving transport-neutral on-chain settlement.
8. **Complete Grillo effect evidence.** Capture owner, lamport, data-length,
   account creation/closure, and downstream byte changes from validator replay.

## Validation gates

The repository workflow now requires:

```bash
cargo build-sbf -- -p hopper-cicada
cargo build-sbf -- -p hopper-cicada-route-fixture
cargo test -p hopper-cicada
cargo test -p hopper-runtime scoped_context_runtime_segments_preserve_write_policy
cargo run -p hopper-cli -- lint --project examples/hopper-cicada --deny-escapes
```

It also rejects `get_mut`, `load_mut`, and `with_mut` calls in the Cicada source
so shared state cannot accidentally move from exact segment access to a safe
whole-account wrapper.
