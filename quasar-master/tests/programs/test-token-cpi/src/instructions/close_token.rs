use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint, Token},
};

/// Tests closing a token account via the `close =` attribute.
/// The macro's epilogue calls `Account::close(dest)` which zeros the account,
/// transfers lamports, and reassigns to the system program.
#[derive(Accounts)]
pub struct CloseToken<'info> {
    pub authority: &'info Signer,
    #[account(close = destination, token::mint = mint, token::authority = authority)]
    pub token_account: &'info mut Account<Token>,
    pub mint: &'info Account<Mint>,
    /// CHECK: destination may alias authority (close sends lamports to it).
    #[account(mut, dup)]
    pub destination: &'info mut UncheckedAccount,
    pub token_program: &'info Program<Token>,
}

impl<'info> CloseToken<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
