//! Fixed-size slab allocator for on-chain data.
//!
//! A `Slab` manages a fixed pool of identically-sized slots. It provides
//! O(1) allocation and deallocation using a free-list encoded in the
//! freed slots themselves, plus an occupancy bitmap that prevents
//! double-free and access to freed slots.
//!
//! ## Wire Format
//!
//! ```text
//! [count: u32 LE]      -- number of currently allocated slots
//! [capacity: u32 LE]   -- total slot count
//! [free_head: u32 LE]  -- index of first free slot (0xFFFFFFFF = none)
//! [_reserved: u32 LE]
//! [bitmap: ceil(capacity/8) bytes]  -- 1 bit per slot, 1 = allocated
//! [slot 0: element_size bytes]
//! [slot 1: element_size bytes]
//! ...
//! [slot capacity-1: element_size bytes]
//! ```
//!
//! Free slots store a `u32 LE` next-free pointer in their first 4 bytes.
//! This means the minimum element size is 4 bytes.
//!
//! ## Usage
//!
//! ```ignore
//! let mut slab = Slab::<MyEntry>::from_bytes_mut(data)?;
//! let idx = slab.alloc(entry)?;     // O(1)
//! let val = slab.get(idx)?;         // O(1), fails on freed slot
//! slab.free(idx)?;                  // O(1), fails on double-free
//! ```

use crate::account::{FixedLayout, Pod};
use hopper_runtime::error::ProgramError;

/// Slab header size in bytes.
pub const SLAB_HEADER_SIZE: usize = 16;

/// Sentinel value for "no free slot".
const NO_FREE: u32 = 0xFFFF_FFFF;

/// Compute the number of bitmap bytes needed for `capacity` slots.
#[inline(always)]
pub const fn bitmap_bytes(capacity: usize) -> usize {
    capacity.div_ceil(8)
}

/// A fixed-size slab allocator over a byte slice.
///
/// Tracks slot occupancy with an inline bitmap. Double-free, reads of
/// freed slots, and writes to freed slots are all rejected.
pub struct Slab<'a, T: Pod + FixedLayout> {
    data: &'a mut [u8],
    capacity: usize,
    _phantom: core::marker::PhantomData<T>,
}

impl<'a, T: Pod + FixedLayout> Slab<'a, T> {
    /// Parse a slab from a mutable byte slice.
    #[inline]
    pub fn from_bytes_mut(data: &'a mut [u8]) -> Result<Self, ProgramError> {
        // Compile-time: `FixedLayout::SIZE == size_of::<T>()` (the slot
        // copy moves `T::SIZE` bytes out of a `T` value, so a mismatch
        // reads or writes past the value) and `> 0`. The `< 4` runtime
        // check below is a *separate* slab-specific requirement: freed
        // slots store a `u32` next-free pointer in their first 4 bytes.
        const { super::assert_zero_copy_element::<T>() };
        if data.len() < SLAB_HEADER_SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        if T::SIZE < 4 {
            return Err(ProgramError::InvalidArgument);
        }
        let capacity = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
        let needed = SLAB_HEADER_SIZE + bitmap_bytes(capacity) + capacity * T::SIZE;
        if data.len() < needed {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(Self {
            data,
            capacity,
            _phantom: core::marker::PhantomData,
        })
    }

    /// Initialize a slab with the given capacity.
    ///
    /// Must be called on a zeroed buffer. Sets up the free list and
    /// clears the occupancy bitmap.
    #[inline]
    pub fn init(data: &mut [u8], capacity: usize) -> Result<(), ProgramError> {
        if T::SIZE < 4 {
            return Err(ProgramError::InvalidArgument);
        }
        let bmap_len = bitmap_bytes(capacity);
        let needed = SLAB_HEADER_SIZE + bmap_len + capacity * T::SIZE;
        if data.len() < needed {
            return Err(ProgramError::AccountDataTooSmall);
        }

        // Write header: count=0, capacity, free_head=0, reserved=0
        data[0..4].copy_from_slice(&0u32.to_le_bytes());
        data[4..8].copy_from_slice(&(capacity as u32).to_le_bytes());
        data[8..12].copy_from_slice(&0u32.to_le_bytes()); // free_head = 0
        data[12..16].copy_from_slice(&0u32.to_le_bytes());

        // Clear bitmap (all slots free)
        let bmap_start = SLAB_HEADER_SIZE;
        let mut i = 0;
        while i < bmap_len {
            data[bmap_start + i] = 0;
            i += 1;
        }

        // Build free list: each slot points to the next
        let slots_start = SLAB_HEADER_SIZE + bmap_len;
        i = 0;
        while i < capacity {
            let slot_offset = slots_start + i * T::SIZE;
            let next = if i + 1 < capacity {
                (i + 1) as u32
            } else {
                NO_FREE
            };
            data[slot_offset..slot_offset + 4].copy_from_slice(&next.to_le_bytes());
            i += 1;
        }

        Ok(())
    }

    /// Byte offset where the bitmap starts.
    #[inline(always)]
    fn bitmap_offset(&self) -> usize {
        SLAB_HEADER_SIZE
    }

    /// Byte offset where slots start (after header + bitmap).
    #[inline(always)]
    fn slots_offset(&self) -> usize {
        SLAB_HEADER_SIZE + bitmap_bytes(self.capacity)
    }

    /// Check if a slot is marked as allocated in the bitmap.
    #[inline(always)]
    fn is_allocated(&self, index: usize) -> bool {
        let bmap = self.bitmap_offset();
        let byte_idx = index / 8;
        let bit_idx = index % 8;
        (self.data[bmap + byte_idx] >> bit_idx) & 1 == 1
    }

    /// Mark a slot as allocated in the bitmap.
    #[inline(always)]
    fn mark_allocated(&mut self, index: usize) {
        let bmap = self.bitmap_offset();
        let byte_idx = index / 8;
        let bit_idx = index % 8;
        self.data[bmap + byte_idx] |= 1 << bit_idx;
    }

    /// Mark a slot as free in the bitmap.
    #[inline(always)]
    fn mark_free(&mut self, index: usize) {
        let bmap = self.bitmap_offset();
        let byte_idx = index / 8;
        let bit_idx = index % 8;
        self.data[bmap + byte_idx] &= !(1 << bit_idx);
    }

    /// Number of allocated slots.
    #[inline(always)]
    pub fn count(&self) -> u32 {
        u32::from_le_bytes([self.data[0], self.data[1], self.data[2], self.data[3]])
    }

    /// Total slot capacity.
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Index of the first free slot.
    #[inline(always)]
    fn free_head(&self) -> u32 {
        u32::from_le_bytes([self.data[8], self.data[9], self.data[10], self.data[11]])
    }

    /// Whether the slab is full.
    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.free_head() == NO_FREE
    }

    /// Whether a slot index is currently allocated.
    #[inline(always)]
    pub fn is_slot_allocated(&self, index: u32) -> bool {
        let idx = index as usize;
        idx < self.capacity && self.is_allocated(idx)
    }

    /// Allocate a slot and write the value. Returns the slot index.
    #[inline]
    pub fn alloc(&mut self, value: T) -> Result<u32, ProgramError> {
        let head = self.free_head();
        if head == NO_FREE {
            return Err(ProgramError::AccountDataTooSmall);
        }

        let idx = head as usize;
        // The free-list head comes from account bytes, which are
        // attacker-influenceable (a malicious or corrupted account can
        // carry any `free_head`). Two guards, both load-bearing:
        //
        // 1. Capacity bound — a `head` between `capacity` and
        //    `NO_FREE - 1` would compute an out-of-bounds `slot_offset`
        //    and the unchecked slot write below would corrupt memory
        //    past the account.
        // 2. Occupancy check — the bitmap is the ground truth. A free
        //    list rewired to point at an ALLOCATED slot would otherwise
        //    hand live data out for silent overwrite; and a free-list
        //    CYCLE (slot chained to itself or an earlier slot) would
        //    alias one slot across "distinct" allocations. Requiring
        //    the head slot to be free in the bitmap defeats both: any
        //    slot this alloc returns gets marked allocated, so a cycle
        //    is refused on its next visit.
        //
        // A bad next-free pointer chained in from a freed slot's bytes
        // is caught by these same guards on the next alloc.
        if idx >= self.capacity {
            return Err(ProgramError::InvalidAccountData);
        }
        if self.is_allocated(idx) {
            return Err(ProgramError::InvalidAccountData);
        }
        let slot_offset = self.slots_offset() + idx * T::SIZE;

        // Read the next-free pointer from this slot before overwriting
        let next_free = u32::from_le_bytes([
            self.data[slot_offset],
            self.data[slot_offset + 1],
            self.data[slot_offset + 2],
            self.data[slot_offset + 3],
        ]);

        // Write the value into the slot
        // SAFETY: T: Pod, alignment-1, bounds checked.
        unsafe {
            core::ptr::copy_nonoverlapping(
                &value as *const T as *const u8,
                self.data.as_mut_ptr().add(slot_offset),
                T::SIZE,
            );
        }

        // Mark allocated in bitmap
        self.mark_allocated(idx);

        // Update free_head
        self.data[8..12].copy_from_slice(&next_free.to_le_bytes());

        // Increment count (saturating: `count` is metadata read from
        // account bytes — a corrupted u32::MAX must not wrap to 0, the
        // same discipline `free` already applies with saturating_sub).
        let count = self.count().saturating_add(1);
        self.data[0..4].copy_from_slice(&count.to_le_bytes());

        Ok(head)
    }

    /// Free a slot and return it to the free list.
    ///
    /// Fails if the slot is not currently allocated (prevents double-free).
    #[inline]
    pub fn free(&mut self, index: u32) -> Result<(), ProgramError> {
        let idx = index as usize;
        if idx >= self.capacity {
            return Err(ProgramError::InvalidArgument);
        }
        if !self.is_allocated(idx) {
            return Err(ProgramError::InvalidArgument);
        }

        let slot_offset = self.slots_offset() + idx * T::SIZE;

        // Write current free_head into the freed slot's first 4 bytes
        let current_head = self.free_head();
        self.data[slot_offset..slot_offset + 4].copy_from_slice(&current_head.to_le_bytes());

        // Zero the rest of the slot (after the free pointer)
        let mut i = 4;
        while i < T::SIZE {
            self.data[slot_offset + i] = 0;
            i += 1;
        }

        // Mark free in bitmap
        self.mark_free(idx);

        // Point free_head to this slot
        self.data[8..12].copy_from_slice(&index.to_le_bytes());

        // Decrement count
        let count = self.count().saturating_sub(1);
        self.data[0..4].copy_from_slice(&count.to_le_bytes());

        Ok(())
    }

    /// Read a value from a slot (copy).
    ///
    /// Fails if the slot is not allocated.
    #[inline]
    pub fn get(&self, index: u32) -> Result<T, ProgramError> {
        let idx = index as usize;
        if idx >= self.capacity || !self.is_allocated(idx) {
            return Err(ProgramError::InvalidArgument);
        }
        let slot_offset = self.slots_offset() + idx * T::SIZE;
        // SAFETY: Bounds checked. T: Pod, alignment-1.
        Ok(unsafe { core::ptr::read_unaligned(self.data.as_ptr().add(slot_offset) as *const T) })
    }

    /// Get a reference to a value in a slot.
    ///
    /// Fails if the slot is not allocated.
    #[inline]
    pub fn get_ref(&self, index: u32) -> Result<&T, ProgramError> {
        let idx = index as usize;
        if idx >= self.capacity || !self.is_allocated(idx) {
            return Err(ProgramError::InvalidArgument);
        }
        let slot_offset = self.slots_offset() + idx * T::SIZE;
        // SAFETY: Bounds checked. T: Pod, alignment-1.
        Ok(unsafe { &*(self.data.as_ptr().add(slot_offset) as *const T) })
    }

    /// Get a mutable reference to a value in a slot.
    ///
    /// Fails if the slot is not allocated.
    #[inline]
    pub fn get_mut(&mut self, index: u32) -> Result<&mut T, ProgramError> {
        let idx = index as usize;
        if idx >= self.capacity || !self.is_allocated(idx) {
            return Err(ProgramError::InvalidArgument);
        }
        let slot_offset = self.slots_offset() + idx * T::SIZE;
        // SAFETY: Bounds checked. T: Pod, alignment-1. Exclusive access.
        Ok(unsafe { &mut *(self.data.as_mut_ptr().add(slot_offset) as *mut T) })
    }

    /// Write a value into an allocated slot.
    ///
    /// Fails if the slot is not allocated.
    #[inline]
    pub fn set(&mut self, index: u32, value: T) -> Result<(), ProgramError> {
        let idx = index as usize;
        if idx >= self.capacity || !self.is_allocated(idx) {
            return Err(ProgramError::InvalidArgument);
        }
        let slot_offset = self.slots_offset() + idx * T::SIZE;
        // SAFETY: T: Pod, bounds checked, alignment-1, exclusive access.
        unsafe {
            core::ptr::copy_nonoverlapping(
                &value as *const T as *const u8,
                self.data.as_mut_ptr().add(slot_offset),
                T::SIZE,
            );
        }
        Ok(())
    }

    /// Bytes required for a slab of given capacity.
    #[inline(always)]
    pub const fn required_bytes(capacity: usize) -> usize {
        SLAB_HEADER_SIZE + bitmap_bytes(capacity) + capacity * T::SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct Entry {
        // Min element size is 4 (free-list pointer lives in the first
        // 4 bytes of a freed slot); use 8.
        v: [u8; 8],
    }
    // SAFETY: repr(C), byte-array field — align 1, all patterns valid.
    unsafe impl crate::account::Zeroable for Entry {}
    // SAFETY: as above.
    unsafe impl crate::account::Pod for Entry {}
    impl FixedLayout for Entry {
        const SIZE: usize = 8;
    }

    fn e(n: u64) -> Entry {
        Entry { v: n.to_le_bytes() }
    }

    fn make(capacity: usize) -> std::vec::Vec<u8> {
        let mut buf = std::vec![0u8; Slab::<Entry>::required_bytes(capacity)];
        Slab::<Entry>::init(&mut buf, capacity).unwrap();
        buf
    }

    #[test]
    fn alloc_get_free_roundtrip_and_double_free_rejected() {
        let mut buf = make(4);
        let mut slab = Slab::<Entry>::from_bytes_mut(&mut buf).unwrap();

        let a = slab.alloc(e(10)).unwrap();
        let b = slab.alloc(e(20)).unwrap();
        assert_eq!(slab.count(), 2);
        assert_eq!(slab.get(a).unwrap(), e(10));
        assert_eq!(slab.get(b).unwrap(), e(20));

        slab.free(a).unwrap();
        assert_eq!(slab.count(), 1);
        // Read of a freed slot is refused.
        assert!(slab.get(a).is_err());
        // Double free is refused.
        assert!(slab.free(a).is_err());
        // The freed slot is reused by the next alloc.
        let c = slab.alloc(e(30)).unwrap();
        assert_eq!(c, a);
        assert_eq!(slab.get(c).unwrap(), e(30));
    }

    #[test]
    fn alloc_fails_when_full_without_overflow() {
        let mut buf = make(2);
        let mut slab = Slab::<Entry>::from_bytes_mut(&mut buf).unwrap();
        slab.alloc(e(1)).unwrap();
        slab.alloc(e(2)).unwrap();
        assert!(slab.is_full());
        assert!(slab.alloc(e(3)).is_err());
    }

    #[test]
    fn free_list_rewired_to_live_slot_is_refused() {
        // free_head pointing at an ALLOCATED slot must not hand live
        // data out for overwrite. This also models the head of a
        // free-list cycle: the cycled slot is allocated on first pop,
        // so the second visit hits this same refusal.
        let mut buf = make(4);
        {
            let mut slab = Slab::<Entry>::from_bytes_mut(&mut buf).unwrap();
            let a = slab.alloc(e(1)).unwrap();
            assert_eq!(a, 0);
        }
        // Rewire free_head back to the allocated slot 0.
        buf[8..12].copy_from_slice(&0u32.to_le_bytes());
        let mut slab = Slab::<Entry>::from_bytes_mut(&mut buf).unwrap();
        assert_eq!(
            slab.alloc(e(2)).unwrap_err(),
            ProgramError::InvalidAccountData
        );
        // The live slot's data is untouched.
        assert_eq!(slab.get(0).unwrap(), e(1));
    }

    #[test]
    fn corrupt_free_head_cannot_force_oob_write() {
        // The regression this guards: a free_head between capacity and
        // NO_FREE-1, planted in the account bytes, previously flowed into
        // an unchecked slot write past the end of the buffer.
        let mut buf = make(4);
        // Overwrite free_head (bytes 8..12) with an in-range-looking but
        // out-of-capacity index.
        buf[8..12].copy_from_slice(&9u32.to_le_bytes());
        let mut slab = Slab::<Entry>::from_bytes_mut(&mut buf).unwrap();
        assert_eq!(
            slab.alloc(e(99)).unwrap_err(),
            ProgramError::InvalidAccountData
        );
    }

    #[test]
    fn from_bytes_mut_rejects_undersized_buffer() {
        // Header claims capacity 4 but the buffer is header-only.
        let mut buf = std::vec![0u8; SLAB_HEADER_SIZE];
        buf[4..8].copy_from_slice(&4u32.to_le_bytes());
        assert!(Slab::<Entry>::from_bytes_mut(&mut buf).is_err());
    }
}
