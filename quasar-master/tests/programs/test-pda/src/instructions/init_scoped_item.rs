use {
    crate::state::{ScopedItem, ScopedItemInner},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
#[instruction(namespace: u32)]
pub struct InitScopedItem<'info> {
    pub payer: &'info mut Signer,
    #[account(init, payer = payer, seeds = ScopedItem::seeds(namespace), bump)]
    pub item: &'info mut Account<ScopedItem>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitScopedItem<'info> {
    pub fn handler(
        &mut self,
        namespace: u32,
        bumps: &InitScopedItemBumps,
    ) -> Result<(), ProgramError> {
        self.item.set_inner(ScopedItemInner {
            namespace,
            data: 0,
            bump: bumps.item,
        });
        Ok(())
    }
}
