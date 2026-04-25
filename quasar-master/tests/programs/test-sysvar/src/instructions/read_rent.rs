use {
    crate::state::{RentSnapshot, RentSnapshotInner},
    quasar_lang::{
        prelude::*,
        sysvars::{rent::Rent, Sysvar as _},
    },
};

#[derive(Accounts)]
pub struct ReadRent<'info> {
    pub payer: &'info mut Signer,
    #[account(init, payer = payer, seeds = RentSnapshot::seeds(), bump)]
    pub snapshot: &'info mut Account<RentSnapshot>,
    pub system_program: &'info Program<System>,
}

impl<'info> ReadRent<'info> {
    #[inline(always)]
    pub fn handler(&mut self) -> Result<(), ProgramError> {
        let rent = Rent::get()?;
        let min_balance = rent.minimum_balance_unchecked(100);
        self.snapshot.set_inner(RentSnapshotInner {
            min_balance_100: min_balance,
        });
        Ok(())
    }
}
