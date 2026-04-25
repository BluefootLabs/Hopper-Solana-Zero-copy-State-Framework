use {
    quasar_lang::prelude::*,
    quasar_spl::{InterfaceAccount, Token, TokenCpi, TokenInterface},
};

#[derive(Accounts)]
pub struct RevokeInterface<'info> {
    pub authority: &'info Signer,
    pub source: &'info mut InterfaceAccount<Token>,
    pub token_program: &'info Interface<TokenInterface>,
}

impl<'info> RevokeInterface<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        self.token_program
            .revoke(self.source, self.authority)
            .invoke()
    }
}
