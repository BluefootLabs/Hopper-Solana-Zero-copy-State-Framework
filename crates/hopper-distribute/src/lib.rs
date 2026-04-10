#![no_std]
//! # hopper-distribute
//!
//! Weighted splits and basis-point fee extraction.
//!
//! Split a token amount N ways by weight, extract protocol fees, and
//! guarantee that `sum(parts) == total` -- no dust left behind.

mod distribute;
pub use distribute::*;
