use {
    crate::state::{NamespaceConfig, ScopedItem},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
pub struct VerifyScopedItem<'info> {
    pub config: &'info Account<NamespaceConfig>,
    #[account(seeds = ScopedItem::seeds(config.namespace), bump = item.bump)]
    pub item: &'info Account<ScopedItem>,
}

impl<'info> VerifyScopedItem<'info> {
    pub fn handler(&mut self) -> Result<(), ProgramError> {
        Ok(())
    }
}
