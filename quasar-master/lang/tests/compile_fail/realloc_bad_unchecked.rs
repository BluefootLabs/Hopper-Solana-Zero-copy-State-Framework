#![allow(unexpected_cfgs)]
use quasar_lang::prelude::*;

solana_address::declare_id!("11111111111111111111111111111112");

#[derive(Accounts)]
pub struct BadReallocUnchecked<'info> {
    #[account(realloc = 64)]
    pub account: &'info mut UncheckedAccount,
    pub payer: &'info mut Signer,
    pub system_program: &'info Program<System>,
}

fn main() {}
