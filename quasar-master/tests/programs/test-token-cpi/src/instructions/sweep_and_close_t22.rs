use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint2022, Token2022},
};

#[derive(Accounts)]
pub struct SweepAndCloseT22<'info> {
    pub authority: &'info Signer,
    #[account(sweep = receiver, close = destination, token::mint = mint, token::authority = authority)]
    pub source: &'info mut Account<Token2022>,
    #[account(mut)]
    pub receiver: &'info mut Account<Token2022>,
    pub mint: &'info Account<Mint2022>,
    #[account(mut)]
    pub destination: &'info mut UncheckedAccount,
    pub token_program: &'info Program<Token2022>,
}

impl<'info> SweepAndCloseT22<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
