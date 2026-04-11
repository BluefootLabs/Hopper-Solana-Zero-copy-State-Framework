#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;
use hopper::{hopper_context, hopper_program, hopper_state};

#[hopper_state]
pub struct Vault {
    pub balance: WireU64,
    pub pending_rewards: WireU64,
}

#[hopper_context]
pub struct Deposit {
    #[account(mut(balance))]
    pub vault: Vault,

    #[signer]
    pub authority: AccountView,
}

#[hopper_program]
mod vault_program {
    use super::*;

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