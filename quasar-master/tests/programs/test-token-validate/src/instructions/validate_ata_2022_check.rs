use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint2022, Token2022},
};

#[derive(Accounts)]
pub struct ValidateAta2022Check<'info> {
    #[account(associated_token::mint = mint, associated_token::authority = wallet)]
    pub ata: &'info Account<Token2022>,
    pub mint: &'info Account<Mint2022>,
    pub wallet: &'info Signer,
    pub token_program: &'info Program<Token2022>,
}

impl<'info> ValidateAta2022Check<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
