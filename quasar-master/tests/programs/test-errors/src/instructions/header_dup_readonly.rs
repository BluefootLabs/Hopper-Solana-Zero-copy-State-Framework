use quasar_lang::prelude::*;

/// Tests: duplicate readonly aliases are accepted when explicitly annotated.
#[derive(Accounts)]
pub struct HeaderDupReadonly<'info> {
    pub source: &'info Signer,
    /// CHECK: test-only — validates that duplicate readonly aliases are parsed
    /// correctly.
    #[account(dup)]
    pub destination: &'info UncheckedAccount,
}

impl<'info> HeaderDupReadonly<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
