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
    /// from untrusted account bytes. Two consistency checks reject a
    /// corrupt header up front:
    ///
    /// 1. `count > capacity` is inconsistent geometry.
    /// 2. `count` must equal the number of slots whose occupied flag is
    ///    actually set — the flags are the ground truth every access is
    ///    gated on, so a disagreeing header is corruption, not state.
    ///
    /// The reconciliation scan is O(capacity), one flag byte per slot,
    /// paid once at construction. (Access itself was already sound —
    /// every `SlotKey` index is bounds-checked against `capacity` and
    /// the generation counter defeats ABA — so this adds consistency,
    /// not a missing bound.)
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
        // Reconcile the stored count against the per-slot occupancy
        // flags (offset +4 within each slot). The flags are what insert/
        // remove/get are gated on; a header that disagrees with them is
        // corrupt and is rejected rather than papered over downstream.
        let mut occupied = 0usize;
        for i in 0..capacity {
            if data[MAP_HEADER + i * Self::SLOT_SIZE + 4] != 0 {
                occupied += 1;
            }
        }
        if occupied != count {
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
                // `from_bytes` now rejects a `count` that disagrees with the
                // occupancy flags, so a consistent map cannot reach `count ==
                // capacity` with a free slot. Keep the clamp anyway (defense
                // in depth): slot access is gated on the flags, not `count`,
                // so `count` is a reporting value only and clamping can never
                // hide a real slot.
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
        // Saturating (defense in depth): `from_bytes` now rejects a stored
        // `count` that disagrees with the occupancy flags, so a consistent
        // map cannot reach `count == 0` here. Should the header still be
        // corrupted post-construction, an unchecked `count - 1` would
        // underflow — debug-panic, or wrap to `usize::MAX` in release.
        // `count` is a reporting value only (slot access is flag-gated), so
        // saturating is both safe and correct.
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
        // `from_bytes` now rejects a count/occupancy mismatch at
        // construction, so this corruption is injected afterwards
        // (defense in depth for the mutation-site guard). Slot 0 is
        // occupied (a real element removal succeeds its flag/generation
        // gate) but the `count` header is forced to 0, so the decrement
        // would underflow. It must saturate, not panic or wrap.
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
        // `from_bytes` now rejects a count/occupancy mismatch at
        // construction, so this corruption is injected afterwards
        // (defense in depth for the mutation-site guard). A crafted
        // `count == capacity` with a genuinely free slot must not push
        // the reported length past capacity when that slot is filled.
        let mut buf = [0u8; 8 + (8 + 8) * 2]; // capacity 2
        let mut map = SlotMap::<WireU64>::from_bytes(&mut buf).unwrap();
        map.set_count(map.capacity()); // lie: "full", but both slots are free
        let _ = map.insert(WireU64::new(7)).unwrap();
        assert!(map.count() <= map.capacity());
    }

    /// Capacity-2 WireU64 map: header (8) + 2 slots of (8 overhead + 8).
    const CAP2_LEN: usize = 8 + (8 + 8) * 2;
    /// Occupied-flag byte offset of slot `i` (header + i*slot_size + 4).
    const fn occ_off(i: usize) -> usize {
        8 + i * 16 + 4
    }

    #[test]
    fn from_bytes_rejects_count_higher_than_occupancy() {
        // count = 1 (<= capacity, so it passes the geometry check) but
        // no slot has its occupied flag set: corrupt header, rejected.
        let mut buf = [0u8; CAP2_LEN];
        buf[0..4].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(
            SlotMap::<WireU64>::from_bytes(&mut buf),
            Err(ProgramError::InvalidAccountData)
        ));
    }

    #[test]
    fn from_bytes_rejects_count_lower_than_occupancy() {
        // count = 0 but slot 1's occupied flag is set: corrupt header,
        // rejected.
        let mut buf = [0u8; CAP2_LEN];
        buf[occ_off(1)] = 1;
        assert!(matches!(
            SlotMap::<WireU64>::from_bytes(&mut buf),
            Err(ProgramError::InvalidAccountData)
        ));
    }

    #[test]
    fn from_bytes_accepts_consistent_count_and_occupancy() {
        // count = 2 with both occupied flags set: consistent, accepted,
        // and the parsed map reports the stored count.
        let mut buf = [0u8; CAP2_LEN];
        buf[0..4].copy_from_slice(&2u32.to_le_bytes());
        buf[occ_off(0)] = 1;
        buf[occ_off(1)] = 1;
        let map = SlotMap::<WireU64>::from_bytes(&mut buf).unwrap();
        assert_eq!(map.count(), 2);
        assert_eq!(map.capacity(), 2);

        // And the empty (all-zero) buffer is consistent too.
        let mut zeroed = [0u8; CAP2_LEN];
        let map = SlotMap::<WireU64>::from_bytes(&mut zeroed).unwrap();
        assert_eq!(map.count(), 0);
    }

    proptest::proptest! {
        /// Constructive both-ways property: build a well-formed buffer
        /// with independently chosen `count` header and occupied-flag
        /// bytes, and assert `from_bytes` accepts it IFF the stored
        /// count equals the number of nonzero flags. (A fully random
        /// byte strategy is vacuous here — a random header matches its
        /// flag popcount with probability ~2^-32, so the accept branch
        /// would never execute and deleting the reconciliation loop
        /// would go unnoticed.)
        #[test]
        fn from_bytes_only_accepts_reconciled_counts(
            capacity in 0usize..8,
            flags in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..8),
            count in 0u32..10,
        ) {
            // Header (8) + capacity slots of (8 overhead + 8 value).
            let mut buf = std::vec![0u8; 8 + capacity * 16];
            buf[0..4].copy_from_slice(&count.to_le_bytes());
            let mut occupied = 0u32;
            for i in 0..capacity {
                let flag = *flags.get(i).unwrap_or(&0);
                buf[occ_off(i)] = flag;
                if flag != 0 {
                    occupied += 1;
                }
            }
            let parsed = SlotMap::<WireU64>::from_bytes(&mut buf);
            if count == occupied {
                let map = parsed.expect("consistent header must parse");
                proptest::prop_assert_eq!(map.count(), occupied as usize);
                proptest::prop_assert_eq!(map.capacity(), capacity);
            } else {
                // Rejected by the count/occupancy reconciliation (or by
                // the count<=capacity geometry check when count is also
                // past capacity).
                proptest::prop_assert!(parsed.is_err());
            }
        }
    }
}
