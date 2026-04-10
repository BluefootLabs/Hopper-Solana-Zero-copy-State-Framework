//! Verified CPI -- pre/post state assertions around cross-program invocations.
//!
//! Every Solana framework fires CPI and blindly trusts the result. If the
//! called program has a bug, state corruption propagates silently. Hopper
//! is the first framework to provide substrate-level CPI verification.
//!
//! The pattern: snapshot relevant state before CPI, invoke, then assert
//! post-conditions. If the assertion fails, the instruction aborts before
//! the corrupted state can be read by downstream logic.
//!
//! # Usage
//!
//! ```ignore
//! use hopper_native::verify::{LamportSnapshot, verify_transfer};
//!
//! // Before CPI transfer:
//! let snap = LamportSnapshot::capture(source, destination);
//!
//! // Do the CPI:
//! system_transfer(&source, &destination, amount)?;
//!
//! // Verify the transfer actually happened correctly:
//! snap.verify_transfer(source, destination, amount)?;
//! ```
//!
//! This catches:
//! - Called program transferring wrong amount
//! - Called program not deducting from source
//! - Called program crediting wrong destination
//! - Integer overflow in lamport accounting

use crate::account_view::AccountView;
use crate::error::ProgramError;
use crate::ProgramResult;

// ---- Lamport snapshot ------------------------------------------------

/// Snapshot of lamport balances for two accounts before a CPI.
///
/// Captures the source and destination balances so that after the CPI
/// completes, we can verify the expected transfer occurred.
#[derive(Clone, Copy, Debug)]
pub struct LamportSnapshot {
    source_before: u64,
    destination_before: u64,
}

impl LamportSnapshot {
    /// Capture the current lamport balances of source and destination.
    #[inline(always)]
    pub fn capture(source: &AccountView, destination: &AccountView) -> Self {
        Self {
            source_before: source.lamports(),
            destination_before: destination.lamports(),
        }
    }

    /// Verify that exactly `amount` lamports moved from source to destination.
    ///
    /// Checks:
    /// 1. Source decreased by exactly `amount`
    /// 2. Destination increased by exactly `amount`
    /// 3. No overflow/underflow occurred
    #[inline]
    pub fn verify_transfer(
        &self,
        source: &AccountView,
        destination: &AccountView,
        amount: u64,
    ) -> ProgramResult {
        let source_after = source.lamports();
        let dest_after = destination.lamports();

        // Source must have decreased by exactly `amount`.
        let source_delta = self.source_before.checked_sub(source_after)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if source_delta != amount {
            return Err(ProgramError::InvalidAccountData);
        }

        // Destination must have increased by exactly `amount`.
        let dest_delta = dest_after.checked_sub(self.destination_before)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if dest_delta != amount {
            return Err(ProgramError::InvalidAccountData);
        }

        Ok(())
    }

    /// Verify that the source decreased by exactly `amount` (one-sided check).
    ///
    /// Use this when the destination is a program-controlled escrow or
    /// fee account where you only care about the deduction.
    #[inline]
    pub fn verify_deduction(
        &self,
        source: &AccountView,
        amount: u64,
    ) -> ProgramResult {
        let delta = self.source_before.checked_sub(source.lamports())
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if delta != amount {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }

    /// Verify that neither balance changed (no-op CPI or read-only call).
    #[inline]
    pub fn verify_unchanged(
        &self,
        source: &AccountView,
        destination: &AccountView,
    ) -> ProgramResult {
        if source.lamports() != self.source_before
            || destination.lamports() != self.destination_before
        {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }

    /// Get the pre-CPI source balance.
    #[inline(always)]
    pub fn source_before(&self) -> u64 {
        self.source_before
    }

    /// Get the pre-CPI destination balance.
    #[inline(always)]
    pub fn destination_before(&self) -> u64 {
        self.destination_before
    }
}

// ---- Single-account snapshot -----------------------------------------

/// Snapshot of a single account's lamports for simple balance assertions.
#[derive(Clone, Copy, Debug)]
pub struct BalanceSnapshot {
    before: u64,
}

impl BalanceSnapshot {
    /// Capture a single account's lamport balance.
    #[inline(always)]
    pub fn capture(account: &AccountView) -> Self {
        Self { before: account.lamports() }
    }

    /// Verify the balance increased by at least `min_increase`.
    #[inline]
    pub fn verify_increased_by(
        &self,
        account: &AccountView,
        min_increase: u64,
    ) -> ProgramResult {
        let current = account.lamports();
        let delta = current.checked_sub(self.before)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if delta < min_increase {
            return Err(ProgramError::InsufficientFunds);
        }
        Ok(())
    }

    /// Verify the balance decreased by at most `max_decrease`.
    #[inline]
    pub fn verify_decreased_by_at_most(
        &self,
        account: &AccountView,
        max_decrease: u64,
    ) -> ProgramResult {
        let current = account.lamports();
        let delta = self.before.checked_sub(current)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        if delta > max_decrease {
            return Err(ProgramError::InsufficientFunds);
        }
        Ok(())
    }

    /// Verify the balance is unchanged.
    #[inline]
    pub fn verify_unchanged(&self, account: &AccountView) -> ProgramResult {
        if account.lamports() != self.before {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }

    /// Get the captured balance.
    #[inline(always)]
    pub fn before(&self) -> u64 {
        self.before
    }

    /// Compute the net change (positive = gained, negative = lost).
    ///
    /// Returns the change as an i128 to avoid overflow.
    #[inline(always)]
    pub fn net_change(&self, account: &AccountView) -> i128 {
        account.lamports() as i128 - self.before as i128
    }
}

// ---- Data integrity snapshot -----------------------------------------

/// Fast integrity check for account data using FNV-1a hash.
///
/// Use this to detect unexpected data mutations around CPI calls.
/// Not cryptographically secure -- purely for integrity assertions.
#[derive(Clone, Copy, Debug)]
pub struct DataFingerprint {
    hash: u64,
    data_len: usize,
}

impl DataFingerprint {
    /// Compute a fast fingerprint of the first `len` bytes of account data.
    ///
    /// Uses FNV-1a (fast, no dependencies, good collision resistance for
    /// short inputs). Not suitable for cryptographic purposes.
    #[inline]
    pub fn capture(account: &AccountView, len: usize) -> Self {
        let data_len = account.data_len().min(len);
        let data_ptr = account.data_ptr();

        // FNV-1a hash.
        let mut hash: u64 = 0xcbf29ce484222325;
        let mut i = 0;
        while i < data_len {
            let byte = unsafe { *data_ptr.add(i) };
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
            i += 1;
        }

        Self { hash, data_len }
    }

    /// Verify the data has not changed since the snapshot.
    #[inline]
    pub fn verify_unchanged(&self, account: &AccountView) -> ProgramResult {
        let current = Self::capture(account, self.data_len);
        if current.hash != self.hash || current.data_len != self.data_len {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(())
    }

    /// Get the fingerprint hash.
    #[inline(always)]
    pub fn hash(&self) -> u64 {
        self.hash
    }
}
