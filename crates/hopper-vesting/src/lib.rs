#![no_std]
//! # hopper-vesting
//!
//! Linear, cliff, stepped, and periodic unlock schedules.
//!
//! Calculate how many tokens a user can claim right now, given a schedule
//! and a timestamp. Supports linear schedules with a cliff, stepped schedules,
//! periodic schedules, and combinations built from those primitives.

mod vesting;
pub use vesting::*;
