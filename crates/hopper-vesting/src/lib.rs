#![no_std]
//! # hopper-vesting
//!
//! Linear, cliff, stepped, and periodic unlock schedules.
//!
//! Calculate how many tokens a user can claim right now, given a schedule
//! and a timestamp. Covers every common vesting curve: linear with cliff,
//! stepped (monthly/quarterly), periodic, and custom combinations.

mod vesting;
pub use vesting::*;
