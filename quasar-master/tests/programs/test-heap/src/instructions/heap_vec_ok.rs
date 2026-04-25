// NOTE: This instruction MUST be declared with #[instruction(heap)] in lib.rs.
// Without the heap attribute, the cursor is not initialized and alloc will
// corrupt memory or abort.
extern crate alloc;
use {alloc::vec, quasar_lang::prelude::*};

#[derive(Accounts)]
pub struct HeapVecOk<'info> {
    pub signer: &'info Signer,
}

impl<'info> HeapVecOk<'info> {
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
