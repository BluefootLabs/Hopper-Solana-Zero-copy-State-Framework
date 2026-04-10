//! State machine validation.
//!
//! Enforces valid state transitions for account lifecycle enums.
//! Prevents invalid state jumps (e.g., Pending -> Closed skipping Active).

use hopper_runtime::error::ProgramError;

/// Check that a state transition is valid.
///
/// `valid_transitions` is a list of `(from, to)` pairs.
/// Returns error if `(current, next)` is not in the list.
#[inline]
pub fn check_state_transition(
    current: u8,
    next: u8,
    valid_transitions: &[(u8, u8)],
) -> Result<(), ProgramError> {
    for &(from, to) in valid_transitions {
        if from == current && to == next {
            return Ok(());
        }
    }
    Err(ProgramError::InvalidAccountData)
}

/// Check that the state byte at `offset` in `data` matches `expected`.
#[inline(always)]
pub fn check_state(data: &[u8], offset: usize, expected: u8) -> Result<(), ProgramError> {
    if offset >= data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    if data[offset] != expected {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Check that the state byte at `offset` is NOT `rejected`.
#[inline(always)]
pub fn check_state_not(data: &[u8], offset: usize, rejected: u8) -> Result<(), ProgramError> {
    if offset >= data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    if data[offset] == rejected {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Check that the state byte at `offset` is in the `allowed` set.
#[inline]
pub fn check_state_in(
    data: &[u8],
    offset: usize,
    allowed: &[u8],
) -> Result<(), ProgramError> {
    if offset >= data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let state = data[offset];
    for &a in allowed {
        if state == a {
            return Ok(());
        }
    }
    Err(ProgramError::InvalidAccountData)
}

/// Write a new state byte at `offset`, validating the transition.
#[inline]
pub fn transition_state(
    data: &mut [u8],
    offset: usize,
    next: u8,
    valid_transitions: &[(u8, u8)],
) -> Result<(), ProgramError> {
    if offset >= data.len() {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let current = data[offset];
    check_state_transition(current, next, valid_transitions)?;
    data[offset] = next;
    Ok(())
}
