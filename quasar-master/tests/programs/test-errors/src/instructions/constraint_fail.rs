use {crate::errors::TestError, quasar_lang::prelude::*};

#[derive(Accounts)]
pub struct ConstraintFail<'info> {
    #[account(constraint = false @ TestError::ConstraintCustom)]
    pub target: &'info SystemAccount,
}

impl<'info> ConstraintFail<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
