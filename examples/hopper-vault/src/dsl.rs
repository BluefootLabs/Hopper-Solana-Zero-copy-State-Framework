//! Account DSL alternative for vault operations.
//!
//! Demonstrates hopper_accounts! macro for typed context generation.
//! The deposit instruction is reimplemented using the Account DSL pattern.

use super::Vault;
use hopper::prelude::*;
use hopper::systems::*;

// --- Typed contexts via hopper_accounts! macro ----------------------

hopper_accounts! {
    pub struct DepositContext {
        depositor: (mut signer),
        vault: (mut account<Vault>),
        system_program: (program),
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
        if data.len() != 8 {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(u64::from_le_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]))
    }
}

/// Process deposit via the typed Account DSL entry model.
///
/// Standalone alternative, not a dispatched instruction in the example ELF.
/// Includes explicit authority and System Program checks alongside the DSL's
/// signer/writable/owner validation.
#[allow(dead_code)]
pub(crate) fn process_deposit_dsl(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    hopper_entry::<DslDepositIx, _>(program_id, accounts, data, |ctx, amount| {
        hopper_require!(amount > 0, super::ZeroAmount);

        let vault = ctx.accounts.vault.read()?;
        if &vault.get().authority != ctx.accounts.depositor.to_account_view().address() {
            return Err(super::Unauthorized.into());
        }
        drop(vault);
        if ctx.accounts.system_program.to_account_view().address() != &System::ID {
            return Err(ProgramError::IncorrectProgramId);
        }

        // A program cannot directly debit a system-owned depositor.
        let dep_view = ctx.accounts.depositor.to_account_view();
        let vault_view = ctx.accounts.vault.to_account_view();
        hopper::system::Transfer {
            from: dep_view,
            to: vault_view,
            lamports: amount,
        }
        .invoke()?;

        // Update balance in layout
        let mut vault = ctx.accounts.vault.write()?;
        let v = vault.get_mut();
        v.balance.checked_add_assign(amount)?;

        Ok(())
    })
}

// --- Withdraw using Account DSL -------------------------------------

struct DslWithdrawIx;

impl<'a> HopperIx<'a> for DslWithdrawIx {
    type Accounts = WithdrawContext<'a>;
    type Args = u64;

    fn parse_args(data: &'a [u8]) -> Result<u64, ProgramError> {
        if data.len() != 8 {
            return Err(ProgramError::InvalidInstructionData);
        }
        Ok(u64::from_le_bytes([
            data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
        ]))
    }
}

/// Process withdraw via the typed Account DSL entry model.
#[allow(dead_code)]
pub(crate) fn process_withdraw_dsl(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    hopper_entry::<DslWithdrawIx, _>(program_id, accounts, data, |ctx, amount| {
        hopper_require!(amount > 0, super::ZeroAmount);

        // Check authority
        let vault = ctx.accounts.vault.read()?;
        let v = vault.get();
        if &v.authority != ctx.accounts.authority.to_account_view().address() {
            return Err(super::Unauthorized.into());
        }

        // Check balance
        let balance = v.balance.get();
        if balance < amount {
            return Err(super::InsufficientBalance.into());
        }
        drop(vault);

        // Update balance
        let mut vault_mut = ctx.accounts.vault.write()?;
        let vm = vault_mut.get_mut();
        vm.balance.checked_sub_assign(amount)?;
        drop(vault_mut);

        // Transfer SOL: vault -> authority
        let vault_view = ctx.accounts.vault.to_account_view();
        let auth_view = ctx.accounts.authority.to_account_view();
        transfer_lamports(vault_view, auth_view, amount)
    })
}
