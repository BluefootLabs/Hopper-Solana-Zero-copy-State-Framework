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

use hopper_runtime::error::ProgramError;
use crate::account::{Pod, FixedLayout};

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
        Ok(Self { data, capacity, _phantom: core::marker::PhantomData })
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
            let next = if i + 1 < capacity { (i + 1) as u32 } else { NO_FREE };
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

        // Increment count
        let count = self.count() + 1;
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
