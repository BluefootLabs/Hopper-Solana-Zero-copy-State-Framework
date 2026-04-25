use quasar_lang::prelude::*;

#[derive(Accounts)]
pub struct OptionAddressSome<'info> {
    pub signer: &'info Signer,
}

impl<'info> OptionAddressSome<'info> {
    #[inline(always)]
    pub fn handler(&self, addr: Option<Address>) -> Result<(), ProgramError> {
        require!(addr.is_some(), ProgramError::InvalidInstructionData);
        Ok(())
    }
}
