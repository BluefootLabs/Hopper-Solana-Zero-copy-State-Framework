//! Checked arithmetic operations.
//!
//! All operations return `ProgramError::ArithmeticOverflow` on failure.
//! No panics on-chain. u128 intermediates where needed to prevent
//! premature overflow on real token amounts.

use hopper_runtime::error::ProgramError;

// ── Basic checked ops ────────────────────────────────────────────────────────

/// Checked addition.
#[inline(always)]
pub fn checked_add(a: u64, b: u64) -> Result<u64, ProgramError> {
    a.checked_add(b).ok_or(ProgramError::ArithmeticOverflow)
}

/// Checked subtraction.
#[inline(always)]
pub fn checked_sub(a: u64, b: u64) -> Result<u64, ProgramError> {
    a.checked_sub(b).ok_or(ProgramError::ArithmeticOverflow)
}

/// Checked multiplication.
#[inline(always)]
pub fn checked_mul(a: u64, b: u64) -> Result<u64, ProgramError> {
    a.checked_mul(b).ok_or(ProgramError::ArithmeticOverflow)
}

/// Checked division (returns error on divide by zero).
#[inline(always)]
pub fn checked_div(a: u64, b: u64) -> Result<u64, ProgramError> {
    a.checked_div(b).ok_or(ProgramError::ArithmeticOverflow)
}

/// Checked addition for i64.
#[inline(always)]
pub fn checked_add_i64(a: i64, b: i64) -> Result<i64, ProgramError> {
    a.checked_add(b).ok_or(ProgramError::ArithmeticOverflow)
}

/// Checked subtraction for i64.
#[inline(always)]
pub fn checked_sub_i64(a: i64, b: i64) -> Result<i64, ProgramError> {
    a.checked_sub(b).ok_or(ProgramError::ArithmeticOverflow)
}

// ── Ceiling division ─────────────────────────────────────────────────────────

/// Compute `ceil(a / b)` without overflow (for u64).
#[inline(always)]
pub fn div_ceil(a: u64, b: u64) -> Result<u64, ProgramError> {
    if b == 0 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    Ok(a.div_ceil(b))
}

/// Checked ceiling division: `ceil(a / b)`.
///
/// Rounds up instead of truncating. Use for fee calculations and minimum
/// outputs where truncation would favor the user at the protocol's expense.
#[inline(always)]
pub fn checked_div_ceil(a: u64, b: u64) -> Result<u64, ProgramError> {
    if b == 0 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    Ok(a.checked_add(b - 1)
        .ok_or(ProgramError::ArithmeticOverflow)?
        / b)
}

// ── u128-intermediate ops ────────────────────────────────────────────────────

/// Compute `(a * b) / c` with u128 intermediate to prevent overflow.
///
/// **The core DeFi math primitive.** Without u128 intermediate, `a * b`
/// overflows for any token amounts > ~4.2B (common with 9-decimal mints).
/// Returns floor division.
///
/// ```rust,ignore
/// // Constant-product swap: dy = (y * dx) / (x + dx)
/// let output = checked_mul_div(reserve_y, input, reserve_x + input)?;
/// ```
#[inline(always)]
pub fn checked_mul_div(a: u64, b: u64, c: u64) -> Result<u64, ProgramError> {
    if c == 0 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    let result = (a as u128)
        .checked_mul(b as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?
        / (c as u128);
    to_u64(result)
}

/// Compute `ceil((a * b) / c)` with u128 intermediate.
///
/// Same as [`checked_mul_div`] but rounds up. Use for fee calculations
/// so the protocol never rounds down to zero fee.
#[inline(always)]
pub fn checked_mul_div_ceil(a: u64, b: u64, c: u64) -> Result<u64, ProgramError> {
    if c == 0 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    let numerator = (a as u128)
        .checked_mul(b as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let c128 = c as u128;
    let result = numerator
        .checked_add(c128 - 1)
        .ok_or(ProgramError::ArithmeticOverflow)?
        / c128;
    to_u64(result)
}

/// Safe narrowing cast from u128 to u64.
#[inline(always)]
pub fn to_u64(val: u128) -> Result<u64, ProgramError> {
    if val > u64::MAX as u128 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    Ok(val as u64)
}

// ── Basis-point helpers ──────────────────────────────────────────────────────

/// Scale a value in basis points (BPS).
/// `value * bps / 10_000`, with overflow protection via u128 intermediate.
#[inline(always)]
pub fn scale_bps(value: u64, bps: u64) -> Result<u64, ProgramError> {
    checked_mul_div(value, bps, 10_000)
}

/// Basis-point fee (floor): `amount * bps / 10_000`.
///
/// Nearly every DeFi program computes fees in basis points.
#[inline(always)]
pub fn bps_of(amount: u64, basis_points: u16) -> Result<u64, ProgramError> {
    checked_mul_div(amount, basis_points as u64, 10_000)
}

/// Basis-point fee (ceiling): `ceil(amount * bps / 10_000)`.
///
/// Fees must never round to zero. Use this so the protocol always
/// collects at least 1 token unit of fee when configured.
#[inline(always)]
pub fn bps_of_ceil(amount: u64, basis_points: u16) -> Result<u64, ProgramError> {
    checked_mul_div_ceil(amount, basis_points as u64, 10_000)
}

/// Scale a value by a fraction `(numerator / denominator)`.
#[inline(always)]
pub fn scale_fraction(value: u64, numerator: u64, denominator: u64) -> Result<u64, ProgramError> {
    checked_mul_div(value, numerator, denominator)
}

// ── Decimal scaling ──────────────────────────────────────────────────────────

/// Scale a token amount between different decimal precisions (floor).
///
/// Converts `amount` denominated in `from_decimals` to the equivalent
/// value in `to_decimals`. Uses u128 intermediate to prevent overflow.
///
/// ```rust,ignore
/// let scaled = scale_amount(1_000_000, 6, 9)?; // USDC → SOL precision
/// assert_eq!(scaled, 1_000_000_000);
/// ```
#[inline(always)]
pub fn scale_amount(amount: u64, from_decimals: u8, to_decimals: u8) -> Result<u64, ProgramError> {
    if from_decimals == to_decimals {
        return Ok(amount);
    }
    if to_decimals > from_decimals {
        let factor = 10u128
            .checked_pow((to_decimals - from_decimals) as u32)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        let result = (amount as u128)
            .checked_mul(factor)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        to_u64(result)
    } else {
        let factor = 10u64
            .checked_pow((from_decimals - to_decimals) as u32)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        checked_div(amount, factor)
    }
}

/// Scale a token amount between decimal precisions, rounding up.
///
/// Same as [`scale_amount`] but uses ceiling division when scaling down.
/// Use for protocol-side calculations where truncating would short-change
/// the protocol (e.g., minimum collateral requirements).
#[inline(always)]
pub fn scale_amount_ceil(
    amount: u64,
    from_decimals: u8,
    to_decimals: u8,
) -> Result<u64, ProgramError> {
    if from_decimals == to_decimals {
        return Ok(amount);
    }
    if to_decimals > from_decimals {
        let factor = 10u128
            .checked_pow((to_decimals - from_decimals) as u32)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        let result = (amount as u128)
            .checked_mul(factor)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        to_u64(result)
    } else {
        let factor = 10u64
            .checked_pow((from_decimals - to_decimals) as u32)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        checked_div_ceil(amount, factor)
    }
}

// ── Exponentiation ───────────────────────────────────────────────────────────

/// Checked exponentiation via repeated squaring.
///
/// Computes `base^exp` with overflow checking at each step. Useful for
/// compound interest and exponential decay.
#[inline(always)]
pub fn checked_pow(base: u64, exp: u32) -> Result<u64, ProgramError> {
    if exp == 0 {
        return Ok(1);
    }
    let mut result: u64 = 1;
    let mut b = base;
    let mut e = exp;
    while e > 0 {
        if e & 1 == 1 {
            result = result
                .checked_mul(b)
                .ok_or(ProgramError::ArithmeticOverflow)?;
        }
        e >>= 1;
        if e > 0 {
            b = b.checked_mul(b).ok_or(ProgramError::ArithmeticOverflow)?;
        }
    }
    Ok(result)
}

// ── Fixed-point and protocol unit wrappers ─────────────────────────────────

/// Signed fixed-point value with 80 integer bits and 48 fractional bits.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct I80F48(i128);

impl I80F48 {
    pub const FRAC_BITS: u32 = 48;
    pub const ONE_BITS: i128 = 1i128 << Self::FRAC_BITS;
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(Self::ONE_BITS);

    /// Construct from raw fixed-point bits.
    #[inline(always)]
    pub const fn from_bits(bits: i128) -> Self {
        Self(bits)
    }

    /// Return raw fixed-point bits.
    #[inline(always)]
    pub const fn bits(self) -> i128 {
        self.0
    }

    /// Convert an integer into I80F48.
    #[inline(always)]
    pub fn from_integer(value: i64) -> Result<Self, ProgramError> {
        (value as i128)
            .checked_mul(Self::ONE_BITS)
            .map(Self)
            .ok_or(ProgramError::ArithmeticOverflow)
    }

    /// Construct `numerator / denominator` as I80F48, truncating toward zero.
    #[inline(always)]
    pub fn from_ratio(numerator: i128, denominator: i128) -> Result<Self, ProgramError> {
        if denominator == 0 {
            return Err(ProgramError::ArithmeticOverflow);
        }
        numerator
            .checked_mul(Self::ONE_BITS)
            .ok_or(ProgramError::ArithmeticOverflow)?
            .checked_div(denominator)
            .map(Self)
            .ok_or(ProgramError::ArithmeticOverflow)
    }

    #[inline(always)]
    pub fn checked_add(self, rhs: Self) -> Result<Self, ProgramError> {
        self.0
            .checked_add(rhs.0)
            .map(Self)
            .ok_or(ProgramError::ArithmeticOverflow)
    }

    #[inline(always)]
    pub fn checked_sub(self, rhs: Self) -> Result<Self, ProgramError> {
        self.0
            .checked_sub(rhs.0)
            .map(Self)
            .ok_or(ProgramError::ArithmeticOverflow)
    }

    /// Checked multiplication, truncating toward zero.
    #[inline(always)]
    pub fn checked_mul(self, rhs: Self) -> Result<Self, ProgramError> {
        self.0
            .checked_mul(rhs.0)
            .ok_or(ProgramError::ArithmeticOverflow)?
            .checked_div(Self::ONE_BITS)
            .map(Self)
            .ok_or(ProgramError::ArithmeticOverflow)
    }

    /// Checked division, truncating toward zero.
    #[inline(always)]
    pub fn checked_div(self, rhs: Self) -> Result<Self, ProgramError> {
        if rhs.0 == 0 {
            return Err(ProgramError::ArithmeticOverflow);
        }
        self.0
            .checked_mul(Self::ONE_BITS)
            .ok_or(ProgramError::ArithmeticOverflow)?
            .checked_div(rhs.0)
            .map(Self)
            .ok_or(ProgramError::ArithmeticOverflow)
    }

    /// Truncate to an integer, toward zero.
    #[inline(always)]
    pub const fn to_integer_trunc(self) -> i128 {
        self.0 / Self::ONE_BITS
    }
}

/// Basis points. `10_000` represents 100%.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct BasisPoints(u32);

impl BasisPoints {
    pub const ONE_HUNDRED_PERCENT: u32 = 10_000;

    #[inline(always)]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[inline(always)]
    pub const fn get(self) -> u32 {
        self.0
    }

    #[inline(always)]
    pub fn of_floor(self, amount: u64) -> Result<u64, ProgramError> {
        checked_mul_div(amount, self.0 as u64, Self::ONE_HUNDRED_PERCENT as u64)
    }

    #[inline(always)]
    pub fn of_ceil(self, amount: u64) -> Result<u64, ProgramError> {
        checked_mul_div_ceil(amount, self.0 as u64, Self::ONE_HUNDRED_PERCENT as u64)
    }
}

#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Price(pub I80F48);

#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct FundingRate(pub I80F48);

#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct MarginRatio(pub BasisPoints);

#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct QuoteAmount(pub i128);

#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct BaseAmount(pub i128);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i80f48_integer_and_ratio_round_trip() {
        let two = I80F48::from_integer(2).unwrap();
        let half = I80F48::from_ratio(1, 2).unwrap();
        let one = two.checked_mul(half).unwrap();
        assert_eq!(one, I80F48::ONE);
        assert_eq!(one.to_integer_trunc(), 1);
    }

    #[test]
    fn i80f48_checked_division_rejects_zero() {
        assert_eq!(
            I80F48::ONE.checked_div(I80F48::ZERO).unwrap_err(),
            ProgramError::ArithmeticOverflow
        );
    }

    #[test]
    fn basis_points_apply_floor_and_ceil() {
        let bps = BasisPoints::new(25);
        assert_eq!(bps.of_floor(10_000).unwrap(), 25);
        assert_eq!(bps.of_ceil(1).unwrap(), 1);
    }
}
