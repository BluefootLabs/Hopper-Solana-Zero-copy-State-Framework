//! Dust-safe proportional distribution and fee extraction.
//!
//! Splitting a token amount among N recipients with integer division
//! always leaves a remainder. These functions handle the dust so that
//! `sum(parts) == total` is guaranteed, and `net + fee == amount` holds
//! exactly.

use hopper_runtime::error::ProgramError;

/// Split `total` proportionally by `shares`, writing results to `out`.
///
/// Uses the largest-remainder method: floor-divide first, then award each
/// leftover unit to the greatest fractional remainder. Equal remainders are
/// resolved by input order. Guarantees `out[0] + out[1] + ... == total`.
///
/// `shares` and `out` must have the same length.
#[inline(always)]
pub fn proportional_split(total: u64, shares: &[u64], out: &mut [u64]) -> Result<(), ProgramError> {
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

    // First pass: floor division.
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

    // The sum of the fractional remainders is less than the recipient count,
    // so each recipient can receive at most one leftover unit. Rank each
    // fractional remainder with O(n^2) comparisons to avoid allocating
    // scratch space. The comparisons use multiplications only: u128 `%` is a
    // software routine on SBF and dominated the cost of this loop.
    let remainder = total
        .checked_sub(distributed)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    if remainder == 0 {
        return Ok(());
    }

    let mut awarded = 0u64;
    i = 0;
    while i < shares.len() && awarded < remainder {
        let fractional = fractional_part(shares[i], t128, total_shares, out[i]);
        let mut rank = 0u64;
        let mut j = 0;
        while j < shares.len() && rank < remainder {
            let other = fractional_part(shares[j], t128, total_shares, out[j]);
            if other > fractional || (other == fractional && j < i) {
                rank += 1;
            }
            j += 1;
        }
        if rank < remainder {
            out[i] = out[i]
                .checked_add(1)
                .ok_or(ProgramError::ArithmeticOverflow)?;
            awarded += 1;
        }
        i += 1;
    }

    Ok(())
}

/// `share * total mod total_shares`, recovered from a part that holds either
/// the floor quotient or the floor quotient plus one leftover unit.
///
/// The true value lies in `[0, total_shares)` for a floor part and in
/// `[-total_shares, 0)` for a part that already received its unit, so
/// wrapping arithmetic modulo 2^128 recovers it exactly as long as
/// `total_shares <= 2^127`, which a sum of `u64` shares always satisfies.
#[inline(always)]
fn fractional_part(share: u64, total: u128, total_shares: u128, part: u64) -> u128 {
    let exact = (share as u128).wrapping_mul(total);
    let r = exact.wrapping_sub((part as u128).wrapping_mul(total_shares));
    if r >= total_shares {
        r.wrapping_add(total_shares)
    } else {
        r
    }
}

/// Extract a fee from `amount` and return `(net, fee)`.
///
/// `fee = ceil(amount * fee_bps / 10_000) + flat_fee`
///
/// Ceiling rounds in favor of the protocol. Guarantees `net + fee == amount`.
#[inline(always)]
pub fn extract_fee(amount: u64, fee_bps: u64, flat_fee: u64) -> Result<(u64, u64), ProgramError> {
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
