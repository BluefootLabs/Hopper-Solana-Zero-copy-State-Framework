use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint, Token},
};

#[derive(Accounts)]
pub struct ValidateAtaCheck<'info> {
    #[account(associated_token::mint = mint, associated_token::authority = wallet)]
    pub ata: &'info Account<Token>,
    pub mint: &'info Account<Mint>,
    pub wallet: &'info Signer,
    pub token_program: &'info Program<Token>,
}

impl<'info> ValidateAtaCheck<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
