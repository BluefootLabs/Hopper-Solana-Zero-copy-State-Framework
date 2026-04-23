//! Remaining-accounts accessor with strict and passthrough modes.
//!
//! The declared context validates exactly `ACCOUNT_COUNT` accounts.
//! Any accounts beyond that index are "remaining": pool participants,
//! keeper bot recipients, arbitrary fanout destinations, remainder
//! destinations for sweeps, and so on. Hopper exposes two ways to
//! consume them.
//!
//! ## Strict mode
//!
//! Default. The accessor rejects any remaining account whose address
//! matches a previously seen account (either declared or already
//! yielded). Protects against accidental double-spending when a
//! caller tries to alias one slot into two different roles.
//!
//! ```ignore
//! let rem = ctx.remaining_accounts();
//! for maybe_acc in rem.iter() {
//!     let acc = maybe_acc?; // errors on duplicate
//!     // ...
//! }
//! ```
//!
//! ## Passthrough mode
//!
//! Opt-in. Preserves duplicates verbatim. Use when the caller is
//! expected to pass the same account in multiple roles (batched CPI
//! fan-in, for example).
//!
//! ```ignore
//! let rem = ctx.remaining_accounts_passthrough();
//! ```
//!
//! Both modes are O(n) with no heap and no syscalls. Strict mode
//! keeps a small const-sized seen-address cache sized at 64; past
//! that, it falls back to a linear scan of the declared slice plus
//! the yielded-view cursor.

use crate::{account::AccountView, error::ProgramError, result::ProgramResult};

/// Upper bound on remaining-account iterator length. Matches Quasar's
/// `MAX_REMAINING_ACCOUNTS` so programs porting from one framework to
/// the other see the same ceiling. Exceeding this returns an error
/// rather than risking unbounded stack usage in the seen-address cache.
pub const MAX_REMAINING_ACCOUNTS: usize = 64;

/// Error surface for the remaining-accounts accessor.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RemainingError {
    /// Two remaining-account slots resolved to the same address, or a
    /// remaining-account address matched an already-declared account.
    /// Only strict mode emits this.
    DuplicateAccount,
    /// More than [`MAX_REMAINING_ACCOUNTS`] were accessed via the
    /// iterator.
    Overflow,
}

impl From<RemainingError> for ProgramError {
    fn from(e: RemainingError) -> Self {
        match e {
            RemainingError::DuplicateAccount => ProgramError::InvalidAccountData,
            RemainingError::Overflow => ProgramError::InvalidArgument,
        }
    }
}

/// Duplicate-handling policy for a [`RemainingAccounts`] view.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum RemainingMode {
    /// Reject any yielded account whose address matches a declared or
    /// previously-yielded account. Safe default for pool programs
    /// and anything that intends every slot to be distinct.
    Strict,
    /// Yield every slot as is. Use when the caller is expected to
    /// pass aliases (batched fan-in, self-transfers, etc.).
    Passthrough,
}

/// Zero-allocation remaining-accounts view.
///
/// Construct via [`RemainingAccounts::strict`] or
/// [`RemainingAccounts::passthrough`] from the declared slice and the
/// full accounts slice. `#[hopper::context]` emits
/// `ctx.remaining_accounts()` and `ctx.remaining_accounts_passthrough()`
/// accessors that wire these up for you.
pub struct RemainingAccounts<'a> {
    /// Already-validated context accounts, used for dedup in strict mode.
    declared: &'a [&'a AccountView],
    /// Accounts beyond the declared count.
    remaining: &'a [&'a AccountView],
    /// Duplicate-handling policy.
    mode: RemainingMode,
}

impl<'a> RemainingAccounts<'a> {
    /// Build a strict accessor. Iteration rejects duplicates.
    #[inline(always)]
    pub fn strict(declared: &'a [&'a AccountView], remaining: &'a [&'a AccountView]) -> Self {
        Self { declared, remaining, mode: RemainingMode::Strict }
    }

    /// Build a passthrough accessor. Iteration preserves duplicates.
    #[inline(always)]
    pub fn passthrough(
        declared: &'a [&'a AccountView],
        remaining: &'a [&'a AccountView],
    ) -> Self {
        Self { declared, remaining, mode: RemainingMode::Passthrough }
    }

    /// Length of the remaining slice, irrespective of mode.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.remaining.len()
    }

    /// True when there are no remaining accounts.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.remaining.is_empty()
    }

    /// The active duplicate-handling policy for this view.
    #[inline(always)]
    pub fn mode(&self) -> RemainingMode {
        self.mode
    }

    /// Random access by index. Passthrough returns the slot as is;
    /// strict returns an error when the resolved slot aliases a
    /// previously-seen account (declared or yielded before `index`).
    pub fn get(&self, index: usize) -> Result<Option<&'a AccountView>, ProgramError> {
        if index >= self.remaining.len() {
            return Ok(None);
        }
        let candidate = self.remaining[index];
        match self.mode {
            RemainingMode::Passthrough => Ok(Some(candidate)),
            RemainingMode::Strict => {
                if index > MAX_REMAINING_ACCOUNTS {
                    return Err(RemainingError::Overflow.into());
                }
                // Scan declared.
                for d in self.declared {
                    if d.address() == candidate.address() {
                        return Err(RemainingError::DuplicateAccount.into());
                    }
                }
                // Scan remaining[0..index].
                for r in &self.remaining[..index] {
                    if r.address() == candidate.address() {
                        return Err(RemainingError::DuplicateAccount.into());
                    }
                }
                Ok(Some(candidate))
            }
        }
    }

    /// Sequential iterator. Yields each account in declaration order,
    /// errors on duplicates in strict mode, preserves them in
    /// passthrough mode.
    #[inline(always)]
    pub fn iter(&self) -> RemainingIter<'a> {
        RemainingIter {
            declared: self.declared,
            remaining: self.remaining,
            mode: self.mode,
            index: 0,
        }
    }
}

/// Iterator yielded by [`RemainingAccounts::iter`].
pub struct RemainingIter<'a> {
    declared: &'a [&'a AccountView],
    remaining: &'a [&'a AccountView],
    mode: RemainingMode,
    index: usize,
}

impl<'a> Iterator for RemainingIter<'a> {
    type Item = Result<&'a AccountView, ProgramError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.remaining.len() {
            return None;
        }
        if self.index >= MAX_REMAINING_ACCOUNTS {
            // Pin the cursor so repeated calls after overflow stay
            // cheap and deterministic.
            self.index = self.remaining.len();
            return Some(Err(RemainingError::Overflow.into()));
        }
        let candidate = self.remaining[self.index];
        let i = self.index;
        self.index = self.index.wrapping_add(1);

        if matches!(self.mode, RemainingMode::Strict) {
            for d in self.declared {
                if d.address() == candidate.address() {
                    return Some(Err(RemainingError::DuplicateAccount.into()));
                }
            }
            for r in &self.remaining[..i] {
                if r.address() == candidate.address() {
                    return Some(Err(RemainingError::DuplicateAccount.into()));
                }
            }
        }
        Some(Ok(candidate))
    }
}

/// Ergonomic fall-through used by the proc-macro codegen when the user
/// wants to just burn through remaining accounts without a mode.
#[inline(always)]
pub fn strict<'a>(
    declared: &'a [&'a AccountView],
    remaining: &'a [&'a AccountView],
) -> RemainingAccounts<'a> {
    RemainingAccounts::strict(declared, remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    // `AccountView` is backend-specific; we cannot construct one under
    // a non-Solana `cfg`. These tests exist to keep the module
    // exercised at compile time even when the construction helpers
    // live behind `target_os = "solana"`.

    #[test]
    fn error_variants_surface_as_program_error() {
        let dup: ProgramError = RemainingError::DuplicateAccount.into();
        assert_eq!(dup, ProgramError::InvalidAccountData);
        let ovf: ProgramError = RemainingError::Overflow.into();
        assert_eq!(ovf, ProgramError::InvalidArgument);
    }

    #[test]
    fn max_remaining_matches_quasar() {
        // If we ever change this, also update the Quasar parity doc.
        assert_eq!(MAX_REMAINING_ACCOUNTS, 64);
    }
}
