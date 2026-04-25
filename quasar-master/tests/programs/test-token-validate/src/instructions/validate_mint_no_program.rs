use {quasar_lang::prelude::*, quasar_spl::Mint};

/// No `token_program` field — program is known at compile time from
/// Account<Mint>.
#[derive(Accounts)]
pub struct ValidateMintNoProgram<'info> {
    #[account(mint::authority = mint_authority, mint::decimals = 6)]
    pub mint: &'info Account<Mint>,
    pub mint_authority: &'info Signer,
}

impl<'info> ValidateMintNoProgram<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
