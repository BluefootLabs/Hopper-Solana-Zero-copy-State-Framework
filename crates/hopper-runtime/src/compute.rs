//! Compute-budget introspection helpers.

use crate::{ProgramError, ProgramResult};

/// Read the current remaining compute units.
///
/// Off-chain tests return `u64::MAX`, treating local execution as unlimited.
#[inline(always)]
pub fn remaining_compute_units() -> u64 {
    crate::syscalls::sol_remaining_compute_units()
}

/// Require at least `minimum` compute units and return the current balance.
#[inline(always)]
pub fn require_compute_units(minimum: u64) -> Result<u64, ProgramError> {
    let remaining = remaining_compute_units();
    if remaining < minimum {
        return Err(ProgramError::InvalidArgument);
    }
    Ok(remaining)
}

/// Fail if fewer than `minimum` compute units remain.
#[inline(always)]
pub fn check_compute_units(minimum: u64) -> ProgramResult {
    require_compute_units(minimum).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offchain_compute_is_unlimited() {
        assert_eq!(remaining_compute_units(), u64::MAX);
        assert_eq!(require_compute_units(10).unwrap(), u64::MAX);
        assert!(check_compute_units(u64::MAX).is_ok());
    }
}
