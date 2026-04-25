#![allow(unexpected_cfgs)]
extern crate alloc;
use quasar_lang::prelude::*;

solana_address::declare_id!("11111111111111111111111111111112");

#[account(discriminator = 1)]
#[seeds(b"vault", authority: Address)]
pub struct Vault {
    pub authority: Address,
    pub bump: u8,
}

#[derive(Accounts)]
pub struct Bad<'info> {
    pub authority: &'info Signer,
    #[account(seeds = Vault::seeds(), bump = vault.bump)]
    pub vault: &'info Account<Vault>,
}

fn main() {}
