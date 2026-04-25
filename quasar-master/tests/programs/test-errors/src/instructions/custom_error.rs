use {crate::errors::TestError, quasar_lang::prelude::*};

#[derive(Accounts)]
pub struct CustomError<'info> {
    pub signer: &'info Signer,
}

impl<'info> CustomError<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Err(TestError::Hello.into())
    }
}
