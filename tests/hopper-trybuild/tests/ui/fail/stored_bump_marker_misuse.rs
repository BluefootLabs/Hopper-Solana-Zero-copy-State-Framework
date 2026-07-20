//! `#[bump]` marker misuse: more than one marked field, or a marked field
//! that is not a `u8`, must fail with the state macro's own diagnostics.

use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 2, version = 1)]
pub struct TwoMarkers {
    #[bump]
    pub bump_a: u8,
    #[bump]
    pub bump_b: u8,
    pub reserved: [u8; 6],
}

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 3, version = 1)]
pub struct WideMarker {
    #[bump]
    pub bump: WireU64,
}

fn main() {}
