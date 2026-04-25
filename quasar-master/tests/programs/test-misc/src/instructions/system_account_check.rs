use quasar_lang::prelude::*;

#[derive(Accounts)]
pub struct SystemAccountCheck<'info> {
    pub target: &'info SystemAccount,
}

impl<'info> SystemAccountCheck<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
