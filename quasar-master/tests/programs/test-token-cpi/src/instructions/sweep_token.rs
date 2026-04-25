use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint, Token},
};

/// Tests sweep without close — transfers all remaining tokens at end of
/// instruction.
#[derive(Accounts)]
pub struct SweepToken<'info> {
    pub authority: &'info Signer,
    #[account(sweep = receiver, token::mint = mint, token::authority = authority)]
    pub source: &'info mut Account<Token>,
    #[account(mut)]
    pub receiver: &'info mut Account<Token>,
    pub mint: &'info Account<Mint>,
    pub token_program: &'info Program<Token>,
}

impl<'info> SweepToken<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
