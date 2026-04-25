#![allow(unexpected_cfgs)]
use quasar_lang::prelude::*;

solana_address::declare_id!("11111111111111111111111111111112");

#[account(discriminator = [1])]
pub struct DemoAccount {
    pub value: u64,
}

#[derive(Accounts)]
pub struct BadReallocOptional<'info> {
    #[account(realloc = 64)]
    pub account: Option<&'info mut Account<DemoAccount>>,
    pub payer: &'info mut Signer,
    pub system_program: &'info Program<System>,
}

fn main() {}
