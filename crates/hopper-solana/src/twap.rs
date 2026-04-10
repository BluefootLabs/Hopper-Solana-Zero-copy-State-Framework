//! TWAP (time-weighted average price) accumulator math.
//!
//! Maintains a cumulative price sum that increases by `price * elapsed`
//! each update. The TWAP between any two observations is just the
//! difference in cumulative values divided by elapsed time.
//!
//! Uses u128 throughout to avoid overflow when accumulating over long
//! periods.

use hopper_runtime::error::ProgramError;

/// Advance the cumulative price accumulator.
///
/// `cumulative` - current running sum (scaled by time).
/// `price`      - current spot price as a u64.
/// `last_ts`    - unix timestamp of the previous update.
/// `now_ts`     - current unix timestamp.
///
/// Returns the new cumulative value. If `now_ts <= last_ts` the value
/// is returned unchanged (no time elapsed).
#[inline(always)]
pub fn update_twap_cumulative(
    cumulative: u128,
    price: u64,
    last_ts: i64,
    now_ts: i64,
) -> Result<u128, ProgramError> {
    if now_ts <= last_ts {
        return Ok(cumulative);
    }
    let elapsed = (now_ts - last_ts) as u128;
    let increment = (price as u128)
        .checked_mul(elapsed)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    cumulative
        .checked_add(increment)
        .ok_or(ProgramError::ArithmeticOverflow)
}

/// Compute a TWAP from two cumulative observations.
///
/// `twap = (end_cumulative - start_cumulative) / (end_ts - start_ts)`
///
/// Returns the time-weighted average as a u64.
#[inline(always)]
pub fn compute_twap(
    cumulative_start: u128,
    cumulative_end: u128,
    ts_start: i64,
    ts_end: i64,
) -> Result<u64, ProgramError> {
    if ts_end <= ts_start {
        return Err(ProgramError::InvalidArgument);
    }
    let elapsed = (ts_end - ts_start) as u128;
    let diff = cumulative_end
        .checked_sub(cumulative_start)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let twap = diff / elapsed;
    if twap > u64::MAX as u128 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    Ok(twap as u64)
}

/// Fail if `spot_price` deviates from `twap_price` by more than
/// `max_deviation_bps` basis points.
///
/// Anti-manipulation guard: a large spot/TWAP spread suggests the
/// current price is being moved artificially.
#[inline(always)]
pub fn check_twap_deviation(
    spot_price: u64,
    twap_price: u64,
    max_deviation_bps: u64,
) -> Result<(), ProgramError> {
    if twap_price == 0 {
        return Err(ProgramError::InvalidArgument);
    }
    let diff = spot_price.abs_diff(twap_price);
    let deviation_bps = (diff as u128) * 10_000 / (twap_price as u128);
    if deviation_bps > max_deviation_bps as u128 {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cumulative_accumulates() {
        let c = update_twap_cumulative(0, 100, 0, 10).unwrap();
        assert_eq!(c, 1_000); // 100 * 10

        let c = update_twap_cumulative(c, 200, 10, 20).unwrap();
        assert_eq!(c, 3_000); // 1000 + 200*10
    }

    #[test]
    fn no_time_elapsed_unchanged() {
        let c = update_twap_cumulative(500, 100, 10, 10).unwrap();
        assert_eq!(c, 500);

        let c = update_twap_cumulative(500, 100, 10, 5).unwrap();
        assert_eq!(c, 500);
    }

    #[test]
    fn compute_twap_basic() {
        // Price constant at 100 for 10 seconds
        let twap = compute_twap(0, 1_000, 0, 10).unwrap();
        assert_eq!(twap, 100);
    }

    #[test]
    fn compute_twap_mixed() {
        // 100 for 10s then 200 for 10s => cumulative = 1000 + 2000 = 3000
        // TWAP over 20s = 3000/20 = 150
        let twap = compute_twap(0, 3_000, 0, 20).unwrap();
        assert_eq!(twap, 150);
    }

    #[test]
    fn compute_twap_rejects_zero_elapsed() {
        assert!(compute_twap(0, 100, 10, 10).is_err());
        assert!(compute_twap(0, 100, 10, 5).is_err());
    }

    #[test]
    fn deviation_check_passes_within_bounds() {
        // spot=105, twap=100 => 5% = 500 bps
        assert!(check_twap_deviation(105, 100, 500).is_ok());
        assert!(check_twap_deviation(95, 100, 500).is_ok());
    }

    #[test]
    fn deviation_check_rejects_excess() {
        // spot=106, twap=100 => 6% = 600 bps > 500
        assert!(check_twap_deviation(106, 100, 500).is_err());
    }

    #[test]
    fn deviation_check_rejects_zero_twap() {
        assert!(check_twap_deviation(100, 0, 500).is_err());
    }

    #[test]
    fn full_twap_cycle() {
        let mut cumulative = 0u128;
        let mut last_ts = 1_000i64;

        // 5 updates at different prices
        let prices = [100u64, 120, 80, 150, 110];
        for (i, &p) in prices.iter().enumerate() {
            let now = last_ts + 10;
            cumulative = update_twap_cumulative(cumulative, p, last_ts, now).unwrap();
            last_ts = now;
            let _ = i;
        }

        // TWAP over entire period (50 seconds)
        let twap = compute_twap(0, cumulative, 1_000, 1_050).unwrap();
        // sum = (100+120+80+150+110) * 10 = 5600, /50 = 112
        assert_eq!(twap, 112);

        // Deviation check
        assert!(check_twap_deviation(115, twap, 300).is_ok()); // ~2.7%
        assert!(check_twap_deviation(150, twap, 300).is_err()); // ~33.9%
    }
}
