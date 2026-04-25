use {
    quasar_lang::prelude::*,
    quasar_spl::{InterfaceAccount, Mint, Token, TokenCpi, TokenInterface},
};

#[derive(Accounts)]
pub struct MintToInterface<'info> {
    pub authority: &'info Signer,
    pub mint: &'info mut InterfaceAccount<Mint>,
    pub to: &'info mut InterfaceAccount<Token>,
    pub token_program: &'info Interface<TokenInterface>,
}

impl<'info> MintToInterface<'info> {
    #[inline(always)]
    pub fn handler(&self, amount: u64) -> Result<(), ProgramError> {
        self.token_program
            .mint_to(self.mint, self.to, self.authority, amount)
            .invoke()
    }
}
