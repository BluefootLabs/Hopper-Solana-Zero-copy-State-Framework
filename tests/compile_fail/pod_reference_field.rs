//! References are fundamentally not Pod: they carry provenance and
//! cannot be reconstructed from arbitrary bytes. Must be rejected.

use hopper::pod;

#[pod]
#[derive(Copy, Clone)]
#[repr(C)]
pub struct BadRef {
    pub ptr: &'static u8,
}

fn main() {}
