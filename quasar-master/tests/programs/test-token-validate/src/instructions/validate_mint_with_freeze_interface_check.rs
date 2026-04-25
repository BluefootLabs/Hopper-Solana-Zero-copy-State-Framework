use {
    quasar_lang::prelude::*,
    quasar_spl::{InterfaceAccount, Mint, TokenInterface},
};

#[derive(Accounts)]
pub struct ValidateMintWithFreezeInterfaceCheck<'info> {
    #[account(mint::authority = mint_authority, mint::decimals = 6, mint::freeze_authority = freeze_authority)]
    pub mint: &'info InterfaceAccount<Mint>,
    pub mint_authority: &'info Signer,
    pub freeze_authority: &'info UncheckedAccount,
    pub token_program: &'info Interface<TokenInterface>,
}

impl<'info> ValidateMintWithFreezeInterfaceCheck<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
