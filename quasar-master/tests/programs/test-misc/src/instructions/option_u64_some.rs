use quasar_lang::prelude::*;

#[derive(Accounts)]
pub struct OptionU64Some<'info> {
    pub signer: &'info Signer,
}

impl<'info> OptionU64Some<'info> {
    #[inline(always)]
    pub fn handler(&self, value: Option<u64>) -> Result<(), ProgramError> {
        require!(value == Some(42), ProgramError::InvalidInstructionData);
        Ok(())
    }
}
