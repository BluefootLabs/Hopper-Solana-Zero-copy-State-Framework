use quasar_lang::prelude::*;

#[account(discriminator = 1, set_inner)]
#[seeds(b"clock")]
pub struct ClockSnapshot {
    pub slot: u64,
    pub unix_timestamp: i64,
}

#[account(discriminator = 2, set_inner)]
#[seeds(b"rent")]
pub struct RentSnapshot {
    pub min_balance_100: u64,
}

#[account(discriminator = 3, set_inner)]
#[seeds(b"clock_full")]
pub struct ClockFullSnapshot {
    pub slot: u64,
    pub epoch_start_timestamp: i64,
    pub epoch: u64,
    pub leader_schedule_epoch: u64,
    pub unix_timestamp: i64,
}

#[account(discriminator = 4, set_inner)]
#[seeds(b"rent_calc")]
pub struct RentCalcSnapshot {
    pub min_balance: u64,
}
