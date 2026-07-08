//! Fixed-slot map with generation counters for safe handles.
//!
//! Wire layout:
//! ```text
//! [count: u32 LE][free_head: u32 LE][slot 0][slot 1]...[slot capacity-1]
//! ```
//!
//! Each slot:
//! ```text
//! [generation: u32 LE][occupied: u8][_pad: u8 x 3][element data: T::SIZE bytes]
//! ```
//!
//! The generation counter prevents ABA bugs when handles are reused.

use crate::account::{FixedLayout, Pod};
use hopper_runtime::error::ProgramError;

/// Map header: count (4) + free_head (4) = 8 bytes.
const MAP_HEADER: usize = 8;

/// Per-slot overhead: generation (4) + occupied (1) + padding (3) = 8 bytes.
const SLOT_OVERHEAD: usize = 8;

/// A handle to a slot in the SlotMap.
/// Contains the slot index and its generation at insertion time.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct SlotKey {
    pub index: u32,
    pub generation: u32,
}

const _: () = assert!(core::mem::size_of::<SlotKey>() == 8);
const _: () = assert!(core::mem::align_of::<SlotKey>() == 4); // OK for non-wire use

/// Fixed-slot map overlaid on a byte slice.
///
/// - O(1) insert (into free slot), O(1) remove, O(1) access by SlotKey.
/// - Generation counters prevent ABA bugs.
/// - Used for registries, entity systems, order books with stable handles.
pub struct SlotMap<'a, T: Pod + FixedLayout> {
    data: &'a mut [u8],
    /// Slot capacity, derived once at construction
    /// (parse-don't-validate).
    capacity: usize,
    _phantom: core::marker::PhantomData<T>,
}

impl<'a, T: Pod + FixedLayout> SlotMap<'a, T> {
    /// Size of one slot (overhead + element). `SLOT_OVERHEAD > 0`, so
    /// this is always nonzero even for a (rejected) zero-sized `T`, and
    /// the capacity division below cannot divide by zero.
    const SLOT_SIZE: usize = SLOT_OVERHEAD + T::SIZE;

    /// Overlay a SlotMap on a mutable byte slice.
    ///
    /// **Parse, don't validate.** The stored occupied-slot count comes
    /// from untrusted account bytes; a header claiming `count > capacity`
    /// is inconsistent geometry and is rejected here. (Access itself was
    /// already sound — every `SlotKey` index is bounds-checked against
    /// `capacity` and the generation counter defeats ABA — so this adds
    /// consistency, not a missing bound.)
    #[inline]
    pub fn from_bytes(data: &'a mut [u8]) -> Result<Self, ProgramError> {
        const { super::assert_zero_copy_element::<T>() };
        if data.len() < MAP_HEADER {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let capacity = (data.len() - MAP_HEADER) / Self::SLOT_SIZE;
        let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if count > capacity {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(Self {
            data,
            capacity,
            _phantom: core::marker::PhantomData,
        })
    }

    /// Maximum capacity (number of slots).
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Current number of occupied slots.
    #[inline(always)]
    pub fn count(&self) -> usize {
        u32::from_le_bytes([self.data[0], self.data[1], self.data[2], self.data[3]]) as usize
    }

    /// Set count.
    #[inline(always)]
    fn set_count(&mut self, count: usize) {
        self.data[0..4].copy_from_slice(&(count as u32).to_le_bytes());
    }

    /// Byte offset to slot `index`.
    #[inline(always)]
    fn slot_offset(&self, index: usize) -> usize {
        MAP_HEADER + index * Self::SLOT_SIZE
    }

    /// Read the generation counter at a slot.
    #[inline(always)]
    fn slot_generation(&self, index: usize) -> u32 {
        let off = self.slot_offset(index);
        u32::from_le_bytes([
            self.data[off],
            self.data[off + 1],
            self.data[off + 2],
            self.data[off + 3],
        ])
    }

    /// Is the slot occupied?
    #[inline(always)]
    fn slot_occupied(&self, index: usize) -> bool {
        let off = self.slot_offset(index) + 4;
        self.data[off] != 0
    }

    /// Insert a value, returning a SlotKey handle.
    ///
    /// Scans for the first free slot. O(capacity) worst case.
    #[inline]
    pub fn insert(&mut self, value: T) -> Result<SlotKey, ProgramError> {
        let cap = self.capacity();
        for i in 0..cap {
            if !self.slot_occupied(i) {
                let off = self.slot_offset(i);
                let gen = self.slot_generation(i);
                // Mark occupied
                self.data[off + 4] = 1;
                // Write value
                let val_off = off + SLOT_OVERHEAD;
                // SAFETY: T: Pod, alignment-1. Bounds checked: slot index < capacity.
                unsafe {
                    core::ptr::write_unaligned(
                        self.data.as_mut_ptr().add(val_off) as *mut T,
                        value,
                    );
                }
                // The stored `count` header is untrusted (account bytes are
                // attacker-writable) and `from_bytes` does not reconcile it
                // against the per-slot occupancy flags — slot access is gated
                // on the flags, not `count`, so `count` is a reporting value
                // only. Clamp to capacity so a crafted `count == capacity`
                // with a free slot can never push the reported length past the
                // real bound.
                self.set_count(self.count().saturating_add(1).min(cap));
                return Ok(SlotKey {
                    index: i as u32,
                    generation: gen,
                });
            }
        }
        Err(ProgramError::AccountDataTooSmall)
    }

    /// Get a value by key. Returns error if generation doesn't match.
    #[inline]
    pub fn get(&self, key: SlotKey) -> Result<T, ProgramError> {
        let index = key.index as usize;
        if index >= self.capacity() {
            return Err(ProgramError::InvalidArgument);
        }
        if !self.slot_occupied(index) || self.slot_generation(index) != key.generation {
            return Err(ProgramError::InvalidArgument);
        }
        let off = self.slot_offset(index) + SLOT_OVERHEAD;
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        Ok(unsafe { core::ptr::read_unaligned(self.data.as_ptr().add(off) as *const T) })
    }

    /// Remove a value by key. Bumps the generation counter.
    #[inline]
    pub fn remove(&mut self, key: SlotKey) -> Result<T, ProgramError> {
        let index = key.index as usize;
        if index >= self.capacity() {
            return Err(ProgramError::InvalidArgument);
        }
        if !self.slot_occupied(index) || self.slot_generation(index) != key.generation {
            return Err(ProgramError::InvalidArgument);
        }
        let off = self.slot_offset(index);
        let val_off = off + SLOT_OVERHEAD;
        // SAFETY: This block is part of Hopper's audited zero-copy/backend boundary; surrounding checks and caller contracts uphold the required raw-pointer, layout, and aliasing invariants.
        let value =
            unsafe { core::ptr::read_unaligned(self.data.as_ptr().add(val_off) as *const T) };
        // Clear occupied flag
        self.data[off + 4] = 0;
        // Bump generation
        let new_gen = self.slot_generation(index).wrapping_add(1);
        self.data[off..off + 4].copy_from_slice(&new_gen.to_le_bytes());
        // Zero element data
        for byte in &mut self.data[val_off..val_off + T::SIZE] {
            *byte = 0;
        }
        // Saturating: the stored `count` is untrusted and may not agree with
        // the occupancy flags this removal is actually gated on. An
        // unchecked `count - 1` on a crafted `count == 0` underflows —
        // debug-panics, or in release wraps to `usize::MAX`, bricking the
        // reported length. `count` is a reporting value only (slot access is
        // flag-gated), so saturating is both safe and correct.
        self.set_count(self.count().saturating_sub(1));
        Ok(value)
    }

    /// Compute the byte size needed for a SlotMap with the given capacity.
    #[inline(always)]
    pub const fn required_bytes(capacity: usize) -> usize {
        MAP_HEADER + capacity * (SLOT_OVERHEAD + T::SIZE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::WireU64;

    #[test]
    fn insert_get_remove() {
        let mut buf = [0u8; 8 + (8 + 8) * 4]; // capacity 4
        let mut map = SlotMap::<WireU64>::from_bytes(&mut buf).unwrap();

        let k1 = map.insert(WireU64::new(100)).unwrap();
        let k2 = map.insert(WireU64::new(200)).unwrap();
        assert_eq!(map.count(), 2);

        assert_eq!(map.get(k1).unwrap().get(), 100);
        assert_eq!(map.get(k2).unwrap().get(), 200);

        let removed = map.remove(k1).unwrap();
        assert_eq!(removed.get(), 100);
        assert_eq!(map.count(), 1);

        // Old key should fail (generation bumped)
        assert!(map.get(k1).is_err());
    }

    #[test]
    fn generation_prevents_aba() {
        let mut buf = [0u8; 8 + (8 + 8) * 2];
        let mut map = SlotMap::<WireU64>::from_bytes(&mut buf).unwrap();

        let k1 = map.insert(WireU64::new(1)).unwrap();
        map.remove(k1).unwrap();

        // Re-insert into same slot
        let k2 = map.insert(WireU64::new(2)).unwrap();
        assert_eq!(k2.index, k1.index); // Same slot
        assert_ne!(k2.generation, k1.generation); // Different generation

        // Old key cannot access new value
        assert!(map.get(k1).is_err());
        assert_eq!(map.get(k2).unwrap().get(), 2);
    }

    #[test]
    fn remove_on_hostile_zero_count_does_not_underflow() {
        // Account bytes are attacker-writable and `from_bytes` does not
        // reconcile the stored `count` with the occupancy flags. Craft a
        // buffer whose slot 0 is occupied (a real element removal succeeds
        // its flag/generation gate) but whose `count` header is 0, so the
        // decrement would underflow. It must saturate, not panic or wrap.
        let mut buf = [0u8; 8 + (8 + 8) * 2];
        {
            let mut map = SlotMap::<WireU64>::from_bytes(&mut buf).unwrap();
            let k = map.insert(WireU64::new(42)).unwrap();
            // Corrupt the count header back to 0 while slot 0 stays occupied.
            map.set_count(0);
            // The removal is gated on the occupancy flag (still set), so it
            // proceeds; the count decrement must saturate at 0.
            let removed = map.remove(k).unwrap();
            assert_eq!(removed.get(), 42);
            assert_eq!(map.count(), 0);
        }
    }

    #[test]
    fn insert_on_hostile_full_count_clamps_to_capacity() {
        // A crafted `count == capacity` with a genuinely free slot must not
        // push the reported length past capacity when that slot is filled.
        let mut buf = [0u8; 8 + (8 + 8) * 2]; // capacity 2
        let mut map = SlotMap::<WireU64>::from_bytes(&mut buf).unwrap();
        map.set_count(map.capacity()); // lie: "full", but both slots are free
        let _ = map.insert(WireU64::new(7)).unwrap();
        assert!(map.count() <= map.capacity());
    }
}
