//! State Diff Engine: field-level change tracking.
//!
//! Captures before/after snapshots of account data and computes diffs.
//! Use cases:
//! - Audit trails
//! - Test assertions
//! - Post-mutation invariant verification
//! - Debugging state transitions
//!
//! ## Usage
//!
//! ```ignore
//! // Capture before state
//! let snap = StateSnapshot::<256>::capture(account_data);
//!
//! // ... mutations happen ...
//!
//! // Compute diff
//! let diff = snap.diff(account_data);
//! if diff.has_changes() {
//!     let regions = diff.changed_regions::<8>();
//!     let mut i = 0;
//!     while i < regions.len() {
//!         if let Some(r) = regions.get(i) {
//!             // r.offset, r.length
//!         }
//!         i += 1;
//!     }
//! }
//! ```
//!
//! ## Design
//!
//! Snapshots are stack-allocated using const generics. The maximum snapshot
//! size is a compile-time parameter. For accounts larger than the snapshot
//! buffer, only the first N bytes are captured. Use `was_truncated()` to
//! detect this.

use hopper_runtime::error::ProgramError;

// -- State Snapshot --

/// A stack-allocated snapshot of account data.
///
/// `SIZE` is the maximum number of bytes captured.
pub struct StateSnapshot<const SIZE: usize> {
    data: [u8; SIZE],
    len: usize,
    /// True if the source data was longer than SIZE (truncated capture).
    truncated: bool,
}

impl<const SIZE: usize> StateSnapshot<SIZE> {
    /// Capture a snapshot of account data.
    ///
    /// If the data is longer than SIZE, only the first SIZE bytes are captured
    /// and `was_truncated()` returns true.
    #[inline]
    pub fn capture(data: &[u8]) -> Self {
        let truncated = data.len() > SIZE;
        let len = if truncated { SIZE } else { data.len() };
        let mut snapshot = Self {
            data: [0u8; SIZE],
            len,
            truncated,
        };
        let mut i = 0;
        while i < len {
            snapshot.data[i] = data[i];
            i += 1;
        }
        snapshot
    }

    /// Length of captured data.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether no data was captured.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether the source data was larger than the snapshot buffer.
    #[inline(always)]
    pub fn was_truncated(&self) -> bool {
        self.truncated
    }

    /// Get the captured data.
    #[inline(always)]
    pub fn data(&self) -> &[u8] {
        &self.data[..self.len]
    }

    /// Compute a diff against current data.
    ///
    /// Returns a `StateDiff` describing all changed regions.
    #[inline]
    pub fn diff<'a>(&'a self, current: &'a [u8]) -> StateDiff<'a> {
        let compare_len = if current.len() < self.len {
            current.len()
        } else {
            self.len
        };

        StateDiff {
            old: &self.data[..compare_len],
            new: &current[..compare_len],
            old_full_len: self.len,
            new_full_len: current.len(),
            // Carry the truncation flag forward: without it, every
            // downstream query silently operates on only the first
            // `SIZE` bytes and can miss a real mutation past the window.
            truncated: self.truncated,
        }
    }

    /// Check if any bytes changed compared to current data.
    #[inline]
    pub fn has_changes(&self, current: &[u8]) -> bool {
        if current.len() != self.len {
            return true;
        }
        let mut i = 0;
        while i < self.len {
            if self.data[i] != current[i] {
                return true;
            }
            i += 1;
        }
        false
    }

    /// Check if a specific byte range changed.
    #[inline]
    pub fn range_changed(&self, current: &[u8], offset: usize, len: usize) -> bool {
        // Checked: an `offset + len` that wraps would produce a small
        // `end` that slips past the bound below and then mis-indexes.
        // A wrapped/overrunning range is treated as changed, matching
        // the out-of-bounds arm.
        let end = match offset.checked_add(len) {
            Some(e) => e,
            None => return true,
        };
        if end > self.len || end > current.len() {
            return true; // Range exceeds bounds -- consider it changed
        }
        let mut i = offset;
        while i < end {
            if self.data[i] != current[i] {
                return true;
            }
            i += 1;
        }
        false
    }

    /// Restore the snapshot data back into a mutable slice (rollback).
    ///
    /// **Refuses a truncated snapshot.** A snapshot captured from an
    /// account larger than `SIZE` holds only the first `SIZE` bytes, so
    /// restoring it would rewrite the head and silently leave the
    /// mutated tail in place — a partial rollback masquerading as a full
    /// one. That is a correctness/security trap for the rollback use
    /// case, so a truncated snapshot returns `InvalidAccountData`
    /// instead. Use a `StateSnapshot<SIZE>` whose `SIZE` covers the whole
    /// account, or [`restore_head_into`](Self::restore_head_into) when a
    /// deliberate head-only restore is genuinely intended.
    #[inline]
    pub fn restore_into(&self, target: &mut [u8]) -> Result<(), ProgramError> {
        if self.truncated {
            return Err(ProgramError::InvalidAccountData);
        }
        self.restore_head_into(target)
    }

    /// Restore the captured (possibly truncated) prefix into `target`.
    ///
    /// Explicit head-only restore: copies exactly the `len()` captured
    /// bytes and makes no claim about anything past them. Callers that
    /// use this on a truncated snapshot are opting into a partial
    /// rollback with eyes open (see [`was_truncated`](Self::was_truncated)).
    #[inline]
    pub fn restore_head_into(&self, target: &mut [u8]) -> Result<(), ProgramError> {
        if target.len() < self.len {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let mut i = 0;
        while i < self.len {
            target[i] = self.data[i];
            i += 1;
        }
        Ok(())
    }
}

// -- State Diff --

/// A diff between two states of account data.
pub struct StateDiff<'a> {
    old: &'a [u8],
    new: &'a [u8],
    old_full_len: usize,
    new_full_len: usize,
    /// Whether the source snapshot was truncated (account larger than
    /// the snapshot buffer). When true, this diff only reflects the
    /// captured prefix — see [`is_complete`](StateDiff::is_complete).
    truncated: bool,
}

impl<'a> StateDiff<'a> {
    /// Whether this diff observed the entire account.
    ///
    /// `false` means the before-snapshot was truncated to its buffer
    /// size, so any mutation confined to bytes past the window is
    /// invisible here. **Audit-trail and invariant consumers must treat
    /// an incomplete diff as inconclusive** — a `has_changes() == false`
    /// on an incomplete diff does not prove the account was unchanged.
    #[inline(always)]
    pub fn is_complete(&self) -> bool {
        !self.truncated
    }

    /// Whether the source snapshot was truncated (inverse of
    /// [`is_complete`](Self::is_complete)).
    #[inline(always)]
    pub fn was_truncated(&self) -> bool {
        self.truncated
    }

    /// Whether the data changed at all.
    ///
    /// Conservative under truncation: if the snapshot could not see the
    /// whole account, this returns `true` rather than falsely reporting
    /// "unchanged" for a tail mutation it cannot observe. Use
    /// [`is_complete`](Self::is_complete) to distinguish a proven change
    /// from an unprovable one.
    #[inline]
    pub fn has_changes(&self) -> bool {
        let mut i = 0;
        while i < self.old.len() {
            if self.old[i] != self.new[i] {
                return true;
            }
            i += 1;
        }
        // Resized, or truncated (can't rule out a change past the window).
        self.old_full_len != self.new_full_len || self.truncated
    }

    /// Whether the account was resized.
    #[inline(always)]
    pub fn was_resized(&self) -> bool {
        self.old_full_len != self.new_full_len
    }

    /// Old data length.
    #[inline(always)]
    pub fn old_len(&self) -> usize {
        self.old_full_len
    }

    /// New data length.
    #[inline(always)]
    pub fn new_len(&self) -> usize {
        self.new_full_len
    }

    /// Check if a specific field (by offset and size) changed.
    #[inline]
    pub fn field_changed(&self, offset: usize, size: usize) -> bool {
        // Checked (see `StateSnapshot::range_changed`): a wrapped range
        // is treated as changed rather than mis-indexed.
        let end = match offset.checked_add(size) {
            Some(e) => e,
            None => return true,
        };
        if end > self.old.len() || end > self.new.len() {
            return true;
        }
        let mut i = offset;
        while i < end {
            if self.old[i] != self.new[i] {
                return true;
            }
            i += 1;
        }
        false
    }

    /// Count the number of bytes that changed.
    #[inline]
    pub fn changed_byte_count(&self) -> usize {
        let compare_len = if self.old.len() < self.new.len() {
            self.old.len()
        } else {
            self.new.len()
        };
        let mut count = 0;
        let mut i = 0;
        while i < compare_len {
            if self.old[i] != self.new[i] {
                count += 1;
            }
            i += 1;
        }
        // Bytes beyond the shorter slice are all "changed"
        if self.old_full_len > self.new_full_len {
            count += self.old_full_len - self.new_full_len;
        } else {
            count += self.new_full_len - self.old_full_len;
        }
        count
    }

    /// Iterate over changed regions (runs of consecutive changed bytes).
    ///
    /// Returns up to `MAX_REGIONS` contiguous changed regions.
    #[inline]
    pub fn changed_regions<const MAX_REGIONS: usize>(&self) -> ChangedRegions<MAX_REGIONS> {
        let compare_len = if self.old.len() < self.new.len() {
            self.old.len()
        } else {
            self.new.len()
        };

        let mut regions = ChangedRegions {
            entries: [ChangedRegion {
                offset: 0,
                length: 0,
            }; MAX_REGIONS],
            count: 0,
        };

        let mut i = 0;
        while i < compare_len && regions.count < MAX_REGIONS {
            if self.old[i] != self.new[i] {
                let start = i;
                while i < compare_len && self.old[i] != self.new[i] {
                    i += 1;
                }
                regions.entries[regions.count] = ChangedRegion {
                    offset: start,
                    length: i - start,
                };
                regions.count += 1;
            } else {
                i += 1;
            }
        }

        regions
    }
}

// -- Changed Region --

/// A contiguous region of changed bytes.
#[derive(Clone, Copy)]
pub struct ChangedRegion {
    /// Byte offset from the start of the data.
    pub offset: usize,
    /// Number of consecutive changed bytes.
    pub length: usize,
}

/// Stack-allocated list of changed regions.
pub struct ChangedRegions<const N: usize> {
    entries: [ChangedRegion; N],
    count: usize,
}

impl<const N: usize> ChangedRegions<N> {
    /// Number of changed regions.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether there are no changes.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Get a changed region by index.
    #[inline(always)]
    pub fn get(&self, index: usize) -> Option<&ChangedRegion> {
        if index < self.count {
            Some(&self.entries[index])
        } else {
            None
        }
    }

    /// Iterate over changed regions.
    #[inline]
    pub fn iter(&self) -> ChangedRegionIter<'_> {
        ChangedRegionIter {
            entries: &self.entries[..self.count],
            pos: 0,
        }
    }
}

/// Iterator over changed regions.
pub struct ChangedRegionIter<'a> {
    entries: &'a [ChangedRegion],
    pos: usize,
}

impl<'a> Iterator for ChangedRegionIter<'a> {
    type Item = &'a ChangedRegion;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.entries.len() {
            return None;
        }
        let item = &self.entries[self.pos];
        self.pos += 1;
        Some(item)
    }
}

// -- Field-Level Diff Helper --

/// Build a field-level diff report for a known layout.
///
/// `fields` is an array of `(name, offset, size)`.
/// Returns a bitmask where bit N is set if field N changed.
#[inline]
pub fn field_diff_mask(old: &[u8], new: &[u8], fields: &[(&str, usize, usize)]) -> u64 {
    let mut mask: u64 = 0;
    let mut i = 0;
    while i < fields.len() && i < 64 {
        let (_, offset, size) = fields[i];
        // Checked: a wrapped `offset + size` must not read as an
        // in-bounds field; treat it as changed (the out-of-bounds arm).
        let Some(end) = offset.checked_add(size) else {
            mask |= 1u64 << i;
            i += 1;
            continue;
        };
        if end <= old.len() && end <= new.len() {
            let mut j = offset;
            while j < end {
                if old[j] != new[j] {
                    mask |= 1u64 << i;
                    break;
                }
                j += 1;
            }
        } else {
            mask |= 1u64 << i; // Out of bounds = changed
        }
        i += 1;
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_diff_detects_tail_change() {
        // Snapshot buffer covers the whole 8-byte account.
        let original = [1u8; 8];
        let snap = StateSnapshot::<8>::capture(&original);
        let mut modified = original;
        modified[7] = 9; // tail byte
        let diff = snap.diff(&modified);
        assert!(diff.is_complete());
        assert!(diff.has_changes());
        assert_eq!(diff.changed_byte_count(), 1);
    }

    #[test]
    fn truncated_diff_is_conservative_not_silently_clean() {
        // Account is larger than the snapshot buffer: only the first 4
        // bytes are captured, a change beyond byte 4 is unobservable.
        let original = [1u8; 8];
        let snap = StateSnapshot::<4>::capture(&original);
        assert!(snap.was_truncated());
        // Mutate ONLY the tail (bytes 4..8), which the snapshot can't see.
        let mut modified = original;
        modified[6] = 9;
        let diff = snap.diff(&modified);
        // Pre-fix, has_changes() returned false here (silent miss). Now
        // the diff reports incomplete and refuses to claim "unchanged".
        assert!(!diff.is_complete());
        assert!(diff.was_truncated());
        assert!(diff.has_changes());
    }

    #[test]
    fn restore_into_refuses_truncated_but_head_variant_allows_it() {
        let original = [7u8; 8];
        let snap = StateSnapshot::<4>::capture(&original);
        let mut target = [0u8; 8];
        // Full restore is refused: it could not faithfully roll back the tail.
        assert!(snap.restore_into(&mut target).is_err());
        assert_eq!(target, [0u8; 8]); // untouched
        // Explicit head-only restore is allowed and copies the 4 captured bytes.
        snap.restore_head_into(&mut target).unwrap();
        assert_eq!(&target[..4], &[7u8; 4]);
        assert_eq!(&target[4..], &[0u8; 4]);
    }

    #[test]
    fn non_truncated_restore_roundtrips() {
        let original = [3u8, 1, 4, 1, 5, 9, 2, 6];
        let snap = StateSnapshot::<8>::capture(&original);
        let mut target = [0u8; 8];
        snap.restore_into(&mut target).unwrap();
        assert_eq!(target, original);
    }
}
