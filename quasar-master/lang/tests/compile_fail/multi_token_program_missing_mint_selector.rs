#![allow(unexpected_cfgs)]
use quasar_lang::prelude::*;
use quasar_spl::{Mint, Token, Token2022};

solana_address::declare_id!("11111111111111111111111111111112");

#[derive(Accounts)]
pub struct BadMintProgramSelector<'info> {
    pub payer: &'info mut Signer,
    pub mint_authority: &'info Signer,
    #[account(
        init,
        payer = payer,
        mint::decimals = 6,
        mint::authority = mint_authority,
    )]
    pub mint: &'info mut Account<Mint>,
    pub token_program: &'info Program<Token>,
    pub token_program_2022: &'info Program<Token2022>,
    pub system_program: &'info Program<System>,
}

fn main() {}
