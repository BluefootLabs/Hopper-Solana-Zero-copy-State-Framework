//! On-chain application quotas: authority, limits and neighboring cells stay
//! outside a consumer's mutable byte grant. These units are not SPL tokens.
#![cfg_attr(target_os = "solana", no_std)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

pub const CELLS: usize = 4;

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 92, version = 1)]
pub struct AllowanceBook {
    pub authority: Address,
    pub delegates: [Address; CELLS],
    pub limits: [WireU64; CELLS],
    pub spent: [WireU64; CELLS],
    pub revisions: [WireU64; CELLS],
}

hopper::hopper_error! {
    base = 7800;
    UnauthorizedDelegate,
    StaleRevision,
    LimitExceeded,
    ZeroAmount,
    LimitBelowSpent
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(init, payer = authority, space = AllowanceBook::LEN)]
    pub book: InitAccount<'info, AllowanceBook>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[accounts(strict_writes, lamports())]
#[instruction(slot: u16)]
pub struct Consume<'info> {
    pub delegate: Signer<'info>,
    #[account(cells(slot; spent, revisions))]
    pub book: Account<'info, AllowanceBook>,
}

#[derive(Accounts)]
#[accounts(strict_writes, lamports())]
#[instruction(slot: u16)]
pub struct SetLimit<'info> {
    pub authority: Signer<'info>,
    #[account(cells(slot; limits, revisions), has_one = authority)]
    pub book: Account<'info, AllowanceBook>,
}

#[program(max_accounts = 3, sealed)]
pub mod byte_allowance {
    use super::*;

    #[instruction(0)]
    pub fn initialize(
        ctx: Ctx<Initialize>,
        delegate0: Address,
        delegate1: Address,
        delegate2: Address,
        delegate3: Address,
        limit: u64,
    ) -> ProgramResult {
        ctx.init_book()?;
        let mut book = ctx.accounts.book.get_mut_after_init()?;
        book.authority = *ctx.accounts.authority.key();
        book.delegates = [delegate0, delegate1, delegate2, delegate3];
        book.limits = [WireU64::new(limit); CELLS];
        book.spent = [WireU64::new(0); CELLS];
        book.revisions = [WireU64::new(0); CELLS];
        Ok(())
    }

    #[instruction(1, ctx_args = 1)]
    pub fn consume(
        mut ctx: Ctx<Consume>,
        slot: u16,
        expected_revision: u64,
        amount: u64,
    ) -> ProgramResult {
        let index = usize::from(slot);
        hopper::hopper_require!(index < CELLS, ProgramError::InvalidInstructionData);
        hopper::hopper_require!(amount != 0, ZeroAmount);
        let (spent, limit, revision) = {
            let book = ctx.accounts.book.get()?;
            hopper::hopper_require!(
                book.delegates[index] == *ctx.accounts.delegate.key(),
                UnauthorizedDelegate
            );
            (
                book.spent[index].get(),
                book.limits[index].get(),
                book.revisions[index].get(),
            )
        };
        hopper::hopper_require!(revision == expected_revision, StaleRevision);
        let next_spent = spent
            .checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        let next_revision = revision
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        hopper::hopper_require!(next_spent <= limit, LimitExceeded);
        // The selector is captured by bind(). No manually repeated byte offset,
        // element type, or second selector can drift from the declared policy.
        *ctx.book_spent_cell_mut()? = WireU64::new(next_spent);
        *ctx.book_revisions_cell_mut()? = WireU64::new(next_revision);
        Ok(())
    }

    #[instruction(2, ctx_args = 1)]
    pub fn set_limit(
        mut ctx: Ctx<SetLimit>,
        slot: u16,
        expected_revision: u64,
        limit: u64,
    ) -> ProgramResult {
        let index = usize::from(slot);
        hopper::hopper_require!(index < CELLS, ProgramError::InvalidInstructionData);
        let (spent, revision) = {
            let book = ctx.accounts.book.get()?;
            (book.spent[index].get(), book.revisions[index].get())
        };
        hopper::hopper_require!(revision == expected_revision, StaleRevision);
        hopper::hopper_require!(limit >= spent, LimitBelowSpent);
        let next_revision = revision
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        *ctx.book_limits_cell_mut()? = WireU64::new(limit);
        *ctx.book_revisions_cell_mut()? = WireU64::new(next_revision);
        Ok(())
    }
}

hopper::program_manifest! {
    program = byte_allowance,
    layouts = [AllowanceBook],
    events = [],
}
