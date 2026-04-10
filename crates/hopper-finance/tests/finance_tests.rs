//! Tests for hopper-finance: AMM math + slippage guards.

use hopper_finance::amm::*;
use hopper_finance::slippage::*;

// =====================================================================
// isqrt tests
// =====================================================================

#[test]
fn test_isqrt_zero() {
    assert_eq!(isqrt(0).unwrap(), 0);
}

#[test]
fn test_isqrt_perfect_squares() {
    assert_eq!(isqrt(1).unwrap(), 1);
    assert_eq!(isqrt(4).unwrap(), 2);
    assert_eq!(isqrt(9).unwrap(), 3);
    assert_eq!(isqrt(100).unwrap(), 10);
    assert_eq!(isqrt(10_000).unwrap(), 100);
    assert_eq!(isqrt(1_000_000).unwrap(), 1_000);
}

#[test]
fn test_isqrt_non_perfect() {
    // floor(sqrt(2)) = 1
    assert_eq!(isqrt(2).unwrap(), 1);
    // floor(sqrt(8)) = 2
    assert_eq!(isqrt(8).unwrap(), 2);
    // floor(sqrt(1_000_001)) = 1000
    assert_eq!(isqrt(1_000_001).unwrap(), 1_000);
}

#[test]
fn test_isqrt_large_values() {
    // u64::MAX * u64::MAX fits in u128
    let val = u64::MAX as u128;
    assert_eq!(isqrt(val * val).unwrap(), u64::MAX);
}

// =====================================================================
// constant_product_out tests
// =====================================================================

#[test]
fn test_cp_out_basic_no_fee() {
    // x=1M, y=2M, input=100k, 0 bps fee
    // out = (2M * 100k) / (1M + 100k) = 200B / 1.1M ≈ 181818
    let out = constant_product_out(1_000_000, 2_000_000, 100_000, 0).unwrap();
    assert_eq!(out, 181818);
}

#[test]
fn test_cp_out_with_fee() {
    // 30 bps (0.3%) fee
    let out_no_fee = constant_product_out(1_000_000, 2_000_000, 100_000, 0).unwrap();
    let out_with_fee = constant_product_out(1_000_000, 2_000_000, 100_000, 30).unwrap();
    assert!(out_with_fee < out_no_fee);
}

#[test]
fn test_cp_out_zero_input_errors() {
    assert!(constant_product_out(1_000_000, 2_000_000, 0, 30).is_err());
}

#[test]
fn test_cp_out_zero_reserves_errors() {
    assert!(constant_product_out(0, 2_000_000, 100_000, 30).is_err());
    assert!(constant_product_out(1_000_000, 0, 100_000, 30).is_err());
}

// =====================================================================
// constant_product_in tests
// =====================================================================

#[test]
fn test_cp_in_basic() {
    // Inverse of out: if we want 181818 out, how much in?
    let needed = constant_product_in(1_000_000, 2_000_000, 181818, 0).unwrap();
    // Should be close to 100_000 (may differ by rounding)
    assert!(needed >= 99_999 && needed <= 100_001, "needed={needed}");
}

#[test]
fn test_cp_in_amount_out_too_large() {
    // amount_out >= reserve_out must fail
    assert!(constant_product_in(1_000_000, 2_000_000, 2_000_000, 0).is_err());
    assert!(constant_product_in(1_000_000, 2_000_000, 3_000_000, 0).is_err());
}

// =====================================================================
// check_k_invariant tests
// =====================================================================

#[test]
fn test_k_invariant_ok() {
    // K stays the same
    check_k_invariant(1_000, 2_000, 1_100, 1_819).unwrap();
    // K increases
    check_k_invariant(1_000, 2_000, 1_100, 1_900).unwrap();
}

#[test]
fn test_k_invariant_decreased() {
    // K decreases: 1000*2000 = 2M > 1100*1800 = 1.98M
    assert!(check_k_invariant(1_000, 2_000, 1_100, 1_800).is_err());
}

// =====================================================================
// price_impact_bps tests
// =====================================================================

#[test]
fn test_price_impact_small_trade() {
    // 1k into 1M pool → ~10 bps
    let impact = price_impact_bps(1_000, 1_000_000);
    assert!(impact <= 10, "impact={impact}");
}

#[test]
fn test_price_impact_large_trade() {
    // 100k into 1M pool → ~909 bps (9.09%)
    let impact = price_impact_bps(100_000, 1_000_000);
    assert_eq!(impact, 909);
}

#[test]
fn test_price_impact_empty_pool() {
    assert_eq!(price_impact_bps(100, 0), 10_000);
}

// =====================================================================
// LP minting tests
// =====================================================================

#[test]
fn test_initial_lp_amount() {
    // sqrt(1M * 1M) = 1M
    assert_eq!(initial_lp_amount(1_000_000, 1_000_000).unwrap(), 1_000_000);
    // sqrt(4M * 9M) = sqrt(36T) = 6M
    assert_eq!(initial_lp_amount(4_000_000, 9_000_000).unwrap(), 6_000_000);
}

#[test]
fn test_proportional_lp_amount() {
    // Pool has 1M/1M, LP supply 1M. Deposit 100k/100k → 100k LP
    let lp = proportional_lp_amount(100_000, 100_000, 1_000_000, 1_000_000, 1_000_000).unwrap();
    assert_eq!(lp, 100_000);
}

#[test]
fn test_proportional_lp_imbalanced() {
    // Deposit 200k/100k with equal reserves → min(200k,100k) = 100k LP
    let lp = proportional_lp_amount(200_000, 100_000, 1_000_000, 1_000_000, 1_000_000).unwrap();
    assert_eq!(lp, 100_000);
}

// =====================================================================
// Slippage guard tests
// =====================================================================

#[test]
fn test_check_slippage_ok() {
    check_slippage(100, 90).unwrap();
    check_slippage(100, 100).unwrap();
}

#[test]
fn test_check_slippage_fail() {
    assert!(check_slippage(89, 90).is_err());
}

#[test]
fn test_check_max_input_ok() {
    check_max_input(90, 100).unwrap();
    check_max_input(100, 100).unwrap();
}

#[test]
fn test_check_max_input_fail() {
    assert!(check_max_input(101, 100).is_err());
}

#[test]
fn test_check_nonzero_ok() {
    check_nonzero(1).unwrap();
    check_nonzero(u64::MAX).unwrap();
}

#[test]
fn test_check_nonzero_fail() {
    assert!(check_nonzero(0).is_err());
}

#[test]
fn test_check_min_amount() {
    check_min_amount(100, 100).unwrap();
    check_min_amount(101, 100).unwrap();
    assert!(check_min_amount(99, 100).is_err());
}

#[test]
fn test_check_max_amount() {
    check_max_amount(100, 100).unwrap();
    check_max_amount(99, 100).unwrap();
    assert!(check_max_amount(101, 100).is_err());
}

#[test]
fn test_check_within_bps_ok() {
    // price 1000 vs expected 1000 → 0 bps deviation
    check_within_bps(1000, 1000, 0).unwrap();
    // price 1010 vs expected 1000 → 100 bps; tolerance 100 → ok
    check_within_bps(1010, 1000, 100).unwrap();
    // same in other direction
    check_within_bps(990, 1000, 100).unwrap();
}

#[test]
fn test_check_within_bps_fail() {
    // price 1020 vs expected 1000 → 200 bps; tolerance 100 → fail
    assert!(check_within_bps(1020, 1000, 100).is_err());
}

#[test]
fn test_check_within_bps_zero_expected() {
    check_within_bps(0, 0, 0).unwrap();
    assert!(check_within_bps(1, 0, 100).is_err());
}

#[test]
fn test_check_price_bounds_ok() {
    check_price_bounds(100, 50, 200).unwrap();
    check_price_bounds(50, 50, 200).unwrap();
    check_price_bounds(200, 50, 200).unwrap();
}

#[test]
fn test_check_price_bounds_fail() {
    assert!(check_price_bounds(49, 50, 200).is_err());
    assert!(check_price_bounds(201, 50, 200).is_err());
}
