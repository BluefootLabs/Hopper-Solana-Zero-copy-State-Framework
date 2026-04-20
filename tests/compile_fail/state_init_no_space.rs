//! `#[account(init)]` without `space = ...` must be rejected.
//! Hopper needs the byte count for the CreateAccount CPI.

use hopper::prelude::*;
use hopper::{context, state};

#[state(disc = 1, version = 1)]
#[derive(Copy, Clone)]
#[repr(C)]
pub struct Vault {
    pub balance: WireU64,
}

#[context]
pub struct InitVault {
    pub payer: AccountView,

    #[account(init, payer = payer)]
    pub vault: Vault,

    pub system_program: AccountView,
}

fn main() {}
