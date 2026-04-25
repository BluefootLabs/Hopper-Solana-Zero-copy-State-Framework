use {
    crate::state::{NoDiscAccount, NoDiscAccountInner},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
pub struct InitNoDisc<'info> {
    pub payer: &'info mut Signer,
    #[account(init, payer = payer, seeds = NoDiscAccount::seeds(payer), bump)]
    pub account: &'info mut Account<NoDiscAccount>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitNoDisc<'info> {
    #[inline(always)]
    pub fn handler(&mut self, value: u64, _bumps: &InitNoDiscBumps) -> Result<(), ProgramError> {
        self.account.set_inner(NoDiscAccountInner {
            authority: *self.payer.address(),
            value,
        });
        Ok(())
    }
}

#[derive(Accounts)]
pub struct ReadNoDisc<'info> {
    #[account(mut)]
    pub account: &'info Account<NoDiscAccount>,
}

impl<'info> ReadNoDisc<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        // Just access the fields to verify Deref works.
        let _authority = self.account.authority;
        let _value = self.account.value;
        Ok(())
    }
}
