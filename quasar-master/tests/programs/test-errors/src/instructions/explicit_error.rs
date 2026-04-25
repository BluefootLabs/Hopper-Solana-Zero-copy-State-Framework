use {crate::errors::TestError, quasar_lang::prelude::*};

#[derive(Accounts)]
pub struct ExplicitError<'info> {
    pub signer: &'info Signer,
}

impl<'info> ExplicitError<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Err(TestError::ExplicitNum.into())
    }
}
