//! Segment-level borrow registry for fine-grained access control.
//!
//! The account-level [`BorrowRegistry`](crate::borrow_registry) prevents
//! aliasing across entire accounts. This module adds **segment-level**
//! conflict detection: two borrows of the *same* account are allowed when
//! their byte ranges don't overlap, or when both are read-only.
//!
//! ## Conflict Rules
//!
//! | Existing | New   | Overlapping? | Allowed |
//! |----------|-------|--------------|---------|
//! | Read     | Read  | yes          | ✅       |
//! | Read     | Write | yes          | ❌       |
//! | Write    | Read  | yes          | ❌       |
//! | Write    | Write | yes          | ❌       |
//! | *any*    | *any* | no           | ✅       |
//!
//! ## Zero-Cost Design
//!
//! - Fixed-capacity array (no heap)
//! - Inline conflict checks
//! - Deterministic iteration (bounded loop)

use crate::address::Address;
use crate::error::ProgramError;

/// Maximum simultaneous segment borrows per instruction.
///
/// 32 is generous for any real instruction — most use 2–6 segments.
/// Keeping it fixed avoids heap allocation while staying well within
/// Solana's CU budget.
pub const MAX_SEGMENT_BORROWS: usize = 32;

/// Read or write access intent for a segment borrow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum AccessKind {
    /// Shared (immutable) access.
    Read = 0,
    /// Exclusive (mutable) access.
    Write = 1,
}

/// A single active segment borrow.
#[derive(Clone, Copy, Debug)]
pub struct SegmentBorrow {
    /// Account address this borrow targets.
    pub key: Address,
    /// Byte offset within the account data.
    pub offset: u32,
    /// Byte size of the borrowed segment.
    pub size: u32,
    /// Access kind (read or write).
    pub kind: AccessKind,
}

/// Check whether two byte ranges overlap.
#[inline(always)]
const fn ranges_overlap(a_off: u32, a_size: u32, b_off: u32, b_size: u32) -> bool {
    let a_end = a_off + a_size;
    let b_end = b_off + b_size;
    // Non-overlapping iff one ends before the other starts.
    !(a_end <= b_off || b_end <= a_off)
}

/// Instruction-scoped segment borrow registry.
///
/// Tracks active segment borrows and enforces conflict rules. Designed
/// for inline use in an execution context — no heap, no dynamic dispatch.
///
/// # Example
///
/// ```ignore
/// let mut borrows = SegmentBorrowRegistry::new();
///
/// // Read balance — allowed
/// borrows.register(SegmentBorrow {
///     key: vault_key,
///     offset: 0,
///     size: 8,
///     kind: AccessKind::Read,
/// })?;
///
/// // Write metadata — different range, allowed
/// borrows.register(SegmentBorrow {
///     key: vault_key,
///     offset: 8,
///     size: 32,
///     kind: AccessKind::Write,
/// })?;
///
/// // Write balance — overlaps with read, REJECTED
/// borrows.register(SegmentBorrow {
///     key: vault_key,
///     offset: 0,
///     size: 8,
///     kind: AccessKind::Write,
/// }).unwrap_err(); // AccountBorrowFailed
/// ```
pub struct SegmentBorrowRegistry {
    entries: [Option<SegmentBorrow>; MAX_SEGMENT_BORROWS],
    len: usize,
}

impl SegmentBorrowRegistry {
    /// Create an empty registry.
    #[inline(always)]
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_SEGMENT_BORROWS],
            len: 0,
        }
    }

    /// Number of active borrows.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Whether the registry is empty.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Register a new segment borrow, checking for conflicts.
    ///
    /// Returns `Err(AccountBorrowFailed)` if the new borrow overlaps an
    /// existing borrow with incompatible access (read+write or write+write).
    #[inline]
    pub fn register(&mut self, new: SegmentBorrow) -> Result<(), ProgramError> {
        if self.len >= MAX_SEGMENT_BORROWS {
            return Err(ProgramError::AccountBorrowFailed);
        }

        // Check conflicts against all active borrows.
        let mut i = 0;
        while i < self.len {
            if let Some(ref existing) = self.entries[i] {
                // Only check borrows targeting the same account.
                if existing.key == new.key
                    && ranges_overlap(existing.offset, existing.size, new.offset, new.size)
                {
                    // Overlapping ranges — only read+read is allowed.
                    match (existing.kind, new.kind) {
                        (AccessKind::Read, AccessKind::Read) => {}
                        _ => return Err(ProgramError::AccountBorrowFailed),
                    }
                }
            }
            i += 1;
        }

        self.entries[self.len] = Some(new);
        self.len += 1;
        Ok(())
    }

    /// Convenience: register a read borrow for the given account region.
    #[inline(always)]
    pub fn register_read(
        &mut self,
        key: Address,
        offset: u32,
        size: u32,
    ) -> Result<(), ProgramError> {
        self.register(SegmentBorrow { key, offset, size, kind: AccessKind::Read })
    }

    /// Convenience: register a write borrow for the given account region.
    #[inline(always)]
    pub fn register_write(
        &mut self,
        key: Address,
        offset: u32,
        size: u32,
    ) -> Result<(), ProgramError> {
        self.register(SegmentBorrow { key, offset, size, kind: AccessKind::Write })
    }

    /// Release a previously registered borrow.
    ///
    /// Finds the first matching entry (key + offset + size + kind) and
    /// removes it, compacting the array.
    #[inline]
    pub fn release(&mut self, borrow: &SegmentBorrow) -> bool {
        let mut i = 0;
        while i < self.len {
            if let Some(ref existing) = self.entries[i] {
                if existing.key == borrow.key
                    && existing.offset == borrow.offset
                    && existing.size == borrow.size
                    && existing.kind == borrow.kind
                {
                    // Swap-remove: move last entry into this slot.
                    self.len -= 1;
                    self.entries[i] = if i < self.len {
                        self.entries[self.len].take()
                    } else {
                        None
                    };
                    return true;
                }
            }
            i += 1;
        }
        false
    }

    /// Reset the registry, clearing all active borrows.
    #[inline(always)]
    pub fn clear(&mut self) {
        self.len = 0;
        // Don't bother zeroing entries — len gate prevents reading stale data.
    }

    /// Check if a proposed borrow would conflict, without registering it.
    #[inline]
    pub fn would_conflict(&self, proposed: &SegmentBorrow) -> bool {
        let mut i = 0;
        while i < self.len {
            if let Some(ref existing) = self.entries[i] {
                if existing.key == proposed.key
                    && ranges_overlap(existing.offset, existing.size, proposed.offset, proposed.size)
                {
                    match (existing.kind, proposed.kind) {
                        (AccessKind::Read, AccessKind::Read) => {}
                        _ => return true,
                    }
                }
            }
            i += 1;
        }
        false
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Address;

    fn test_addr(seed: u8) -> Address {
        Address::new([seed; 32])
    }

    #[test]
    fn read_read_same_range_allowed() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        assert!(reg.register_read(key, 0, 8).is_ok());
        assert!(reg.register_read(key, 0, 8).is_ok());
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn read_write_same_range_rejected() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        assert!(reg.register_read(key, 0, 8).is_ok());
        assert!(reg.register_write(key, 0, 8).is_err());
    }

    #[test]
    fn write_write_same_range_rejected() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        assert!(reg.register_write(key, 0, 8).is_ok());
        assert!(reg.register_write(key, 0, 8).is_err());
    }

    #[test]
    fn write_read_same_range_rejected() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        assert!(reg.register_write(key, 0, 8).is_ok());
        assert!(reg.register_read(key, 0, 8).is_err());
    }

    #[test]
    fn non_overlapping_write_write_allowed() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        // balance: [0..8), metadata: [8..40)
        assert!(reg.register_write(key, 0, 8).is_ok());
        assert!(reg.register_write(key, 8, 32).is_ok());
    }

    #[test]
    fn partially_overlapping_rejected() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        // [0..16) and [8..24) overlap at [8..16)
        assert!(reg.register_write(key, 0, 16).is_ok());
        assert!(reg.register_write(key, 8, 16).is_err());
    }

    #[test]
    fn different_accounts_always_allowed() {
        let mut reg = SegmentBorrowRegistry::new();
        assert!(reg.register_write(test_addr(1), 0, 8).is_ok());
        assert!(reg.register_write(test_addr(2), 0, 8).is_ok());
    }

    #[test]
    fn release_then_reacquire() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        let borrow = SegmentBorrow {
            key,
            offset: 0,
            size: 8,
            kind: AccessKind::Write,
        };
        assert!(reg.register(borrow).is_ok());
        assert!(reg.register_write(key, 0, 8).is_err()); // conflict
        assert!(reg.release(&borrow));
        assert!(reg.register_write(key, 0, 8).is_ok()); // now OK
    }

    #[test]
    fn capacity_limit() {
        let mut reg = SegmentBorrowRegistry::new();
        for i in 0..MAX_SEGMENT_BORROWS {
            assert!(reg.register_read(test_addr(1), i as u32 * 8, 8).is_ok());
        }
        // One more should fail.
        assert!(reg.register_read(test_addr(1), 256, 8).is_err());
    }

    #[test]
    fn would_conflict_does_not_mutate() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        assert!(reg.register_write(key, 0, 8).is_ok());
        let proposed = SegmentBorrow {
            key,
            offset: 0,
            size: 8,
            kind: AccessKind::Write,
        };
        assert!(reg.would_conflict(&proposed));
        assert_eq!(reg.len(), 1); // unchanged
    }

    #[test]
    fn adjacent_ranges_no_conflict() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        // [0..8) and [8..16) are adjacent, not overlapping.
        assert!(reg.register_write(key, 0, 8).is_ok());
        assert!(reg.register_write(key, 8, 8).is_ok());
    }
}
