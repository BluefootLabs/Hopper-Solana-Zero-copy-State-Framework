use hopper::prelude::*;

#[hopper::state]
#[repr(C)]
pub struct MissingCopy {
    pub balance: WireU64,
}

fn main() {}