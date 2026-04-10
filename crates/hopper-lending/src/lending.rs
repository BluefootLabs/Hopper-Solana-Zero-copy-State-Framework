//! Lending protocol math primitives.
//!
//! Collateralization ratios, health checks, liquidation amounts,
//! utilization rates, and simple interest. All u128 intermediates,
//! all basis-point denominated.

use hopper_runtime::error::ProgramError;

/// Collateralization ratio in basis points.
///
/// `ratio = collateral_value * 10_000 / debt_value`
///
/// Returns `u64::MAX` when `debt_value` is 0 (infinite collateral).
///
/// ```rust,ignore
/// let ratio = collateralization_ratio_bps(150_000, 100_000)?;
/// // 15_000 bps = 150%
/// ```
#[inline(always)]
pub fn collateralization_ratio_bps(
    collateral_value: u64,
    debt_value: u64,
) -> Result<u64, ProgramError> {
    if debt_value == 0 {
        return Ok(u64::MAX);
    }
    let ratio = (collateral_value as u128)
        .checked_mul(10_000)
        .ok_or(ProgramError::ArithmeticOverflow)?
        / (debt_value as u128);
    Ok(ratio as u64)
}

/// Fail if the position is under-collateralized (ratio < threshold).
///
/// `liquidation_threshold_bps`: e.g. 12_500 for a 125% minimum.
///
/// ```rust,ignore
/// check_healthy(collateral_val, debt_val, 12_500)?;
/// ```
#[inline(always)]
pub fn check_healthy(
    collateral_value: u64,
    debt_value: u64,
    liquidation_threshold_bps: u64,
) -> Result<(), ProgramError> {
    let ratio = collateralization_ratio_bps(collateral_value, debt_value)?;
    if ratio < liquidation_threshold_bps {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

/// Fail if the position is NOT eligible for liquidation (ratio >= threshold).
///
/// Mirror of [`check_healthy`] for the top of liquidation handlers.
#[inline(always)]
pub fn check_liquidatable(
    collateral_value: u64,
    debt_value: u64,
    liquidation_threshold_bps: u64,
) -> Result<(), ProgramError> {
    let ratio = collateralization_ratio_bps(collateral_value, debt_value)?;
    if ratio >= liquidation_threshold_bps {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

/// Maximum debt repayable in a single liquidation call.
///
/// `close_factor_bps`: e.g. 5_000 = 50% of the debt per call.
///
/// ```rust,ignore
/// let max_repay = max_liquidation_amount(debt, 5_000)?;
/// ```
#[inline(always)]
pub fn max_liquidation_amount(
    debt_value: u64,
    close_factor_bps: u64,
) -> Result<u64, ProgramError> {
    let max = (debt_value as u128)
        .checked_mul(close_factor_bps as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?
        / 10_000;
    Ok(max.min(debt_value as u128) as u64)
}

/// Collateral seized by the liquidator.
///
/// `seized = repay_amount * (10_000 + bonus_bps) / 10_000`
///
/// `bonus_bps`: liquidation incentive, e.g. 500 = 5% bonus.
///
/// ```rust,ignore
/// let seized = liquidation_seize_amount(repay, 500)?;
/// ```
#[inline(always)]
pub fn liquidation_seize_amount(
    repay_amount: u64,
    bonus_bps: u64,
) -> Result<u64, ProgramError> {
    let seized = (repay_amount as u128)
        .checked_mul((10_000u64 + bonus_bps) as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?
        / 10_000;
    if seized > u64::MAX as u128 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    Ok(seized as u64)
}

/// Simple interest: `principal * rate_bps * periods / 10_000`.
///
/// Returns the interest amount only (not principal + interest).
///
/// ```rust,ignore
/// let interest = simple_interest(1_000_000, 500, 365)?;
/// ```
#[inline(always)]
pub fn simple_interest(
    principal: u64,
    rate_bps_per_period: u64,
    periods: u64,
) -> Result<u64, ProgramError> {
    let interest = (principal as u128)
        .checked_mul(rate_bps_per_period as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?
        .checked_mul(periods as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?
        / 10_000;
    if interest > u64::MAX as u128 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    Ok(interest as u64)
}

/// Utilization rate in basis points: `borrows * 10_000 / (cash + borrows)`.
///
/// Returns 0 if both `cash` and `borrows` are 0.
///
/// ```rust,ignore
/// let util = utilization_rate_bps(80_000, 20_000)?;
/// // 8_000 bps = 80%
/// ```
#[inline(always)]
pub fn utilization_rate_bps(borrows: u64, cash: u64) -> Result<u64, ProgramError> {
    let total = (borrows as u128) + (cash as u128);
    if total == 0 {
        return Ok(0);
    }
    let util = (borrows as u128) * 10_000 / total;
    Ok(util as u64)
}
