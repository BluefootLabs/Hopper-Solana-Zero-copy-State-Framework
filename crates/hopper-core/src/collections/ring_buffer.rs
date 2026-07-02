//! Fixed-capacity circular buffer for journals, event logs, and queues.
//!
//! Wire layout:
//! ```text
//! [head: u32 LE][count: u32 LE][element 0][element 1]...[element capacity-1]
//! ```
//!
//! Elements wrap around when the buffer is full. Oldest elements are overwritten.

use crate::account::{FixedLayout, Pod};
use hopper_runtime::error::ProgramError;

/// Header: 4 bytes head + 4 bytes count = 8 bytes.
const RING_HEADER: usize = 8;

/// Fixed-capacity circular buffer overlaid on a byte slice.
///
/// - `push` always succeeds. When full, overwrites the oldest entry.
/// - Use for event journals, audit logs, price history, etc.
/// - O(1) push, O(1) read by logical index.
pub struct RingBuffer<'a, T: Pod + FixedLayout> {
    data: &'a mut [u8],
    /// Element capacity, computed and cached once at construction. Like
    /// [`Slab`](super::slab::Slab) and [`Journal`](super::journal::Journal),
    /// the geometry is derived from `data.len()` a single time rather
    /// than re-divided on every call.
    capacity: usize,
    _phantom: core::marker::PhantomData<T>,
}

impl<'a, T: Pod + FixedLayout> RingBuffer<'a, T> {
    /// Overlay a RingBuffer on a mutable byte slice.
    ///
    /// **Parse, don't validate.** The stored `head` and `count` come
    /// from account bytes, which are untrusted between instructions.
    /// Rather than re-defend that invariant in every method (the pattern
    /// that let earlier bugs slip through when one method forgot its
    /// guard), this validates the geometry *once*, here: a buffer whose
    /// header claims `head >= capacity` or `count > capacity` is
    /// **rejected** with `InvalidAccountData`. Every method then operates
    /// on a proven-consistent view — the mutators (`set_head` via
    /// `% cap`, `set_count` via clamp) preserve the invariant, and since
    /// the handler holds `&mut [u8]` exclusively the bytes cannot change
    /// underneath the view. Corrupt input fails fast and loud at the
    /// boundary instead of silently clamping mid-operation.
    #[inline]
    pub fn from_bytes(data: &'a mut [u8]) -> Result<Self, ProgramError> {
        const { super::assert_zero_copy_element::<T>() };
        if data.len() < RING_HEADER {
            return Err(ProgramError::AccountDataTooSmall);
        }
        let capacity = (data.len() - RING_HEADER) / T::SIZE;
        let head = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let count = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
        // A valid ring has `head` in `0..capacity` (or 0 when empty) and
        // `count` in `0..=capacity`. A zeroed (fresh) account satisfies
        // both. `capacity == 0` makes any nonzero head/count inconsistent
        // and an empty ring the only valid state.
        if head > capacity || count > capacity || (capacity == 0 && (head != 0 || count != 0)) {
            return Err(ProgramError::InvalidAccountData);
        }
        // `head == capacity` is only reachable for a just-wrapped write
        // position that was never renormalized; fold it to 0 so the
        // invariant `head < capacity` (for capacity > 0) holds exactly.
        let head = if capacity > 0 { head % capacity } else { 0 };
        data[0..4].copy_from_slice(&(head as u32).to_le_bytes());
        Ok(Self {
            data,
            capacity,
            _phantom: core::marker::PhantomData,
        })
    }

    /// Maximum capacity (number of elements).
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Current number of elements. May be less than capacity before wraparound.
    ///
    /// Reads the validated count field. `from_bytes` proved
    /// `count <= capacity` and every mutator preserves it, so this needs
    /// no clamp — the value is trusted by construction.
    #[inline(always)]
    pub fn count(&self) -> usize {
        let bytes = [self.data[4], self.data[5], self.data[6], self.data[7]];
        u32::from_le_bytes(bytes) as usize
    }

    /// Head pointer (index of the next write position). Trusted `< capacity`
    /// (or 0 when `capacity == 0`) by the `from_bytes` invariant.
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

        // `head < cap` is an invariant established by `from_bytes` and
        // preserved by `set_head` (`% cap`). This is a cheap backstop
        // assertion, not the primary defense: an invalid `head` was
        // already rejected at construction, so this branch is
        // unreachable for a validly-built ring.
        let head = self.head();
        if head >= cap {
            return Err(ProgramError::InvalidAccountData);
        }
        let offset = self.slot_offset(head);

        // Prove the write's *actual* precondition locally, not by
        // convention: `write_unaligned::<T>` touches
        // `offset .. offset + size_of::<T>()`, so that exact range — not
        // the logical `head < cap` — is what must be in-bounds. Checking
        // it here makes the unsafe block self-evidently sound regardless
        // of the geometry math upstream (`assert_zero_copy_element`
        // already ties `SIZE == size_of`, but the direct check stands on
        // its own).
        match offset.checked_add(core::mem::size_of::<T>()) {
            Some(end) if end <= self.data.len() => {}
            _ => return Err(ProgramError::InvalidAccountData),
        }
        // SAFETY: `offset .. offset + size_of::<T>()` is within
        // `self.data` (checked directly above); `T: Pod` so any byte
        // pattern is a valid `T`, and `write_unaligned` imposes no
        // alignment requirement.
        unsafe {
            core::ptr::write_unaligned(self.data.as_mut_ptr().add(offset) as *mut T, value);
        }

        let new_head = (head + 1) % cap;
        self.set_head(new_head);

        // Preserve the `count <= cap` invariant `from_bytes` established:
        // `min(count + 1, cap)` grows toward capacity and saturates there
        // (full-ring overwrite keeps count == cap). Writing on every push
        // — not only when `count < cap` — also self-heals should the
        // field ever drift, so the stored count stays a valid `1..=cap`.
        let count = self.count();
        self.set_count((count + 1).min(cap));

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
        // `head` is the next-write cursor, so the oldest live element is
        // at `(head - count) mod cap`; logical index 0 is the oldest.
        // `from_bytes` proved `head < cap` and `count <= cap`, so the
        // else-branch subtraction cannot underflow. (This is why `oldest`
        // returns the first-pushed element — verified by the
        // `ring_push_and_read` / `ring_wraps_around` tests.)
        let start = if head >= count {
            head - count
        } else {
            cap - (count - head)
        };
        let physical = (start + logical_index) % cap;
        let offset = self.slot_offset(physical);

        // Direct read precondition (see `push`): the exact touched range
        // `offset .. offset + size_of::<T>()` must be in-bounds. `% cap`
        // guarantees `physical < cap`, so this always holds for a valid
        // view, but the local check makes the unsafe block self-proving.
        match offset.checked_add(core::mem::size_of::<T>()) {
            Some(e) if e <= self.data.len() => {}
            _ => return Err(ProgramError::InvalidAccountData),
        }
        // SAFETY: `offset .. offset + size_of::<T>()` is within
        // `self.data` (checked directly above); `T: Pod`, unaligned read.
        Ok(unsafe { core::ptr::read_unaligned(self.data.as_ptr().add(offset) as *const T) })
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

    #[test]
    fn corrupt_head_is_rejected_at_construction() {
        // Parse-don't-validate: a header claiming head=9 on a 3-slot
        // ring is inconsistent geometry, refused at the boundary rather
        // than defended in every method. (Pre-refactor `push` guarded
        // this per-call; now the invalid view never exists.)
        let mut buf = [0u8; 8 + 4 * 3]; // capacity 3
        buf[0..4].copy_from_slice(&9u32.to_le_bytes());
        assert!(matches!(
            RingBuffer::<WireU32>::from_bytes(&mut buf),
            Err(ProgramError::InvalidAccountData)
        ));
    }

    #[test]
    fn corrupt_count_is_rejected_at_construction() {
        // A count larger than capacity is inconsistent geometry: the
        // whole ring is refused, so no method ever sees the poison (and
        // the `get` underflow the fuzz harness found can't arise).
        let mut buf = [0u8; 8 + 4 * 3]; // capacity 3
        buf[4..8].copy_from_slice(&99u32.to_le_bytes()); // count = 99
        assert!(matches!(
            RingBuffer::<WireU32>::from_bytes(&mut buf),
            Err(ProgramError::InvalidAccountData)
        ));
    }

    #[test]
    fn head_equal_to_capacity_is_normalized_at_construction() {
        // `head == capacity` (an un-renormalized wrap position) is folded
        // to 0 so the `head < capacity` invariant holds exactly, rather
        // than rejected — a zeroed-then-lightly-touched account stays
        // usable.
        let mut buf = [0u8; 8 + 4 * 3]; // capacity 3
        buf[0..4].copy_from_slice(&3u32.to_le_bytes()); // head = capacity
        let ring = RingBuffer::<WireU32>::from_bytes(&mut buf).unwrap();
        assert_eq!(ring.count(), 0);
        // Header was rewritten to the normalized head.
        assert_eq!(u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]), 0);
    }
}
