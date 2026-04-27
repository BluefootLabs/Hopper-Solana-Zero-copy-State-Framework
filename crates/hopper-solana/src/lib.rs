//! # Hopper Solana
//!
//! Solana integration layer for the Hopper framework.
//!
//! Provides zero-copy readers for SPL Token/Mint accounts, Token-2022
//! extension screening, CPI guards, token-specific validation helpers,
//! transaction introspection, Ed25519/Merkle cryptography, balance delta
//! guards, compute budget monitoring, and two-step authority rotation.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

pub mod constants;
pub mod token;
pub mod mint;
pub mod interface;
pub mod cpi_guard;
pub mod typed_cpi;
pub mod introspect;
pub mod authority;
pub mod balance;
pub mod compute;
pub mod crypto;
pub mod ata;
pub mod token2022_ext;
pub mod oracle;
pub mod twap;
