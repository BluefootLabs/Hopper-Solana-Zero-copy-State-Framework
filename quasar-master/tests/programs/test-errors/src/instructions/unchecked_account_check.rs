use quasar_lang::prelude::*;

#[derive(Accounts)]
pub struct UncheckedAccountCheck<'info> {
    pub account: &'info UncheckedAccount,
}

impl<'info> UncheckedAccountCheck<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
