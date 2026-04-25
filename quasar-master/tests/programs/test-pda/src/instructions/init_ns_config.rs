use {
    crate::state::{NamespaceConfig, NamespaceConfigInner},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
pub struct InitNsConfig<'info> {
    pub payer: &'info mut Signer,
    #[account(init, payer = payer, seeds = NamespaceConfig::seeds(), bump)]
    pub config: &'info mut Account<NamespaceConfig>,
    pub system_program: &'info Program<System>,
}

impl<'info> InitNsConfig<'info> {
    pub fn handler(
        &mut self,
        namespace: u32,
        bumps: &InitNsConfigBumps,
    ) -> Result<(), ProgramError> {
        self.config.set_inner(NamespaceConfigInner {
            namespace,
            bump: bumps.config,
        });
        Ok(())
    }
}
