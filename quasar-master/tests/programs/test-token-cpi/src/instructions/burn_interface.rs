use {
    quasar_lang::prelude::*,
    quasar_spl::{InterfaceAccount, Mint, Token, TokenCpi, TokenInterface},
};

#[derive(Accounts)]
pub struct BurnInterface<'info> {
    pub authority: &'info Signer,
    pub from: &'info mut InterfaceAccount<Token>,
    pub mint: &'info mut InterfaceAccount<Mint>,
    pub token_program: &'info Interface<TokenInterface>,
}

impl<'info> BurnInterface<'info> {
    #[inline(always)]
    pub fn handler(&self, amount: u64) -> Result<(), ProgramError> {
        self.token_program
            .burn(self.from, self.mint, self.authority, amount)
            .invoke()
    }
}
