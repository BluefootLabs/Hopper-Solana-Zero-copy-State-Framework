use {
    crate::state::{EmptySeedAccount, EmptySeedAccountInner},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
pub struct InitEmptySeed<'info> {
    pub payer: &'info mut Signer,
    #[account(init, payer = payer, seeds = EmptySeedAccount::seeds(), bump)]
    pub empty: &'info mut Account<EmptySeedAccount>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitEmptySeed<'info> {
    #[inline(always)]
    pub fn handler(&mut self, bumps: &InitEmptySeedBumps) -> Result<(), ProgramError> {
        self.empty
            .set_inner(EmptySeedAccountInner { bump: bumps.empty });
        Ok(())
    }
}
