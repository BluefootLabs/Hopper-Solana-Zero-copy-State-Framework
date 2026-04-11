# hopper-core

Core engine for the Hopper zero-copy state framework on Solana.

This is the foundation everything else sits on. Account headers, ABI types,
typed overlays, phased execution, zero-copy collections, policy enforcement,
state receipts, segment-level borrow enforcement, and cross-program interfaces.
All `no_std`, all `no_alloc`, no proc macros.

## What's in here

- **Account header** - 16-byte self-describing header on every account (disc, version, flags, layout fingerprint)
- **ABI types** - Wire-safe primitives (`WireU64`, `WireI64`, `WireU128`, `TypedAddress`, `WireBool`) that are alignment-1 and endian-correct
- **Overlay system** - Map `#[repr(C)]` structs directly onto account bytes. No copy, no deserialization
- **Tiered loading** - 5 load tiers from full validation down to raw pointer cast. Pick the trust level you need
- **Frame** - Phased execution model (Resolve -> Validate -> Execute) enforced at compile time via typestate, with segment-level borrow tracking
- **SegmentMap** - Compile-time field→offset mapping. `SegmentMap::segment("balance")` resolves to a `StaticSegment` with const offset and size. No string lookups at runtime
- **Segment borrows** - `SegmentBorrowRegistry` enforces that no two mutable references overlap the same byte range. Read `authority` while writing `balance` on the same account — fine. Write `balance` twice — caught
- **Collections** - `FixedVec`, `RingBuffer`, `SlotMap`, `BitSet`, `Journal`, `Slab`, `PackedMap`, `SortedVec`. All zero-copy, all operate directly on account bytes
- **Policy** - Declare what capabilities an instruction needs, auto-resolve validation requirements
- **Receipts** - Structured mutation proof: before/after fingerprints, changed fields, byte diffs, segment tracking, CPI flags
- **Segments** - Typed segment roles (Core, Extension, Journal, Index, Cache, Audit, Shard) with behavioral semantics
- **Virtual state** - Map state across multiple accounts with `hopper_virtual!`
- **Cross-program reads** - `hopper_interface!` reads foreign accounts by fingerprint without crate dependencies

## Quick example

```rust
use hopper::prelude::*;

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

## License

Apache-2.0
