use {
    quasar_lang::prelude::*,
    quasar_spl::{InterfaceAccount, Mint, Token, TokenCpi, TokenInterface},
};

#[derive(Accounts)]
pub struct TransferCheckedInterface<'info> {
    pub authority: &'info Signer,
    pub from: &'info mut InterfaceAccount<Token>,
    pub mint: &'info InterfaceAccount<Mint>,
    pub to: &'info mut InterfaceAccount<Token>,
    pub token_program: &'info Interface<TokenInterface>,
}

impl<'info> TransferCheckedInterface<'info> {
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
