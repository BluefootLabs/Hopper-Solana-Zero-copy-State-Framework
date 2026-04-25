use {
    quasar_lang::prelude::*,
    quasar_spl::{InterfaceAccount, Mint, TokenInterface},
};

#[derive(Accounts)]
pub struct InitMintInterface<'info> {
    pub payer: &'info mut Signer,
    #[account(init, mint::decimals = 6, mint::authority = mint_authority)]
    pub mint: &'info mut InterfaceAccount<Mint>,
    pub mint_authority: &'info Signer,
    pub token_program: &'info Interface<TokenInterface>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitMintInterface<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
