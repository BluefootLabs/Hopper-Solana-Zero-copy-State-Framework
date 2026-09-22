//! Hopper macro half of the pina counter comparison.
//!
//! Same instruction contract as pina's counter fixture: PDA seeded by
//! `b"counter" + authority`, `initialize` takes the bump as its one
//! argument (`initializeTakesBump: true`), `increment` adds one with a
//! checked add. This is the day-to-day Hopper shape: `#[derive(Accounts)]`
//! with `init`/`seeds`/`bump`, a `#[program]` module, no hand-written
//! checks.
//!
//! The account is a headered Hopper account, so it is 25 bytes, not 10:
//! the 16-byte universal header (discriminator at byte 0, version, flags,
//! 8-byte layout fingerprint) then `[bump][count u64 LE]`. Pina's verifier
//! is told `--account-size 25 --bump-offset 16 --count-offset 17`; the
//! same caveat the table already carries for Anchor's 24-byte account.
//! `increment` verifies the PDA hash from the stored bump (`bump = stored`)
//! after owner/header/layout validation. Like Quasar's validated-account
//! path, it omits the curve check; the substrate, Pinocchio, and Pina rows
//! retain full PDA derivation. This fixture does not enable `strict_writes`.
#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

pub const SEED_COUNTER: &[u8] = b"counter";

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 1, version = 1)]
pub struct Counter {
    #[bump]
    pub bump: u8,
    pub count: WireU64,
}

#[derive(Accounts)]
#[instruction(bump: u8)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = Counter::INIT_SPACE,
        seeds = [SEED_COUNTER, authority.address().as_array()],
        bump = bump,
    )]
    pub counter: InitAccount<'info, Counter>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Increment<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [SEED_COUNTER, authority.address().as_array()],
        bump = stored,
    )]
    pub counter: Account<'info, Counter>,
}

// `initialize` is the widest instruction at three accounts; the bound is
// declared like the substrate fixture's `program_entrypoint!(_, 3)`.
#[program(profile = "tiny", max_accounts = 3)]
mod counter_program {
    use super::*;

    /// `ctx_args = 1` hands `bump` to the context binder, where
    /// `bump = bump` proves the PDA before the account is created.
    #[instruction(0, ctx_args = 1)]
    pub fn initialize(ctx: Ctx<Initialize>, bump: u8) -> ProgramResult {
        // One `CreateAccount` CPI signed with the PDA seeds, then the
        // header stamp; the generated lifecycle helper owns both.
        ctx.init_counter(bump)?;
        let mut counter = ctx.accounts.counter.get_mut_after_init()?;
        counter.bump = bump;
        counter.count = WireU64::ZERO;
        Ok(())
    }

    #[instruction(1)]
    pub fn increment(ctx: Ctx<Increment>) -> ProgramResult {
        let mut counter = ctx.accounts.counter.get_mut()?;
        counter.count.checked_add_assign(1)?;
        Ok(())
    }
}
