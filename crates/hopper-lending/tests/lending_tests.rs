//! Tests for hopper-lending: collateralization, liquidation, interest, utilization.

use hopper_lending::*;

#[test]
fn test_collateralization_ratio_basic() {
    // 150k collateral / 100k debt = 15_000 bps = 150%
    assert_eq!(collateralization_ratio_bps(150_000, 100_000).unwrap(), 15_000);
    // 100k / 100k = 10_000 bps = 100%
    assert_eq!(collateralization_ratio_bps(100_000, 100_000).unwrap(), 10_000);
}

#[test]
fn test_collateralization_zero_debt() {
    assert_eq!(collateralization_ratio_bps(100_000, 0).unwrap(), u64::MAX);
}

#[test]
fn test_check_healthy() {
    // 150% collateral, 125% threshold → healthy
    check_healthy(150_000, 100_000, 12_500).unwrap();
    // 120% collateral, 125% threshold → unhealthy
    assert!(check_healthy(120_000, 100_000, 12_500).is_err());
}

#[test]
fn test_check_liquidatable() {
    // 120% collateral, 125% threshold → liquidatable
    check_liquidatable(120_000, 100_000, 12_500).unwrap();
    // 150% → not liquidatable
    assert!(check_liquidatable(150_000, 100_000, 12_500).is_err());
}

#[test]
fn test_max_liquidation_amount() {
    // 50% close factor on 100k debt → 50k max
    assert_eq!(max_liquidation_amount(100_000, 5_000).unwrap(), 50_000);
    // 100% close factor → full debt
    assert_eq!(max_liquidation_amount(100_000, 10_000).unwrap(), 100_000);
}

#[test]
fn test_liquidation_seize_amount() {
    // 5% bonus on 50k repay → 52_500
    assert_eq!(liquidation_seize_amount(50_000, 500).unwrap(), 52_500);
    // 0% bonus → same as repay
    assert_eq!(liquidation_seize_amount(50_000, 0).unwrap(), 50_000);
}

#[test]
fn test_simple_interest() {
    // 1M principal, 500 bps (5%), 1 period
    assert_eq!(simple_interest(1_000_000, 500, 1).unwrap(), 50_000);
    // 1M, 500 bps, 365 periods
    assert_eq!(simple_interest(1_000_000, 500, 365).unwrap(), 18_250_000);
}

#[test]
fn test_utilization_rate() {
    // 80k borrows, 20k cash → 8_000 bps = 80%
    assert_eq!(utilization_rate_bps(80_000, 20_000).unwrap(), 8_000);
    // 0 borrows → 0%
    assert_eq!(utilization_rate_bps(0, 100_000).unwrap(), 0);
    // all borrowed → 10_000 bps = 100%
    assert_eq!(utilization_rate_bps(100_000, 0).unwrap(), 10_000);
    // both 0 → 0
    assert_eq!(utilization_rate_bps(0, 0).unwrap(), 0);
}
