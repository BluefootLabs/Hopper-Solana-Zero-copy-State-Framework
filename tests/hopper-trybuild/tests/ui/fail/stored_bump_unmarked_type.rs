//! `bump = stored` reads the canonical bump from the state type's
//! `#[bump]`-marked field. A type WITHOUT the marker has no
//! `CANONICAL_BUMP_ABS_OFFSET`, so the context must fail to compile —
//! and a field merely NAMED `bump` changes nothing (no name-based
//! auto-detection).

use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct NoMarker {
    pub admin: Address,
    // Named `bump` but NOT `#[bump]`-marked: never auto-detected.
    pub bump: u8,
    pub reserved: [u8; 7],
}

#[derive(Accounts)]
pub struct UsesStored<'info> {
    #[account(seeds = [b"cfg"], bump = stored)]
    pub config: Account<'info, NoMarker>,
}

fn main() {}
