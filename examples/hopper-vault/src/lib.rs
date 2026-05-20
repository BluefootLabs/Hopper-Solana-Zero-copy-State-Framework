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
    #[cfg(not(feature = "solana-program-backend"))]
    hopper::no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    hopper::nostd_panic_handler!();
}

/// Account DSL alternative for teams that want the older systems-style context.
mod dsl;

#[cfg(test)]
mod tests;

// --- State ----------------------------------------------------------

#[derive(Clone, Copy)]
#[repr(C)]
#[account(discriminator = 1, version = 1)]
pub struct Vault {
    pub authority: Address,
    pub balance: WireU64,
    pub bump: u8,
}

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
        ctx.init_vault()?;
        ctx.accounts.initialize()
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
        vault.set_inner(*self.payer.key(), 0, 0)
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

        let mut vault = self.vault.get_mut()?;
        vault.balance.checked_add_assign(amount)?;
        Ok(())
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
        vault_account.set_lamports(
            vault_account
                .lamports()
                .checked_sub(amount)
                .ok_or(ProgramError::InsufficientFunds)?,
        );
        authority.set_lamports(
            authority
                .lamports()
                .checked_add(amount)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        );

        Ok(())
    }
}
