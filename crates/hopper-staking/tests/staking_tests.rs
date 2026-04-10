//! Tests for hopper-staking: reward-per-token accumulator, pending rewards, emission.

use hopper_staking::*;

#[test]
fn test_update_reward_per_token_basic() {
    // 1000 rewards, 100 staked → increment = 1000 * 1e12 / 100 = 1e13
    let rpt = update_reward_per_token(0, 1000, 100).unwrap();
    assert_eq!(rpt, 10_000_000_000_000);
}

#[test]
fn test_update_reward_per_token_zero_staked() {
    // No stakers → unchanged
    assert_eq!(update_reward_per_token(42, 1000, 0).unwrap(), 42);
}

#[test]
fn test_pending_rewards_basic() {
    // User staked 100, rpt = 1e13, no prior debt
    let rpt = 10_000_000_000_000u128;
    let pending = pending_rewards(100, rpt, 0).unwrap();
    // accumulated = 100 * 1e13 / 1e12 = 1000
    assert_eq!(pending, 1000);
}

#[test]
fn test_pending_rewards_with_debt() {
    let rpt = 20_000_000_000_000u128;
    let debt = update_reward_debt(100, 10_000_000_000_000);
    // accumulated = 100 * 2e13 / 1e12 = 2000, debt_norm = 100*1e13 / 1e12 = 1000
    let pending = pending_rewards(100, rpt, debt).unwrap();
    assert_eq!(pending, 1000);
}

#[test]
fn test_update_reward_debt() {
    let rpt = 5_000_000_000_000u128;
    let debt = update_reward_debt(200, rpt);
    assert_eq!(debt, 200 * rpt);
}

#[test]
fn test_emission_rate() {
    // 1M rewards over 100 seconds
    assert_eq!(emission_rate(1_000_000, 100).unwrap(), 10_000);
    // Zero duration → error
    assert!(emission_rate(1000, 0).is_err());
}

#[test]
fn test_rewards_earned() {
    // 10k per sec * 60 sec = 600k
    assert_eq!(rewards_earned(10_000, 60).unwrap(), 600_000);
}

#[test]
fn test_full_staking_cycle() {
    // Simulate: 2 users, rewards distributed over time
    let mut rpt = 0u128;

    // User A stakes 100
    let user_a_staked = 100u64;
    let user_a_debt = update_reward_debt(user_a_staked, rpt);

    // 1000 rewards arrive, total staked = 100
    rpt = update_reward_per_token(rpt, 1000, 100).unwrap();

    // User A's pending = 1000
    let a_pending = pending_rewards(user_a_staked, rpt, user_a_debt).unwrap();
    assert_eq!(a_pending, 1000);

    // User B stakes 100, total = 200
    let user_b_staked = 100u64;
    let user_b_debt = update_reward_debt(user_b_staked, rpt);

    // 1000 more rewards arrive, total staked = 200
    rpt = update_reward_per_token(rpt, 1000, 200).unwrap();

    // User A: gets 1000 (first) + 500 (second) = 1500
    let a_pending_2 = pending_rewards(user_a_staked, rpt, user_a_debt).unwrap();
    assert_eq!(a_pending_2, 1500);

    // User B: gets 500 (second only)
    let b_pending = pending_rewards(user_b_staked, rpt, user_b_debt).unwrap();
    assert_eq!(b_pending, 500);
}
