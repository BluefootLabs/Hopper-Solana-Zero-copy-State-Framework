//! Hopper macro half of the pina hello comparison.
//!
//! Same contract as pina's `hello` fixture: instruction data `00`, one
//! readonly signer, log exactly `Hello, Solana!`. This is what a Hopper
//! user writes day to day: a context with a `Signer` and a `#[program]`
//! module. Like Pina and Quasar (and unlike the pinocchio floor) the
//! signer is actually checked before the handler runs.
#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

#[derive(Accounts)]
pub struct Hello<'info> {
    pub authority: Signer<'info>,
}

#[program(profile = "tiny")]
mod hello_program {
    use super::*;

    #[instruction(0)]
    pub fn hello(_ctx: Ctx<Hello>) -> ProgramResult {
        hopper::substrate::log::log("Hello, Solana!");
        Ok(())
    }
}
