use {
    quasar_lang::prelude::*,
    quasar_spl::{InterfaceAccount, Token, TokenCpi, TokenInterface},
};

#[derive(Accounts)]
pub struct ApproveInterface<'info> {
    pub authority: &'info Signer,
    pub source: &'info mut InterfaceAccount<Token>,
    pub delegate: &'info UncheckedAccount,
    pub token_program: &'info Interface<TokenInterface>,
}

impl<'info> ApproveInterface<'info> {
    #[inline(always)]
    pub fn handler(&self, amount: u64) -> Result<(), ProgramError> {
        self.token_program
            .approve(self.source, self.delegate, self.authority, amount)
            .invoke()
    }
}
