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
        let end = offset + len;
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

    /// Restore the snapshot data back into a mutable slice.
    ///
    /// Useful for rollback scenarios.
    #[inline]
    pub fn restore_into(&self, target: &mut [u8]) -> Result<(), ProgramError> {
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
}

impl<'a> StateDiff<'a> {
    /// Whether the data changed at all.
    #[inline]
    pub fn has_changes(&self) -> bool {
        if self.old.len() != self.new.len() {
            return true;
        }
        let mut i = 0;
        while i < self.old.len() {
            if self.old[i] != self.new[i] {
                return true;
            }
            i += 1;
        }
        self.old_full_len != self.new_full_len
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
        let end = offset + size;
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
            entries: [ChangedRegion { offset: 0, length: 0 }; MAX_REGIONS],
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
pub fn field_diff_mask(
    old: &[u8],
    new: &[u8],
    fields: &[(&str, usize, usize)],
) -> u64 {
    let mut mask: u64 = 0;
    let mut i = 0;
    while i < fields.len() && i < 64 {
        let (_, offset, size) = fields[i];
        let end = offset + size;
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
