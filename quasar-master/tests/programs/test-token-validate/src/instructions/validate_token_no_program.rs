use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint, Token},
};

/// No `token_program` field — program is known at compile time from
/// Account<Token>.
#[derive(Accounts)]
pub struct ValidateTokenNoProgram<'info> {
    #[account(token::mint = mint, token::authority = authority)]
    pub token_account: &'info Account<Token>,
    pub mint: &'info Account<Mint>,
    pub authority: &'info Signer,
}

impl<'info> ValidateTokenNoProgram<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
