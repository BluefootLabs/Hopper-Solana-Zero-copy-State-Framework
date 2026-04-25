use {crate::state::SimpleAccount, quasar_lang::prelude::*};

#[derive(Accounts)]
pub struct UpdateHasOne<'info> {
    pub authority: &'info Signer,
    #[account(has_one = authority, seeds = SimpleAccount::seeds(authority), bump = account.bump)]
    pub account: &'info Account<SimpleAccount>,
}

impl<'info> UpdateHasOne<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
