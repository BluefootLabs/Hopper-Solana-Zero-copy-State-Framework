use {
    quasar_lang::prelude::*,
    quasar_spl::{Token2022, TokenCpi},
};

#[derive(Accounts)]
pub struct RevokeT22<'info> {
    pub authority: &'info Signer,
    pub source: &'info mut Account<Token2022>,
    pub token_program: &'info Program<Token2022>,
}

impl<'info> RevokeT22<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        self.token_program
            .revoke(self.source, self.authority)
            .invoke()
    }
}
