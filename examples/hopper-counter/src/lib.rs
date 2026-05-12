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
#[account(disc = 1, version = 1)]
pub struct Counter {
    pub authority: Address,
    pub value: WireU64,
}

#[accounts]
pub struct Increment {
    #[account(mut)]
    pub counter: Counter,

    #[signer]
    pub authority: AccountView,
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
    pub fn increment(ctx: Context<Increment>) -> ProgramResult {
        let authority = *ctx.authority_account()?.address();
        let mut counter = ctx.counter_load_mut()?;

        require_keys_eq!(
            counter.authority,
            authority,
            ProgramError::IncorrectAuthority
        );

        let next = counter
            .value
            .get()
            .checked_add(1)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        counter.value = WireU64::new(next);
        Ok(())
    }
}
