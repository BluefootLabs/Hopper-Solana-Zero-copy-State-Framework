// NOTE: This instruction is intentionally NOT marked #[instruction(heap)].
// In release builds, the heap cursor is set past the end of the heap region,
// so any allocation triggers: alloc returns null -> handle_alloc_error -> panic
// -> abort.
extern crate alloc;
use {alloc::vec, quasar_lang::prelude::*};

#[derive(Accounts)]
pub struct NoHeapAllocAttempt<'info> {
    pub signer: &'info Signer,
}

impl<'info> NoHeapAllocAttempt<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        #[allow(clippy::useless_vec)]
        let v = vec![1u8; 64];
        if core::hint::black_box(v.len()) != 64 {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(())
    }
}
