use {
    quasar_lang::prelude::*,
    quasar_spl::{InterfaceAccount, Mint, Token, TokenInterface},
};

#[derive(Accounts)]
pub struct SweepAndCloseInterface<'info> {
    pub authority: &'info Signer,
    #[account(sweep = receiver, close = destination, token::mint = mint, token::authority = authority)]
    pub source: &'info mut InterfaceAccount<Token>,
    #[account(mut)]
    pub receiver: &'info mut InterfaceAccount<Token>,
    pub mint: &'info InterfaceAccount<Mint>,
    #[account(mut)]
    pub destination: &'info mut UncheckedAccount,
    pub token_program: &'info Interface<TokenInterface>,
}

impl<'info> SweepAndCloseInterface<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
