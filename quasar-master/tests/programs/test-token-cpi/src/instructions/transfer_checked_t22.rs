use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint2022, Token2022, TokenCpi},
};

#[derive(Accounts)]
pub struct TransferCheckedT22<'info> {
    pub authority: &'info Signer,
    pub from: &'info mut Account<Token2022>,
    pub mint: &'info Account<Mint2022>,
    pub to: &'info mut Account<Token2022>,
    pub token_program: &'info Program<Token2022>,
}

impl<'info> TransferCheckedT22<'info> {
    #[inline(always)]
    pub fn handler(&self, amount: u64, decimals: u8) -> Result<(), ProgramError> {
        self.token_program
            .transfer_checked(
                self.from,
                self.mint,
                self.to,
                self.authority,
                amount,
                decimals,
            )
            .invoke()
    }
}
