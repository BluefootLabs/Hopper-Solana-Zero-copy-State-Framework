use {
    crate::state::{MaxSeedAccount, MaxSeedAccountInner},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
pub struct InitMaxSeedLength<'info> {
    pub payer: &'info mut Signer,
    #[account(init, payer = payer, seeds = MaxSeedAccount::seeds(), bump)]
    pub max_seed: &'info mut Account<MaxSeedAccount>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitMaxSeedLength<'info> {
    #[inline(always)]
    pub fn handler(&mut self, bumps: &InitMaxSeedLengthBumps) -> Result<(), ProgramError> {
        self.max_seed.set_inner(MaxSeedAccountInner {
            bump: bumps.max_seed,
        });
        Ok(())
    }
}
