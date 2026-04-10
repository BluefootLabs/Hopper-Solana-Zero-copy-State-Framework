//! Balance delta checks for CPI composition.
//!
//! When your program CPIs into an AMM or lending protocol, you can't trust
//! return values. Snapshot balances before the CPI, re-read after, and
//! assert the delta is what you expect.
//!
//! ```rust,ignore
//! let before = snapshot_token_balance(vault)?;
//! safe_transfer_tokens(...)?;
//! check_balance_increased(vault, before, min_expected)?;
//! ```

use hopper_runtime::{error::ProgramError, AccountView, ProgramResult};

use crate::token::TOKEN_ACCOUNT_LEN;

/// Snapshot the current token balance from a token account.
///
/// Reads amount at bytes 64..72 of the SPL Token layout.
#[inline(always)]
pub fn snapshot_token_balance(account: &AccountView) -> Result<u64, ProgramError> {
    let data = account.try_borrow()?;
    if data.len() < TOKEN_ACCOUNT_LEN {
        return Err(ProgramError::AccountDataTooSmall);
    }
    Ok(u64::from_le_bytes([
        data[64], data[65], data[66], data[67], data[68], data[69], data[70], data[71],
    ]))
}

/// Snapshot the current lamport balance of any account.
#[inline(always)]
pub fn snapshot_lamport_balance(account: &AccountView) -> u64 {
    account.lamports()
}

/// Verify a token account balance increased by at least `min_increase` since the snapshot.
#[inline(always)]
pub fn check_balance_increased(
    account: &AccountView,
    balance_before: u64,
    min_increase: u64,
) -> ProgramResult {
    let current = snapshot_token_balance(account)?;
    if current < balance_before {
        return Err(ProgramError::InvalidAccountData);
    }
    let delta = current - balance_before;
    if delta < min_increase {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Verify a token account balance decreased by at most `max_decrease` since the snapshot.
#[inline(always)]
pub fn check_balance_decreased(
    account: &AccountView,
    balance_before: u64,
    max_decrease: u64,
) -> ProgramResult {
    let current = snapshot_token_balance(account)?;
    if current > balance_before {
        return Err(ProgramError::InvalidAccountData);
    }
    let delta = balance_before - current;
    if delta > max_decrease {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Verify a token balance delta is within tolerance of the expected amount.
///
/// `tolerance_bps` is the acceptable deviation in basis points.
#[inline(always)]
pub fn check_balance_delta(
    account: &AccountView,
    balance_before: u64,
    expected_delta: u64,
    tolerance_bps: u16,
) -> ProgramResult {
    let current = snapshot_token_balance(account)?;
    let actual_delta = current.abs_diff(balance_before);

    let diff = actual_delta.abs_diff(expected_delta);

    let max_allowed = (expected_delta as u128)
        .saturating_mul(tolerance_bps as u128)
        / 10_000;

    if diff as u128 > max_allowed {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Verify a lamport balance increased by at least `min_increase` since the snapshot.
#[inline(always)]
pub fn check_lamport_balance_increased(
    account: &AccountView,
    balance_before: u64,
    min_increase: u64,
) -> ProgramResult {
    let current = account.lamports();
    if current < balance_before {
        return Err(ProgramError::InvalidAccountData);
    }
    let delta = current - balance_before;
    if delta < min_increase {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}
