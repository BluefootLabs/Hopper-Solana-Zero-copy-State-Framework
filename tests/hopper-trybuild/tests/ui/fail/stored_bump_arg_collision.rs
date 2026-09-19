//! The bare identifier `stored` is reserved in `bump = ...` position (it
//! selects the account's `#[bump]`-marked field). A context that ALSO
//! declares an `#[instruction(stored: ...)]` argument previously captured
//! that argument silently, the reservation must refuse the ambiguous
//! combination loudly and point at the `bump = (stored)` spelling.

use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct Config {
    pub admin: Address,
    #[bump]
    pub bump: u8,
    pub reserved: [u8; 7],
}

#[derive(Accounts)]
#[instruction(stored: u8)]
pub struct Ambiguous<'info> {
    #[account(seeds = [b"cfg"], bump = stored)]
    pub config: Account<'info, Config>,
}

fn main() {}
