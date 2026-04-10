#![no_std]
//! # hopper-anchor
//!
//! Anchor framework interoperability bridge.
//!
//! Read accounts created by Anchor programs from Hopper programs.
//! Verify 8-byte SHA256 discriminators, extract the body after the
//! discriminator, and compute instruction/event discriminators.
//!
//! No dependency on `anchor-lang` -- discriminators are computed from
//! first principles (SHA256 prefix).

mod anchor;
pub use anchor::*;
