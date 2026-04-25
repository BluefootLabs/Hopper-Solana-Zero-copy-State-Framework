use {
    quasar_lang::prelude::*,
    quasar_spl::{Token2022, TokenCpi},
};

#[derive(Accounts)]
pub struct CloseTokenAccountT22<'info> {
    pub account: &'info mut Account<Token2022>,
    pub destination: &'info mut Signer,
    /// CHECK: authority may equal destination when the signer is closing to
    /// themselves.
    #[account(dup)]
    pub authority: &'info Signer,
    pub token_program: &'info Program<Token2022>,
}

impl<'info> CloseTokenAccountT22<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        self.token_program
            .close_account(self.account, self.destination, self.authority)
            .invoke()
    }
}
