//! R5 audit closure (positive case): a struct built from alignment-1
//! wire types and fixed-size byte arrays must be accepted by
//! `const_assert_pod!`. This pins the happy path so the fail fixture
//! cannot regress into "everything is rejected".

use hopper::prelude::*;

#[repr(C)]
pub struct SafeEntry {
    pub authority: [u8; 32],
    pub counter: WireU64,
    pub bump: u8,
}

const_assert_pod!(SafeEntry, 41);

unsafe impl Pod for SafeEntry {}

fn main() {
    // Compile-time only: confirm the size assertion fired and the
    // type can be mentioned in const contexts.
    const _: usize = core::mem::size_of::<SafeEntry>();
    const _: usize = core::mem::align_of::<SafeEntry>();
}
