use {
    crate::state::{TestMiscProgram, RETURN_U64_VALUE},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
pub struct ReturnU64<'info> {
    pub program: &'info Program<TestMiscProgram>,
}

impl<'info> ReturnU64<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<u64, ProgramError> {
        Ok(RETURN_U64_VALUE)
    }
}
