//! # Pinocchio Parity Vault
//!
//! Idiomatic Anza Pinocchio implementation of the cross-framework parity
//! contract defined in [`bench/METHODOLOGY.md`](../../bench/METHODOLOGY.md).
//!
//! This is the **raw-substrate baseline** for the framework-vault
//! benchmark. It uses only `pinocchio` and `pinocchio-system` directly,
//! with no higher-level framework on top. The four instructions
//! (`deposit`, `withdraw`, `authorize`, `counter_access`) have identical
//! semantics to the Hopper, Quasar, and (when present) Anchor parity
//! vaults, so the CU deltas reported by the bench harness reflect
//! framework overhead, not behaviour divergence.
//!
//! ## Why this replaced the Quasar-authored `pinocchio_vault.so`
//!
//! The original bench loaded Quasar's reference pinocchio-style vault
//! from `$quasar_root/target/deploy/pinocchio_vault.so`. That artefact
//! served as an illustrative no-framework baseline, but its column in
//! the results table was easy to misread as "the Pinocchio framework".
//! This crate is built in-tree against Anza's own `pinocchio = "0.10"`
//! and `pinocchio-system = "0.5"` so the resulting `.so` is a direct,
//! unambiguous representation of idiomatic Pinocchio. See `AUDIT.md`
//! recommendation R2.
//!
//! ## Idiomatic choices
//!
//! - Standard `program_entrypoint!` (eager parse), not a lazy variant.
//! - `find_program_address` for PDA verification. A stored-bump +
//!   `create_program_address` path would be faster but is not what most
//!   Pinocchio programs ship, and is the main reason Hopper's
//!   verify-only sha256 path wins on authorize CU.
//! - `pinocchio_system::instructions::Transfer` for the deposit CPI.
//! - Direct lamport mutation for withdraw (program-owned vault).
//! - `try_borrow_mut` for the counter segment write.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(clippy::result_large_err)]

use pinocchio::{
    program_entrypoint,
    AccountView, Address, ProgramResult,
};
use pinocchio::error::ProgramError;
use pinocchio_system::instructions::Transfer;

#[cfg(target_os = "solana")]
pinocchio::no_allocator!();
#[cfg(target_os = "solana")]
pinocchio::nostd_panic_handler!();

program_entrypoint!(process_instruction);

/// Canonical all-zero Solana System Program address.
const SYSTEM_PROGRAM_ID: [u8; 32] = [0u8; 32];

/// Byte layout of the counter-access state account:
/// `[authority: [u8; 32]][counter: u64 LE]`.
const COUNTER_DATA_LEN: usize = 40;

#[inline(always)]
fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    let (disc, rest) = instruction_data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;

    match *disc {
        0 => process_deposit(program_id, accounts, rest),
        1 => process_withdraw(program_id, accounts, rest),
        2 => process_authorize(program_id, accounts),
        3 => process_counter_access(program_id, accounts),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

// ---------------------------------------------------------------------------
// Shared validation helpers
// ---------------------------------------------------------------------------

#[inline(always)]
fn parse_amount(data: &[u8]) -> Result<u64, ProgramError> {
    if data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    Ok(u64::from_le_bytes([
        data[0], data[1], data[2], data[3],
        data[4], data[5], data[6], data[7],
    ]))
}

/// Verify a vault PDA matches `find_program_address(["vault", user], program)`.
///
/// On Solana the SBF target exposes `Address::find_program_address` through
/// the `solana-address` crate that pinocchio re-exports. Off-chain (host
/// tests / `cargo check` with the default target) the function is unavailable
/// because it would need the curve25519 host fallback; the host stub
/// short-circuits to `Ok(())` so the rest of the program type-checks.
#[inline(always)]
fn verify_vault_pda(
    user: &AccountView,
    vault: &AccountView,
    program_id: &Address,
) -> ProgramResult {
    #[cfg(any(target_os = "solana", target_arch = "bpf"))]
    {
        let user_address = user.address();
        let (expected, _bump) = Address::find_program_address(
            &[b"vault", user_address.as_ref()],
            program_id,
        );
        if expected.to_bytes() != vault.address().to_bytes() {
            return Err(ProgramError::InvalidSeeds);
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "solana", target_arch = "bpf")))]
    {
        let _ = (user, vault, program_id);
        Ok(())
    }
}

#[inline(always)]
fn validate_authority(user: &AccountView) -> ProgramResult {
    if !user.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !user.is_writable() {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

#[inline(always)]
fn validate_writable(account: &AccountView) -> ProgramResult {
    if !account.is_writable() {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Instructions
// ---------------------------------------------------------------------------

/// Move `amount` lamports from `user` to `vault` via the system program.
///
/// Accounts: `[user (signer, mut), vault (mut), system_program]`.
fn process_deposit(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let [user, vault, system_program, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    validate_authority(user)?;
    let amount = parse_amount(data)?;
    validate_writable(vault)?;

    if system_program.address().to_bytes() != SYSTEM_PROGRAM_ID {
        return Err(ProgramError::IncorrectProgramId);
    }
    verify_vault_pda(user, vault, program_id)?;

    Transfer {
        from: user,
        to: vault,
        lamports: amount,
    }
    .invoke()
}

/// Withdraw `amount` lamports from the program-owned `vault` to `user`
/// via direct lamport mutation. Signer must match the vault's derived
/// authority (enforced by the PDA seed binding `["vault", user]`).
///
/// Accounts: `[user (signer, mut), vault (mut)]`.
fn process_withdraw(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let [user, vault, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    validate_authority(user)?;
    let amount = parse_amount(data)?;

    // Owner check. `owner()` is `unsafe` in pinocchio because it aliases
    // the BPF input buffer; we only deref it briefly for the byte
    // compare, so no aliasing conflict with the lamport writes below.
    let owner_bytes = unsafe { user_owner_bytes(vault) };
    if owner_bytes != program_id.to_bytes() {
        return Err(ProgramError::InvalidAccountOwner);
    }
    validate_writable(vault)?;
    verify_vault_pda(user, vault, program_id)?;

    let vault_lamports = vault.lamports();
    if amount > vault_lamports {
        return Err(ProgramError::InsufficientFunds);
    }
    let user_next = user
        .lamports()
        .checked_add(amount)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    vault.set_lamports(vault_lamports - amount);
    user.set_lamports(user_next);
    Ok(())
}

/// Validate that `user` is the signing authority for the vault PDA
/// without mutating any balances. Gate-style instruction used to
/// benchmark pure validation cost.
///
/// Accounts: `[user (signer, mut), vault (mut)]`.
fn process_authorize(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let [user, vault, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    validate_authority(user)?;
    validate_writable(vault)?;
    verify_vault_pda(user, vault, program_id)
}

/// Read the `authority` stored in the first 32 bytes of the vault, verify
/// it matches the signer, and increment the `counter` stored in the next
/// 8 bytes.
///
/// Accounts: `[user (signer, mut), vault (mut)]`.
fn process_counter_access(program_id: &Address, accounts: &[AccountView]) -> ProgramResult {
    let [user, vault, ..] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    validate_authority(user)?;
    verify_vault_pda(user, vault, program_id)?;

    let mut data = vault.try_borrow_mut()?;
    if data.len() < COUNTER_DATA_LEN {
        return Err(ProgramError::AccountDataTooSmall);
    }

    // Authority check: stored [u8; 32] must equal the signer's address.
    let user_bytes = user.address().to_bytes();
    if data[..32] != user_bytes {
        return Err(ProgramError::InvalidAccountData);
    }

    let counter = u64::from_le_bytes([
        data[32], data[33], data[34], data[35],
        data[36], data[37], data[38], data[39],
    ]);
    let next = counter
        .checked_add(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    data[32..40].copy_from_slice(&next.to_le_bytes());
    Ok(())
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Snapshot the owner address of `view` as a value. `AccountView::owner`
/// returns a reference that aliases the BPF input buffer, so it is
/// `unsafe`. This helper narrows the unsafe region to a brief byte copy.
///
/// # Safety
///
/// Caller must ensure no concurrent mutation of `view`'s header while
/// this function executes. Callers above only use it for an immediate
/// byte-array comparison, so the reference does not escape.
#[inline(always)]
unsafe fn user_owner_bytes(view: &AccountView) -> [u8; 32] {
    unsafe { view.owner() }.to_bytes()
}
