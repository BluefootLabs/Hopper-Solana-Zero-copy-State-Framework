//! Dust-safe proportional distribution and fee extraction.
//!
//! Splitting a token amount among N recipients with integer division
//! always leaves a remainder. These functions handle the dust so that
//! `sum(parts) == total` is guaranteed, and `net + fee == amount` holds
//! exactly.

use hopper_runtime::error::ProgramError;

/// Split `total` proportionally by `shares`, writing results to `out`.
///
/// Uses the largest-remainder method: floor-divide first, then hand out
/// the leftover one unit at a time to the first N recipients. Guarantees
/// `out[0] + out[1] + ... == total`.
///
/// `shares` and `out` must have the same length.
#[inline(always)]
pub fn proportional_split(
    total: u64,
    shares: &[u64],
    out: &mut [u64],
) -> Result<(), ProgramError> {
    if shares.len() != out.len() || shares.is_empty() {
        return Err(ProgramError::InvalidArgument);
    }
    let total_shares: u128 = {
        let mut s = 0u128;
        let mut i = 0;
        while i < shares.len() {
            s += shares[i] as u128;
            i += 1;
        }
        s
    };
    if total_shares == 0 {
        return Err(ProgramError::InvalidArgument);
    }

    let t128 = total as u128;

    // First pass: floor division
    let mut distributed = 0u64;
    let mut i = 0;
    while i < shares.len() {
        let amt = ((shares[i] as u128) * t128 / total_shares) as u64;
        out[i] = amt;
        distributed = distributed
            .checked_add(amt)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        i += 1;
    }

    // Second pass: distribute remainder one unit at a time
    let mut remainder = total
        .checked_sub(distributed)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let mut j = 0;
    while remainder > 0 {
        out[j] += 1;
        remainder -= 1;
        j += 1;
        if j >= out.len() {
            j = 0;
        }
    }

    Ok(())
}

/// Extract a fee from `amount` and return `(net, fee)`.
///
/// `fee = ceil(amount * fee_bps / 10_000) + flat_fee`
///
/// Ceiling rounds in favor of the protocol. Guarantees `net + fee == amount`.
#[inline(always)]
pub fn extract_fee(
    amount: u64,
    fee_bps: u64,
    flat_fee: u64,
) -> Result<(u64, u64), ProgramError> {
    #[allow(clippy::manual_div_ceil)]
    let bps_fee = ((amount as u128) * (fee_bps as u128) + 9_999) / 10_000;
    let total_fee_128 = bps_fee + flat_fee as u128;
    if total_fee_128 > amount as u128 {
        return Err(ProgramError::InsufficientFunds);
    }
    let total_fee = total_fee_128 as u64;
    let net = amount - total_fee;
    Ok((net, total_fee))
}
