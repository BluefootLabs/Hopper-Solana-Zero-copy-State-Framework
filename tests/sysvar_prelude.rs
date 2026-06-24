//! Sysvar surface parity: the Clock/Rent sysvars that nearly every program
//! needs (on-chain time, rent-exempt minimums) must be reachable from the
//! ergonomic surfaces, not only the low-level `hopper::substrate` path.
//!
//! Off-chain (`not(target_os = "solana")`) the syscall blocks compile out, so
//! the getters return zeroed defaults — enough to prove the API exists, is
//! callable, and matches the `Sysvar::get()` ergonomics other frameworks ship.

use hopper::prelude::*;

#[test]
fn clock_and_rent_are_in_the_prelude() {
    // `Clock` / `Rent` resolve straight from `hopper::prelude::*`.
    let clock: Clock = Clock::get().expect("clock getter is callable");
    let rent: Rent = Rent::get().expect("rent getter is callable");

    // Off-chain default image.
    let _ = clock.unix_timestamp;
    let _ = clock.slot;
    let _ = rent.lamports_per_byte_year;
}

#[test]
fn sysvar_module_exposes_full_surface() {
    // Free-function getters mirror the method-style aliases.
    let _ = hopper::sysvar::get_clock().expect("get_clock");
    let _ = hopper::sysvar::get_rent().expect("get_rent");
    let _ = hopper::sysvar::get_epoch_schedule().expect("get_epoch_schedule");

    // Previously the dedicated `sol_get_last_restart_slot` syscall had no safe
    // wrapper; it does now.
    let restart = hopper::sysvar::get_last_restart_slot().expect("last restart slot");
    assert_eq!(restart, 0, "off-chain default");

    // EpochRewards (SIMD-0118) is module-level (niche, not prelude): the
    // typed reader and its address constant are reachable here.
    let rewards: hopper::sysvar::EpochRewards =
        hopper::sysvar::get_epoch_rewards().expect("get_epoch_rewards");
    assert!(!rewards.active, "off-chain default is inactive");
    assert_eq!(rewards.total_rewards, 0);

    // Well-known sysvar addresses are constants.
    assert_ne!(
        hopper::sysvar::CLOCK_ID,
        hopper::sysvar::LAST_RESTART_SLOT_ID
    );
    assert_ne!(
        hopper::sysvar::EPOCH_REWARDS_ID,
        hopper::sysvar::EPOCH_SCHEDULE_ID
    );
}

#[test]
fn rent_minimum_balance_matches_const_path() {
    // The const rent math and the sysvar-driven method agree for the canonical
    // mainnet constants, so a program can size accounts either way.
    let rent = Rent {
        lamports_per_byte_year: hopper::sysvar::LAMPORTS_PER_BYTE_YEAR,
        exemption_threshold: hopper::sysvar::EXEMPTION_THRESHOLD_YEARS as f64,
        burn_percent: 50,
    };
    let data_len = 165usize; // SPL token account size
    assert_eq!(
        rent.minimum_balance(data_len),
        hopper::sysvar::rent_exempt_minimum(data_len)
    );
}
