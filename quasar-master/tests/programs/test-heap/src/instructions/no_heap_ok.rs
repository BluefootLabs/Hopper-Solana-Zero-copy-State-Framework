use quasar_lang::prelude::*;

#[derive(Accounts)]
pub struct NoHeapOk<'info> {
    pub signer: &'info Signer,
}

impl<'info> NoHeapOk<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
