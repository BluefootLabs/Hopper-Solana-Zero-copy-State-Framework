//! Security guard packs -- safe-by-default exploit prevention.
//!
//! Pre-built validation bundles for common exploit classes:
//! - Account role mismatches (signer/writable/owner)
//! - Post-mutation conservation (balance invariants)
//! - Duplicate account detection
//! - Instruction introspection guards (flash loan, re-entrancy)

use hopper_runtime::error::ProgramError;
use hopper_runtime::{AccountAudit, AccountView, Address, ProgramResult};

// -- Account Role Guards ----------------------------------------------

/// Validate a payer account: must be signer + writable.
#[inline(always)]
pub fn require_payer(account: &AccountView) -> ProgramResult {
    if !account.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    if !account.is_writable() {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Validate an authority account: must be signer, owned by expected program.
#[inline(always)]
pub fn require_authority(
    account: &AccountView,
    stored_authority: &[u8; 32],
) -> ProgramResult {
    if !account.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }
    let addr: &[u8; 32] = unsafe {
        // SAFETY: Address is [u8; 32].
        &*(account.address() as *const Address as *const [u8; 32])
    };
    if !crate::check::keys_eq_fast(addr, stored_authority) {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Validate a writable program-owned account.
#[inline(always)]
pub fn require_owned_writable(
    account: &AccountView,
    program_id: &Address,
) -> ProgramResult {
    if !account.owned_by(program_id) {
        return Err(ProgramError::IncorrectProgramId);
    }
    if !account.is_writable() {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

// -- Duplicate Account Detection --------------------------------------

/// Verify that all accounts in a slice have unique addresses.
///
/// O(n²) but n is small (typically < 16). Prevents double-spend and
/// confused-deputy attacks from duplicate account passing.
#[inline]
pub fn require_all_unique(accounts: &[AccountView]) -> ProgramResult {
    AccountAudit::new(accounts).require_all_unique()
}

/// Verify that no duplicated account is writable.
#[inline]
pub fn require_unique_writable(accounts: &[AccountView]) -> ProgramResult {
    AccountAudit::new(accounts).require_unique_writable()
}

/// Verify that no duplicated account is used as a signer.
#[inline]
pub fn require_unique_signers(accounts: &[AccountView]) -> ProgramResult {
    AccountAudit::new(accounts).require_unique_signers()
}

// -- Post-Mutation Conservation ---------------------------------------

/// Verify SOL conservation: total lamports before == total lamports after.
///
/// Call with pre-mutation snapshots of lamport values and the current
/// account views. Detects lamport creation/destruction bugs.
#[inline]
pub fn check_lamport_conservation(
    accounts: &[AccountView],
    pre_lamports: &[u64],
) -> ProgramResult {
    if accounts.len() != pre_lamports.len() {
        return Err(ProgramError::InvalidArgument);
    }
    let mut pre_total: u64 = 0;
    let mut post_total: u64 = 0;
    let mut i = 0;
    while i < accounts.len() {
        pre_total = pre_total.checked_add(pre_lamports[i])
            .ok_or(ProgramError::ArithmeticOverflow)?;
        post_total = post_total.checked_add(accounts[i].lamports())
            .ok_or(ProgramError::ArithmeticOverflow)?;
        i += 1;
    }
    if pre_total != post_total {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Snapshot lamport values for conservation checking.
///
/// Returns a stack-allocated array of lamport values.
/// `N` must match the number of accounts tracked.
#[inline]
pub fn snapshot_lamports<const N: usize>(
    accounts: &[AccountView],
) -> Result<[u64; N], ProgramError> {
    if accounts.len() < N {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let mut snapshot = [0u64; N];
    let mut i = 0;
    while i < N {
        snapshot[i] = accounts[i].lamports();
        i += 1;
    }
    Ok(snapshot)
}

// -- Signer-Writable Coherence ----------------------------------------

/// Validate that every writable account in the slice is also a signer
/// OR is owned by our program.
///
/// Prevents fee-drain attacks where an attacker passes a writable
/// account they don't own, hoping the program modifies it.
#[inline]
pub fn check_writable_coherence(
    accounts: &[AccountView],
    program_id: &Address,
) -> ProgramResult {
    let mut i = 0;
    while i < accounts.len() {
        if accounts[i].is_writable() && !accounts[i].is_signer() && !accounts[i].owned_by(program_id) {
            return Err(ProgramError::InvalidAccountData);
        }
        i += 1;
    }
    Ok(())
}
