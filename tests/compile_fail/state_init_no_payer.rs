//! `#[account(init)]` without a `payer = ...` attribute must be
//! rejected at macro-expansion time. `init` triggers a System Program
//! CreateAccount CPI and Hopper needs to know which account funds it.

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
    #[account(init, space = Vault::LEN)]
    pub vault: Vault,

    pub system_program: AccountView,
}

fn main() {}
