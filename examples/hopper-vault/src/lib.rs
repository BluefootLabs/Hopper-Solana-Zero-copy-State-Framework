//! # Hopper Vault Example
//!
//! Demonstrates the Hopper framework with a simple SOL vault program.
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
    use super::*;

    #[cfg(not(feature = "solana-program-backend"))]
    no_allocator!();

    #[cfg(not(feature = "solana-program-backend"))]
    nostd_panic_handler!();
}

/// Account DSL alternative -- same vault logic using hopper_accounts! macro.
mod dsl;

#[cfg(test)]
mod tests;

// --- Layout ---------------------------------------------------------

hopper_layout! {
    /// A simple SOL vault account.
    pub struct Vault, disc = 1, version = 1 {
        authority: TypedAddress<Authority> = 32,
        balance:   WireU64                = 8,
        bump:      u8                     = 1,
    }
}

// --- Errors ---------------------------------------------------------

hopper_error! {
    base = 6000;
    Unauthorized,
    InsufficientBalance,
    ZeroAmount,
}

// --- Entrypoint -----------------------------------------------------

#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    hopper::hopper_dispatch! {
        program_id, accounts, instruction_data;
        0 => process_init,
        1 => process_deposit,
        2 => process_withdraw,
    }
}

// --- Init -----------------------------------------------------------

fn process_init(program_id: &Address, accounts: &[AccountView], _data: &[u8]) -> ProgramResult {
    if accounts.len() < 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let payer = &accounts[0];
    let vault_account = &accounts[1];
    let system_program = &accounts[2];

    payer.check_signer()?.check_writable()?;
    vault_account.check_writable()?;

    // Create account via CPI
    hopper_init!(payer, vault_account, system_program, program_id, Vault)?;

    // Write initial state
    let mut vault = Vault::load_mut(vault_account, program_id)?;
    let vault = vault.get_mut();
    vault.authority = TypedAddress::from_account(payer);
    vault.balance = WireU64::new(0);

    Ok(())
}

// --- Deposit (phased) -----------------------------------------------

struct DepositArgs {
    amount: u64,
}

impl<'a> InstructionArgs<'a> for DepositArgs {
    fn parse(data: &'a [u8]) -> Result<Self, ProgramError> {
        if data.len() < 8 {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(Self {
            amount: u64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]),
        })
    }
}

impl ValidateArgs for DepositArgs {
    fn validate(&self) -> Result<(), ProgramError> {
        hopper_require!(self.amount > 0, ZeroAmount);
        Ok(())
    }
}

struct DepositAccounts<'a> {
    depositor: &'a AccountView,
    vault: &'a AccountView,
}

fn process_deposit(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let args = DepositArgs::parse(data)?;
    args.validate()?;

    PhasedFrame::new(program_id, accounts, data)?
        .resolve(2, |accts, _pid| {
            Ok(DepositAccounts {
                depositor: &accts[0],
                vault: &accts[1],
            })
        })?
        .validate_with_args(&args, |ctx, pid, _args| {
            ctx.depositor.check_signer()?.check_writable()?;
            ctx.vault.check_owned_by(pid)?.check_writable()?;
            Ok(())
        })?
        .execute_with_args(&args, |ctx, args| {
            // Load vault
            let mut vault = Vault::load_mut(ctx.resolved().vault, ctx.program_id())?;

            // Transfer SOL: depositor -> vault
            let dep_lamports = ctx.resolved().depositor.lamports();
            ctx.resolved().depositor.set_lamports(
                dep_lamports
                    .checked_sub(args.amount)
                    .ok_or(ProgramError::InsufficientFunds)?,
            );
            let vault_lamports = ctx.resolved().vault.lamports();
            ctx.resolved().vault.set_lamports(
                vault_lamports
                    .checked_add(args.amount)
                    .ok_or(ProgramError::ArithmeticOverflow)?,
            );

            // Update balance
            let v = vault.get_mut();
            let new_balance = v
                .balance
                .get()
                .checked_add(args.amount)
                .ok_or(ProgramError::ArithmeticOverflow)?;
            v.balance = WireU64::new(new_balance);

            Ok(())
        })
}

// --- Withdraw (phased) ----------------------------------------------

fn process_withdraw(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let args = DepositArgs::parse(data)?; // Same format: 8-byte LE amount
    args.validate()?;

    PhasedFrame::new(program_id, accounts, data)?
        .resolve(2, |accts, _pid| {
            Ok(DepositAccounts {
                depositor: &accts[0], // authority
                vault: &accts[1],
            })
        })?
        .validate_with_args(&args, |ctx, pid, _args| {
            ctx.depositor.check_signer()?;
            ctx.vault.check_owned_by(pid)?.check_writable()?;
            Ok(())
        })?
        .execute_with_args(&args, |ctx, args| {
            let mut vault = Vault::load_mut(ctx.resolved().vault, ctx.program_id())?;
            let v = vault.get_mut();

            // Check authority
            v.authority.require_eq_account(ctx.resolved().depositor)?;

            // Check balance
            let balance = v.balance.get();
            if balance < args.amount {
                return Err(InsufficientBalance.into());
            }
            v.balance = WireU64::new(balance - args.amount);

            // Transfer SOL: vault -> authority
            let vault_lamports = ctx.resolved().vault.lamports();
            ctx.resolved().vault.set_lamports(
                vault_lamports
                    .checked_sub(args.amount)
                    .ok_or(ProgramError::InsufficientFunds)?,
            );
            let auth_lamports = ctx.resolved().depositor.lamports();
            ctx.resolved().depositor.set_lamports(
                auth_lamports
                    .checked_add(args.amount)
                    .ok_or(ProgramError::ArithmeticOverflow)?,
            );

            Ok(())
        })
}
