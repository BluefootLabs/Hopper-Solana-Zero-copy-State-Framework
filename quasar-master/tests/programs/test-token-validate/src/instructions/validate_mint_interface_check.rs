use {
    quasar_lang::prelude::*,
    quasar_spl::{InterfaceAccount, Mint, TokenInterface},
};

#[derive(Accounts)]
pub struct ValidateMintInterfaceCheck<'info> {
    #[account(mint::authority = mint_authority, mint::decimals = 6)]
    pub mint: &'info InterfaceAccount<Mint>,
    pub mint_authority: &'info Signer,
    pub token_program: &'info Interface<TokenInterface>,
}

impl<'info> ValidateMintInterfaceCheck<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
