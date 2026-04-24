//! R5 audit closure: the `const_assert_pod!` macro must reject any
//! type whose alignment is greater than 1, even if the user hand-rolls
//! an `unsafe impl Pod`. This is the compile-time gate that closes the
//! "hand-written `unsafe impl Pod` on an aligned type" hole flagged in
//! AUDIT.md.
//!
//! A misaligned Pod would produce UB if cast from `&[u8]` via
//! `pod_from_bytes`, because the underlying pointer is only guaranteed
//! to be `align_of::<u8>() == 1`. The alignment check lives inside the
//! macro, so any hand-authored Pod type that wants Hopper's tier-A
//! overlays to stay sound must call `const_assert_pod!(T, SIZE)` and
//! that call must fail when alignment is wrong.
//!
//! This fixture passes a 16-byte struct whose inner `u64` forces an
//! 8-byte alignment. The macro should reject it at compile time with
//! the documented diagnostic. The `.stderr` file next to this `.rs`
//! captures the expected error.

use hopper::prelude::*;

#[repr(C)]
pub struct AlignedEntry {
    // `u64` forces align_of::<AlignedEntry>() == 8, which violates the
    // Pod alignment-1 invariant. Wire types like `WireU64` are the
    // alignment-1 substitute users should reach for instead.
    pub counter: u64,
    pub bump: u8,
    pub _pad: [u8; 7],
}

// Expected to fail here with a "Pod type `AlignedEntry` must have
// alignment 1" message from the const_assert_pod! macro.
const_assert_pod!(AlignedEntry, 16);

unsafe impl Pod for AlignedEntry {}

fn main() {}
