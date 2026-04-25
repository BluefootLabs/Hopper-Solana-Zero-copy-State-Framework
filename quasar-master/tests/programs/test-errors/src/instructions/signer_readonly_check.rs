use quasar_lang::prelude::*;

#[derive(Accounts)]
pub struct SignerReadonlyCheck<'info> {
    pub signer: &'info Signer,
}

impl<'info> SignerReadonlyCheck<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
