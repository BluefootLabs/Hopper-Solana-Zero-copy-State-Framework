use {crate::state::SimpleAccount, quasar_lang::prelude::*};

#[derive(Accounts)]
pub struct UpdateAddress<'info> {
    #[account(address = crate::EXPECTED_ADDRESS)]
    pub target: &'info Account<SimpleAccount>,
}

impl<'info> UpdateAddress<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
