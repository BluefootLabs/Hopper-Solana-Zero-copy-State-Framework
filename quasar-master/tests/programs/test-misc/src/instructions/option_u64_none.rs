use quasar_lang::prelude::*;

#[derive(Accounts)]
pub struct OptionU64None<'info> {
    pub signer: &'info Signer,
}

impl<'info> OptionU64None<'info> {
    #[inline(always)]
    pub fn handler(&self, value: Option<u64>) -> Result<(), ProgramError> {
        require!(value.is_none(), ProgramError::InvalidInstructionData);
        Ok(())
    }
}
