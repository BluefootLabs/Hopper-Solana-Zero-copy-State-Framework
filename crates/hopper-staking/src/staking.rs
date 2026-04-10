//! Staking rewards math (reward-per-token accumulator).
//!
//! Distributes rewards proportionally to stakers without iterating over
//! all users. A global accumulator tracks rewards per staked unit; each
//! user stores a debt checkpoint.
//!
//! The pattern:
//! 1. Global accumulator: `reward_per_token += rewards_earned / total_staked`
//! 2. User debt: `user.reward_debt = user.staked * global.reward_per_token`
//! 3. Pending: `claimable = user.staked * global.reward_per_token - user.reward_debt`
//!
//! All values use a `PRECISION` scaling factor (1e12) to avoid precision loss
//! when dividing small reward amounts by large total stakes.

use hopper_runtime::error::ProgramError;

/// Scaling factor for reward-per-token accumulator (1e12).
///
/// Provides 12 decimal places of precision. Enough for any practical
/// staking scenario without overflowing u128 for reasonable token amounts.
pub const REWARD_PRECISION: u128 = 1_000_000_000_000;

/// Update the global reward-per-token accumulator.
///
/// Call this every time rewards are distributed or a user stakes/unstakes.
///
/// `reward_per_token` is the current accumulator value (scaled by `REWARD_PRECISION`).
/// `rewards_since_last` is the new rewards to distribute since the last update.
/// `total_staked` is the total amount currently staked across all users.
///
/// Returns the updated accumulator. If `total_staked == 0`, returns the
/// current value unchanged (rewards are not distributed to nobody).
///
/// ```rust,ignore
/// let new_rpt = update_reward_per_token(pool.reward_per_token, new_rewards, pool.total_staked)?;
/// ```
#[inline(always)]
pub fn update_reward_per_token(
    reward_per_token: u128,
    rewards_since_last: u64,
    total_staked: u64,
) -> Result<u128, ProgramError> {
    if total_staked == 0 {
        return Ok(reward_per_token);
    }
    let increment = (rewards_since_last as u128)
        .checked_mul(REWARD_PRECISION)
        .ok_or(ProgramError::ArithmeticOverflow)?
        / total_staked as u128;
    reward_per_token
        .checked_add(increment)
        .ok_or(ProgramError::ArithmeticOverflow)
}

/// Calculate a user's pending (claimable) rewards.
///
/// `user_staked` is the user's staked amount.
/// `reward_per_token` is the current global accumulator.
/// `user_reward_debt` is the user's stored reward debt.
///
/// Returns the claimable reward amount as u64.
///
/// ```rust,ignore
/// let claimable = pending_rewards(user.staked, pool.reward_per_token, user.reward_debt)?;
/// ```
#[inline(always)]
pub fn pending_rewards(
    user_staked: u64,
    reward_per_token: u128,
    user_reward_debt: u128,
) -> Result<u64, ProgramError> {
    let accumulated = (user_staked as u128)
        .checked_mul(reward_per_token)
        .ok_or(ProgramError::ArithmeticOverflow)?
        / REWARD_PRECISION;
    let debt_normalized = user_reward_debt / REWARD_PRECISION;
    let pending = accumulated.saturating_sub(debt_normalized);
    if pending > u64::MAX as u128 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    Ok(pending as u64)
}

/// Compute the reward debt for a user after staking or claiming.
///
/// Store this value in the user's account after every stake/unstake/claim.
///
/// ```rust,ignore
/// user.reward_debt = update_reward_debt(user.staked, pool.reward_per_token);
/// ```
#[inline(always)]
pub fn update_reward_debt(user_staked: u64, reward_per_token: u128) -> u128 {
    (user_staked as u128) * reward_per_token
}

/// Calculate the emission rate (rewards per second).
///
/// ```rust,ignore
/// let rate = emission_rate(total_rewards, duration_seconds)?;
/// ```
#[inline(always)]
pub fn emission_rate(total_rewards: u64, duration_seconds: u64) -> Result<u64, ProgramError> {
    if duration_seconds == 0 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    Ok(total_rewards / duration_seconds)
}

/// Calculate rewards earned since the last update, given an emission rate
/// and elapsed time.
///
/// ```rust,ignore
/// let earned = rewards_earned(rate, elapsed_seconds)?;
/// ```
#[inline(always)]
pub fn rewards_earned(rate_per_second: u64, elapsed_seconds: u64) -> Result<u64, ProgramError> {
    (rate_per_second as u128)
        .checked_mul(elapsed_seconds as u128)
        .ok_or(ProgramError::ArithmeticOverflow)
        .and_then(|v| {
            if v > u64::MAX as u128 {
                Err(ProgramError::ArithmeticOverflow)
            } else {
                Ok(v as u64)
            }
        })
}
