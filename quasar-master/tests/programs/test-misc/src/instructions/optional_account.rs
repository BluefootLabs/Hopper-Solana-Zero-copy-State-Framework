use {crate::state::SimpleAccount, quasar_lang::prelude::*};

#[derive(Accounts)]
pub struct OptionalAccount<'info> {
    pub required: &'info Account<SimpleAccount>,
    pub optional: Option<&'info Account<SimpleAccount>>,
}

impl<'info> OptionalAccount<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
