use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint2022, Token2022},
};

#[derive(Accounts)]
pub struct ValidateMintWithFreeze2022Check<'info> {
    #[account(mint::authority = mint_authority, mint::decimals = 6, mint::freeze_authority = freeze_authority)]
    pub mint: &'info Account<Mint2022>,
    pub mint_authority: &'info Signer,
    pub freeze_authority: &'info UncheckedAccount,
    pub token_program: &'info Program<Token2022>,
}

impl<'info> ValidateMintWithFreeze2022Check<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
