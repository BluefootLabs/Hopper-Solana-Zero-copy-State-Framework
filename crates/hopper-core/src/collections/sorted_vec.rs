//! Sorted bounded dynamic array with O(log n) binary search.
//!
//! Wire layout (identical to FixedVec):
//! ```text
//! [count: u32 LE][element 0][element 1]...[element capacity-1]
//! ```
//!
//! Elements are maintained in sorted order. Requires `T: Ord + Pod + FixedLayout`.
//! Binary search costs ~3-15 CU per lookup vs ~50-500 CU linear scan (depending on N).

use hopper_runtime::error::ProgramError;
use crate::account::{Pod, FixedLayout};

const HEADER_SIZE: usize = 4;

/// Sorted bounded dynamic array -- zero-copy with O(log n) binary search.
///
/// Elements are kept in ascending order. Insert is O(n) (shift right),
/// remove is O(n) (shift left), search is O(log n).
pub struct SortedVec<'a, T: Pod + FixedLayout + Ord> {
    data: &'a mut [u8],
    _phantom: core::marker::PhantomData<T>,
}

impl<'a, T: Pod + FixedLayout + Ord> SortedVec<'a, T> {
    /// Overlay a SortedVec on a mutable byte slice.
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
        u32::from_le_bytes([self.data[0], self.data[1], self.data[2], self.data[3]]) as usize
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

    #[inline(always)]
    fn set_len(&mut self, len: usize) {
        let bytes = (len as u32).to_le_bytes();
        self.data[0] = bytes[0];
        self.data[1] = bytes[1];
        self.data[2] = bytes[2];
        self.data[3] = bytes[3];
    }

    #[inline(always)]
    fn element_offset(index: usize) -> usize {
        HEADER_SIZE + index * T::SIZE
    }

    /// Read element at index by copy.
    #[inline]
    fn read_at(&self, index: usize) -> T {
        let offset = Self::element_offset(index);
        // SAFETY: Caller ensures index < len. T: Pod, alignment-1.
        unsafe { core::ptr::read_unaligned(self.data.as_ptr().add(offset) as *const T) }
    }

    /// Write element at index.
    #[inline]
    fn write_at(&mut self, index: usize, value: T) {
        let offset = Self::element_offset(index);
        // SAFETY: Caller ensures index < capacity. T: Pod.
        unsafe { core::ptr::write_unaligned(self.data.as_mut_ptr().add(offset) as *mut T, value) }
    }

    /// Binary search for a value. Returns `Ok(index)` if found, or
    /// `Err(insert_index)` where `insert_index` is where the value would go.
    #[inline]
    pub fn binary_search(&self, target: &T) -> Result<usize, usize> {
        let len = self.len();
        if len == 0 {
            return Err(0);
        }
        let mut lo: usize = 0;
        let mut hi: usize = len;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let elem = self.read_at(mid);
            match elem.cmp(target) {
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Equal => return Ok(mid),
                core::cmp::Ordering::Greater => hi = mid,
            }
        }
        Err(lo)
    }

    /// Check if the value exists in the vec. O(log n).
    #[inline]
    pub fn contains(&self, target: &T) -> bool {
        self.binary_search(target).is_ok()
    }

    /// Get element at index (bounds-checked).
    #[inline]
    pub fn get(&self, index: usize) -> Result<T, ProgramError> {
        if index >= self.len() {
            return Err(ProgramError::InvalidArgument);
        }
        Ok(self.read_at(index))
    }

    /// Insert a value in sorted position. O(n) due to shift.
    /// Returns the index where it was inserted.
    #[inline]
    pub fn insert(&mut self, value: T) -> Result<usize, ProgramError> {
        let len = self.len();
        if len >= self.capacity() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let insert_idx = match self.binary_search(&value) {
            Ok(idx) => idx,  // Duplicate -- insert at same position (stable)
            Err(idx) => idx,
        };
        // Shift elements right from insert_idx to len-1
        if insert_idx < len {
            let src_offset = Self::element_offset(insert_idx);
            let dst_offset = Self::element_offset(insert_idx + 1);
            let byte_count = (len - insert_idx) * T::SIZE;
            // SAFETY: Non-overlapping copy within bounds.
            unsafe {
                core::ptr::copy(
                    self.data.as_ptr().add(src_offset),
                    self.data.as_mut_ptr().add(dst_offset),
                    byte_count,
                );
            }
        }
        self.write_at(insert_idx, value);
        self.set_len(len + 1);
        Ok(insert_idx)
    }

    /// Insert a value only if it doesn't already exist.
    /// Returns `Ok(index)` on successful insert, or `Err(existing_index)` if duplicate.
    #[inline]
    pub fn insert_unique(&mut self, value: T) -> Result<usize, usize> {
        match self.binary_search(&value) {
            Ok(existing) => Err(existing),
            Err(insert_idx) => {
                let len = self.len();
                if len >= self.capacity() {
                    return Err(usize::MAX); // Signal capacity full
                }
                if insert_idx < len {
                    let src_offset = Self::element_offset(insert_idx);
                    let dst_offset = Self::element_offset(insert_idx + 1);
                    let byte_count = (len - insert_idx) * T::SIZE;
                    unsafe {
                        core::ptr::copy(
                            self.data.as_ptr().add(src_offset),
                            self.data.as_mut_ptr().add(dst_offset),
                            byte_count,
                        );
                    }
                }
                self.write_at(insert_idx, value);
                self.set_len(len + 1);
                Ok(insert_idx)
            }
        }
    }

    /// Remove value at index (shift left). O(n).
    #[inline]
    pub fn remove(&mut self, index: usize) -> Result<T, ProgramError> {
        let len = self.len();
        if index >= len {
            return Err(ProgramError::InvalidArgument);
        }
        let removed = self.read_at(index);
        // Shift elements left
        if index + 1 < len {
            let src_offset = Self::element_offset(index + 1);
            let dst_offset = Self::element_offset(index);
            let byte_count = (len - index - 1) * T::SIZE;
            unsafe {
                core::ptr::copy(
                    self.data.as_ptr().add(src_offset),
                    self.data.as_mut_ptr().add(dst_offset),
                    byte_count,
                );
            }
        }
        // Zero the vacated last slot
        let last_offset = Self::element_offset(len - 1);
        for b in &mut self.data[last_offset..last_offset + T::SIZE] {
            *b = 0;
        }
        self.set_len(len - 1);
        Ok(removed)
    }

    /// Remove a specific value by searching for it first. O(n).
    #[inline]
    pub fn remove_value(&mut self, value: &T) -> Result<T, ProgramError> {
        match self.binary_search(value) {
            Ok(idx) => self.remove(idx),
            Err(_) => Err(ProgramError::InvalidArgument),
        }
    }

    /// Returns the minimum element (first). O(1).
    #[inline]
    pub fn min(&self) -> Result<T, ProgramError> {
        if self.is_empty() {
            return Err(ProgramError::InvalidArgument);
        }
        Ok(self.read_at(0))
    }

    /// Returns the maximum element (last). O(1).
    #[inline]
    pub fn max(&self) -> Result<T, ProgramError> {
        let len = self.len();
        if len == 0 {
            return Err(ProgramError::InvalidArgument);
        }
        Ok(self.read_at(len - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::WireU64;

    #[test]
    fn sorted_vec_insert_and_search() {
        let mut buf = [0u8; 4 + 8 * 8]; // capacity 8
        let mut sv = SortedVec::<WireU64>::from_bytes(&mut buf).unwrap();

        // Insert out of order
        sv.insert(WireU64::new(50)).unwrap();
        sv.insert(WireU64::new(10)).unwrap();
        sv.insert(WireU64::new(30)).unwrap();
        sv.insert(WireU64::new(20)).unwrap();
        sv.insert(WireU64::new(40)).unwrap();

        assert_eq!(sv.len(), 5);

        // Should be sorted
        assert_eq!(sv.get(0).unwrap().get(), 10);
        assert_eq!(sv.get(1).unwrap().get(), 20);
        assert_eq!(sv.get(2).unwrap().get(), 30);
        assert_eq!(sv.get(3).unwrap().get(), 40);
        assert_eq!(sv.get(4).unwrap().get(), 50);

        // Binary search
        assert!(sv.contains(&WireU64::new(30)));
        assert!(!sv.contains(&WireU64::new(25)));
        assert_eq!(sv.binary_search(&WireU64::new(30)), Ok(2));
        assert_eq!(sv.binary_search(&WireU64::new(25)), Err(2));
    }

    #[test]
    fn sorted_vec_min_max() {
        let mut buf = [0u8; 4 + 8 * 4];
        let mut sv = SortedVec::<WireU64>::from_bytes(&mut buf).unwrap();
        sv.insert(WireU64::new(100)).unwrap();
        sv.insert(WireU64::new(5)).unwrap();
        sv.insert(WireU64::new(42)).unwrap();

        assert_eq!(sv.min().unwrap().get(), 5);
        assert_eq!(sv.max().unwrap().get(), 100);
    }

    #[test]
    fn sorted_vec_remove() {
        let mut buf = [0u8; 4 + 8 * 4];
        let mut sv = SortedVec::<WireU64>::from_bytes(&mut buf).unwrap();
        sv.insert(WireU64::new(10)).unwrap();
        sv.insert(WireU64::new(20)).unwrap();
        sv.insert(WireU64::new(30)).unwrap();

        sv.remove_value(&WireU64::new(20)).unwrap();
        assert_eq!(sv.len(), 2);
        assert_eq!(sv.get(0).unwrap().get(), 10);
        assert_eq!(sv.get(1).unwrap().get(), 30);
    }

    #[test]
    fn sorted_vec_insert_unique() {
        let mut buf = [0u8; 4 + 8 * 4];
        let mut sv = SortedVec::<WireU64>::from_bytes(&mut buf).unwrap();
        assert!(sv.insert_unique(WireU64::new(10)).is_ok());
        assert!(sv.insert_unique(WireU64::new(20)).is_ok());
        // Duplicate should fail
        assert!(sv.insert_unique(WireU64::new(10)).is_err());
        assert_eq!(sv.len(), 2);
    }
}
