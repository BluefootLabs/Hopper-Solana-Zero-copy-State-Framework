use {
    crate::state::{ItemAccount, ItemAccountInner},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
pub struct InitInstructionSeed<'info> {
    pub payer: &'info mut Signer,
    pub authority: &'info Signer,
    #[account(init, payer = payer, seeds = ItemAccount::seeds(authority), bump)]
    pub item: &'info mut Account<ItemAccount>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitInstructionSeed<'info> {
    #[inline(always)]
    pub fn handler(
        &mut self,
        id: u64,
        bumps: &InitInstructionSeedBumps,
    ) -> Result<(), ProgramError> {
        self.item.set_inner(ItemAccountInner {
            id,
            bump: bumps.item,
        });
        Ok(())
    }
}
