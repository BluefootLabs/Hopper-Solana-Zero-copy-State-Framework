//! Slippage, price bounds, and economic constraint checks.
//!
//! DeFi-specific safety primitives that catch the most common exploits:
//! missing slippage protection, dust attacks, and price manipulation.
//! Every swap, deposit, and withdrawal should use at least one of these.

use hopper_runtime::{error::ProgramError, ProgramResult};

/// Verify actual output meets the user's minimum (slippage protection).
///
/// Fails if `actual < minimum`. Put this between the swap math and the
/// transfer to the user.
///
/// ```rust,ignore
/// let output = do_swap(input_amount, pool)?;
/// check_slippage(output, user_min_output)?;
/// transfer_to_user(output)?;
/// ```
#[inline(always)]
pub fn check_slippage(actual_output: u64, minimum_output: u64) -> ProgramResult {
    if actual_output < minimum_output {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

/// Verify actual input does not exceed the user's maximum (exact-output swaps).
///
/// For swap interfaces where the user specifies the desired output and a
/// maximum input they're willing to spend.
#[inline(always)]
pub fn check_max_input(actual_input: u64, maximum_input: u64) -> ProgramResult {
    if actual_input > maximum_input {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

/// Verify an amount is at least a minimum value (anti-dust).
///
/// Prevents economically meaningless operations that waste compute and
/// can be used for griefing. Common in deposit, trade, and bid operations.
#[inline(always)]
pub fn check_min_amount(amount: u64, minimum: u64) -> ProgramResult {
    if amount < minimum {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

/// Verify an amount does not exceed a maximum (exposure limit).
///
/// Limits single-operation impact on pools, vaults, or markets.
/// Prevents flash-loan-sized operations from destabilizing the system.
#[inline(always)]
pub fn check_max_amount(amount: u64, maximum: u64) -> ProgramResult {
    if amount > maximum {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

/// Verify an amount is non-zero.
///
/// Surprisingly common bug: accepting zero-amount transfers, zero-amount
/// swaps, or zero-price listings. This is a dedicated check because the
/// error should be maximally clear in audit logs.
#[inline(always)]
pub fn check_nonzero(amount: u64) -> ProgramResult {
    if amount == 0 {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

/// Verify a value is within ±N basis points of an expected value.
///
/// Uses u128 intermediate math to avoid overflow. Common uses:
/// - Oracle price deviation checks (reject if > 50bps from TWAP)
/// - Fair-price assertions in liquidations
/// - Fee validation (actual fee within tolerance of expected)
///
/// ```rust,ignore
/// // Verify oracle price is within 100bps (1%) of TWAP
/// check_within_bps(oracle_price, twap_price, 100)?;
/// ```
#[inline(always)]
pub fn check_within_bps(actual: u64, expected: u64, tolerance_bps: u16) -> ProgramResult {
    if expected == 0 {
        return if actual == 0 {
            Ok(())
        } else {
            Err(ProgramError::InvalidArgument)
        };
    }

    let (larger, smaller) = if actual >= expected {
        (actual as u128, expected as u128)
    } else {
        (expected as u128, actual as u128)
    };

    let diff = larger - smaller;
    let deviation_bps = (diff * 10_000) / (expected as u128);

    if deviation_bps > tolerance_bps as u128 {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

/// Verify a price is within inclusive bounds (circuit-breaker pattern).
///
/// Use this to reject oracle prices that are clearly wrong or manipulated.
/// Common pattern: store a last-known-good price and reject if the new
/// price is outside a band.
///
/// ```rust,ignore
/// check_price_bounds(oracle_price, min_price, max_price)?;
/// ```
#[inline(always)]
pub fn check_price_bounds(price: u64, min_price: u64, max_price: u64) -> ProgramResult {
    if price < min_price || price > max_price {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}
