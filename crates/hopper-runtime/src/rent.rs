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
//! 1. [`minimum_balance`] - a pure snapshot calculation using the launch-era
//!    constants (`lamports_per_byte_year = 3480`, `exemption_threshold = 2
//!    years`, `account_storage_overhead = 128 bytes`). It is useful for host
//!    tests and fixed-config calculations, but is not authoritative after an
//!    on-chain rent reprice.
//!
//! 2. [`check_rent_exempt`] - the runtime guard backing the
//!    `#[account(rent_exempt = enforce)]` field keyword emitted by
//!    `#[hopper::context]`. Compares `account.lamports()` to the live Rent
//!    sysvar minimum and returns
//!    `ProgramError::AccountNotRentExempt` (a builtin variant mapping
//!    to Solana's canonical code) on failure.
//!
//! The enforcement path deliberately reads `sol_get_rent_sysvar`. Rent is a
//! runtime-owned parameter, so a safety gate must fail closed if that read
//! fails rather than accepting an account against stale constants.

use crate::account::AccountView;
use crate::error::ProgramError;
use crate::ProgramResult;

/// Lamports charged per byte of account storage per year.
///
/// Launch-era snapshot. SIMD-0194 moved the full effective price into the
/// first Rent-sysvar field, and SIMD-0437 began repricing it in September
/// 2026. Runtime decisions must use [`minimum_balance_live`].
pub const LAMPORTS_PER_BYTE_YEAR: u64 = 3_480;

/// Years of rent an account must prepay to be exempt.
///
/// Launch-era snapshot. SIMD-0194 deprecated the threshold and changed its
/// live wire marker to `1.0`; this constant exists only for the paired legacy
/// calculation below.
pub const EXEMPTION_THRESHOLD_YEARS: u64 = 2;

/// Fixed per-account storage overhead the cluster charges on top of
/// user data. 128 bytes (header + metadata).
pub const ACCOUNT_STORAGE_OVERHEAD: u64 = 128;

/// Minimum lamport balance for an account with `data_len` bytes of data under
/// Solana's launch-era rent snapshot.
///
/// `(data_len + 128) * 3480 * 2` - constant-folded at the call site
/// when `data_len` is a `const`.
#[inline]
pub const fn minimum_balance(data_len: usize) -> u64 {
    (data_len as u64 + ACCOUNT_STORAGE_OVERHEAD)
        * LAMPORTS_PER_BYTE_YEAR
        * EXEMPTION_THRESHOLD_YEARS
}

/// Rent-exempt minimum read from the **live** Rent sysvar on-chain.
/// Host tests use the compile-time snapshot because no runtime sysvar exists.
///
/// Use this for value-bearing decisions, funding a new account, the
/// realloc top-up; so that if the cluster ever re-governs the rent
/// parameters, Hopper charges the live amount rather than a stale
/// hard-coded one. An on-chain sysvar read failure is returned to the caller;
/// value-bearing checks must not silently fall back to stale constants.
#[inline]
pub fn minimum_balance_live(data_len: usize) -> Result<u64, ProgramError> {
    #[cfg(target_os = "solana")]
    {
        let rent = hopper_native::sysvar::get_rent()?;
        Ok(rent.minimum_balance(data_len))
    }
    #[cfg(not(target_os = "solana"))]
    {
        Ok(minimum_balance(data_len))
    }
}

/// Assert that `account` holds enough lamports to be rent-exempt for
/// its current data length. Used by the `#[account(rent_exempt =
/// enforce)]` constraint lowering in `hopper-derive`.
///
/// Returns `ProgramError::AccountNotRentExempt` on underrun (builtin
/// index 14 in Hopper's error ABI, matching Solana's canonical
/// `AccountNotRentExempt` code).
#[inline]
pub fn check_rent_exempt(account: &AccountView<'_>) -> ProgramResult {
    let data_len = account.data_len();
    let required = minimum_balance_live(data_len)?;
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
    fn minimum_balance_matches_launch_snapshot() {
        // Historical empty-account minimum before SIMD-0437:
        // (0 + 128) * 3480 * 2 = 890,880 lamports.
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

    #[test]
    fn host_live_minimum_uses_the_documented_snapshot() {
        assert_eq!(minimum_balance_live(56), Ok(minimum_balance(56)));
    }
}
