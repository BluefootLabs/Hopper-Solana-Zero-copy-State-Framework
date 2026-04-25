use solana_address::Address;

pub const ID: Address = solana_address::address!("33333333333333333333333333333333333333333333");

pub mod instructions;
pub mod pda;

pub use {instructions::*, pda::*};
