use {
    crate::state::{ClockSnapshot, ClockSnapshotInner},
    quasar_lang::{
        prelude::*,
        sysvars::{clock::Clock, Sysvar as _},
    },
};

#[derive(Accounts)]
pub struct ReadClock<'info> {
    pub payer: &'info mut Signer,
    #[account(init, payer = payer, seeds = ClockSnapshot::seeds(), bump)]
    pub snapshot: &'info mut Account<ClockSnapshot>,
    pub system_program: &'info Program<System>,
}

impl<'info> ReadClock<'info> {
    #[inline(always)]
    pub fn handler(&mut self) -> Result<(), ProgramError> {
        let clock = Clock::get()?;
        self.snapshot.set_inner(ClockSnapshotInner {
            slot: clock.slot.get(),
            unix_timestamp: clock.unix_timestamp.get(),
        });
        Ok(())
    }
}
