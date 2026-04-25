#![allow(unexpected_cfgs)]
use quasar_lang::prelude::*;

solana_address::declare_id!("11111111111111111111111111111112");

#[account(discriminator = 1)]
pub struct Config {
    pub bump: u8,
}

#[derive(Accounts)]
pub struct Bad<'info> {
    #[account(seeds = [b"config"], bump = config.bump)]
    pub config: &'info Account<Config>,
}

fn main() {}
