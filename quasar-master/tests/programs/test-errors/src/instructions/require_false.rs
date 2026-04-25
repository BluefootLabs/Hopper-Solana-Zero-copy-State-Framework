use {crate::errors::TestError, quasar_lang::prelude::*};

#[derive(Accounts)]
pub struct RequireFalse<'info> {
    pub signer: &'info Signer,
}

impl<'info> RequireFalse<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        require!(false, TestError::RequireFailed);
        Ok(())
    }
}
