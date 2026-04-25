use {
    crate::state::{ClockFullSnapshot, ClockFullSnapshotInner},
    quasar_lang::{
        prelude::*,
        sysvars::{clock::Clock, Sysvar as _},
    },
};

#[derive(Accounts)]
pub struct ReadClockFull<'info> {
    pub payer: &'info mut Signer,
    #[account(init, payer = payer, seeds = ClockFullSnapshot::seeds(), bump)]
    pub snapshot: &'info mut Account<ClockFullSnapshot>,
    pub system_program: &'info Program<System>,
}

impl<'info> ReadClockFull<'info> {
    #[inline(always)]
    pub fn handler(&mut self) -> Result<(), ProgramError> {
        let clock = Clock::get()?;
        self.snapshot.set_inner(ClockFullSnapshotInner {
            slot: clock.slot.get(),
            epoch_start_timestamp: clock.epoch_start_timestamp.get(),
            epoch: clock.epoch.get(),
            leader_schedule_epoch: clock.leader_schedule_epoch.get(),
            unix_timestamp: clock.unix_timestamp.get(),
        });
        Ok(())
    }
}
