use {crate::state::SmallPrefixAccount, quasar_lang::prelude::*};

#[derive(Accounts)]
pub struct SmallPrefixCheck<'info> {
    pub account: Account<SmallPrefixAccount<'info>>,
}

impl<'info> SmallPrefixCheck<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
