//! Append-only journal for on-chain audit trails.
//!
//! A `Journal` is a bounded, append-only log of fixed-size entries.
//! Once full, it either rejects new entries (strict mode) or wraps
//! around like a ring buffer (circular mode).
//!
//! ## Wire Format
//!
//! ```text
//! [write_head: u32 LE]     -- index of next write position
//! [total_written: u32 LE]  -- total entries ever written (for wrap detection)
//! [flags: u32 LE]          -- bit 0: circular mode
//! [_reserved: u32 LE]
//! [entry 0: T bytes]
//! [entry 1: T bytes]
//! ...
//! [entry capacity-1: T bytes]
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! #[repr(C)]
//! #[derive(Clone, Copy)]
//! struct AuditEntry {
//!     actor: [u8; 32],
//!     action: u8,
//!     timestamp: WireU64,
//! }
//!
//! let mut journal = Journal::<AuditEntry>::from_bytes_mut(data)?;
//! journal.append(AuditEntry { ... })?;
//!
//! // Read latest entries
//! let latest = journal.latest()?;
//! ```

use hopper_runtime::error::ProgramError;
use crate::account::{Pod, FixedLayout};

/// Journal header size in bytes.
pub const JOURNAL_HEADER_SIZE: usize = 16;

/// Flag: circular mode (wrap around when full).
pub const JOURNAL_FLAG_CIRCULAR: u32 = 1 << 0;

/// An append-only journal of fixed-size entries.
pub struct Journal<'a, T: Pod + FixedLayout> {
    data: &'a mut [u8],
    capacity: usize,
    _phantom: core::marker::PhantomData<T>,
}

impl<'a, T: Pod + FixedLayout> Journal<'a, T> {
    /// Parse a journal from a mutable byte slice.
    #[inline]
    pub fn from_bytes_mut(data: &'a mut [u8]) -> Result<Self, ProgramError> {
        if data.len() < JOURNAL_HEADER_SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let usable = data.len() - JOURNAL_HEADER_SIZE;
        if T::SIZE == 0 {
            return Err(ProgramError::InvalidArgument);
        }
        let capacity = usable / T::SIZE;
        Ok(Self { data, capacity, _phantom: core::marker::PhantomData })
    }

    /// Create a read-only journal view.
    #[inline]
    pub fn from_bytes(data: &[u8]) -> Result<JournalReader<'_, T>, ProgramError> {
        if data.len() < JOURNAL_HEADER_SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let usable = data.len() - JOURNAL_HEADER_SIZE;
        if T::SIZE == 0 {
            return Err(ProgramError::InvalidArgument);
        }
        let capacity = usable / T::SIZE;
        Ok(JournalReader { data, capacity, _phantom: core::marker::PhantomData })
    }

    /// Maximum number of entries.
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Current write head position.
    #[inline(always)]
    pub fn write_head(&self) -> u32 {
        u32::from_le_bytes([self.data[0], self.data[1], self.data[2], self.data[3]])
    }

    /// Total entries ever written.
    #[inline(always)]
    pub fn total_written(&self) -> u32 {
        u32::from_le_bytes([self.data[4], self.data[5], self.data[6], self.data[7]])
    }

    /// Journal flags.
    #[inline(always)]
    pub fn flags(&self) -> u32 {
        u32::from_le_bytes([self.data[8], self.data[9], self.data[10], self.data[11]])
    }

    /// Whether circular mode is enabled.
    #[inline(always)]
    pub fn is_circular(&self) -> bool {
        self.flags() & JOURNAL_FLAG_CIRCULAR != 0
    }

    /// Whether the journal has wrapped at least once (total_written > capacity).
    #[inline(always)]
    pub fn has_wrapped(&self) -> bool {
        (self.total_written() as usize) > self.capacity
    }

    /// Number of valid entries (min of total_written and capacity).
    #[inline(always)]
    pub fn entry_count(&self) -> usize {
        let total = self.total_written() as usize;
        if total < self.capacity { total } else { self.capacity }
    }

    /// Append an entry to the journal.
    #[inline]
    pub fn append(&mut self, entry: T) -> Result<(), ProgramError> {
        let mut head = self.write_head() as usize;

        if head >= self.capacity {
            if !self.is_circular() {
                return Err(ProgramError::AccountDataTooSmall);
            }
            // Normalize: wrap head back into range
            head %= self.capacity;
        }

        if !self.is_circular() && head >= self.capacity {
            return Err(ProgramError::AccountDataTooSmall);
        }

        // Write entry at head
        let offset = JOURNAL_HEADER_SIZE + head * T::SIZE;
        let end = offset + T::SIZE;
        if end > self.data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }

        // SAFETY: T: Pod, bounds checked, alignment-1.
        unsafe {
            core::ptr::copy_nonoverlapping(
                &entry as *const T as *const u8,
                self.data.as_mut_ptr().add(offset),
                T::SIZE,
            );
        }

        // Advance head
        let new_head = if self.is_circular() {
            ((head + 1) % self.capacity) as u32
        } else {
            (head + 1) as u32
        };
        self.set_write_head(new_head);

        // Increment total written
        let total = self.total_written().wrapping_add(1);
        self.set_total_written(total);

        Ok(())
    }

    /// Read entry at logical index (0 = oldest visible entry).
    #[inline]
    pub fn read(&self, index: usize) -> Result<T, ProgramError> {
        let count = self.entry_count();
        if index >= count {
            return Err(ProgramError::InvalidArgument);
        }

        let physical = if self.total_written() as usize > self.capacity {
            // Wrapped: oldest is at write_head
            (self.write_head() as usize + index) % self.capacity
        } else {
            index
        };

        let offset = JOURNAL_HEADER_SIZE + physical * T::SIZE;
        let end = offset + T::SIZE;
        if end > self.data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }

        // SAFETY: Bounds checked. T: Pod, alignment-1.
        Ok(unsafe { core::ptr::read_unaligned(self.data.as_ptr().add(offset) as *const T) })
    }

    /// Read the most recent entry.
    #[inline]
    pub fn latest(&self) -> Result<T, ProgramError> {
        let count = self.entry_count();
        if count == 0 {
            return Err(ProgramError::InvalidArgument);
        }
        self.read(count - 1)
    }

    /// Bytes required for a journal with the given capacity.
    #[inline(always)]
    pub const fn required_bytes(capacity: usize) -> usize {
        JOURNAL_HEADER_SIZE + capacity * T::SIZE
    }

    /// Initialize the journal header (circular or strict).
    #[inline]
    pub fn init(&mut self, circular: bool) {
        self.set_write_head(0);
        self.set_total_written(0);
        let flags: u32 = if circular { JOURNAL_FLAG_CIRCULAR } else { 0 };
        self.data[8..12].copy_from_slice(&flags.to_le_bytes());
        self.data[12..16].copy_from_slice(&0u32.to_le_bytes());
    }

    #[inline(always)]
    fn set_write_head(&mut self, head: u32) {
        self.data[0..4].copy_from_slice(&head.to_le_bytes());
    }

    #[inline(always)]
    fn set_total_written(&mut self, total: u32) {
        self.data[4..8].copy_from_slice(&total.to_le_bytes());
    }
}

/// Read-only journal view.
pub struct JournalReader<'a, T: Pod + FixedLayout> {
    data: &'a [u8],
    capacity: usize,
    _phantom: core::marker::PhantomData<T>,
}

impl<'a, T: Pod + FixedLayout> JournalReader<'a, T> {
    /// Capacity.
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Write head.
    #[inline(always)]
    pub fn write_head(&self) -> u32 {
        u32::from_le_bytes([self.data[0], self.data[1], self.data[2], self.data[3]])
    }

    /// Total written.
    #[inline(always)]
    pub fn total_written(&self) -> u32 {
        u32::from_le_bytes([self.data[4], self.data[5], self.data[6], self.data[7]])
    }

    /// Number of valid entries.
    #[inline(always)]
    pub fn entry_count(&self) -> usize {
        let total = self.total_written() as usize;
        if total < self.capacity { total } else { self.capacity }
    }

    /// Is circular.
    #[inline(always)]
    pub fn is_circular(&self) -> bool {
        let flags = u32::from_le_bytes([self.data[8], self.data[9], self.data[10], self.data[11]]);
        flags & JOURNAL_FLAG_CIRCULAR != 0
    }

    /// Read entry at logical index.
    #[inline]
    pub fn read(&self, index: usize) -> Result<T, ProgramError> {
        let count = self.entry_count();
        if index >= count {
            return Err(ProgramError::InvalidArgument);
        }

        let physical = if self.total_written() as usize > self.capacity {
            (self.write_head() as usize + index) % self.capacity
        } else {
            index
        };

        let offset = JOURNAL_HEADER_SIZE + physical * T::SIZE;
        let end = offset + T::SIZE;
        if end > self.data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }

        Ok(unsafe { core::ptr::read_unaligned(self.data.as_ptr().add(offset) as *const T) })
    }

    /// Read the most recent entry.
    #[inline]
    pub fn latest(&self) -> Result<T, ProgramError> {
        let count = self.entry_count();
        if count == 0 {
            return Err(ProgramError::InvalidArgument);
        }
        self.read(count - 1)
    }
}
