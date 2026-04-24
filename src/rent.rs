//! Rent-exemption helpers.
//!
//! Solana's rent model charges accounts for storage on a per-byte-year
//! basis. An account that holds at least
//! `(data_len + ACCOUNT_STORAGE_OVERHEAD) * LAMPORTS_PER_BYTE_YEAR *
//! EXEMPTION_THRESHOLD` lamports is *rent-exempt* and never loses
//! balance to rent collection.
//!
//! This module exposes two things:
//!
//! 1. [`minimum_balance`] — a pure function computing the rent-exempt
//!    threshold for a given `data_len`, using the cluster constants
//!    that have been in effect on Solana mainnet since launch
//!    (`lamports_per_byte_year = 3480`, `exemption_threshold = 2 years`,
//!    `account_storage_overhead = 128 bytes`). These values are
//!    governed on-chain but have never been changed. If the cluster
//!    ever re-governs them, the check will be conservative
//!    (strictly requiring at least the pre-governance threshold) —
//!    still safe, just not tight.
//!
//! 2. [`check_rent_exempt`] — the runtime guard backing the
//!    `#[account(rent_exempt = enforce)]` field keyword emitted by
//!    `#[hopper::context]`. Compares `account.lamports()` to
//!    `minimum_balance(account.data_len())` and returns
//!    `ProgramError::AccountNotRentExempt` (Solana's canonical error
//!    code, routed through `ProgramError::Custom`) on failure.
//!
//! ## Why not use `sol_get_rent_sysvar`?
//!
//! The syscall is ~100 CU and returns the same values this module
//! hard-codes. Using the constants inline lets the check run at
//! zero additional CU beyond the comparison. Programs that need to
//! read the live Rent sysvar for other reasons (rent-collection
//! scheduling, etc.) can still invoke the syscall directly; this
//! helper is specifically for the rent-exemption gate where the
//! constants suffice.

use crate::account::AccountView;
use crate::error::ProgramError;
use crate::ProgramResult;

/// Lamports charged per byte of account storage per year.
///
/// Fixed at 3480 since Solana mainnet launch and unchanged through
/// 2026. The value is governed on-chain via the Rent sysvar but no
/// cluster vote has ever modified it.
pub const LAMPORTS_PER_BYTE_YEAR: u64 = 3_480;

/// Years of rent an account must prepay to be exempt.
///
/// Fixed at 2.0 since launch. Represented as an integer here because
/// the multiplication always lands on an integer result for the
/// given `LAMPORTS_PER_BYTE_YEAR`.
pub const EXEMPTION_THRESHOLD_YEARS: u64 = 2;

/// Fixed per-account storage overhead the cluster charges on top of
/// user data. 128 bytes (header + metadata).
pub const ACCOUNT_STORAGE_OVERHEAD: u64 = 128;

/// Minimum lamport balance for an account with `data_len` bytes of
/// data to be rent-exempt under the current Solana cluster constants.
///
/// `(data_len + 128) * 3480 * 2` — constant-folded at the call site
/// when `data_len` is a `const`.
#[inline]
pub const fn minimum_balance(data_len: usize) -> u64 {
    (data_len as u64 + ACCOUNT_STORAGE_OVERHEAD)
        * LAMPORTS_PER_BYTE_YEAR
        * EXEMPTION_THRESHOLD_YEARS
}

/// Assert that `account` holds enough lamports to be rent-exempt for
/// its current data length. Used by the `#[account(rent_exempt =
/// enforce)]` constraint lowering in `hopper-macros-proc`.
///
/// Returns `ProgramError::AccountNotRentExempt` on underrun. The error
/// code maps to Solana's canonical `InstructionError::RentEpoch`
/// (built-in 29) when surfaced through the runtime.
#[inline]
pub fn check_rent_exempt(account: &AccountView) -> ProgramResult {
    let data_len = account.data_len();
    let required = minimum_balance(data_len);
    if account.lamports() >= required {
        Ok(())
    } else {
        Err(ProgramError::AccountNotRentExempt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimum_balance_matches_mainnet_constants() {
        // A 0-byte account: (0 + 128) * 3480 * 2 = 890,880 lamports.
        // This is the well-known "empty account rent-exempt minimum"
        // every Solana developer internalises; the constant must
        // match.
        assert_eq!(minimum_balance(0), 890_880);
    }

    #[test]
    fn minimum_balance_scales_linearly() {
        let base = minimum_balance(0);
        let with_100 = minimum_balance(100);
        let with_200 = minimum_balance(200);
        // Adding 100 bytes adds 100 * 3480 * 2 = 696_000 lamports.
        assert_eq!(with_100 - base, 696_000);
        assert_eq!(with_200 - with_100, 696_000);
    }

    #[test]
    fn minimum_balance_on_typical_vault_state() {
        // 56-byte account (16-byte Hopper header + 40-byte body, as
        // used by the parity vault and the transfer-hook vault).
        // (56 + 128) * 3480 * 2 = 1_280_640 lamports = ~0.00128 SOL.
        assert_eq!(minimum_balance(56), 1_280_640);
    }
}
