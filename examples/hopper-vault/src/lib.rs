//! # Hopper Vault Example
//!
//! Macro-first SOL vault using Hopper's first-touch API:
//! `#[account]`, `#[derive(Accounts)]`, `Ctx<T>`, and `ctx.accounts.*`.
//!
//! Instructions:
//! - `0` = Initialize vault
//! - `1` = Deposit SOL
//! - `2` = Withdraw SOL

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

/// Account DSL alternative for teams that want the older systems-style context.
mod dsl;

#[cfg(test)]
mod tests;

// --- State ----------------------------------------------------------

mod state;
pub use state::{Vault, VaultFields};

// --- Errors ---------------------------------------------------------

hopper::hopper_error! {
    base = 6000;
    Unauthorized,
    InsufficientBalance,
    ZeroAmount,
}

// --- Contexts -------------------------------------------------------

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(init, payer = payer, space = Vault::INIT_SPACE)]
    pub vault: InitAccount<'info, Vault>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut, has_one = authority)]
    pub vault: Account<'info, Vault>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut, has_one = authority)]
    pub vault: Account<'info, Vault>,
}

#[program]
mod vault_program {
    use super::*;

    #[instruction(0)]
    pub fn initialize(ctx: Ctx<Initialize>) -> ProgramResult {
        ctx.init_vault_with(VaultFields {
            authority: *ctx.accounts.payer.key(),
            balance: 0,
            bump: 0,
        })
    }

    #[instruction(1)]
    pub fn deposit(ctx: Ctx<Deposit>, amount: u64) -> ProgramResult {
        ctx.accounts.deposit(amount)
    }

    #[instruction(2)]
    pub fn withdraw(ctx: Ctx<Withdraw>, amount: u64) -> ProgramResult {
        ctx.accounts.withdraw(amount)
    }
}

impl<'info> Initialize<'info> {
    pub fn initialize(&self) -> ProgramResult {
        let mut vault = self.vault.get_mut_after_init()?;
        vault.set_fields(VaultFields {
            authority: *self.payer.key(),
            balance: 0,
            bump: 0,
        })
    }
}

impl<'info> Deposit<'info> {
    pub fn deposit(&self, amount: u64) -> ProgramResult {
        hopper::hopper_require!(amount > 0, ZeroAmount);

        let authority = self.authority.as_account();
        let vault_account = self.vault.as_account();
        hopper::system::Transfer {
            from: authority,
            to: vault_account,
            lamports: amount,
        }
        .invoke()?;

        self.vault
            .with_mut(|vault| vault.balance.checked_add_assign(amount))
    }
}

impl<'info> Withdraw<'info> {
    pub fn withdraw(&self, amount: u64) -> ProgramResult {
        hopper::hopper_require!(amount > 0, ZeroAmount);

        let mut vault = self.vault.get_mut()?;
        if vault.balance.get() < amount {
            return Err(InsufficientBalance.into());
        }
        vault.balance.checked_sub_assign(amount)?;
        drop(vault);

        let authority = self.authority.as_account();
        let vault_account = self.vault.as_account();

        // The vault is program-owned, so withdraw debits lamports directly
        // after Hopper has validated authority and account layout.
        transfer_lamports(vault_account, authority, amount)
    }
}
