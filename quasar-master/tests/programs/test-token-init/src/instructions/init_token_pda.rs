use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint, Token},
};

#[derive(Accounts)]
pub struct InitTokenPda<'info> {
    pub payer: &'info mut Signer,
    #[account(init, seeds = [b"token", payer], bump, token::mint = mint, token::authority = payer)]
    pub token_account: &'info mut Account<Token>,
    pub mint: &'info Account<Mint>,
    pub token_program: &'info Program<Token>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitTokenPda<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
