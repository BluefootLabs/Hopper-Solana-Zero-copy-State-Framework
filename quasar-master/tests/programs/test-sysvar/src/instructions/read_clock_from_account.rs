use {
    crate::state::{ClockSnapshot, ClockSnapshotInner},
    quasar_lang::{prelude::*, sysvars::clock::Clock},
};

#[derive(Accounts)]
pub struct ReadClockFromAccount<'info> {
    pub _payer: &'info Signer,
    #[account(mut)]
    pub snapshot: &'info mut Account<ClockSnapshot>,
    pub clock: &'info Sysvar<Clock>,
}

impl<'info> ReadClockFromAccount<'info> {
    #[inline(always)]
    pub fn handler(&mut self) -> Result<(), ProgramError> {
        let clock = self.clock;
        self.snapshot.set_inner(ClockSnapshotInner {
            slot: clock.slot.get(),
            unix_timestamp: clock.unix_timestamp.get(),
        });
        Ok(())
    }
}
