use quasar_lang::prelude::*;

#[derive(Accounts)]
pub struct OptionAddressNone<'info> {
    pub signer: &'info Signer,
}

impl<'info> OptionAddressNone<'info> {
    #[inline(always)]
    pub fn handler(&self, addr: Option<Address>) -> Result<(), ProgramError> {
        require!(addr.is_none(), ProgramError::InvalidInstructionData);
        Ok(())
    }
}
