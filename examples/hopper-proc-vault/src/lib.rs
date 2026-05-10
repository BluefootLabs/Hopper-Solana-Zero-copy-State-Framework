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

#[cfg(test)]
mod abs_offset_tests {
    //! Regression tests for the `{FIELD}_ABS_OFFSET` inherent constants
    //! emitted by `#[hopper::state]`. These close the Hopper Safety
    //! Audit's DX2 ergonomic gap: segment callers should not have to
    //! hand-assemble `HEADER_LEN + Vault::BALANCE_OFFSET` at every
    //! call site.
    use super::Vault;
    use hopper::hopper_core::account::HEADER_LEN;

    #[test]
    fn balance_abs_offset_is_header_plus_offset() {
        assert_eq!(
            Vault::BALANCE_ABS_OFFSET,
            HEADER_LEN as u32 + Vault::BALANCE_OFFSET
        );
    }

    #[test]
    fn pending_rewards_abs_offset_is_header_plus_offset() {
        assert_eq!(
            Vault::PENDING_REWARDS_ABS_OFFSET,
            HEADER_LEN as u32 + Vault::PENDING_REWARDS_OFFSET
        );
    }

    #[test]
    fn abs_offsets_are_strictly_increasing() {
        assert!(Vault::BALANCE_ABS_OFFSET < Vault::PENDING_REWARDS_ABS_OFFSET);
    }
}

#[cfg(test)]
mod schema_metadata_tests {
    //! Stage 2.5 regression tests. `#[hopper::context]` must emit a
    //! `SCHEMA_METADATA` const carrying every Anchor-grade constraint
    //! field so downstream IDL/client tooling can consume it without
    //! re-parsing source.
    use super::{AdminSweep, Deposit};
    use hopper::hopper_schema::accounts::AccountLifecycle;

    #[test]
    fn deposit_metadata_names_context_and_accounts() {
        assert_eq!(Deposit::SCHEMA_METADATA.name, "Deposit");
        assert_eq!(Deposit::SCHEMA_METADATA.accounts.len(), 2);
        assert_eq!(Deposit::SCHEMA_METADATA.accounts[0].name, "vault");
        assert_eq!(Deposit::SCHEMA_METADATA.accounts[1].name, "authority");
    }

    #[test]
    fn deposit_vault_is_existing_not_init() {
        let vault = &Deposit::SCHEMA_METADATA.accounts[0];
        assert_eq!(vault.lifecycle, AccountLifecycle::Existing);
        assert!(vault.writable);
        assert_eq!(vault.layout_ref, "Vault");
        assert_eq!(vault.init_space, 0);
    }

    #[test]
    fn deposit_authority_is_signer_only() {
        let authority = &Deposit::SCHEMA_METADATA.accounts[1];
        assert!(authority.signer);
        // `#[signer]` is a segment marker, not full `mut`; writable
        // defaults false for pure-signer fields.
        assert_eq!(authority.layout_ref, "");
        assert_eq!(authority.lifecycle, AccountLifecycle::Existing);
    }

    #[test]
    fn admin_sweep_metadata_roundtrips() {
        assert_eq!(AdminSweep::SCHEMA_METADATA.name, "AdminSweep");
        assert_eq!(AdminSweep::SCHEMA_METADATA.accounts.len(), 2);
    }
}
