use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 1, version = 1)]
pub struct Vault {
    pub authority: Address,
    pub balance: WireU64,
    pub bump: u8,
}
