use {
    crate::state::{ReturnPayload, TestMiscProgram, RETURN_PAYLOAD_VALUE},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
pub struct ReturnPayloadInstruction<'info> {
    pub program: &'info Program<TestMiscProgram>,
}

impl<'info> ReturnPayloadInstruction<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<ReturnPayload, ProgramError> {
        Ok(RETURN_PAYLOAD_VALUE)
    }
}
