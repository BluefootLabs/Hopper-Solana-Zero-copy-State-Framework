#![allow(unexpected_cfgs)]
use quasar_lang::prelude::*;
use quasar_spl::{Mint, Token, Token2022};

solana_address::declare_id!("11111111111111111111111111111112");

#[derive(Accounts)]
pub struct BadTokenProgramSelector<'info> {
    pub payer: &'info mut Signer,
    pub authority: &'info Signer,
    pub mint: &'info Account<Mint>,
    #[account(
        init,
        payer = payer,
        token::mint = mint,
        token::authority = authority,
    )]
    pub token: &'info mut Account<Token>,
    pub token_program: &'info Program<Token>,
    pub token_program_2022: &'info Program<Token2022>,
    pub system_program: &'info Program<System>,
}

fn main() {}
