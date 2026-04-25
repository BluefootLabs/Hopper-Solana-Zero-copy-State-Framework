use {
    crate::state::{ComplexAccount, ComplexAccountInner},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
pub struct InitMultiSeeds<'info> {
    pub payer: &'info mut Signer,
    pub authority: &'info Signer,
    #[account(init, payer = payer, seeds = ComplexAccount::seeds(payer, authority), bump)]
    pub complex: &'info mut Account<ComplexAccount>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitMultiSeeds<'info> {
    #[inline(always)]
    pub fn handler(
        &mut self,
        amount: u64,
        bumps: &InitMultiSeedsBumps,
    ) -> Result<(), ProgramError> {
        self.complex.set_inner(ComplexAccountInner {
            authority: *self.authority.address(),
            amount,
            bump: bumps.complex,
        });
        Ok(())
    }
}
