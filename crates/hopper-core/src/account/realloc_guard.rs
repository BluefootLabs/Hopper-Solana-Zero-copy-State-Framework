//! Realloc session guard -- cumulative growth-limit enforcement.
//!
//! Tracks original account sizes at instruction entry and enforces
//! a per-instruction growth budget. Prevents runaway realloc chains
//! that could exhaust transaction compute or memory budgets.
//!
//! ## Wire model
//!
//! Entirely stack-based. No account data overhead -- the guard lives
//! in the instruction handler's stack frame and is discarded at return.
//!
//! ## Usage
//!
//! ```ignore
//! let mut guard = ReallocGuard::<8>::new(10240); // 10KB budget
//! guard.register(0, vault_account.data_len());
//! guard.register(1, config_account.data_len());
//!
//! // Later, when reallocating:
//! guard.check_growth(0, new_size)?;
//! safe_realloc(vault_account, new_size, payer)?;
//! guard.commit_growth(0, new_size);
//! ```

use hopper_runtime::error::ProgramError;

/// Per-instruction realloc budget guard.
///
/// `N` is the maximum number of accounts to track (const generic, stack-allocated).
/// Typical values: 4, 8, or 16.
pub struct ReallocGuard<const N: usize> {
    /// Original sizes of tracked accounts.
    original: [u32; N],
    /// Current sizes of tracked accounts (updated after each realloc).
    current: [u32; N],
    /// Number of registered accounts.
    count: usize,
    /// Maximum cumulative growth allowed (bytes).
    budget: u32,
    /// Cumulative growth consumed so far.
    consumed: u32,
}

impl<const N: usize> ReallocGuard<N> {
    /// Create a new guard with the given growth budget (bytes).
    #[inline(always)]
    pub const fn new(budget: u32) -> Self {
        Self {
            original: [0u32; N],
            current: [0u32; N],
            count: 0,
            budget,
            consumed: 0,
        }
    }

    /// Register an account's original size for tracking.
    ///
    /// `slot` is the local tracking index (0..N-1), not the account index.
    /// `size` is the current data length.
    /// Returns `Err` if `slot >= N`.
    #[inline(always)]
    pub fn register(&mut self, slot: usize, size: usize) -> Result<(), ProgramError> {
        if slot >= N {
            return Err(ProgramError::InvalidArgument);
        }
        let size32 = size as u32;
        self.original[slot] = size32;
        self.current[slot] = size32;
        if slot >= self.count {
            self.count = slot + 1;
        }
        Ok(())
    }

    /// Check if growing account `slot` to `new_size` is within budget.
    ///
    /// Does NOT commit the growth -- call `commit_growth` after the
    /// actual realloc succeeds.
    #[inline]
    pub fn check_growth(
        &self,
        slot: usize,
        new_size: usize,
    ) -> Result<(), ProgramError> {
        if slot >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let new_size32 = new_size as u32;
        let current = self.current[slot];

        if new_size32 <= current {
            // Shrinking or same -- always allowed.
            return Ok(());
        }

        let delta = new_size32 - current;
        let new_consumed = self.consumed.checked_add(delta)
            .ok_or(ProgramError::ArithmeticOverflow)?;

        if new_consumed > self.budget {
            return Err(ProgramError::InvalidRealloc);
        }

        Ok(())
    }

    /// Commit a growth after successful realloc.
    ///
    /// Must be called after the actual `safe_realloc` succeeds.
    /// Returns `Err` if `slot` is not registered.
    #[inline(always)]
    pub fn commit_growth(&mut self, slot: usize, new_size: usize) -> Result<(), ProgramError> {
        if slot >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let new_size32 = new_size as u32;
        let current = self.current[slot];

        if new_size32 > current {
            let delta = new_size32 - current;
            self.consumed += delta;
        } else if new_size32 < current {
            // Shrinking: return budget credit.
            let credit = current - new_size32;
            self.consumed = self.consumed.saturating_sub(credit);
        }

        self.current[slot] = new_size32;
        Ok(())
    }

    /// Remaining budget (bytes).
    #[inline(always)]
    pub const fn remaining(&self) -> u32 {
        self.budget.saturating_sub(self.consumed)
    }

    /// Total consumed growth (bytes).
    #[inline(always)]
    pub const fn consumed(&self) -> u32 {
        self.consumed
    }

    /// Budget cap (bytes).
    #[inline(always)]
    pub const fn budget(&self) -> u32 {
        self.budget
    }

    /// Growth of a specific slot from its original size (bytes).
    #[inline(always)]
    pub fn slot_growth(&self, slot: usize) -> i32 {
        if slot >= self.count {
            return 0;
        }
        self.current[slot] as i32 - self.original[slot] as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_growth_tracking() {
        let mut guard = ReallocGuard::<4>::new(1024);
        guard.register(0, 100).unwrap();
        guard.register(1, 200).unwrap();

        // Growth within budget
        assert!(guard.check_growth(0, 200).is_ok());
        guard.commit_growth(0, 200).unwrap();
        assert_eq!(guard.consumed(), 100);
        assert_eq!(guard.remaining(), 924);

        // Check slot growth
        assert_eq!(guard.slot_growth(0), 100);
        assert_eq!(guard.slot_growth(1), 0);
    }

    #[test]
    fn budget_exceeded() {
        let mut guard = ReallocGuard::<4>::new(100);
        guard.register(0, 50).unwrap();

        // Try to grow beyond budget
        assert!(guard.check_growth(0, 200).is_err()); // 150 > 100 budget
    }

    #[test]
    fn shrink_returns_credit() {
        let mut guard = ReallocGuard::<4>::new(200);
        guard.register(0, 100).unwrap();

        // Grow
        guard.commit_growth(0, 200).unwrap();
        assert_eq!(guard.consumed(), 100);

        // Shrink -- returns credit
        guard.commit_growth(0, 150).unwrap();
        assert_eq!(guard.consumed(), 50);
        assert_eq!(guard.remaining(), 150);
    }

    #[test]
    fn same_size_is_noop() {
        let mut guard = ReallocGuard::<4>::new(100);
        guard.register(0, 100).unwrap();

        assert!(guard.check_growth(0, 100).is_ok());
        guard.commit_growth(0, 100).unwrap();
        assert_eq!(guard.consumed(), 0);
    }

    #[test]
    fn register_out_of_bounds() {
        let mut guard = ReallocGuard::<2>::new(1024);
        assert!(guard.register(0, 100).is_ok());
        assert!(guard.register(1, 200).is_ok());
        assert!(guard.register(2, 300).is_err()); // N=2, slot 2 is OOB
    }

    #[test]
    fn commit_unregistered_slot() {
        let mut guard = ReallocGuard::<4>::new(1024);
        guard.register(0, 100).unwrap();
        // slot 3 was never registered
        assert!(guard.commit_growth(3, 200).is_err());
    }
}
