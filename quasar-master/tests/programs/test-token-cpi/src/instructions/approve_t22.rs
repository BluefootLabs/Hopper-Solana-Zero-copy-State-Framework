use {
    quasar_lang::prelude::*,
    quasar_spl::{Token2022, TokenCpi},
};

#[derive(Accounts)]
pub struct ApproveT22<'info> {
    pub authority: &'info Signer,
    pub source: &'info mut Account<Token2022>,
    pub delegate: &'info UncheckedAccount,
    pub token_program: &'info Program<Token2022>,
}

impl<'info> ApproveT22<'info> {
    #[inline(always)]
    pub fn handler(&self, amount: u64) -> Result<(), ProgramError> {
        self.token_program
            .approve(self.source, self.delegate, self.authority, amount)
            .invoke()
    }
}
