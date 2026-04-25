use {
    quasar_lang::prelude::*,
    quasar_spl::{InterfaceAccount, Mint, Token, TokenInterface},
};

#[derive(Accounts)]
pub struct CloseTokenInterface<'info> {
    pub authority: &'info Signer,
    #[account(close = destination, token::mint = mint, token::authority = authority)]
    pub token_account: &'info mut InterfaceAccount<Token>,
    pub mint: &'info InterfaceAccount<Mint>,
    /// CHECK: destination may alias authority (close sends lamports to it).
    #[account(mut, dup)]
    pub destination: &'info mut UncheckedAccount,
    pub token_program: &'info Interface<TokenInterface>,
}

impl<'info> CloseTokenInterface<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
