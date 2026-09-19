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
Creation is bound on chain to the executing program's live deployment
authority. The live deployment path is loader v3, which validates the exact
ProgramData link and upgrade authority. The source's loader-v4-format branch is
compatibility/test modeling only: loader v4 was abandoned and its program
address was burned, so it is not a current deployment target. An immutable
loader-v3 deployment must therefore initialize this singleton before authority
revocation. An arbitrary first caller cannot claim administration.
The emergency authority must be nonzero because V1 does not expose an authority
rotation instruction that could recover an unreachable pause key.

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

The owner creates and funds a token account that remains under the owner's token
authority, then submits `create_intent`. Cicada validates the source's complete
custody surface and atomically transfers token-account authority to this PDA
through the canonical SPL Token or Token-2022 program. If any later create step
fails, transaction rollback restores the original authority. Binding the PDA to
both owner and source prevents a later user from adopting an abandoned or
refilled custody account.

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

Reclaim is dust-safe. It revalidates the committed source, token program, and
vault PDA, then restores owner authority even if someone deposited tokens after
the record became final. `SetAuthority(AccountOwner)` is balance-independent,
so the late balance stays in the user's original source account. After
re-reading the restored authority, Cicada clears the shard cell and closes the
source lease. Public token-account credits therefore cannot pin a final record
or consume a shard slot permanently, and reclaim does not depend on the refund
account still being live.

## Route policy

### Exact mode

Commits to the target program, route bytes, account order, duplicates, and
writable/signer flags.

Duplicate positions must be read-only and carry identical signer flags. The SVM
unions privileges for duplicate Pubkeys, so mixed flags would make the apparent
per-position envelope weaker than the privilege the callee actually receives.
Hopper's safe deduplicated CPI tier also rejects repeated writable metas, so
Cicada rejects them during route validation instead of admitting a route that
cannot execute.

### Program mode

Commits to one executable program while allowing the solver to select the call
envelope. The route commitment must be zero to keep the representation
canonical.

Neither mode binds the target program's deployed bytecode. A future production
version should optionally require a ledger-authenticated deployment record for
upgradeable route programs and let Grillo consume it. Current Grillo verifies
caller-supplied identity consistency; it does not establish ledger provenance.

## CPI signer containment

The route may receive only the owner-bound source-vault PDA as an escalated PDA
signer. It may not write the PDA account itself. Cicada also refuses writable
delegation of:

- the config or intent shard;
- any Cicada-owned account;
- the refund token account;
- either committed input or output mint;
- another token account controlled by the same vault PDA.

Committed mints may still appear as read-only route aliases. Rejecting writable
aliases before CPI closes supply-neutral MintTo-plus-Burn sequences that would
restore every mint byte before an end-state hash could observe them.

The dynamic CPI call runs in a separate non-inlined stack frame so its bounded
meta and account-info scratch buffers do not share one SBF frame with the intent
snapshot and policy commitments.

## Settlement invariants

Before CPI, Cicada denies writable route aliases to either committed mint,
snapshots source and destination amounts and lamports, records wrapped-SOL
status, and hashes token-account policy excluding only the amount bytes. It
hashes every mint byte, including supply. After CPI:

```text
source_policy_after      == source_policy_before
destination_policy_after == destination_policy_before
input_mint_policy_after  == input_mint_policy_before
output_mint_policy_after == output_mint_policy_before
spent                    = source_before - source_after
received                 = destination_after - destination_before
0 < spent <= max_input
received >= min_output
non-native source lamports      >= source lamports before
native source lamports          >= source lamports before - spent
non-native destination lamports >= destination lamports before
native destination lamports     >= destination lamports before + received
```

Those native-aware floors close a canonical close-and-recreate path in which a
route that can sign for a token-account address restores the same token bytes
but diverts rent or excess SOL. The compiled hostile-route matrix verifies that
the debit is detected and the complete instruction rolls back.

Every remaining source token is refunded before the record becomes final.
Source custody admits only exactly initialized accounts with no delegate or
delegated balance. Every bound token account must have no separate close
authority, preventing an additional actor from deleting an empty refund or
destination before finalization. Token-2022 TLV parsing is complete and fail
closed: malformed, duplicate, unknown, wrong-shape, and unsupported extensions
are rejected. This prevents an admitted extension from blocking the refund or
the final authority restoration.
Legacy SPL shapes are exact (82-byte mint, 165-byte token account), and
Token-2022's canonical 355-byte multisig collision is rejected before overlay
reads. A multisig cannot masquerade as an immutable settlement account that the
canonical transfer program would later refuse.

## Hopper-specific guarantees

For Cicada-owned state:

- user constraints are never in execute/claim write ranges;
- the selected lifecycle cell is derived from the decoded slot and neighboring
  cells in the same column are refused;
- the segment borrow ledger prevents overlapping mutable leases;
- touch maps record the acquired shared-state ranges;
- CI denies Hopper raw policy escapes;
- CI additionally denies safe whole-account mutation wrappers in Cicada.

The last rule is deliberately defense in depth for Cicada. Hopper's ambient
write gate now covers safe wrapper paths framework-wide, while the source lint
keeps this custody example on exact segment access and makes any later
whole-account widening an explicit review event.

## Explicit non-guarantees

Cicada does not prove that a route program is bug-free or that an upgradeable
program retains the same behavior. Program-trust mode intentionally delegates
broad authority to the chosen route over the accounts the caller supplies.
Cicada protects its custody capability, its own state, token/mint policy, and
the user's economic result.

Cicada also does not neutralize an asset issuer between lifecycle
instructions. A retained legacy mint or freeze authority can change supply or
freeze source/refund accounts after creation. Writable-mint containment keeps
the route CPI from changing either committed mint, and the full mint hash checks
for persistent changes as defense in depth. These controls do not replace an
asset allowlist or a creation-time policy commitment.

The Hopper mutation manifest describes Cicada-owned writes and declared fixed
accounts. Dynamic downstream route effects require validator/RPC account-delta
capture and Grillo attribution.

The same boundary prevents a lossless current Solana IDL v0.1 projection:
Cicada uses Hopper's u16-prefixed bounded route data and a dynamic
remaining-account contract. The fail-closed exporter therefore refuses this
program instead of emitting a misleading IDL.

## Highest-value next work

1. **Devnet and replay evidence.** Archive the same full legacy SPL Token and
   Token-2022 lifecycle against deployed programs, then capture complete
   account deltas from validator replay. The current proofs are deterministic
   Mollusk SVM executions, not public-cluster evidence.
2. **Immutable final receipts.** Persist settlement/cancellation evidence in a
   sequence-derived receipt PDA so shard slots can be garbage-collected without
   losing history.
3. **One-transaction account funding.** Add a client helper that creates and
   funds the owner-controlled source account immediately before Cicada's atomic
   custody adoption, without requiring manual setup transactions.
4. **Ledger-authenticated Grillo deployment binding.** Let exact/program
   intents require a binary or deployment revision authenticated from ledger
   data, protecting users from same-address upgrades.
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
cargo build-sbf --manifest-path examples/hopper-cicada/Cargo.toml -- --locked
cargo build-sbf --manifest-path examples/hopper-cicada-route-fixture/Cargo.toml -- --locked
cargo build-sbf --manifest-path examples/hopper-cicada-canonical-route-fixture/Cargo.toml -- --locked
HOPPER_REQUIRE_CICADA_SBF=1 cargo test -p hopper-cicada
cargo test -p hopper-cicada-canonical-route-fixture
cargo test -p hopper-runtime scoped_context_runtime_segments_preserve_write_policy
cargo run -p hopper-cli -- lint --project examples/hopper-cicada --deny-escapes
```

The current proof set is 23 compiled Cicada lifecycle/adversarial tests plus 25
host policy and manifest tests. The compiled suite loads all three repository
ELFs and Mollusk's canonical SPL Token and Token-2022 programs. CI requires the
ELFs to exist, so missing compiled coverage fails closed rather than skipping.
The canonical matrix includes both processors' native wrapped-SOL accounts,
with exact source, destination, reserve, and refund lamport coupling, plus an
atomic rollback proof for a modeled under-backed close/reinitialize end state.

It also rejects `get_mut`, `load_mut`, and `with_mut` calls in the Cicada source
so shared state cannot accidentally move from exact segment access to a safe
whole-account wrapper.
