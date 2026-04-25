use {
    crate::state::{ExplicitPayerAccount, ExplicitPayerAccountInner},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
pub struct ExplicitPayer<'info> {
    pub funder: &'info mut Signer,
    #[account(init, payer = funder, seeds = ExplicitPayerAccount::seeds(funder), bump)]
    pub account: &'info mut Account<ExplicitPayerAccount>,
    pub system_program: &'info Program<System>,
}

impl<'info> ExplicitPayer<'info> {
    #[inline(always)]
    pub fn handler(&mut self, value: u64, bumps: &ExplicitPayerBumps) -> Result<(), ProgramError> {
        self.account.set_inner(ExplicitPayerAccountInner {
            authority: *self.funder.address(),
            value,
            bump: bumps.account,
        });
        Ok(())
    }
}
