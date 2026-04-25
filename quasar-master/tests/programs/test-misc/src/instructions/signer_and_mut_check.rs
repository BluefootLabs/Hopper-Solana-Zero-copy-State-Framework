use {
    crate::state::{SimpleAccount, SimpleAccountInner},
    quasar_lang::prelude::*,
};

#[derive(Accounts)]
pub struct SignerAndMutCheck<'info> {
    #[account(mut)]
    pub account: &'info mut Account<SimpleAccount>,
    pub signer: &'info Signer,
}

impl<'info> SignerAndMutCheck<'info> {
    #[inline(always)]
    pub fn handler(&mut self, new_value: u64) -> Result<(), ProgramError> {
        let authority = self.account.authority;
        let bump = self.account.bump;
        self.account.set_inner(SimpleAccountInner {
            authority,
            value: new_value,
            bump,
        });
        Ok(())
    }
}
