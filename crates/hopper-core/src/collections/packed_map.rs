//! Packed key->value map for on-chain registries -- zero-copy, no allocation.
//!
//! Wire layout:
//! ```text
//! [count: u32 LE][entry 0: K+V][entry 1: K+V]...[entry capacity-1: K+V]
//! ```
//!
//! Keys are unique. Lookup is O(n) linear scan (optimal for small N < ~64).
//! For sorted key access, pair with `SortedVec` instead.
//!
//! ## Usage
//!
//! ```ignore
//! let mut map = PackedMap::<Address32, WireU64>::from_bytes(&mut data[offset..])?;
//! map.insert(key, value)?;
//! let balance = map.get(&key)?;
//! ```

use hopper_runtime::error::ProgramError;
use crate::account::{Pod, FixedLayout};

const HEADER_SIZE: usize = 4;

/// An entry in the packed map: key followed by value.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct MapEntry<K: Pod + FixedLayout + PartialEq, V: Pod + FixedLayout> {
    pub key: K,
    pub value: V,
}

// SAFETY: MapEntry is repr(C) of Pod types, alignment inherited.
unsafe impl<K: Pod + FixedLayout + PartialEq, V: Pod + FixedLayout> Pod for MapEntry<K, V> {}

impl<K: Pod + FixedLayout + PartialEq, V: Pod + FixedLayout> FixedLayout for MapEntry<K, V> {
    const SIZE: usize = K::SIZE + V::SIZE;
}

/// Packed key->value map overlaid on a byte slice.
///
/// Supports O(n) lookup, O(1) insert (append), O(1) remove (swap-last).
/// No heap allocation.
pub struct PackedMap<'a, K: Pod + FixedLayout + PartialEq, V: Pod + FixedLayout> {
    data: &'a mut [u8],
    _phantom: core::marker::PhantomData<(K, V)>,
}

impl<'a, K: Pod + FixedLayout + PartialEq, V: Pod + FixedLayout> PackedMap<'a, K, V> {
    const ENTRY_SIZE: usize = K::SIZE + V::SIZE;

    /// Overlay a PackedMap on a mutable byte slice.
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

    /// Current number of entries.
    #[inline(always)]
    pub fn len(&self) -> usize {
        u32::from_le_bytes([self.data[0], self.data[1], self.data[2], self.data[3]]) as usize
    }

    /// Maximum capacity.
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        (self.data.len() - HEADER_SIZE) / Self::ENTRY_SIZE
    }

    /// Is the map empty?
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Is the map full?
    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.len() >= self.capacity()
    }

    #[inline(always)]
    fn set_len(&mut self, count: usize) {
        let bytes = (count as u32).to_le_bytes();
        self.data[0] = bytes[0];
        self.data[1] = bytes[1];
        self.data[2] = bytes[2];
        self.data[3] = bytes[3];
    }

    #[inline(always)]
    fn entry_offset(index: usize) -> usize {
        HEADER_SIZE + index * (K::SIZE + V::SIZE)
    }

    /// Read key at index.
    #[inline]
    fn read_key(&self, index: usize) -> K {
        let offset = Self::entry_offset(index);
        // SAFETY: Caller ensures index < len. K: Pod, alignment-1.
        unsafe { core::ptr::read_unaligned(self.data.as_ptr().add(offset) as *const K) }
    }

    /// Read value at index.
    #[inline]
    fn read_value(&self, index: usize) -> V {
        let offset = Self::entry_offset(index) + K::SIZE;
        // SAFETY: Caller ensures index < len. V: Pod, alignment-1.
        unsafe { core::ptr::read_unaligned(self.data.as_ptr().add(offset) as *const V) }
    }

    /// Write key at index.
    #[inline]
    fn write_key(&mut self, index: usize, key: K) {
        let offset = Self::entry_offset(index);
        // SAFETY: Caller ensures index < capacity. K: Pod, alignment-1.
        unsafe { core::ptr::write_unaligned(self.data.as_mut_ptr().add(offset) as *mut K, key) }
    }

    /// Write value at index.
    #[inline]
    fn write_value(&mut self, index: usize, value: V) {
        let offset = Self::entry_offset(index) + K::SIZE;
        // SAFETY: Caller ensures index < capacity. V: Pod, alignment-1.
        unsafe { core::ptr::write_unaligned(self.data.as_mut_ptr().add(offset) as *mut V, value) }
    }

    /// Find the index of a key, or None.
    #[inline]
    pub fn find(&self, key: &K) -> Option<usize> {
        let len = self.len();
        let mut i = 0;
        while i < len {
            if self.read_key(i) == *key {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    /// Get the value for a key, or error if not found.
    #[inline]
    pub fn get(&self, key: &K) -> Result<V, ProgramError> {
        match self.find(key) {
            Some(idx) => Ok(self.read_value(idx)),
            None => Err(ProgramError::InvalidArgument),
        }
    }

    /// Get a mutable reference to the value bytes for a key.
    /// Returns the byte offset of the value within data for direct manipulation.
    #[inline]
    pub fn get_value_offset(&self, key: &K) -> Result<usize, ProgramError> {
        match self.find(key) {
            Some(idx) => Ok(Self::entry_offset(idx) + K::SIZE),
            None => Err(ProgramError::InvalidArgument),
        }
    }

    /// Insert or update a key-value pair.
    ///
    /// If the key already exists, updates the value and returns `true`.
    /// If the key is new, appends and returns `false`.
    /// Returns error if full and key doesn't exist.
    #[inline]
    pub fn insert(&mut self, key: K, value: V) -> Result<bool, ProgramError> {
        // Check if key already exists
        if let Some(idx) = self.find(&key) {
            self.write_value(idx, value);
            return Ok(true); // updated
        }
        // New entry
        let len = self.len();
        if len >= self.capacity() {
            return Err(ProgramError::AccountDataTooSmall);
        }
        self.write_key(len, key);
        self.write_value(len, value);
        self.set_len(len + 1);
        Ok(false) // inserted
    }

    /// Remove a key-value pair by swapping with the last entry. O(1).
    ///
    /// Returns the removed value, or error if key not found.
    #[inline]
    pub fn remove(&mut self, key: &K) -> Result<V, ProgramError> {
        let idx = self.find(key).ok_or(ProgramError::InvalidArgument)?;
        let value = self.read_value(idx);
        let len = self.len();
        let last = len - 1;

        if idx != last {
            // Swap with last entry
            let last_key = self.read_key(last);
            let last_value = self.read_value(last);
            self.write_key(idx, last_key);
            self.write_value(idx, last_value);
        }

        // Zero the removed last slot
        let last_offset = Self::entry_offset(last);
        let entry_end = last_offset + Self::ENTRY_SIZE;
        for byte in &mut self.data[last_offset..entry_end] {
            *byte = 0;
        }

        self.set_len(last);
        Ok(value)
    }

    /// Check if the map contains a key.
    #[inline(always)]
    pub fn contains(&self, key: &K) -> bool {
        self.find(key).is_some()
    }

    /// Clear all entries.
    #[inline]
    pub fn clear(&mut self) {
        let len = self.len();
        let end = Self::entry_offset(len);
        for byte in &mut self.data[HEADER_SIZE..end] {
            *byte = 0;
        }
        self.set_len(0);
    }

    /// Iterate over all entries (key, value) by index.
    ///
    /// Returns an iterator that yields (K, V) copies.
    #[inline]
    pub fn iter(&self) -> PackedMapIter<'_, K, V> {
        PackedMapIter {
            map: self,
            index: 0,
            len: self.len(),
        }
    }
}

/// Iterator over PackedMap entries.
pub struct PackedMapIter<'a, K: Pod + FixedLayout + PartialEq, V: Pod + FixedLayout> {
    map: &'a PackedMap<'a, K, V>,
    index: usize,
    len: usize,
}

impl<'a, K: Pod + FixedLayout + PartialEq + Copy, V: Pod + FixedLayout + Copy> Iterator
    for PackedMapIter<'a, K, V>
{
    type Item = (K, V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.len {
            return None;
        }
        let key = self.map.read_key(self.index);
        let value = self.map.read_value(self.index);
        self.index += 1;
        Some((key, value))
    }

    #[inline(always)]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.len - self.index;
        (remaining, Some(remaining))
    }
}
