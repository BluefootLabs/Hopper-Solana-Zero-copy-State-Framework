//! Audit Finding 2: compile-time proof that no naked `&mut T` can
//! satisfy a `HopperRefOnly` bound. The sealed marker trait is only
//! implemented for Hopper's four borrow guards (`Ref`, `RefMut`,
//! `SegRef`, `SegRefMut`), so any API that requires it rejects raw
//! references at the call site.
//!
//! This is the closure for "borrow safety compile-proven, not just
//! runtime-enforced". A Hopper program cannot route an account
//! mutation through a `&mut U` detour because the type system refuses
//! to coerce a raw reference into the guard contract.

use hopper::hopper_runtime::HopperRefOnly;

fn require_guard<G: HopperRefOnly>(_: G) {}

fn main() {
    let mut x: u64 = 0;
    let raw_mut: &mut u64 = &mut x;
    // Expected compile failure: `&mut u64` does not implement the
    // sealed `HopperRefOnly` marker, so the bound cannot be satisfied.
    require_guard(raw_mut);
}
