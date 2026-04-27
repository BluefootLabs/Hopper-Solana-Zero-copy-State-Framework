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
/// 16 covers any realistic instruction, most use 2-6 segments.
/// Keeping it fixed avoids heap allocation while staying well within
/// Solana's CU budget.  The compact entry representation keeps the
/// total stack footprint under 200 bytes.
pub const MAX_SEGMENT_BORROWS: usize = 16;

/// Read or write access intent for a segment borrow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum AccessKind {
    /// Shared (immutable) access.
    Read = 0,
    /// Exclusive (mutable) access.
    Write = 1,
}

/// First-8-byte prefix of an account address, used as a fast-path
/// comparator in the conflict scan.
///
/// The **audit-correct** model is fingerprint-then-verify: a hot-path
/// `u64` compare rejects unrelated accounts immediately; the slow-path
/// 32-byte compare fires only when the prefixes match. Because a
/// full-address compare always follows, fingerprint collisions produce
/// **no** false conflicts, they only cost one extra 32-byte compare
/// for the extremely rare collision pair.
#[inline(always)]
fn address_fingerprint(address: &Address) -> u64 {
    let bytes = address.as_array();
    u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

/// Full-identity equality check on the slow path.
#[inline(always)]
fn address_eq(a: &Address, b: &Address) -> bool {
    a.as_array() == b.as_array()
}

/// A single active segment borrow.
///
/// Carries both a fast `u64` fingerprint and the full 32-byte account
/// address. The fingerprint is the hot-path comparator; the full
/// address resolves collisions so conflict detection is never
/// probabilistic.
#[derive(Clone, Copy, Debug)]
pub struct SegmentBorrow {
    /// Fast-path prefix of the account address.
    pub key_fp: u64,
    /// Full account address, authoritative identity, checked whenever
    /// the fast-path fingerprint matches. Pre-audit we relied on the
    /// fingerprint alone and claimed it was "collision-free for any
    /// realistic instruction"; that was probabilistic, not a guarantee.
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
/// for inline use in an execution context, no heap, no dynamic dispatch.
///
/// Uses compact 8-byte address fingerprints and a flat array of
/// fixed-size entries.  Total stack footprint: ~280 bytes (vs ~1.3 KB
/// with full 32-byte addresses and Option wrappers).
///
/// # Example
///
/// ```ignore
/// let mut borrows = SegmentBorrowRegistry::new();
/// borrows.register_read(&vault_key, 0, 8)?;   // read balance
/// borrows.register_write(&vault_key, 8, 32)?;  // write metadata, OK, non-overlapping
/// borrows.register_write(&vault_key, 0, 8)?;   // REJECTED, overlaps read
/// ```
pub struct SegmentBorrowRegistry {
    entries: [SegmentBorrow; MAX_SEGMENT_BORROWS],
    len: u8,
}

impl SegmentBorrowRegistry {
    /// Create an empty registry.
    #[inline(always)]
    pub const fn new() -> Self {
        const EMPTY: SegmentBorrow = SegmentBorrow {
            key_fp: 0,
            key: Address::new([0u8; 32]),
            offset: 0,
            size: 0,
            kind: AccessKind::Read,
        };
        Self {
            entries: [EMPTY; MAX_SEGMENT_BORROWS],
            len: 0,
        }
    }

    /// Number of active borrows.
    #[inline(always)]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Whether the registry is empty.
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Register a new read borrow and return the `SegmentBorrow`
    /// record the caller can hand to `SegmentLease::new` for RAII
    /// release. This is the plumbing that makes
    /// [`crate::segment_lease::SegRef`] possible.
    #[inline(always)]
    pub fn register_leased_read(
        &mut self,
        key: &Address,
        offset: u32,
        size: u32,
    ) -> Result<SegmentBorrow, ProgramError> {
        let borrow = SegmentBorrow {
            key_fp: address_fingerprint(key),
            key: *key,
            offset,
            size,
            kind: AccessKind::Read,
        };
        self.register(borrow)?;
        Ok(borrow)
    }

    /// Mutable counterpart of [`register_leased_read`].
    #[inline(always)]
    pub fn register_leased_write(
        &mut self,
        key: &Address,
        offset: u32,
        size: u32,
    ) -> Result<SegmentBorrow, ProgramError> {
        let borrow = SegmentBorrow {
            key_fp: address_fingerprint(key),
            key: *key,
            offset,
            size,
            kind: AccessKind::Write,
        };
        self.register(borrow)?;
        Ok(borrow)
    }

    /// Register a new segment borrow, checking for conflicts.
    ///
    /// Returns `Err(AccountBorrowFailed)` if the new borrow overlaps an
    /// existing borrow with incompatible access (read+write or write+write)
    /// on the **same** account (full-address identity, not fingerprint).
    #[inline(always)]
    pub fn register(&mut self, new: SegmentBorrow) -> Result<(), ProgramError> {
        let len = self.len as usize;
        if len >= MAX_SEGMENT_BORROWS {
            return Err(ProgramError::AccountBorrowFailed);
        }

        // Check conflicts against all active borrows. Fast path on the
        // 8-byte fingerprint; slow path confirms with the full 32-byte
        // address so fingerprint collisions cannot manufacture false
        // conflicts between unrelated accounts.
        let mut i = 0;
        while i < len {
            let existing = &self.entries[i];
            if existing.key_fp == new.key_fp
                && address_eq(&existing.key, &new.key)
                && ranges_overlap(existing.offset, existing.size, new.offset, new.size)
            {
                match (existing.kind, new.kind) {
                    (AccessKind::Read, AccessKind::Read) => {}
                    _ => return Err(ProgramError::AccountBorrowFailed),
                }
            }
            i += 1;
        }

        self.entries[len] = new;
        self.len = (len + 1) as u8;
        Ok(())
    }

    /// Convenience: register a read borrow for the given account region.
    #[inline(always)]
    pub fn register_read(
        &mut self,
        key: &Address,
        offset: u32,
        size: u32,
    ) -> Result<(), ProgramError> {
        self.register(SegmentBorrow {
            key_fp: address_fingerprint(key),
            key: *key,
            offset,
            size,
            kind: AccessKind::Read,
        })
    }

    /// Convenience: register a write borrow for the given account region.
    #[inline(always)]
    pub fn register_write(
        &mut self,
        key: &Address,
        offset: u32,
        size: u32,
    ) -> Result<(), ProgramError> {
        self.register(SegmentBorrow {
            key_fp: address_fingerprint(key),
            key: *key,
            offset,
            size,
            kind: AccessKind::Write,
        })
    }

    /// Release a previously registered borrow.
    ///
    /// Finds the first matching entry and removes it, compacting the array.
    /// Identity is full-address (not fingerprint) to stay collision-safe.
    #[inline(always)]
    pub fn release(&mut self, borrow: &SegmentBorrow) -> bool {
        let len = self.len as usize;
        let mut i = 0;
        while i < len {
            let existing = &self.entries[i];
            if existing.key_fp == borrow.key_fp
                && address_eq(&existing.key, &borrow.key)
                && existing.offset == borrow.offset
                && existing.size == borrow.size
                && existing.kind == borrow.kind
            {
                // Swap-remove: move last entry into this slot.
                let new_len = len - 1;
                self.len = new_len as u8;
                if i < new_len {
                    self.entries[i] = self.entries[new_len];
                }
                return true;
            }
            i += 1;
        }
        false
    }

    /// Reset the registry, clearing all active borrows.
    #[inline(always)]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Check if a proposed borrow would conflict, without registering it.
    ///
    /// Uses full-address identity, fingerprint collisions do not
    /// produce false positives.
    #[inline(always)]
    pub fn would_conflict(&self, proposed: &SegmentBorrow) -> bool {
        let len = self.len as usize;
        let mut i = 0;
        while i < len {
            let existing = &self.entries[i];
            if existing.key_fp == proposed.key_fp
                && address_eq(&existing.key, &proposed.key)
                && ranges_overlap(existing.offset, existing.size, proposed.offset, proposed.size)
            {
                match (existing.kind, proposed.kind) {
                    (AccessKind::Read, AccessKind::Read) => {}
                    _ => return true,
                }
            }
            i += 1;
        }
        false
    }

    /// Register a borrow and return an RAII guard that auto-releases it on drop.
    ///
    /// This is the preferred way to acquire segment borrows, the guard
    /// ensures the borrow is released even if the caller returns early
    /// via `?` or encounters an error.
    ///
    /// # Example
    ///
    /// ```ignore
    /// {
    ///     let _guard = borrows.register_guard_write(&key, 0, 8)?;
    ///     // ... write to segment ...
    /// } // guard dropped → borrow released
    /// ```
    #[inline(always)]
    pub fn register_guard(
        &mut self,
        borrow: SegmentBorrow,
    ) -> Result<SegmentBorrowGuard<'_>, ProgramError> {
        self.register(borrow)?;
        Ok(SegmentBorrowGuard {
            registry: self,
            borrow,
        })
    }

    /// Register a read borrow with RAII auto-release.
    #[inline(always)]
    pub fn register_guard_read(
        &mut self,
        key: &Address,
        offset: u32,
        size: u32,
    ) -> Result<SegmentBorrowGuard<'_>, ProgramError> {
        let borrow = SegmentBorrow {
            key_fp: address_fingerprint(key),
            key: *key,
            offset,
            size,
            kind: AccessKind::Read,
        };
        self.register_guard(borrow)
    }

    /// Register a write borrow with RAII auto-release.
    #[inline(always)]
    pub fn register_guard_write(
        &mut self,
        key: &Address,
        offset: u32,
        size: u32,
    ) -> Result<SegmentBorrowGuard<'_>, ProgramError> {
        let borrow = SegmentBorrow {
            key_fp: address_fingerprint(key),
            key: *key,
            offset,
            size,
            kind: AccessKind::Write,
        };
        self.register_guard(borrow)
    }

    /// Visit each active borrow in registration order.
    ///
    /// Intended for diagnostics and for the `hopper explain`
    /// introspection path, never for hot-path decisions.
    #[inline]
    pub fn for_each<F: FnMut(&SegmentBorrow)>(&self, mut f: F) {
        let len = self.len as usize;
        let mut i = 0;
        while i < len {
            f(&self.entries[i]);
            i += 1;
        }
    }

    /// Look up an active borrow by exact `(key, offset, size, kind)`.
    #[inline]
    pub fn find_exact(
        &self,
        key: &Address,
        offset: u32,
        size: u32,
        kind: AccessKind,
    ) -> Option<&SegmentBorrow> {
        let fp = address_fingerprint(key);
        let len = self.len as usize;
        let mut i = 0;
        while i < len {
            let e = &self.entries[i];
            if e.key_fp == fp
                && address_eq(&e.key, key)
                && e.offset == offset
                && e.size == size
                && e.kind as u8 == kind as u8
            {
                return Some(e);
            }
            i += 1;
        }
        None
    }
}

/// RAII guard that releases a segment borrow when dropped.
///
/// Created by [`SegmentBorrowRegistry::register_guard()`] and its
/// convenience wrappers. The borrow is automatically released from the
/// registry on drop, preventing borrow leaks.
pub struct SegmentBorrowGuard<'a> {
    registry: &'a mut SegmentBorrowRegistry,
    borrow: SegmentBorrow,
}

impl<'a> SegmentBorrowGuard<'a> {
    /// Access kind of the guarded borrow.
    #[inline(always)]
    pub fn kind(&self) -> AccessKind {
        self.borrow.kind
    }

    /// Byte offset of the guarded segment.
    #[inline(always)]
    pub fn offset(&self) -> u32 {
        self.borrow.offset
    }

    /// Byte size of the guarded segment.
    #[inline(always)]
    pub fn size(&self) -> u32 {
        self.borrow.size
    }
}

impl<'a> Drop for SegmentBorrowGuard<'a> {
    fn drop(&mut self) {
        self.registry.release(&self.borrow);
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
        assert!(reg.register_read(&key, 0, 8).is_ok());
        assert!(reg.register_read(&key, 0, 8).is_ok());
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn read_write_same_range_rejected() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        assert!(reg.register_read(&key, 0, 8).is_ok());
        assert!(reg.register_write(&key, 0, 8).is_err());
    }

    #[test]
    fn write_write_same_range_rejected() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        assert!(reg.register_write(&key, 0, 8).is_ok());
        assert!(reg.register_write(&key, 0, 8).is_err());
    }

    #[test]
    fn write_read_same_range_rejected() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        assert!(reg.register_write(&key, 0, 8).is_ok());
        assert!(reg.register_read(&key, 0, 8).is_err());
    }

    #[test]
    fn non_overlapping_write_write_allowed() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        // balance: [0..8), metadata: [8..40)
        assert!(reg.register_write(&key, 0, 8).is_ok());
        assert!(reg.register_write(&key, 8, 32).is_ok());
    }

    #[test]
    fn partially_overlapping_rejected() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        // [0..16) and [8..24) overlap at [8..16)
        assert!(reg.register_write(&key, 0, 16).is_ok());
        assert!(reg.register_write(&key, 8, 16).is_err());
    }

    #[test]
    fn different_accounts_always_allowed() {
        let mut reg = SegmentBorrowRegistry::new();
        assert!(reg.register_write(&test_addr(1), 0, 8).is_ok());
        assert!(reg.register_write(&test_addr(2), 0, 8).is_ok());
    }

    #[test]
    fn release_then_reacquire() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        let borrow = SegmentBorrow {
            key_fp: address_fingerprint(&key),
            key,
            offset: 0,
            size: 8,
            kind: AccessKind::Write,
        };
        assert!(reg.register(borrow).is_ok());
        assert!(reg.register_write(&key, 0, 8).is_err()); // conflict
        assert!(reg.release(&borrow));
        assert!(reg.register_write(&key, 0, 8).is_ok()); // now OK
    }

    #[test]
    fn capacity_limit() {
        let mut reg = SegmentBorrowRegistry::new();
        for i in 0..MAX_SEGMENT_BORROWS {
            assert!(reg.register_read(&test_addr(1), i as u32 * 8, 8).is_ok());
        }
        // One more should fail.
        assert!(reg.register_read(&test_addr(1), 256, 8).is_err());
    }

    #[test]
    fn would_conflict_does_not_mutate() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        assert!(reg.register_write(&key, 0, 8).is_ok());
        let proposed = SegmentBorrow {
            key_fp: address_fingerprint(&key),
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
        assert!(reg.register_write(&key, 0, 8).is_ok());
        assert!(reg.register_write(&key, 8, 8).is_ok());
    }

    // ── SegmentBorrowGuard RAII tests ────────────────────────────────
    //
    // The guard holds `&mut SegmentBorrowRegistry`, which provides
    // compile-time exclusion: the borrow checker prevents any registry
    // access while a guard is alive, giving *stronger* protection than
    // runtime conflict checks alone.  Tests verify the auto-release
    // behavior by inspecting the registry after the guard drops.

    #[test]
    fn guard_auto_releases_write_on_drop() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        {
            let _guard = reg.register_guard_write(&key, 0, 8).unwrap();
            // guard alive, registry exclusively borrowed at compile time
        }
        // After drop: slot freed, len back to 0.
        assert_eq!(reg.len(), 0);
        // Re-acquire the same range, proves release happened.
        assert!(reg.register_write(&key, 0, 8).is_ok());
    }

    #[test]
    fn guard_auto_releases_read_on_drop() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        {
            let _guard = reg.register_guard_read(&key, 0, 8).unwrap();
        }
        assert_eq!(reg.len(), 0);
        // Write now succeeds, the read borrow was released.
        assert!(reg.register_write(&key, 0, 8).is_ok());
    }

    #[test]
    fn sequential_guards_reuse_slot() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        for _ in 0..4 {
            let _guard = reg.register_guard_write(&key, 0, 8).unwrap();
            // each iteration: acquire, drop at end of loop body
        }
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn guard_accessors() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        let guard = reg.register_guard_write(&key, 16, 32).unwrap();
        assert_eq!(guard.kind(), AccessKind::Write);
        assert_eq!(guard.offset(), 16);
        assert_eq!(guard.size(), 32);
    }

    #[test]
    fn guard_then_manual_register_ok() {
        let mut reg = SegmentBorrowRegistry::new();
        let key = test_addr(1);
        {
            let _guard = reg.register_guard_write(&key, 0, 8).unwrap();
        }
        // Guard released, manual register on overlapping range works.
        assert!(reg.register_read(&key, 0, 8).is_ok());
        assert_eq!(reg.len(), 1);
    }
}
