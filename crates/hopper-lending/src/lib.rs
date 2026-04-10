#![no_std]
//! # hopper-lending
//!
//! Collateralization, liquidation, interest, utilization.
//!
//! Every lending protocol does the same math: collateral ratios, health
//! checks, liquidation thresholds, interest rate curves, utilization rates.
//! This crate gives you the building blocks so you write the logic once
//! and get it right. All basis-point denominated, all overflow-checked.

mod lending;
pub use lending::*;
