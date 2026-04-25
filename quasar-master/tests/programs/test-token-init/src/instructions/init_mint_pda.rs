use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint, Token},
};

#[derive(Accounts)]
pub struct InitMintPda<'info> {
    pub payer: &'info mut Signer,
    #[account(init, seeds = [b"mint", payer], bump, mint::decimals = 6, mint::authority = payer)]
    pub mint: &'info mut Account<Mint>,
    pub token_program: &'info Program<Token>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitMintPda<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
