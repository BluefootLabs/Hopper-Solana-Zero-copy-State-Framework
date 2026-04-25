use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint2022, Token2022},
};

#[derive(Accounts)]
pub struct CloseTokenT22<'info> {
    pub authority: &'info Signer,
    #[account(close = destination, token::mint = mint, token::authority = authority)]
    pub token_account: &'info mut Account<Token2022>,
    pub mint: &'info Account<Mint2022>,
    /// CHECK: destination may alias authority (close sends lamports to it).
    #[account(mut, dup)]
    pub destination: &'info mut UncheckedAccount,
    pub token_program: &'info Program<Token2022>,
}

impl<'info> CloseTokenT22<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
