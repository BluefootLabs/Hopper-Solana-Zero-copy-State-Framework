//! Quasar-style `&mut Account<T>` syntax should fail with a Hopper-specific
//! diagnostic. Hopper records account mutability in `#[account(mut)]` and then
//! grants mutable bytes through wrapper guard methods.

use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 1, version = 1)]
pub struct Vault {
    pub authority: Address,
    pub balance: WireU64,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub vault: &'info mut Account<'info, Vault>,
    pub authority: Signer<'info>,
}

fn main() {}
