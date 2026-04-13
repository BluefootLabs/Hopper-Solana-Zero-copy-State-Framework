#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

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

#[hopper::context]
pub struct AdminSweep {
    #[account(mut)]
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
        let vault = ctx.vault_load()?;
        vault.balance.get() >= vault.pending_rewards.get()
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

    #[instruction(1)]
    pub fn admin_sweep(ctx: Context<AdminSweep>) -> ProgramResult {
        let account = ctx.vault_account()?;
        let _vault_address = account.address();

        {
            let mut vault = ctx.vault_load_mut()?;
            vault.pending_rewards = WireU64::new(0);
        }

        let raw = ctx.vault_raw_ref()?;
        let _cleared_rewards = raw.pending_rewards.get();
        Ok(())
    }
}