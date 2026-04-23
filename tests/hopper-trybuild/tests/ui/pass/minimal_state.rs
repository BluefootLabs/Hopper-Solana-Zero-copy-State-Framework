use hopper::prelude::*;

#[hopper::state]
#[repr(C)]
pub struct Vault {
    pub authority: [u8; 32],
    pub balance: WireU64,
    pub bump: u8,
}

fn main() {
    // Compile-time-only: the state macro must emit a struct that
    // typechecks with INIT_SPACE and LEN consts.
    const _: usize = Vault::INIT_SPACE;
    const _: usize = Vault::LEN;
}
