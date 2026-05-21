# hopper-systems

Advanced state architecture for the Hopper zero-copy framework on Solana.

This is the foundation everything else sits on: account headers, ABI types,
typed overlays, phased execution, zero-copy collections, policy enforcement,
state receipts, segment-level borrow enforcement, and cross-program interfaces.
It is `no_std`, `no_alloc`, and does not require proc macros.

## What's in here

- **Account header** - 16-byte self-describing header on every account: discriminator, version, flags, and layout fingerprint.
- **ABI types** - Wire-safe primitives such as `WireU64`, `WireI64`, `WireU128`, `TypedAddress`, and `WireBool`.
- **Overlay system** - Map `#[repr(C)]` structs directly onto account bytes. No copy, no deserialization.
- **Tiered loading** - Five load tiers from full validation down to raw pointer access.
- **Frame** - Resolve, validate, and execute phases enforced through typestate and segment-level borrow tracking.
- **SegmentMap** - Compile-time field-to-offset mapping. No string lookups at runtime.
- **Segment borrows** - Byte-range borrow tracking for partial reads and writes on the same account.
- **Collections** - `FixedVec`, `RingBuffer`, `SlotMap`, `BitSet`, `Journal`, `Slab`, `PackedMap`, and `SortedVec` over account bytes.
- **Policy** - Declare what capabilities an instruction needs and auto-resolve validation requirements.
- **Checks** - Account, PDA, rent, and Instructions sysvar helpers, including the typed `InstructionsSysvar` view.
- **Receipts** - Structured mutation proof with before/after fingerprints, changed fields, byte diffs, segment tracking, and CPI flags.
- **Segments** - Typed segment roles: Core, Extension, Journal, Index, Cache, Audit, and Shard.
- **Virtual state** - Map state across multiple accounts with `hopper_virtual!`.
- **Cross-program reads** - `hopper_interface!` reads foreign accounts by fingerprint without crate dependencies.

The mental model is simple: Quasar-style programs cast account bytes quickly;
Hopper systems code checks the layout contract first, then casts. That contract
is what lets higher layers add schema epochs, dynamic-tail fingerprints,
segment borrows, receipts, policies, and migration reports without giving up
zero-copy account access.

## Quick example

```rust
use hopper::systems::*;

hopper_layout! {
    pub struct Vault, disc = 1, version = 1 {
        authority: TypedAddress<Authority>  = 32,
        balance:   WireU64                  = 8,
        bump:      u8                       = 1,
    }
}

// Full validation (Tier A)
let vault = Vault::load(account, program_id)?;

// Pod-level access (Tier B)
let vault = pod_from_bytes::<Vault>(data)?;
```

Docs: <https://docs.rs/crate/hopper-systems/0.2.1>

## Support

Public-goods support and donations can be sent to `solanadevdao.sol` /
`F42ZovBoRJZU4av5MiESVwJWnEx8ZQVFkc1RM29zMxNT`.

## License

Apache-2.0
