use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint2022, Token2022, TokenCpi},
};

#[derive(Accounts)]
pub struct BurnT22<'info> {
    pub authority: &'info Signer,
    pub from: &'info mut Account<Token2022>,
    pub mint: &'info mut Account<Mint2022>,
    pub token_program: &'info Program<Token2022>,
}

impl<'info> BurnT22<'info> {
    #[inline(always)]
    pub fn handler(&self, amount: u64) -> Result<(), ProgramError> {
        self.token_program
            .burn(self.from, self.mint, self.authority, amount)
            .invoke()
    }
}
