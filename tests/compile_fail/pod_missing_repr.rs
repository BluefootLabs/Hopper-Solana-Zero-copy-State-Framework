//! `#[hopper::pod]` requires `#[repr(C)]` or `#[repr(transparent)]`
//! so that field offsets are stable. Without it, the compiler may
//! reorder fields and break zero-copy overlays.

use hopper::pod;

#[pod]
#[derive(Copy, Clone)]
pub struct NoRepr {
    pub value: u32,
}

fn main() {}
