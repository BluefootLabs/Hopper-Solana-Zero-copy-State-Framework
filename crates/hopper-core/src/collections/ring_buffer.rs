//! Fixed-capacity circular buffer for journals, event logs, and queues.
//!
//! Wire layout:
//! ```text
//! [head: u32 LE][count: u32 LE][element 0][element 1]...[element capacity-1]
//! ```
//!
//! Elements wrap around when the buffer is full. Oldest elements are overwritten.

use hopper_runtime::error::ProgramError;
use crate::account::{Pod, FixedLayout};

/// Header: 4 bytes head + 4 bytes count = 8 bytes.
const RING_HEADER: usize = 8;

/// Fixed-capacity circular buffer overlaid on a byte slice.
///
/// - `push` always succeeds. When full, overwrites the oldest entry.
/// - Use for event journals, audit logs, price history, etc.
/// - O(1) push, O(1) read by logical index.
pub struct RingBuffer<'a, T: Pod + FixedLayout> {
    data: &'a mut [u8],
    _phantom: core::marker::PhantomData<T>,
}

impl<'a, T: Pod + FixedLayout> RingBuffer<'a, T> {
    /// Overlay a RingBuffer on a mutable byte slice.
    #[inline]
    pub fn from_bytes(data: &'a mut [u8]) -> Result<Self, ProgramError> {
        if data.len() < RING_HEADER {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(Self {
            data,
            _phantom: core::marker::PhantomData,
        })
    }

    /// Maximum capacity (number of elements).
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        (self.data.len() - RING_HEADER) / T::SIZE
    }

    /// Current number of elements (may be less than capacity if not yet full).
    #[inline(always)]
    pub fn count(&self) -> usize {
        let bytes = [self.data[4], self.data[5], self.data[6], self.data[7]];
        u32::from_le_bytes(bytes) as usize
    }

    /// Head pointer (index of the next write position).
    #[inline(always)]
    fn head(&self) -> usize {
        let bytes = [self.data[0], self.data[1], self.data[2], self.data[3]];
        u32::from_le_bytes(bytes) as usize
    }

    /// Set head.
    #[inline(always)]
    fn set_head(&mut self, head: usize) {
        let bytes = (head as u32).to_le_bytes();
        self.data[0..4].copy_from_slice(&bytes);
    }

    /// Set count.
    #[inline(always)]
    fn set_count(&mut self, count: usize) {
        let bytes = (count as u32).to_le_bytes();
        self.data[4..8].copy_from_slice(&bytes);
    }

    /// Byte offset of element at physical slot `index`.
    #[inline(always)]
    fn slot_offset(&self, index: usize) -> usize {
        RING_HEADER + index * T::SIZE
    }

    /// Push an element. If the buffer is full, overwrites the oldest entry.
    #[inline]
    pub fn push(&mut self, value: T) -> Result<(), ProgramError> {
        let cap = self.capacity();
        if cap == 0 {
            return Err(ProgramError::AccountDataTooSmall);
        }

        let head = self.head();
        let offset = self.slot_offset(head);

        // SAFETY: T: Pod, alignment-1, bounds ensured by capacity calculation.
        unsafe {
            core::ptr::write_unaligned(self.data.as_mut_ptr().add(offset) as *mut T, value);
        }

        let new_head = (head + 1) % cap;
        self.set_head(new_head);

        let count = self.count();
        if count < cap {
            self.set_count(count + 1);
        }

        Ok(())
    }

    /// Read the element at logical index (0 = oldest still in buffer).
    #[inline]
    pub fn get(&self, logical_index: usize) -> Result<T, ProgramError> {
        let count = self.count();
        if logical_index >= count {
            return Err(ProgramError::InvalidArgument);
        }
        let cap = self.capacity();
        let head = self.head();
        // The oldest element is at (head - count) mod cap
        let start = if head >= count { head - count } else { cap - (count - head) };
        let physical = (start + logical_index) % cap;
        let offset = self.slot_offset(physical);

        Ok(unsafe {
            core::ptr::read_unaligned(self.data.as_ptr().add(offset) as *const T)
        })
    }

    /// Read the most recently pushed element.
    #[inline]
    pub fn latest(&self) -> Result<T, ProgramError> {
        let count = self.count();
        if count == 0 {
            return Err(ProgramError::InvalidArgument);
        }
        self.get(count - 1)
    }

    /// Read the oldest element.
    #[inline]
    pub fn oldest(&self) -> Result<T, ProgramError> {
        if self.count() == 0 {
            return Err(ProgramError::InvalidArgument);
        }
        self.get(0)
    }

    /// Compute the byte size needed for a RingBuffer with the given capacity.
    #[inline(always)]
    pub const fn required_bytes(capacity: usize) -> usize {
        RING_HEADER + capacity * T::SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::WireU32;

    #[test]
    fn ring_push_and_read() {
        let mut buf = [0u8; 8 + 4 * 3]; // capacity 3
        let mut ring = RingBuffer::<WireU32>::from_bytes(&mut buf).unwrap();

        ring.push(WireU32::new(10)).unwrap();
        ring.push(WireU32::new(20)).unwrap();
        ring.push(WireU32::new(30)).unwrap();

        assert_eq!(ring.count(), 3);
        assert_eq!(ring.oldest().unwrap().get(), 10);
        assert_eq!(ring.latest().unwrap().get(), 30);
    }

    #[test]
    fn ring_wraps_around() {
        let mut buf = [0u8; 8 + 4 * 2]; // capacity 2
        let mut ring = RingBuffer::<WireU32>::from_bytes(&mut buf).unwrap();

        ring.push(WireU32::new(1)).unwrap();
        ring.push(WireU32::new(2)).unwrap();
        ring.push(WireU32::new(3)).unwrap(); // overwrites 1

        assert_eq!(ring.count(), 2);
        assert_eq!(ring.oldest().unwrap().get(), 2);
        assert_eq!(ring.latest().unwrap().get(), 3);
    }
}
