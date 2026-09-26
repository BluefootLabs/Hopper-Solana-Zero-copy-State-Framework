use hopper::prelude::*;

/// Version 2 is intentionally incompatible with the old state-only example.
#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 2, version = 2)]
pub struct Escrow {
    pub maker: Address,
    pub maker_receive: Address,
    pub mint_a: Address,
    pub mint_b: Address,
    pub vault: Address,
    pub amount_offered: WireU64,
    pub amount_wanted: WireU64,
}
