//! Account DSL alternative for vault operations.
//!
//! Demonstrates hopper_accounts! macro for typed context generation.
//! The deposit instruction is reimplemented using the Account DSL pattern.

use super::Vault;
use hopper::prelude::*;

// --- Typed contexts via hopper_accounts! macro ----------------------

hopper_accounts! {
    pub struct DepositContext {
        depositor: (mut signer),
        vault: (mut account<Vault>),
    }
}

hopper_accounts! {
    pub struct WithdrawContext {
        authority: (mut signer),
        vault: (mut account<Vault>),
    }
}

// --- Deposit using Account DSL --------------------------------------

struct DslDepositIx;

impl<'a> HopperIx<'a> for DslDepositIx {
    type Accounts = DepositContext<'a>;
    type Args = u64;

    fn parse_args(data: &'a [u8]) -> Result<u64, ProgramError> {
        if data.len() < 8 {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(u64::from_le_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]))
    }
}

/// Process deposit via the typed Account DSL entry model.
///
/// Equivalent to `process_deposit` in lib.rs but with typed context
/// construction, automatic signer/writable/owner validation, and
/// schema introspection.
#[allow(dead_code)]
fn process_deposit_dsl(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    hopper_entry::<DslDepositIx, _>(program_id, accounts, data, |ctx, amount| {
        hopper_require!(amount > 0, super::ZeroAmount);

        // Transfer SOL: depositor -> vault
        let dep_view = ctx.accounts.depositor.to_account_view();
        let vault_view = ctx.accounts.vault.to_account_view();
        let dep_lamports = dep_view.lamports();
        dep_view.set_lamports(
            dep_lamports
                .checked_sub(amount)
                .ok_or(ProgramError::InsufficientFunds)?,
        );
        let vault_lamports = vault_view.lamports();
        vault_view.set_lamports(
            vault_lamports
                .checked_add(amount)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        );

        // Update balance in layout
        let mut vault = ctx.accounts.vault.write()?;
        let v = vault.get_mut();
        let new_balance = v
            .balance
            .get()
            .checked_add(amount)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        v.balance = WireU64::new(new_balance);

        Ok(())
    })
}

// --- Withdraw using Account DSL -------------------------------------

struct DslWithdrawIx;

impl<'a> HopperIx<'a> for DslWithdrawIx {
    type Accounts = WithdrawContext<'a>;
    type Args = u64;

    fn parse_args(data: &'a [u8]) -> Result<u64, ProgramError> {
        if data.len() < 8 {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(u64::from_le_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]))
    }
}

/// Process withdraw via the typed Account DSL entry model.
#[allow(dead_code)]
fn process_withdraw_dsl(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    hopper_entry::<DslWithdrawIx, _>(program_id, accounts, data, |ctx, amount| {
        hopper_require!(amount > 0, super::ZeroAmount);

        // Check authority
        let vault = ctx.accounts.vault.read()?;
        let v = vault.get();
        v.authority
            .require_eq_account(ctx.accounts.authority.to_account_view())?;

        // Check balance
        let balance = v.balance.get();
        if balance < amount {
            return Err(super::InsufficientBalance.into());
        }

        // Update balance
        let mut vault_mut = ctx.accounts.vault.write()?;
        let vm = vault_mut.get_mut();
        vm.balance = WireU64::new(balance - amount);

        // Transfer SOL: vault -> authority
        let vault_view = ctx.accounts.vault.to_account_view();
        let auth_view = ctx.accounts.authority.to_account_view();
        let vault_lamports = vault_view.lamports();
        vault_view.set_lamports(
            vault_lamports
                .checked_sub(amount)
                .ok_or(ProgramError::InsufficientFunds)?,
        );
        let auth_lamports = auth_view.lamports();
        auth_view.set_lamports(
            auth_lamports
                .checked_add(amount)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        );

        Ok(())
    })
}
