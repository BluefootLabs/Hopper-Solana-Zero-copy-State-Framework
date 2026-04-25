use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint2022, Token2022, TokenCpi},
};

#[derive(Accounts)]
pub struct MintToT22<'info> {
    pub authority: &'info Signer,
    pub mint: &'info mut Account<Mint2022>,
    pub to: &'info mut Account<Token2022>,
    pub token_program: &'info Program<Token2022>,
}

impl<'info> MintToT22<'info> {
    #[inline(always)]
    pub fn handler(&self, amount: u64) -> Result<(), ProgramError> {
        self.token_program
            .mint_to(self.mint, self.to, self.authority, amount)
            .invoke()
    }
}
