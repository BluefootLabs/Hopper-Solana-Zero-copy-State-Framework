//! Minimal macro-first Hopper counter.
//!
//! This is the first-contact example: account, context, handler, done.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    hopper::no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    hopper::nostd_panic_handler!();
}

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 1, version = 1)]
pub struct Counter {
    pub authority: Address,
    pub value: WireU64,
}

#[derive(Accounts)]
pub struct Increment<'info> {
    #[account(mut, has_one = authority)]
    pub counter: Account<'info, Counter>,
    pub authority: Signer<'info>,
}

#[cfg(target_os = "solana")]
hopper::program_entrypoint!(process_instruction);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    counter_program::process_instruction(&mut ctx)
}

#[program]
mod counter_program {
    use super::*;

    #[instruction(0)]
    pub fn increment(ctx: Ctx<Increment>) -> ProgramResult {
        let mut counter = ctx.accounts.counter.get_mut()?;
        counter.value.checked_add_assign(1)?;
        Ok(())
    }
}
