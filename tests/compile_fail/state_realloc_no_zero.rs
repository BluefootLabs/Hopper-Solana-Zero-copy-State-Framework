//! `#[account(realloc = ...)]` without an explicit `realloc_zero`
//! policy must be rejected. Newly-appended bytes contain whatever
//! was in the allocator's memory; leaving the policy implicit is a
//! subtle way to leak cross-instruction state. Hopper requires the
//! caller to opt-in to zeroing (or explicitly declare that leaving
//! garbage is acceptable for their protocol).

use hopper::prelude::*;
use hopper::{context, state};

#[state(disc = 1, version = 1)]
#[derive(Copy, Clone)]
#[repr(C)]
pub struct Vault {
    pub balance: WireU64,
}

#[context]
pub struct ReallocNoZero {
    pub payer: AccountView,

    #[account(realloc = 128, realloc_payer = payer)]
    pub vault: Vault,
}

fn main() {}
