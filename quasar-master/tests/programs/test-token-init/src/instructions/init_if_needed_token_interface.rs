use {
    quasar_lang::prelude::*,
    quasar_spl::{InterfaceAccount, Mint, Token, TokenInterface},
};

#[derive(Accounts)]
pub struct InitIfNeededTokenInterface<'info> {
    pub payer: &'info mut Signer,
    #[account(init_if_needed, token::mint = mint, token::authority = payer)]
    pub token_account: &'info mut InterfaceAccount<Token>,
    pub mint: &'info InterfaceAccount<Mint>,
    pub token_program: &'info Interface<TokenInterface>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitIfNeededTokenInterface<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
