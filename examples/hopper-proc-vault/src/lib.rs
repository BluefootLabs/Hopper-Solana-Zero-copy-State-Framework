#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 1, version = 1)]
pub struct Vault {
    pub balance: WireU64,
    pub pending_rewards: WireU64,
}

#[hopper::context]
pub struct Deposit {
    #[account(mut(balance), read(balance, pending_rewards))]
    pub vault: Vault,

    #[signer]
    pub authority: AccountView,
}

#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    vault_program::process_instruction(&mut ctx)
}

#[hopper::program]
mod vault_program {
    use super::*;

    #[hopper::pipeline]
    #[hopper::receipt]
    #[hopper::invariant({
        let balance = ctx.vault_balance_ref()?.get();
        let pending_rewards = ctx.vault_pending_rewards_ref()?.get();
        balance >= pending_rewards
    })]
    #[instruction(0)]
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> ProgramResult {
        let mut balance = ctx.vault_balance_mut()?;
        let next = balance
            .get()
            .checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        *balance = WireU64::new(next);
        Ok(())
    }
}