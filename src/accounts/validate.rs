//! Standalone validation helpers for the Account DSL.
//!
//! These complement the existing `check` module with hopper-native-native
//! signatures matching the Account DSL's conventions.

use hopper_runtime::error::ProgramError;
use hopper_runtime::{AccountView, Address};

/// Require that the account is a signer.
#[inline]
pub fn require_signer(account: &AccountView) -> Result<(), ProgramError> {
    crate::check::check_signer(account)
}

/// Require that the account is writable.
#[inline]
pub fn require_writable(account: &AccountView) -> Result<(), ProgramError> {
    crate::check::check_writable(account)
}

/// Require that the account is owned by the given address.
#[inline]
pub fn require_owner(account: &AccountView, owner: &Address) -> Result<(), ProgramError> {
    crate::check::check_owner(account, owner)
}

/// Require that the account is executable (for program references).
#[inline]
pub fn require_executable(account: &AccountView) -> Result<(), ProgramError> {
    crate::check::check_executable(account)
}

/// Verify a PDA matches expected seeds + bump + program.
#[inline]
pub fn require_pda(
    account: &AccountView,
    seeds: &[&[u8]],
    bump: u8,
    program_id: &Address,
) -> Result<(), ProgramError> {
    crate::check::verify_pda(account, seeds, bump, program_id)
}
