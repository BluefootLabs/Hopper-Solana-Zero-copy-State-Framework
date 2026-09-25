# hopper-systems

[![Crates.io](https://img.shields.io/crates/v/hopper-systems.svg)](https://crates.io/crates/hopper-systems)
[![Docs.rs](https://img.shields.io/docsrs/hopper-systems)](https://docs.rs/hopper-systems)

Core types and execution primitives for the Hopper zero-copy state framework
on Solana.

The crate is `no_std` and does not require an allocator or procedural macros.
It provides account headers, ABI types, typed overlays, phased execution,
borrowed collections, policy enforcement, mutation summaries, segment-level
borrow checks, and cross-program interfaces.

Part of the **[Hopper](https://hopperzero.dev)** framework.

## What's here

Account header: optional 16-byte self-describing header for standard headered layouts (disc, version, flags, layout fingerprint, schema epoch). Compact layouts use a one-byte discriminator instead.

ABI types: Wire-safe primitives (WireU64, WireI64, WireU128, TypedAddress, WireBool) that are alignment-1 and endian-correct.

Overlay system: Map #[repr(C)] structs directly onto account bytes. No copy, no deserialization.

Headered DSL, modifier, and migration wrappers use `HopperLayout::OVERLAY_OFFSET`:
declarative layouts include the header at offset zero, while proc-macro state
structs begin after the 16-byte header. Borrow guards remain alive across the
projection. `VerifiedAccount` constructors themselves check length only;
ownership, layout, and authorization checks belong to the loader or caller.

Checked fixed-layout casts use a framework-owned compile-time size assertion;
overriding both `SIZE` and the compatibility assertion cannot bypass it. Typed
segment slices reject inconsistent count/capacity/element-size metadata and
out-of-bounds regions before constructing a view.

Loading: Validated, trusted, observational, and explicit unsafe overlay paths
make each caller's trust boundary visible.

Frame: Phased execution model (Resolve -> Validate -> Execute) enforced at compile time via typestate, with segment-level borrow tracking.

SegmentMap: A constant field-to-offset descriptor table. Generated field
accessors use fixed offsets; `SegmentMap::segment("balance")` performs a linear
name lookup when runtime selection is needed.

Segment borrows: `SegmentBorrowRegistry` rejects incompatible overlapping live
ranges while permitting disjoint fields from the same account.

Collections: `FixedVec`, `RingBuffer`, `SlotMap`, `BitSet`, `Journal`, `Slab`,
`PackedMap`, and `SortedVec` operate on caller-provided account byte slices.
`Slab` provides stable slot IDs, free-list allocation, occupancy checks, and
double-free validation. `TailSlab` adapts it to a compact account's dynamic
tail.

Policy: Declare what capabilities an instruction needs. Auto-resolve validation requirements.

Receipts: Structured mutation summaries with before/after fingerprints, changed fields, byte diffs, segment tracking, and CPI flags.

Segments: Typed segment roles (Core, Extension, Journal, Index, Cache, Audit, Shard) with behavioral semantics.

Virtual state: Map state across multiple accounts. The `hopper_virtual!` macro that declares a mapping lives in `hopper-macros`.

Cross-program reads: Read foreign accounts by layout fingerprint without depending on the owning program's crate. The `hopper_interface!` macro lives in `hopper-macros`.

## Quick example

```rust
use hopper::prelude::*;
use hopper::systems::*;

hopper_layout! {
    pub struct Vault, disc = 1, version = 1 {
        authority: TypedAddress<Authority>  = 32,
        balance:   WireU64                  = 8,
        bump:      u8                       = 1,
    }
}

// Full validation
let vault = Vault::load(account, program_id)?;

// Pod-level access
let vault = pod_from_bytes::<Vault>(data)?;
```

## License

Apache-2.0
