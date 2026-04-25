use quasar_lang::prelude::*;

#[account(discriminator = 1)]
pub struct ErrorTestAccount {
    pub authority: Address,
    pub value: u64,
}
