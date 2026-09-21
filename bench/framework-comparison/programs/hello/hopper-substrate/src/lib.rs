//! Hopper substrate half of the pina hello comparison.
//!
//! Same contract as pina's hand-written pinocchio floor
//! (`benchmarks/framework-comparison/programs/hello/pinocchio`): one
//! instruction, no data read, log exactly `Hello, Solana!`, return `Ok`.
//! The verifier passes one readonly signer and asserts the log line.
//!
//! This is the raw `hopper::program_entrypoint!` path: no context binding,
//! no dispatch table, so it measures the entrypoint and the log syscall
//! and nothing else.
#![cfg_attr(target_os = "solana", no_std)]

use hopper::prelude::{AccountView, Address, ProgramResult};

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

// The verifier hands over exactly one account; declaring the bound keeps
// the entrypoint's account scratch at one slot instead of the default
// transaction maximum.
#[cfg(target_os = "solana")]
hopper::program_entrypoint!(process_instruction, 1);

pub fn process_instruction(
    _program_id: &Address,
    _accounts: &[AccountView],
    _instruction_data: &[u8],
) -> ProgramResult {
    hopper::substrate::log::log("Hello, Solana!");
    Ok(())
}
