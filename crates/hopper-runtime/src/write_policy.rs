//! Declared write-sets enforced at borrow acquisition (innovation I12).
//!
//! Sealevel's account model stops at one bit of write granularity: the
//! transaction-level `writable` flag covers the *entire* account. This
//! module extends that to **byte-range granularity**: an instruction
//! declares the exact ranges it is allowed to write, and the
//! [`Context`](crate::context::Context) rejects any write borrow outside
//! the declared set *at acquisition time* — before a single byte moves.
//!
//! ## How it composes
//!
//! - `#[hopper::context(strict_writes)]` compiles the context's declared
//!   `mut` / `mut(seg, ...)` attributes into a `static` [`WritePolicy`]
//!   and installs it during `bind()`. Nothing is computed at runtime; the
//!   policy is a const slice scanned at each write acquire.
//! - Every Context-mediated write path is gated: segment writes
//!   (`segment_mut`, `segment_mut_const`, `segment_mut_typed`,
//!   `split_segments_mut`), whole-account typed loads (`load_mut`), and
//!   the raw escape hatches (`raw_mut`, `as_mut_ptr`).
//! - A whole-account borrow claims `[0, data_len)`, so under a policy that
//!   declares only field ranges, `load_mut` / `as_mut_ptr` are refused and
//!   the handler must use the declared segment accessors. That is the
//!   discipline the policy exists to enforce.
//! - Paired with the `touch-map` feature (I7), the declared set can be
//!   compared against the *actual* footprint in tests: declared-vs-actual
//!   write verification with no instrumentation in the program itself.
//!
//! ## Enforcement boundary
//!
//! The policy governs access **through `Context`** — which is every path
//! `#[hopper::context]`-generated code uses. Direct substrate access
//! (`ctx.account(i)?.try_borrow_mut()` on the raw [`AccountView`]) is
//! outside the governed surface, exactly like the documented raw-pointer
//! escape hatches; it is visible in review and lintable. The Sealevel
//! `writable` flag is still enforced underneath in all cases.
//!
//! [`AccountView`]: crate::account::AccountView

use crate::error::ProgramError;

/// Error page for write-policy violations.
///
/// A rejected write on account index `i` surfaces as
/// `ProgramError::Custom(0xD000 | i)`, mirroring the `0xC000 | i`
/// convention used for declarative constraint failures. The account
/// index in the low byte makes the offending account recoverable from
/// the bare error code in logs and explorers.
pub const WRITE_POLICY_VIOLATION_PAGE: u32 = 0xD0_00;

/// Build the write-policy-violation error for an account index.
#[inline(always)]
pub const fn write_policy_violation(account_index: u8) -> ProgramError {
    ProgramError::Custom(WRITE_POLICY_VIOLATION_PAGE | account_index as u32)
}

/// One allowed write range on one instruction account.
///
/// `account_index` is the position in the instruction's account list
/// (the same index handed to [`Context::account`](crate::context::Context::account)).
/// Offsets are absolute within the account data, including any layout
/// header bytes — the same convention the segment access primitives use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteRange {
    /// Instruction account-list index this range applies to.
    pub account_index: u8,
    /// Absolute byte offset of the allowed range.
    pub offset: u32,
    /// Byte size of the allowed range. [`WriteRange::whole_account`]
    /// uses `u32::MAX`, which contains any request on the account
    /// (account data is capped far below 4 GiB).
    pub size: u32,
}

impl WriteRange {
    /// Allow writes to `[offset, offset + size)` on `account_index`.
    #[inline(always)]
    pub const fn new(account_index: u8, offset: u32, size: u32) -> Self {
        Self {
            account_index,
            offset,
            size,
        }
    }

    /// Allow whole-account writes on `account_index` (what a plain
    /// `mut` declaration grants).
    #[inline(always)]
    pub const fn whole_account(account_index: u8) -> Self {
        Self {
            account_index,
            offset: 0,
            size: u32::MAX,
        }
    }

    /// Whether `[offset, offset + size)` is fully contained in this
    /// range. Widened to `u64` so `offset + size` cannot wrap.
    #[inline(always)]
    pub const fn contains(&self, offset: u32, size: u32) -> bool {
        let req_start = offset as u64;
        let req_end = offset as u64 + size as u64;
        let start = self.offset as u64;
        let end = self.offset as u64 + self.size as u64;
        req_start >= start && req_end <= end
    }
}

/// Declared write-set for one instruction.
///
/// Intended to be a `static` built at macro-expansion time from the
/// context's `mut` / `mut(seg, ...)` declarations and installed via
/// [`Context::set_write_policy`](crate::context::Context::set_write_policy).
/// An **empty** set is a valid policy: it denies every Context-mediated
/// write, turning the instruction into a machine-checked read-only
/// contract.
#[derive(Debug)]
pub struct WritePolicy {
    /// Allowed write ranges. Scanned linearly; contexts declare a
    /// handful of ranges, so a bounded scan beats any lookup structure
    /// at Solana scale.
    pub allows: &'static [WriteRange],
}

impl WritePolicy {
    /// Wrap a const slice of allowed ranges as a policy.
    #[inline(always)]
    pub const fn new(allows: &'static [WriteRange]) -> Self {
        Self { allows }
    }

    /// `Ok(())` iff `[offset, offset + size)` on `account_index` is
    /// fully contained in a **single** declared range. Adjacent declared
    /// ranges are not coalesced at check time; the macro emits ranges
    /// exactly as declared, so a request straddling two declarations is
    /// refused (declare a covering range if that access is intended).
    #[inline(always)]
    pub fn check_write(
        &self,
        account_index: u8,
        offset: u32,
        size: u32,
    ) -> Result<(), ProgramError> {
        let ranges = self.allows;
        let mut i = 0;
        while i < ranges.len() {
            let r = &ranges[i];
            if r.account_index == account_index && r.contains(offset, size) {
                return Ok(());
            }
            i += 1;
        }
        Err(write_policy_violation(account_index))
    }

    /// Non-erroring form of [`check_write`](Self::check_write).
    #[inline(always)]
    pub fn allows_write(&self, account_index: u8, offset: u32, size: u32) -> bool {
        self.check_write(account_index, offset, size).is_ok()
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // vault (account 1): balance [16, 24), nonce [24, 32)
    static POLICY: WritePolicy = WritePolicy::new(&[
        WriteRange::new(1, 16, 8),
        WriteRange::new(1, 24, 8),
        WriteRange::whole_account(2),
    ]);

    #[test]
    fn declared_ranges_allow_exact_and_contained_writes() {
        assert!(POLICY.check_write(1, 16, 8).is_ok());
        assert!(POLICY.check_write(1, 24, 8).is_ok());
        // Strictly inside a declared range is also allowed.
        assert!(POLICY.check_write(1, 18, 4).is_ok());
        // Zero-size request inside a range is trivially contained.
        assert!(POLICY.check_write(1, 20, 0).is_ok());
    }

    #[test]
    fn undeclared_ranges_are_refused_with_indexed_error() {
        // Outside every declared range.
        assert_eq!(
            POLICY.check_write(1, 0, 8),
            Err(ProgramError::Custom(0xD0_01))
        );
        // Overlapping but not contained.
        assert!(POLICY.check_write(1, 12, 8).is_err());
        // Straddling two adjacent declared ranges is refused: containment
        // is per-declaration, not per-union.
        assert!(POLICY.check_write(1, 16, 16).is_err());
        // Right account, range declared on a different account.
        assert_eq!(
            POLICY.check_write(0, 16, 8),
            Err(ProgramError::Custom(0xD0_00))
        );
    }

    #[test]
    fn whole_account_allowance_contains_any_request() {
        assert!(POLICY.check_write(2, 0, 8).is_ok());
        assert!(POLICY.check_write(2, 0, u32::MAX).is_ok());
        assert!(POLICY.check_write(2, 4096, 10 * 1024 * 1024).is_ok());
    }

    #[test]
    fn empty_policy_denies_all_writes() {
        static READ_ONLY: WritePolicy = WritePolicy::new(&[]);
        assert!(READ_ONLY.check_write(0, 0, 1).is_err());
        assert!(READ_ONLY.check_write(255, 0, 0).is_err());
    }

    #[test]
    fn containment_survives_u32_boundary_arithmetic() {
        static EDGE: WritePolicy = WritePolicy::new(&[WriteRange::new(0, u32::MAX - 8, 8)]);
        // `offset + size` at the top of u32 must not wrap into a false allow.
        assert!(EDGE.check_write(0, u32::MAX - 8, 8).is_ok());
        assert!(EDGE.check_write(0, u32::MAX - 4, 8).is_err());
    }
}
