//! Time-based validation: deadlines, cooldowns, staleness.

use hopper_runtime::error::ProgramError;

/// Check that a deadline has passed (now >= deadline).
#[inline(always)]
pub fn check_deadline_passed(deadline: i64, now: i64) -> Result<(), ProgramError> {
    if now < deadline {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Check that a deadline has NOT passed (now < deadline).
#[inline(always)]
pub fn check_deadline_not_passed(deadline: i64, now: i64) -> Result<(), ProgramError> {
    if now >= deadline {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Check that enough time has elapsed since the last operation.
#[inline(always)]
pub fn check_cooldown_elapsed(
    last_op_time: i64,
    now: i64,
    cooldown_seconds: i64,
) -> Result<(), ProgramError> {
    let elapsed = now.saturating_sub(last_op_time);
    if elapsed < cooldown_seconds {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Check that data is not stale (updated recently enough).
#[inline(always)]
pub fn check_staleness(
    updated_at: i64,
    now: i64,
    max_age_seconds: i64,
) -> Result<(), ProgramError> {
    let age = now.saturating_sub(updated_at);
    if age > max_age_seconds {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Check that a timestamp is in the future.
#[inline(always)]
pub fn check_in_future(timestamp: i64, now: i64) -> Result<(), ProgramError> {
    if timestamp <= now {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Check that a timestamp is in the past.
#[inline(always)]
pub fn check_in_past(timestamp: i64, now: i64) -> Result<(), ProgramError> {
    if timestamp > now {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}
