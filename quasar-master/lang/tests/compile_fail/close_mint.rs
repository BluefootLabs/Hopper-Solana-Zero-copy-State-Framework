#![allow(unexpected_cfgs)]
use quasar_lang::prelude::*;
use quasar_spl::Mint;

solana_address::declare_id!("11111111111111111111111111111112");

#[derive(Accounts)]
pub struct BadCloseMint<'info> {
    pub destination: &'info mut UncheckedAccount,
    #[account(close = destination)]
    pub mint: &'info mut Account<Mint>,
}

fn main() {}
