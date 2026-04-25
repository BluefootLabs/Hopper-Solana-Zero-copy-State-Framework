use {crate::state::VaultInterface, quasar_lang::prelude::*};

/// Accepts either VaultV1 or VaultV2 through a single InterfaceAccount field.
#[derive(Accounts)]
pub struct InterfaceMigrationCheck<'info> {
    pub vault: &'info InterfaceAccount<VaultInterface>,
}

impl<'info> InterfaceMigrationCheck<'info> {
    #[inline(always)]
    pub fn handler(&self) -> Result<(), ProgramError> {
        Ok(())
    }
}
