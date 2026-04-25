use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint, Token},
};

#[derive(Accounts)]
pub struct ValidateMintWithFreezeCheck<'info> {
    #[account(mint::authority = mint_authority, mint::decimals = 6, mint::freeze_authority = freeze_authority)]
    pub mint: &'info Account<Mint>,
    pub mint_authority: &'info Signer,
    pub freeze_authority: &'info UncheckedAccount,
    pub token_program: &'info Program<Token>,
}

impl<'info> ValidateMintWithFreezeCheck<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
