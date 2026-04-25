use quasar_lang::prelude::*;

#[derive(Accounts)]
pub struct SignerNeeded<'info> {
    pub signer: &'info Signer,
}

impl<'info> SignerNeeded<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
