use {
    crate::state::{SpaceTestAccount, SpaceTestAccountInner},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
pub struct SpaceOverride<'info> {
    pub payer: &'info mut Signer,
    #[account(init, space = 100, seeds = SpaceTestAccount::seeds(payer), bump)]
    pub account: &'info mut Account<SpaceTestAccount>,
    pub system_program: &'info Program<System>,
}

impl<'info> SpaceOverride<'info> {
    #[inline(always)]
    pub fn handler(&mut self, value: u64, bumps: &SpaceOverrideBumps) -> Result<(), ProgramError> {
        self.account.set_inner(SpaceTestAccountInner {
            authority: *self.payer.address(),
            value,
            bump: bumps.account,
        });
        Ok(())
    }
}
