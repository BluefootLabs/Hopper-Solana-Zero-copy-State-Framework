//! Bounded dynamic array -- zero-copy `FixedVec`.
//!
//! Wire layout:
//! ```text
//! [count: u32 LE][element 0][element 1]...[element capacity-1]
//! ```
//!
//! The capacity is known at construction time from the byte slice size.
//! Elements must implement `Pod + FixedLayout`.

use hopper_runtime::error::ProgramError;
use crate::account::{Pod, FixedLayout};

/// Header size: 4 bytes for the count.
const HEADER_SIZE: usize = 4;

/// Bounded dynamic array overlaid on a byte slice.
///
/// Supports O(1) push, O(1) pop, O(1) swap_remove, O(1) index access.
/// No heap allocation.
pub struct FixedVec<'a, T: Pod + FixedLayout> {
    data: &'a mut [u8],
    _phantom: core::marker::PhantomData<T>,
}

impl<'a, T: Pod + FixedLayout> FixedVec<'a, T> {
    /// Overlay a FixedVec on a mutable byte slice.
    ///
    /// The slice must be at least `HEADER_SIZE` bytes.
    /// Capacity is `(data.len() - HEADER_SIZE) / T::SIZE`.
    #[inline]
    pub fn from_bytes(data: &'a mut [u8]) -> Result<Self, ProgramError> {
        if data.len() < HEADER_SIZE {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(Self {
            data,
            _phantom: core::marker::PhantomData,
        })
    }

    /// Current number of elements.
    #[inline(always)]
    pub fn len(&self) -> usize {
        let bytes = [self.data[0], self.data[1], self.data[2], self.data[3]];
        u32::from_le_bytes(bytes) as usize
    }

    /// Maximum capacity.
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        (self.data.len() - HEADER_SIZE) / T::SIZE
    }

    /// Is the vec empty?
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Is the vec at capacity?
    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.len() >= self.capacity()
    }

    /// Set the count (internal).
    #[inline(always)]
    fn set_len(&mut self, len: usize) {
        let bytes = (len as u32).to_le_bytes();
        self.data[0] = bytes[0];
        self.data[1] = bytes[1];
        self.data[2] = bytes[2];
        self.data[3] = bytes[3];
    }

    /// Byte offset of element at `index`.
    #[inline(always)]
    fn element_offset(&self, index: usize) -> usize {
        HEADER_SIZE + index * T::SIZE
    }

    /// Read element at index (copy).
    #[inline]
    pub fn get(&self, index: usize) -> Result<T, ProgramError> {
        let len = self.len();
        if index >= len {
            return Err(ProgramError::InvalidArgument);
        }
        let offset = self.element_offset(index);
        // SAFETY: Bounds checked. T: Pod, alignment-1.
        Ok(unsafe {
            core::ptr::read_unaligned(self.data.as_ptr().add(offset) as *const T)
        })
    }

    /// Get immutable reference to element at index.
    #[inline]
    pub fn get_ref(&self, index: usize) -> Result<&T, ProgramError> {
        let len = self.len();
        if index >= len {
            return Err(ProgramError::InvalidArgument);
        }
        let offset = self.element_offset(index);
        // SAFETY: Bounds checked. T: Pod, alignment-1.
        Ok(unsafe { &*(self.data.as_ptr().add(offset) as *const T) })
    }

    /// Set element at index.
    #[inline]
    pub fn set(&mut self, index: usize, value: T) -> Result<(), ProgramError> {
        let len = self.len();
        if index >= len {
            return Err(ProgramError::InvalidArgument);
        }
        let offset = self.element_offset(index);
        // SAFETY: Bounds checked. T: Pod. Exclusive access.
        unsafe {
            core::ptr::write_unaligned(self.data.as_mut_ptr().add(offset) as *mut T, value);
        }
        Ok(())
    }

    /// Push an element to the end. Returns error if at capacity.
    #[inline]
    pub fn push(&mut self, value: T) -> Result<(), ProgramError> {
        let len = self.len();
        if len >= self.capacity() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let offset = self.element_offset(len);
        // SAFETY: Bounds checked (len < capacity). T: Pod, alignment-1.
        unsafe {
            core::ptr::write_unaligned(self.data.as_mut_ptr().add(offset) as *mut T, value);
        }
        self.set_len(len + 1);
        Ok(())
    }

    /// Pop the last element.
    #[inline]
    pub fn pop(&mut self) -> Result<T, ProgramError> {
        let len = self.len();
        if len == 0 {
            return Err(ProgramError::InvalidArgument);
        }
        let offset = self.element_offset(len - 1);
        let value = unsafe {
            core::ptr::read_unaligned(self.data.as_ptr().add(offset) as *const T)
        };
        // Zero the removed slot for cleanliness.
        for byte in &mut self.data[offset..offset + T::SIZE] {
            *byte = 0;
        }
        self.set_len(len - 1);
        Ok(value)
    }

    /// Remove element at index by swapping with the last element. O(1).
    #[inline]
    pub fn swap_remove(&mut self, index: usize) -> Result<T, ProgramError> {
        let len = self.len();
        if index >= len {
            return Err(ProgramError::InvalidArgument);
        }
        let removed_offset = self.element_offset(index);
        let removed = unsafe {
            core::ptr::read_unaligned(self.data.as_ptr().add(removed_offset) as *const T)
        };

        let last_index = len - 1;
        if index != last_index {
            let last_offset = self.element_offset(last_index);
            // Copy last element to removed position
            let size = T::SIZE;
            for i in 0..size {
                self.data[removed_offset + i] = self.data[last_offset + i];
            }
            // Zero the old last slot
            for byte in &mut self.data[last_offset..last_offset + size] {
                *byte = 0;
            }
        } else {
            // Zero the removed slot
            for byte in &mut self.data[removed_offset..removed_offset + T::SIZE] {
                *byte = 0;
            }
        }
        self.set_len(last_index);
        Ok(removed)
    }

    /// Clear all elements, setting count to 0.
    #[inline]
    pub fn clear(&mut self) {
        let len = self.len();
        let end = self.element_offset(len);
        for byte in &mut self.data[HEADER_SIZE..end] {
            *byte = 0;
        }
        self.set_len(0);
    }

    /// Compute the byte size needed for a FixedVec with the given capacity.
    #[inline(always)]
    pub const fn required_bytes(capacity: usize) -> usize {
        HEADER_SIZE + capacity * T::SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::WireU64;

    #[test]
    fn push_pop_roundtrip() {
        let mut buf = [0u8; 4 + 8 * 4]; // 4 capacity
        let mut vec = FixedVec::<WireU64>::from_bytes(&mut buf).unwrap();
        assert_eq!(vec.len(), 0);
        assert_eq!(vec.capacity(), 4);

        vec.push(WireU64::new(10)).unwrap();
        vec.push(WireU64::new(20)).unwrap();
        assert_eq!(vec.len(), 2);

        let val = vec.pop().unwrap();
        assert_eq!(val.get(), 20);
        assert_eq!(vec.len(), 1);

        let val = vec.get(0).unwrap();
        assert_eq!(val.get(), 10);
    }

    #[test]
    fn swap_remove_works() {
        let mut buf = [0u8; 4 + 8 * 4];
        let mut vec = FixedVec::<WireU64>::from_bytes(&mut buf).unwrap();
        vec.push(WireU64::new(100)).unwrap();
        vec.push(WireU64::new(200)).unwrap();
        vec.push(WireU64::new(300)).unwrap();

        let removed = vec.swap_remove(0).unwrap();
        assert_eq!(removed.get(), 100);
        assert_eq!(vec.len(), 2);
        // Element 0 is now the old last element (300)
        assert_eq!(vec.get(0).unwrap().get(), 300);
        assert_eq!(vec.get(1).unwrap().get(), 200);
    }

    #[test]
    fn full_returns_error() {
        let mut buf = [0u8; 4 + 8]; // capacity 1
        let mut vec = FixedVec::<WireU64>::from_bytes(&mut buf).unwrap();
        vec.push(WireU64::new(1)).unwrap();
        assert!(vec.push(WireU64::new(2)).is_err());
    }
}
