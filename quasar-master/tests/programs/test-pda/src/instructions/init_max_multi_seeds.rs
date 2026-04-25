use quasar_lang::prelude::*;

#[derive(Accounts)]
pub struct InitMaxMultiSeeds<'info> {
    pub payer: &'info mut Signer,
    pub authority: &'info Signer,
    #[account(
        seeds = [
            b"max", b"max", b"max", b"max", b"max",
            b"max", b"max", b"max", b"max", b"max",
            b"max", b"max", b"max", b"max", b"max",
        ],
        bump
    )]
    pub complex: &'info UncheckedAccount,
    pub system_program: &'info Program<System>,
}

impl<'info> InitMaxMultiSeeds<'info> {
    #[inline(always)]
    pub fn handler(&mut self) -> Result<(), ProgramError> {
        Ok(())
    }
}
