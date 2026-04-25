#![allow(unexpected_cfgs)]
use quasar_lang::prelude::*;
use quasar_spl::{metadata::MetadataProgram, Mint, Token};

solana_address::declare_id!("11111111111111111111111111111112");

#[derive(Accounts)]
pub struct BadMetadataRent<'info> {
    pub payer: &'info mut Signer,
    pub mint_authority: &'info Signer,
    #[account(
        init,
        payer = payer,
        mint::decimals = 0,
        mint::authority = mint_authority,
        metadata::name = b"Test NFT",
        metadata::symbol = b"TNFT",
        metadata::uri = b"https://example.com/nft.json",
    )]
    pub mint: &'info mut Account<Mint>,
    pub metadata: &'info mut UncheckedAccount,
    pub metadata_program: &'info Program<MetadataProgram>,
    pub token_program: &'info Program<Token>,
    pub system_program: &'info Program<System>,
    pub rent: &'info UncheckedAccount,
}

fn main() {}
