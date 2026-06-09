//! Argus-style risk guard subsystem proof.
//!
//! The account tracks reserved exposure under an authority-owned limit. It is a
//! compact real-world state pattern: initialize, reserve, release, and reject
//! limit violations without leaving Hopper's safe account facade.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 91, version = 1)]
pub struct RiskBook {
    pub authority: Address,
    pub exposure: WireU64,
    pub limit: WireU64,
    pub bump: u8,
}

hopper::hopper_error! {
    base = 7100;
    ZeroAmount,
    LimitExceeded,
    InsufficientExposure,
}

#[derive(Accounts)]
pub struct InitializeRisk<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(init, payer = authority, space = RiskBook::INIT_SPACE)]
    pub risk_book: InitAccount<'info, RiskBook>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct MutateRisk<'info> {
    #[account(mut, has_one = authority)]
    pub risk_book: Account<'info, RiskBook>,
    pub authority: Signer<'info>,
}

#[program(profile = "audit")]
mod argus_guard {
    use super::*;

    #[instruction(0)]
    pub fn initialize(ctx: Ctx<InitializeRisk>, limit: u64, bump: u8) -> ProgramResult {
        hopper::hopper_require!(limit > 0, ZeroAmount);
        ctx.init_risk_book()?;
        ctx.accounts.risk_book.with_mut_after_init(|risk| {
            risk.set_inner(*ctx.accounts.authority.key(), 0, limit, bump)
        })
    }

    #[instruction(1)]
    pub fn reserve(ctx: Ctx<MutateRisk>, amount: u64) -> ProgramResult {
        ctx.accounts.reserve(amount)
    }

    #[instruction(2)]
    pub fn release(ctx: Ctx<MutateRisk>, amount: u64) -> ProgramResult {
        ctx.accounts.release(amount)
    }
}

impl<'info> MutateRisk<'info> {
    pub fn reserve(&self, amount: u64) -> ProgramResult {
        hopper::hopper_require!(amount > 0, ZeroAmount);
        self.risk_book.with_mut(|risk| {
            let next = risk
                .exposure
                .get()
                .checked_add(amount)
                .ok_or(ProgramError::ArithmeticOverflow)?;
            if next > risk.limit.get() {
                return Err(LimitExceeded.into());
            }
            risk.exposure.set(next);
            Ok(())
        })
    }

    pub fn release(&self, amount: u64) -> ProgramResult {
        hopper::hopper_require!(amount > 0, ZeroAmount);
        self.risk_book.with_mut(|risk| {
            if risk.exposure.get() < amount {
                return Err(InsufficientExposure.into());
            }
            risk.exposure.checked_sub_assign(amount)
        })
    }
}

#[cfg(test)]
// These tests assert relationships between derived inherent consts; the
// constant value of the assertion is precisely what is under test.
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;

    #[test]
    fn risk_book_layout_is_stable() {
        assert_eq!(RiskBook::DISC, 91);
        assert_eq!(RiskBook::VERSION, 1);
        assert!(RiskBook::INIT_SPACE >= 49);
    }
}
