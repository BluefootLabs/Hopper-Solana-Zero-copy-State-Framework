#![no_std]
//! # hopper-multisig
//!
//! M-of-N signer threshold checks.
//!
//! Verify that at least M out of N configured signers have signed the
//! current transaction, with built-in duplicate-signer prevention.
//! No heap allocation -- the signer set is walked in-place.

mod multisig;
pub use multisig::*;
