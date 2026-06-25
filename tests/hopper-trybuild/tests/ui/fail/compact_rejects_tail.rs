use hopper::prelude::*;

// Compact layouts are fixed-size: combining `compact` with a dynamic
// tail must be rejected at macro-expansion time.
#[hopper::state(compact, raw_tail = true)]
#[repr(C)]
pub struct Bad {
    pub balance: WireU64,
}

fn main() {}
