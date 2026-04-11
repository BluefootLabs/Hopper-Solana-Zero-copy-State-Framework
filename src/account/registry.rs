//! Segmented Accounts for Hopper.
//!
//! This module provides first-class segment-based account architecture:
//!
//! - Typed segment registry with compile-time segment IDs and runtime lookup
//! - Segment introspection: iterate, inspect, and decode segments
//! - Per-segment access control via freeze/lock flags
//!
//! ## Wire Format
//!
//! ```text
//! [AccountHeader: 16 bytes]
//! [SegmentRegistry Header: 4 bytes]
//!   segment_count: u16
//!   registry_flags: u16
//! [SegmentEntry 0: 16 bytes]
//!   segment_id:   [u8; 4]   -- compile-time FNV hash of segment name
//!   offset:       u32       -- byte offset from account start
//!   size:         u32       -- total allocated bytes
//!   flags:        u16       -- locked, frozen, etc.
//!   version:      u8        -- segment schema version
//!   _reserved:    u8
//! [SegmentEntry 1: 16 bytes]
//! ...
//! [Segment 0 data...]
//! [Segment 1 data...]
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! // Define segment IDs at compile time
//! const CORE_SEG: SegmentId = segment_id("core");
//! const PERMS_SEG: SegmentId = segment_id("permissions");
//!
//! // Read a segment
//! let registry = SegmentRegistry::from_account(data)?;
//! let core_data = registry.segment_data(data, CORE_SEG)?;
//! let core = TreasuryCore::overlay(core_data)?;
//! ```

use hopper_runtime::error::ProgramError;
use super::segment_role::SegmentRole;
use crate::collections::{FixedVec, Journal, RingBuffer, Slab, SlotMap, SortedVec};

// -- Segment ID --

/// A 4-byte segment identifier, computed from a name at compile time.
pub type SegmentId = [u8; 4];

/// Compute a segment ID from a name (const FNV-1a hash, truncated to 4 bytes).
#[inline(always)]
pub const fn segment_id(name: &str) -> SegmentId {
    let bytes = name.as_bytes();
    let mut hash: u32 = 0x811c_9dc5; // FNV offset basis
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u32;
        hash = hash.wrapping_mul(0x0100_0193); // FNV prime
        i += 1;
    }
    hash.to_le_bytes()
}

// -- Segment Entry --

/// Size of one segment entry in bytes.
pub const SEGMENT_ENTRY_SIZE: usize = 16;

/// Flags for segment entries.
pub const SEG_FLAG_LOCKED: u16 = 1 << 0;   // Cannot be modified
pub const SEG_FLAG_FROZEN: u16 = 1 << 1;   // Temporarily frozen
pub const SEG_FLAG_DYNAMIC: u16 = 1 << 2;  // Supports realloc

/// A segment entry in the registry.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct SegmentEntry {
    pub id: [u8; 4],
    offset_bytes: [u8; 4],
    size_bytes: [u8; 4],
    flags_bytes: [u8; 2],
    pub version: u8,
    pub _reserved: u8,
}

const _: () = assert!(core::mem::size_of::<SegmentEntry>() == SEGMENT_ENTRY_SIZE);
const _: () = assert!(core::mem::align_of::<SegmentEntry>() == 1);

impl SegmentEntry {
    /// Create a new segment entry.
    #[inline(always)]
    pub const fn new(id: SegmentId, offset: u32, size: u32, flags: u16, version: u8) -> Self {
        Self {
            id,
            offset_bytes: offset.to_le_bytes(),
            size_bytes: size.to_le_bytes(),
            flags_bytes: flags.to_le_bytes(),
            version,
            _reserved: 0,
        }
    }

    /// Byte offset from account start.
    #[inline(always)]
    pub fn offset(&self) -> u32 {
        u32::from_le_bytes(self.offset_bytes)
    }

    /// Total allocated size in bytes.
    #[inline(always)]
    pub fn size(&self) -> u32 {
        u32::from_le_bytes(self.size_bytes)
    }

    /// Segment flags.
    #[inline(always)]
    pub fn flags(&self) -> u16 {
        u16::from_le_bytes(self.flags_bytes)
    }

    /// Whether the segment is locked (immutable).
    #[inline(always)]
    pub fn is_locked(&self) -> bool {
        self.flags() & SEG_FLAG_LOCKED != 0
    }

    /// Whether the segment is frozen.
    #[inline(always)]
    pub fn is_frozen(&self) -> bool {
        self.flags() & SEG_FLAG_FROZEN != 0
    }

    /// The semantic role of this segment (decoded from upper 4 bits of flags).
    #[inline(always)]
    pub fn role(&self) -> SegmentRole {
        SegmentRole::from_flags(self.flags())
    }

    /// Set the offset.
    #[inline(always)]
    pub fn set_offset(&mut self, offset: u32) {
        self.offset_bytes = offset.to_le_bytes();
    }

    /// Set the size.
    #[inline(always)]
    pub fn set_size(&mut self, size: u32) {
        self.size_bytes = size.to_le_bytes();
    }

    /// Set flags.
    #[inline(always)]
    pub fn set_flags(&mut self, flags: u16) {
        self.flags_bytes = flags.to_le_bytes();
    }
}

// -- Registry Header --

/// Size of the segment registry header.
pub const REGISTRY_HEADER_SIZE: usize = 4;

/// Maximum segments in a registry.
pub const MAX_REGISTRY_SEGMENTS: usize = 16;

// -- Segment Registry (read-only) --

/// Read-only view over a segmented account's registry.
///
/// The registry lives right after the 16-byte AccountHeader.
pub struct SegmentRegistry<'a> {
    data: &'a [u8],
    count: usize,
    entries_offset: usize,
}

/// Offset where the registry header starts (after AccountHeader).
pub const REGISTRY_OFFSET: usize = crate::account::HEADER_LEN;

impl<'a> SegmentRegistry<'a> {
    /// Parse a segment registry from full account data.
    ///
    /// Expects account data starting from byte 0 (including AccountHeader).
    #[inline]
    pub fn from_account(account_data: &'a [u8]) -> Result<Self, ProgramError> {
        let start = REGISTRY_OFFSET;
        if account_data.len() < start + REGISTRY_HEADER_SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }

        let count = u16::from_le_bytes([
            account_data[start],
            account_data[start + 1],
        ]) as usize;

        if count > MAX_REGISTRY_SEGMENTS {
            return Err(ProgramError::InvalidAccountData);
        }

        let entries_offset = start + REGISTRY_HEADER_SIZE;
        let needed = entries_offset + count * SEGMENT_ENTRY_SIZE;
        if account_data.len() < needed {
            return Err(ProgramError::AccountDataTooSmall);
        }

        Ok(Self {
            data: account_data,
            count,
            entries_offset,
        })
    }

    /// Number of segments in this account.
    #[inline(always)]
    pub fn segment_count(&self) -> usize {
        self.count
    }

    /// Where segment data begins (after all entries).
    #[inline(always)]
    pub fn data_region_offset(&self) -> usize {
        self.entries_offset + self.count * SEGMENT_ENTRY_SIZE
    }

    /// Get a segment entry by index.
    #[inline]
    pub fn entry(&self, index: usize) -> Result<&SegmentEntry, ProgramError> {
        if index >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let offset = self.entries_offset + index * SEGMENT_ENTRY_SIZE;
        // SAFETY: Bounds checked in from_account. SegmentEntry is align-1.
        Ok(unsafe { &*(self.data.as_ptr().add(offset) as *const SegmentEntry) })
    }

    /// Find a segment by its ID.
    #[inline]
    pub fn find(&self, id: &SegmentId) -> Result<(usize, &SegmentEntry), ProgramError> {
        let mut i = 0;
        while i < self.count {
            let offset = self.entries_offset + i * SEGMENT_ENTRY_SIZE;
            // SAFETY: Bounds validated in from_account.
            let entry = unsafe { &*(self.data.as_ptr().add(offset) as *const SegmentEntry) };
            if entry.id == *id {
                return Ok((i, entry));
            }
            i += 1;
        }
        Err(ProgramError::InvalidArgument)
    }

    /// Get the raw data slice for a segment by ID.
    #[inline]
    pub fn segment_data(&self, id: &SegmentId) -> Result<&'a [u8], ProgramError> {
        let (_, entry) = self.find(id)?;
        let start = entry.offset() as usize;
        let end = start + entry.size() as usize;
        if end > self.data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(&self.data[start..end])
    }

    /// Get a typed overlay for a segment by ID.
    #[inline]
    pub fn segment_overlay<T: super::Pod + super::FixedLayout>(
        &self,
        id: &SegmentId,
    ) -> Result<&'a T, ProgramError> {
        let data = self.segment_data(id)?;
        if data.len() < T::SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        // SAFETY: Size checked. T: Pod, alignment-1.
        Ok(unsafe { &*(data.as_ptr() as *const T) })
    }

    /// Iterate over all segment entries.
    #[inline]
    pub fn iter(&self) -> SegmentIter<'a> {
        SegmentIter {
            data: self.data,
            entries_offset: self.entries_offset,
            count: self.count,
            pos: 0,
        }
    }

    /// Total bytes consumed by the registry (header + all entries).
    #[inline(always)]
    pub fn registry_size(&self) -> usize {
        REGISTRY_HEADER_SIZE + self.count * SEGMENT_ENTRY_SIZE
    }

    /// Compute required account size for given segment specs.
    ///
    /// `specs` is `(segment_data_size,)` per segment.
    #[inline]
    pub const fn required_account_size(
        header_size: usize,
        segment_count: usize,
        total_segment_data: usize,
    ) -> usize {
        header_size + REGISTRY_HEADER_SIZE + segment_count * SEGMENT_ENTRY_SIZE + total_segment_data
    }
}

/// Iterator over segment entries.
pub struct SegmentIter<'a> {
    data: &'a [u8],
    entries_offset: usize,
    count: usize,
    pos: usize,
}

impl<'a> Iterator for SegmentIter<'a> {
    type Item = (usize, &'a SegmentEntry);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.count {
            return None;
        }
        let idx = self.pos;
        let offset = self.entries_offset + idx * SEGMENT_ENTRY_SIZE;
        // SAFETY: Bounds validated by SegmentRegistry::from_account.
        let entry = unsafe { &*(self.data.as_ptr().add(offset) as *const SegmentEntry) };
        self.pos += 1;
        Some((idx, entry))
    }
}

// -- Mutable Registry --

/// Mutable view over a segmented account's registry.
pub struct SegmentRegistryMut<'a> {
    data: &'a mut [u8],
    count: usize,
    entries_offset: usize,
}

impl<'a> SegmentRegistryMut<'a> {
    /// Parse a mutable segment registry from full account data.
    #[inline]
    pub fn from_account_mut(account_data: &'a mut [u8]) -> Result<Self, ProgramError> {
        let start = REGISTRY_OFFSET;
        if account_data.len() < start + REGISTRY_HEADER_SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }

        let count = u16::from_le_bytes([
            account_data[start],
            account_data[start + 1],
        ]) as usize;

        if count > MAX_REGISTRY_SEGMENTS {
            return Err(ProgramError::InvalidAccountData);
        }

        let entries_offset = start + REGISTRY_HEADER_SIZE;
        let needed = entries_offset + count * SEGMENT_ENTRY_SIZE;
        if account_data.len() < needed {
            return Err(ProgramError::AccountDataTooSmall);
        }

        Ok(Self {
            data: account_data,
            count,
            entries_offset,
        })
    }

    /// Number of segments.
    #[inline(always)]
    pub fn segment_count(&self) -> usize {
        self.count
    }

    /// Initialize the registry with segment specifications.
    ///
    /// `specs` is `(segment_id, data_size, version)` per segment.
    /// Must be called on a freshly zeroed account.
    #[inline]
    pub fn init(
        data: &mut [u8],
        specs: &[(SegmentId, u32, u8)],
    ) -> Result<(), ProgramError> {
        let start = REGISTRY_OFFSET;
        if specs.len() > MAX_REGISTRY_SEGMENTS {
            return Err(ProgramError::InvalidArgument);
        }

        // Reject duplicate segment IDs. Max 16 segments so O(n^2) is fine.
        let n = specs.len();
        let mut i = 0;
        while i < n {
            let mut j = i + 1;
            while j < n {
                if specs[i].0 == specs[j].0 {
                    return Err(ProgramError::InvalidArgument);
                }
                j += 1;
            }
            i += 1;
        }

        let count = specs.len();
        let entries_offset = start + REGISTRY_HEADER_SIZE;
        let data_region = entries_offset + count * SEGMENT_ENTRY_SIZE;

        // Write registry header: segment_count (u16 LE) + flags (u16 LE)
        data[start] = (count & 0xFF) as u8;
        data[start + 1] = ((count >> 8) & 0xFF) as u8;
        data[start + 2] = 0; // flags low
        data[start + 3] = 0; // flags high

        // Write entries and compute offsets
        let mut current_offset = data_region as u32;
        for (i, &(id, size, version)) in specs.iter().enumerate() {
            let entry = SegmentEntry::new(id, current_offset, size, 0, version);
            let entry_offset = entries_offset + i * SEGMENT_ENTRY_SIZE;
            // SAFETY: entry_offset + 16 is within bounds (verified above).
            let dst = &mut data[entry_offset..entry_offset + SEGMENT_ENTRY_SIZE];
            // SAFETY: SegmentEntry is repr(C), alignment-1, 16 bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    &entry as *const SegmentEntry as *const u8,
                    dst.as_mut_ptr(),
                    SEGMENT_ENTRY_SIZE,
                );
            }
            current_offset += size;
        }

        Ok(())
    }

    /// Get a mutable entry by index.
    #[inline]
    pub fn entry_mut(&mut self, index: usize) -> Result<&mut SegmentEntry, ProgramError> {
        if index >= self.count {
            return Err(ProgramError::InvalidArgument);
        }
        let offset = self.entries_offset + index * SEGMENT_ENTRY_SIZE;
        // SAFETY: Bounds checked. SegmentEntry is align-1. Exclusive access.
        Ok(unsafe { &mut *(self.data.as_mut_ptr().add(offset) as *mut SegmentEntry) })
    }

    /// Find a segment by ID and return mutable entry.
    #[inline]
    pub fn find_mut(&mut self, id: &SegmentId) -> Result<(usize, &mut SegmentEntry), ProgramError> {
        let mut i = 0;
        while i < self.count {
            let offset = self.entries_offset + i * SEGMENT_ENTRY_SIZE;
            // SAFETY: Bounds validated in from_account_mut.
            let entry = unsafe { &mut *(self.data.as_mut_ptr().add(offset) as *mut SegmentEntry) };
            if entry.id == *id {
                return Ok((i, entry));
            }
            i += 1;
        }
        Err(ProgramError::InvalidArgument)
    }

    /// Get the mutable data slice for a segment by ID.
    ///
    /// Returns an error if the segment is locked, frozen, or has an
    /// immutable role (`Audit`). Use [`segment_data_mut_unchecked`] to
    /// bypass role enforcement (e.g., during initial account setup).
    #[inline]
    pub fn segment_data_mut(&mut self, id: &SegmentId) -> Result<&mut [u8], ProgramError> {
        let (_, entry) = self.find_mut(id)?;
        if entry.is_locked() || entry.is_frozen() {
            return Err(ProgramError::InvalidAccountData);
        }
        // Role-based write guard: Audit segments are immutable after init.
        if entry.role().is_immutable_after_init() {
            return Err(ProgramError::InvalidAccountData);
        }
        let start = entry.offset() as usize;
        let size = entry.size() as usize;
        let end = start + size;
        if end > self.data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(&mut self.data[start..end])
    }

    /// Get the mutable data slice for a segment without role enforcement.
    ///
    /// Use during account initialization when writing to Audit segments
    /// before they become immutable. Still checks locked/frozen flags.
    #[inline]
    pub fn segment_data_mut_unchecked(&mut self, id: &SegmentId) -> Result<&mut [u8], ProgramError> {
        let (_, entry) = self.find_mut(id)?;
        if entry.is_locked() || entry.is_frozen() {
            return Err(ProgramError::InvalidAccountData);
        }
        let start = entry.offset() as usize;
        let size = entry.size() as usize;
        let end = start + size;
        if end > self.data.len() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(&mut self.data[start..end])
    }

    /// Get a mutable typed overlay for a segment.
    ///
    /// Returns an error if the segment is locked or frozen.
    #[inline]
    pub fn segment_overlay_mut<T: super::Pod + super::FixedLayout>(
        &mut self,
        id: &SegmentId,
    ) -> Result<&mut T, ProgramError> {
        let data = self.segment_data_mut(id)?;
        if data.len() < T::SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        // SAFETY: Size checked. T: Pod, alignment-1. Exclusive access.
        Ok(unsafe { &mut *(data.as_mut_ptr() as *mut T) })
    }

    /// Overlay a named segment as a bounded `FixedVec`.
    #[inline]
    pub fn segment_fixed_vec<T: super::Pod + super::FixedLayout>(
        &mut self,
        id: &SegmentId,
    ) -> Result<FixedVec<'_, T>, ProgramError> {
        FixedVec::from_bytes(self.segment_data_mut(id)?)
    }

    /// Overlay a named segment as a `SortedVec`.
    #[inline]
    pub fn segment_sorted_vec<T: super::Pod + super::FixedLayout + Ord>(
        &mut self,
        id: &SegmentId,
    ) -> Result<SortedVec<'_, T>, ProgramError> {
        SortedVec::from_bytes(self.segment_data_mut(id)?)
    }

    /// Overlay a named segment as a `RingBuffer`.
    #[inline]
    pub fn segment_ring_buffer<T: super::Pod + super::FixedLayout>(
        &mut self,
        id: &SegmentId,
    ) -> Result<RingBuffer<'_, T>, ProgramError> {
        RingBuffer::from_bytes(self.segment_data_mut(id)?)
    }

    /// Overlay a named segment as a `SlotMap`.
    #[inline]
    pub fn segment_slot_map<T: super::Pod + super::FixedLayout>(
        &mut self,
        id: &SegmentId,
    ) -> Result<SlotMap<'_, T>, ProgramError> {
        SlotMap::from_bytes(self.segment_data_mut(id)?)
    }

    /// Overlay a named segment as a `Journal`.
    #[inline]
    pub fn segment_journal<T: super::Pod + super::FixedLayout>(
        &mut self,
        id: &SegmentId,
    ) -> Result<Journal<'_, T>, ProgramError> {
        Journal::from_bytes_mut(self.segment_data_mut(id)?)
    }

    /// Overlay a named segment as a `Slab`.
    #[inline]
    pub fn segment_slab<T: super::Pod + super::FixedLayout>(
        &mut self,
        id: &SegmentId,
    ) -> Result<Slab<'_, T>, ProgramError> {
        Slab::from_bytes_mut(self.segment_data_mut(id)?)
    }

    /// Freeze a segment (set frozen flag).
    #[inline]
    pub fn freeze_segment(&mut self, id: &SegmentId) -> Result<(), ProgramError> {
        let (_, entry) = self.find_mut(id)?;
        let new_flags = entry.flags() | SEG_FLAG_FROZEN;
        entry.set_flags(new_flags);
        Ok(())
    }

    /// Unfreeze a segment.
    #[inline]
    pub fn unfreeze_segment(&mut self, id: &SegmentId) -> Result<(), ProgramError> {
        let (_, entry) = self.find_mut(id)?;
        let new_flags = entry.flags() & !SEG_FLAG_FROZEN;
        entry.set_flags(new_flags);
        Ok(())
    }

    /// Lock a segment permanently (cannot be unlocked).
    #[inline]
    pub fn lock_segment(&mut self, id: &SegmentId) -> Result<(), ProgramError> {
        let (_, entry) = self.find_mut(id)?;
        let new_flags = entry.flags() | SEG_FLAG_LOCKED;
        entry.set_flags(new_flags);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    #[derive(Clone, Copy, Default, Eq, Ord, PartialEq, PartialOrd)]
    struct Entry8 {
        value: u8,
    }

    unsafe impl crate::account::Pod for Entry8 {}

    impl crate::account::FixedLayout for Entry8 {
        const SIZE: usize = 1;
    }

    #[test]
    fn segment_fixed_vec_adapter_exposes_vec_api() {
        const CORE: SegmentId = segment_id("core");
        let total = REGISTRY_OFFSET + REGISTRY_HEADER_SIZE + SEGMENT_ENTRY_SIZE + 8;
        let mut account = std::vec![0u8; total];

        SegmentRegistryMut::init(&mut account, &[(CORE, 8, 1)]).unwrap();

        let mut registry = SegmentRegistryMut::from_account_mut(&mut account).unwrap();
        let mut values = registry.segment_fixed_vec::<Entry8>(&CORE).unwrap();
        values.push(Entry8 { value: 7 }).unwrap();
        values.push(Entry8 { value: 9 }).unwrap();

        assert_eq!(values.len(), 2);
        assert_eq!(values.get(0).unwrap().value, 7);
        assert_eq!(values.get(1).unwrap().value, 9);
    }

    #[test]
    fn segment_journal_adapter_exposes_journal_api() {
        const AUDIT: SegmentId = segment_id("audit");
        let segment_bytes = crate::collections::JOURNAL_HEADER_SIZE + 4;
        let total = REGISTRY_OFFSET + REGISTRY_HEADER_SIZE + SEGMENT_ENTRY_SIZE + segment_bytes;
        let mut account = std::vec![0u8; total];

        SegmentRegistryMut::init(&mut account, &[(AUDIT, segment_bytes as u32, 1)]).unwrap();

        {
            let mut registry = SegmentRegistryMut::from_account_mut(&mut account).unwrap();
            let mut journal = registry.segment_journal::<Entry8>(&AUDIT).unwrap();
            journal.init(false);
            journal.append(Entry8 { value: 3 }).unwrap();
            journal.append(Entry8 { value: 4 }).unwrap();
        }

        let registry = SegmentRegistry::from_account(&account).unwrap();
        let bytes = registry.segment_data(&AUDIT).unwrap();
        let reader = crate::collections::Journal::<Entry8>::from_bytes(bytes).unwrap();
        assert_eq!(reader.entry_count(), 2);
        assert_eq!(reader.read(0).unwrap().value, 3);
        assert_eq!(reader.read(1).unwrap().value, 4);
    }
}
