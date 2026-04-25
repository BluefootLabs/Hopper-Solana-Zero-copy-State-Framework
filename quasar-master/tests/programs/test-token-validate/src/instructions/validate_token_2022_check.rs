use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint2022, Token2022},
};

#[derive(Accounts)]
pub struct ValidateToken2022Check<'info> {
    #[account(token::mint = mint, token::authority = authority)]
    pub token_account: &'info Account<Token2022>,
    pub mint: &'info Account<Mint2022>,
    pub authority: &'info Signer,
    pub token_program: &'info Program<Token2022>,
}

impl<'info> ValidateToken2022Check<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
