use {crate::events::HeapTestEvent, quasar_lang::prelude::*};

#[derive(Accounts)]
pub struct EmitEventOk<'info> {
    pub signer: &'info Signer,
}

impl<'info> EmitEventOk<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        emit!(HeapTestEvent { value: 42 });
        Ok(())
    }
}
