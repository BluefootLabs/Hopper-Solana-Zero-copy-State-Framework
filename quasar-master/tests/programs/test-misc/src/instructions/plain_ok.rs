use {crate::state::TestMiscProgram, quasar_lang::prelude::*};

#[derive(Accounts)]
pub struct PlainOk<'info> {
    pub program: &'info Program<TestMiscProgram>,
}

impl<'info> PlainOk<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
