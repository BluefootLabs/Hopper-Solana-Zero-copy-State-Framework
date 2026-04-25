use {crate::state::UserAccount, quasar_lang::prelude::*};

#[derive(Accounts)]
pub struct UpdatePda<'info> {
    pub authority: &'info Signer,
    #[account(mut, has_one = authority, seeds = UserAccount::seeds(authority), bump = user.bump)]
    pub user: &'info mut Account<UserAccount>,
}

impl<'info> UpdatePda<'info> {
    #[inline(always)]
    pub fn handler(&mut self, new_value: u64) -> Result<(), ProgramError> {
        self.user.value = new_value.into();
        Ok(())
    }
}
