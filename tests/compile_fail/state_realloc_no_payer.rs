//! `#[account(realloc = ...)]` without `realloc_payer = <field>` must
//! be rejected. Growing the account may require topping up the
//! rent-exempt lamport minimum, and Hopper needs to know which
//! account funds that top-up.

use hopper::prelude::*;
use hopper::{context, state};

#[state(disc = 1, version = 1)]
#[derive(Copy, Clone)]
#[repr(C)]
pub struct Vault {
    pub balance: WireU64,
}

#[context]
pub struct ReallocNoPayer {
    #[account(realloc = 128, realloc_zero = true)]
    pub vault: Vault,
}

fn main() {}
