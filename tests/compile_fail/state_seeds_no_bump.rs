//! `#[account(seeds = [...])]` without a `bump` (inferred or stored)
//! must be rejected. The seeds alone don't uniquely identify a PDA . 
//! the bump byte is what lifts the 63/64 invalid derivations into the
//! valid address subspace.

use hopper::prelude::*;
use hopper::{context, state};

#[state(disc = 1, version = 1)]
#[derive(Copy, Clone)]
#[repr(C)]
pub struct Vault {
    pub balance: WireU64,
}

#[context]
pub struct SeedsMissBump {
    pub authority: AccountView,

    #[account(mut, seeds = [b"vault", authority.address().as_ref()])]
    pub vault: Vault,
}

fn main() {}
