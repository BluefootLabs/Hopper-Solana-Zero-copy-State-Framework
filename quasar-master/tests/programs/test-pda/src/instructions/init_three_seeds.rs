use {
    crate::state::{ThreeSeedAccount, ThreeSeedAccountInner},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
pub struct InitThreeSeeds<'info> {
    pub payer: &'info mut Signer,
    pub first: &'info Signer,
    pub second: &'info Signer,
    #[account(init, payer = payer, seeds = ThreeSeedAccount::seeds(first, second), bump)]
    pub triple: &'info mut Account<ThreeSeedAccount>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitThreeSeeds<'info> {
    #[inline(always)]
    pub fn handler(&mut self, bumps: &InitThreeSeedsBumps) -> Result<(), ProgramError> {
        self.triple.set_inner(ThreeSeedAccountInner {
            first: *self.first.address(),
            second: *self.second.address(),
            bump: bumps.triple,
        });
        Ok(())
    }
}
