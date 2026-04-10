//! AMM math primitives: integer square root, constant-product swap, LP minting.
//!
//! Core constant-product math with u128 intermediates and overflow protection.
//! Zero alloc.

use hopper_runtime::error::ProgramError;

/// Integer square root via Newton's method.
///
/// Returns `floor(sqrt(val))`. Used for LP token minting: `sqrt(x * y)`.
///
/// ```rust,ignore
/// let lp_mint = isqrt(reserve_a as u128 * reserve_b as u128)?;
/// ```
#[inline(always)]
pub fn isqrt(val: u128) -> Result<u64, ProgramError> {
    if val == 0 {
        return Ok(0);
    }
    // Newton's method: x_{n+1} = (x_n + val / x_n) / 2
    let mut x = val;
    let mut y = (x + 1) >> 1;
    while y < x {
        x = y;
        y = (x + val / x) >> 1;
    }
    // x is now floor(sqrt(val)) as u128, narrow to u64
    if x > u64::MAX as u128 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    Ok(x as u64)
}

/// Compute output amount for a constant-product swap.
///
/// Formula: `out = (reserve_out * amount_in_after_fee) / (reserve_in + amount_in_after_fee)`
/// where `amount_in_after_fee = amount_in * (10_000 - fee_bps) / 10_000`.
///
/// Returns `ArithmeticOverflow` if reserves are zero or result overflows u64.
///
/// ```rust,ignore
/// let out = constant_product_out(1_000_000, 2_000_000, 100_000, 30)?; // 30 bps fee
/// ```
#[inline(always)]
pub fn constant_product_out(
    reserve_in: u64,
    reserve_out: u64,
    amount_in: u64,
    fee_bps: u16,
) -> Result<u64, ProgramError> {
    if reserve_in == 0 || reserve_out == 0 || amount_in == 0 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    let fee_factor = 10_000u128 - fee_bps as u128;
    let amount_in_after_fee = (amount_in as u128)
        .checked_mul(fee_factor)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let numerator = (reserve_out as u128)
        .checked_mul(amount_in_after_fee)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let denominator = (reserve_in as u128)
        .checked_mul(10_000)
        .ok_or(ProgramError::ArithmeticOverflow)?
        .checked_add(amount_in_after_fee)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let out = numerator / denominator;
    if out > u64::MAX as u128 || out == 0 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    Ok(out as u64)
}

/// Compute required input amount for a constant-product swap to get `amount_out`.
///
/// Inverse of `constant_product_out`. Rounds up (protocol-safe).
///
/// ```rust,ignore
/// let needed = constant_product_in(1_000_000, 2_000_000, 50_000, 30)?;
/// ```
#[inline(always)]
pub fn constant_product_in(
    reserve_in: u64,
    reserve_out: u64,
    amount_out: u64,
    fee_bps: u16,
) -> Result<u64, ProgramError> {
    if reserve_in == 0 || reserve_out == 0 || amount_out == 0 || amount_out >= reserve_out {
        return Err(ProgramError::ArithmeticOverflow);
    }
    let fee_factor = 10_000u128 - fee_bps as u128;
    let numerator = (reserve_in as u128)
        .checked_mul(amount_out as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?
        .checked_mul(10_000)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let denominator = (reserve_out as u128 - amount_out as u128)
        .checked_mul(fee_factor)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    // Ceiling division: (num + denom - 1) / denom
    let result = numerator
        .checked_add(denominator - 1)
        .ok_or(ProgramError::ArithmeticOverflow)?
        / denominator;
    if result > u64::MAX as u128 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    Ok(result as u64)
}

/// Verify the constant-product invariant didn't decrease after a swap.
///
/// Checks `reserve_a_after * reserve_b_after >= reserve_a_before * reserve_b_before`.
/// Returns `InvalidAccountData` if k decreased (swap drained the pool).
///
/// ```rust,ignore
/// check_k_invariant(ra_before, rb_before, ra_after, rb_after)?;
/// ```
#[inline(always)]
pub fn check_k_invariant(
    reserve_a_before: u64,
    reserve_b_before: u64,
    reserve_a_after: u64,
    reserve_b_after: u64,
) -> Result<(), ProgramError> {
    let k_before = (reserve_a_before as u128) * (reserve_b_before as u128);
    let k_after = (reserve_a_after as u128) * (reserve_b_after as u128);
    if k_after < k_before {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Approximate price impact in basis points.
///
/// `impact_bps = amount_in * 10_000 / (reserve_in + amount_in)`.
/// Useful for slippage warnings before executing a swap.
///
/// ```rust,ignore
/// let impact = price_impact_bps(100_000, 1_000_000); // ~909 bps (~9.09%)
/// ```
#[inline(always)]
pub fn price_impact_bps(amount_in: u64, reserve_in: u64) -> u16 {
    if reserve_in == 0 {
        return 10_000; // 100% impact
    }
    let total = reserve_in as u128 + amount_in as u128;
    let impact = (amount_in as u128 * 10_000) / total;
    if impact > 10_000 {
        10_000
    } else {
        impact as u16
    }
}

/// Compute LP tokens to mint for an initial deposit.
///
/// `lp_amount = isqrt(amount_a * amount_b)`. Used for the first liquidity
/// provision when no LP tokens exist yet.
///
/// ```rust,ignore
/// let lp = initial_lp_amount(deposit_a, deposit_b)?;
/// ```
#[inline(always)]
pub fn initial_lp_amount(amount_a: u64, amount_b: u64) -> Result<u64, ProgramError> {
    let product = (amount_a as u128)
        .checked_mul(amount_b as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    isqrt(product)
}

/// Compute LP tokens to mint for a proportional deposit.
///
/// `lp_amount = min(amount_a * lp_supply / reserve_a, amount_b * lp_supply / reserve_b)`.
///
/// ```rust,ignore
/// let lp = proportional_lp_amount(dep_a, dep_b, res_a, res_b, supply)?;
/// ```
#[inline(always)]
pub fn proportional_lp_amount(
    amount_a: u64,
    amount_b: u64,
    reserve_a: u64,
    reserve_b: u64,
    lp_supply: u64,
) -> Result<u64, ProgramError> {
    if reserve_a == 0 || reserve_b == 0 || lp_supply == 0 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    let lp_a = (amount_a as u128)
        .checked_mul(lp_supply as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?
        / reserve_a as u128;
    let lp_b = (amount_b as u128)
        .checked_mul(lp_supply as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?
        / reserve_b as u128;
    let min_lp = if lp_a < lp_b { lp_a } else { lp_b };
    if min_lp > u64::MAX as u128 || min_lp == 0 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    Ok(min_lp as u64)
}
