use {
    quasar_lang::prelude::*,
    quasar_spl::{Mint, Token},
};

#[derive(Accounts)]
pub struct InitIfNeededMintWithFreeze<'info> {
    pub payer: &'info mut Signer,
    #[account(
        init_if_needed,
        mint::decimals = 6,
        mint::authority = mint_authority,
        mint::freeze_authority = freeze_authority
    )]
    pub mint: &'info mut Account<Mint>,
    pub mint_authority: &'info Signer,
    pub freeze_authority: &'info UncheckedAccount,
    pub token_program: &'info Program<Token>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitIfNeededMintWithFreeze<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
