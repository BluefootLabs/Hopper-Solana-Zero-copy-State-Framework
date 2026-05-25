# Protocol-grade examples

These examples are the public evidence layer for Hopper's state-contract claim.
They show the workflow surfaces that make Hopper more than a minimal zero-copy
framework.

## Receipt indexing

On chain, emit the fixed receipt bytes. Off chain, parse with `hopper-sdk` and
store the stable index projection.

```rust
use hopper_sdk::receipt::DecodedReceipt;

let receipt = DecodedReceipt::parse(&receipt_bytes)?;
let row = receipt.index_record();

// Suggested primary key: transaction signature + log index + row.index_key.
assert_eq!(&row.index_key[..8], &row.layout_id);
```

The row carries layout ID, phase, compatibility impact, changed field count,
segment mask, policy flags, validation bundle ID, size delta, and failure
metadata. The wire format remains 72 bytes.

## Schema compatibility report

```powershell
hopper schema diff manifests/vault-v1.json manifests/vault-v2.json
hopper compat manifests/vault-v1.json manifests/vault-v2.json
hopper compat --why manifests/vault-v1.json manifests/vault-v2.json
```

Use this in reviews before deploying a layout change. `schema diff` gives field
movement and type changes. `compat --why` adds the operator-facing reason for the
compatibility verdict.

## Risk guard subsystem

[examples/hopper-argus-guard](../examples/hopper-argus-guard) is the real-world
subsystem proof pattern: an authority-owned risk book with checked reserve and
release flows. It demonstrates how Hopper keeps protocol accounting inside a
versioned layout contract while using `with_mut` for concise safe mutation.

## Styx ferry and forward-secret messaging

[examples/hopper-styx-ferry](../examples/hopper-styx-ferry) is the
protocol-shaped sample: a Hopper program that ports the Styx ferry/messaging
control plane while keeping Signal-style X3DH, Double Ratchet state, and payload
encryption in the client protocol. On chain, Hopper verifies the signed-prekey
boundary with the Ed25519 precompile, derives the VSL domain separator with
Keccak, enforces bounded ratchet message envelopes and monotonic counters, checks
the 513-byte Styx proof envelope, and CPIs into a pinned verifier program.

This is the crypto/syscall parity example: Hopper exposes the Solana hashing,
precompile-inspection, stack-height, and processed-instruction surfaces needed by
Pinocchio/Jiminy/Quasar-style programs without dropping the typed account model.

## Migration planner

```powershell
hopper plan manifests/vault-v1.json manifests/vault-v2.json
```

The migration planner is the bridge from schema change to implementation work:
it reports append-safe growth, copy spans, zero-fill spans, backward readability,
and whether a migration instruction is required. The canonical working example
is [examples/hopper-migration](../examples/hopper-migration).

## Cross-program typed reads

```rust
#[derive(Accounts)]
pub struct ReadRemote<'info> {
    pub remote: hopper::prelude::InterfaceAccount<'info, RemoteVault>,
}

pub fn read(ctx: Ctx<ReadRemote>) -> ProgramResult {
    ctx.accounts.remote.with(|remote| {
        let _balance = remote.balance.get();
        Ok(())
    })
}
```

The wrapper validates owner-set membership and Hopper layout identity, then reads
through `load_cross_program`. See [examples/cross-program-read](../examples/cross-program-read).

## Segment leases

```rust
let mut leases = hopper::prelude::SegmentBorrowRegistry::new();

{
    let mut balance = account.segment_mut::<WireU64>(&mut leases, BALANCE_OFFSET, 8)?;
    balance.checked_add_assign(amount)?;
}

{
    let authority = account.segment_ref::<Address>(&mut leases, AUTHORITY_OFFSET, 32)?;
    assert_eq!(authority.as_array(), expected.as_array());
}
```

Segment leases are guard-scoped. Dropping the first guard releases its lease, so
sequential disjoint field access stays ergonomic while overlapping mutable access
still fails. Use segment leases when an audit needs to know exactly which region
an instruction mutates.
