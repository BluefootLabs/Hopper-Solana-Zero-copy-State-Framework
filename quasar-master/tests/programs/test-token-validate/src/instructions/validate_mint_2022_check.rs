use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint2022, Token2022},
};

#[derive(Accounts)]
pub struct ValidateMint2022Check<'info> {
    #[account(mint::authority = mint_authority, mint::decimals = 6)]
    pub mint: &'info Account<Mint2022>,
    pub mint_authority: &'info Signer,
    pub token_program: &'info Program<Token2022>,
}

impl<'info> ValidateMint2022Check<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
