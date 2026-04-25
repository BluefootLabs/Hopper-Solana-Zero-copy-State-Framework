use {
    crate::state::{IndexedAccount, IndexedAccountInner},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
#[instruction(index: u64)]
pub struct InitIxDataSeed<'info> {
    pub payer: &'info mut Signer,
    pub authority: &'info Signer,
    #[account(init, payer = payer, seeds = IndexedAccount::seeds(authority, index), bump)]
    pub item: &'info mut Account<IndexedAccount>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitIxDataSeed<'info> {
    #[inline(always)]
    pub fn handler(&mut self, index: u64, bumps: &InitIxDataSeedBumps) -> Result<(), ProgramError> {
        self.item.set_inner(IndexedAccountInner {
            authority: *self.authority.address(),
            index,
            bump: bumps.item,
        });
        Ok(())
    }
}
