use {
    crate::state::{SimpleAccount, SimpleAccountInner},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
pub struct InitializeSimple<'info> {
    pub payer: &'info mut Signer,
    #[account(init, payer = payer, seeds = SimpleAccount::seeds(payer), bump)]
    pub account: &'info mut Account<SimpleAccount>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitializeSimple<'info> {
    #[inline(always)]
    pub fn handler(
        &mut self,
        value: u64,
        bumps: &InitializeSimpleBumps,
    ) -> Result<(), ProgramError> {
        self.account.set_inner(SimpleAccountInner {
            authority: *self.payer.address(),
            value,
            bump: bumps.account,
        });
        Ok(())
    }
}
