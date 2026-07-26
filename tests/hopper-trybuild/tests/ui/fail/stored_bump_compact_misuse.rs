//! `#[bump]` marker misuse on the COMPACT state tier: the marker's
//! contract (at most one marked field, `u8` only) holds on every layout
//! tier, not just the headered walk — a compact struct previously
//! swallowed the marker silently.

use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(compact, disc = 2, version = 1)]
pub struct CompactTwoMarkers {
    #[bump]
    pub bump_a: u8,
    #[bump]
    pub bump_b: u8,
    pub reserved: [u8; 6],
}

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(compact, disc = 3, version = 1)]
pub struct CompactWideMarker {
    #[bump]
    pub bump: WireU64,
}

fn main() {}
