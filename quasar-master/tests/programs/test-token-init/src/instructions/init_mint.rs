use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint, Token},
};

#[derive(Accounts)]
pub struct InitMint<'info> {
    pub payer: &'info mut Signer,
    #[account(init, mint::decimals = 6, mint::authority = mint_authority)]
    pub mint: &'info mut Account<Mint>,
    pub mint_authority: &'info Signer,
    pub token_program: &'info Program<Token>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitMint<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
