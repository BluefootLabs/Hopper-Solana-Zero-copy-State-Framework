#![no_std]
//! # hopper-staking
//!
//! MasterChef-style reward accumulators.
//!
//! Reward-per-token accumulator, emission rates, pending rewards, and
//! reward debt tracking. The same math everyone copies from MasterChef,
//! except you don't have to re-derive it. u128 precision throughout.

mod staking;
pub use staking::*;
