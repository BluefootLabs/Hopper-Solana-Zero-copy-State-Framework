use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint, Token},
};

/// Tests sweep + close — transfers all tokens, then closes the account.
#[derive(Accounts)]
pub struct SweepAndClose<'info> {
    pub authority: &'info Signer,
    #[account(sweep = receiver, close = destination, token::mint = mint, token::authority = authority)]
    pub source: &'info mut Account<Token>,
    #[account(mut)]
    pub receiver: &'info mut Account<Token>,
    pub mint: &'info Account<Mint>,
    #[account(mut)]
    pub destination: &'info mut UncheckedAccount,
    pub token_program: &'info Program<Token>,
}

impl<'info> SweepAndClose<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
