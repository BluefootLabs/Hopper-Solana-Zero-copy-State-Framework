//! Multi-signer threshold verification.
//!
//! M-of-N signature checking for governance, multisig wallets, and admin
//! operations. Counts signers, checks thresholds, and prevents the
//! duplicate-signer attack (same account passed in multiple slots).

use hopper_runtime::{error::ProgramError, AccountView, ProgramResult};

/// Count how many accounts in the slice are transaction signers.
#[inline(always)]
pub fn count_signers(accounts: &[&AccountView]) -> u8 {
    let mut n: u8 = 0;
    let mut i = 0;
    while i < accounts.len() {
        if accounts[i].is_signer() {
            n = n.saturating_add(1);
        }
        i += 1;
    }
    n
}

/// Require at least `threshold` of the provided accounts to be signers.
///
/// Also checks that all accounts have unique addresses to prevent the
/// duplicate-signer attack (passing the same signer key in multiple slots).
///
/// Returns `MissingRequiredSignature` if fewer than `threshold` are signers.
/// Returns `InvalidArgument` if duplicate addresses are found.
#[inline(always)]
pub fn check_threshold(accounts: &[&AccountView], threshold: u8) -> ProgramResult {
    // Check uniqueness (O(n^2) but n is always small, typically 3-9)
    let len = accounts.len();
    let mut i = 0;
    while i < len {
        let mut j = i + 1;
        while j < len {
            if accounts[i].address() == accounts[j].address() {
                return Err(ProgramError::InvalidArgument);
            }
            j += 1;
        }
        i += 1;
    }

    let signers = count_signers(accounts);
    if signers < threshold {
        return Err(ProgramError::MissingRequiredSignature);
    }
    Ok(())
}

/// Require ALL provided accounts to be signers (N-of-N).
///
/// Also checks uniqueness. Use this for operations that require
/// unanimous consent.
#[inline(always)]
pub fn check_all_signers(accounts: &[&AccountView]) -> ProgramResult {
    let len = accounts.len();
    if len > u8::MAX as usize {
        return Err(ProgramError::InvalidArgument);
    }
    check_threshold(accounts, len as u8)
}

/// Require exactly one of the provided accounts to be a signer (1-of-N).
///
/// Useful for "any admin can act" patterns. Checks uniqueness.
#[inline(always)]
pub fn check_any_signer(accounts: &[&AccountView]) -> ProgramResult {
    check_threshold(accounts, 1)
}
