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
        Ok(live_rent()?.minimum_balance(data_len))
    }
    #[cfg(not(target_os = "solana"))]
    {
        Ok(minimum_balance(data_len))
    }
}

/// Per-invocation cache of the Rent sysvar, in the reserved heap scratch
/// (`hopper_native::RENT_CACHE_HEAP_OFFSET`). All-zero is the empty cache,
/// which is what the VM's zeroed heap gives every invocation; a CPI callee
/// runs in its own VM with its own heap, so no state crosses frames.
#[cfg(target_os = "solana")]
#[repr(C)]
struct RentCache {
    /// Nonzero once loaded: the rate is the loaded flag, so the hot path is
    /// one load and one branch. (A cluster whose rate is zero would read
    /// the sysvar on every call, which is still correct.)
    lamports_per_byte_year: u64,
    threshold_bits: u64,
    burn_percent: u64,
    _spare: u64,
}

#[cfg(target_os = "solana")]
const _: () = assert!(core::mem::size_of::<RentCache>() == hopper_native::RENT_CACHE_BYTES);

/// The live Rent sysvar, read once per invocation.
///
/// The first call reads the sysvar (110 CU through `sol_get_sysvar`) and
/// stores it in the reserved heap scratch; every later call in the same
/// invocation is one load and a branch. An instruction that creates two
/// accounts, tops one up, and checks another's exemption used to pay for
/// four syscalls; it now pays for one. Off-chain this is the documented
/// launch snapshot (3,480 lamports per byte-year at threshold 2.0), the
/// same values [`minimum_balance`] uses.
#[inline]
pub fn live_rent() -> Result<hopper_native::sysvar::Rent, ProgramError> {
    #[cfg(target_os = "solana")]
    {
        let cache = (hopper_native::HEAP_START_ADDRESS + hopper_native::RENT_CACHE_HEAP_OFFSET)
            as *mut RentCache;
        // SAFETY: SBF execution is single-threaded; the cache lies inside
        // `HEAP_RUNTIME_RESERVED`, a range the bump allocator never hands
        // out and that the gate store and touch log stop short of
        // (const-asserted at their definitions); the VM zeroes the heap on
        // every invocation and all-zero is the empty cache; the address is
        // 8-aligned (a multiple of 32 above the 8-aligned heap start).
        unsafe {
            let rate = (*cache).lamports_per_byte_year;
            if rate != 0 {
                return Ok(hopper_native::sysvar::Rent {
                    lamports_per_byte_year: rate,
                    exemption_threshold: f64::from_bits((*cache).threshold_bits),
                    burn_percent: (*cache).burn_percent as u8,
                });
            }
            let rent = hopper_native::sysvar::get_rent()?;
            (*cache).threshold_bits = rent.exemption_threshold.to_bits();
            (*cache).burn_percent = rent.burn_percent as u64;
            // The rate last: it is the loaded flag.
            (*cache).lamports_per_byte_year = rent.lamports_per_byte_year;
            Ok(rent)
        }
    }
    #[cfg(not(target_os = "solana"))]
    {
        Ok(hopper_native::sysvar::Rent {
            lamports_per_byte_year: LAMPORTS_PER_BYTE_YEAR,
            exemption_threshold: EXEMPTION_THRESHOLD_YEARS as f64,
            burn_percent: 50,
        })
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
