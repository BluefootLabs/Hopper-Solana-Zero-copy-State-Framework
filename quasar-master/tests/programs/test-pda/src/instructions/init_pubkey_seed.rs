use {
    crate::state::{UserAccount, UserAccountInner},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
pub struct InitPubkeySeed<'info> {
    pub payer: &'info mut Signer,
    #[account(init, payer = payer, seeds = UserAccount::seeds(payer), bump)]
    pub user: &'info mut Account<UserAccount>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitPubkeySeed<'info> {
    #[inline(always)]
    pub fn handler(&mut self, value: u64, bumps: &InitPubkeySeedBumps) -> Result<(), ProgramError> {
        self.user.set_inner(UserAccountInner {
            authority: *self.payer.address(),
            value,
            bump: bumps.user,
        });
        Ok(())
    }
}
