use {
    quasar_lang::prelude::*,
    quasar_spl::{Token, TokenCpi},
};

#[derive(Accounts)]
pub struct CloseTokenAccount<'info> {
    pub account: &'info mut Account<Token>,
    pub destination: &'info mut Signer,
    /// CHECK: authority may equal destination when the signer is closing to
    /// themselves.
    #[account(dup)]
    pub authority: &'info Signer,
    pub token_program: &'info Program<Token>,
}

impl<'info> CloseTokenAccount<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        self.token_program
            .close_account(self.account, self.destination, self.authority)
            .invoke()
    }
}
