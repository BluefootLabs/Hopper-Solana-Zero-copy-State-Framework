use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint2022, Token2022},
};

#[derive(Accounts)]
pub struct InitMintPdaT22<'info> {
    pub payer: &'info mut Signer,
    #[account(init, seeds = [b"mint", payer], bump, mint::decimals = 6, mint::authority = payer)]
    pub mint: &'info mut Account<Mint2022>,
    pub token_program: &'info Program<Token2022>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitMintPdaT22<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
