//! `{ a: u8, b: u64 }` with `#[repr(C)]` has 7 bytes of implicit
//! padding between `a` and `b`. Pod types must have *no* padding . 
//! the alignment + size asserts emitted by `#[hopper::pod]` detect
//! this by comparing `size_of::<Self>()` against the sum of field
//! sizes. Use alignment-1 wire types (`WireU64`) instead of raw
//! `u64` to eliminate padding.

use hopper::pod;

#[pod]
#[derive(Copy, Clone)]
#[repr(C)]
pub struct Padded {
    pub a: u8,
    pub b: u64,
}

fn main() {}
