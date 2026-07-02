//! Zero-copy collections for on-chain account data.
//!
//! All collections operate directly on byte slices -- no heap allocation,
//! no `Vec`, no `Box`. They are BPF-safe, deterministic, and audit-friendly.
//!
//! ## Available Collections
//!
//! - [`FixedVec`] -- Bounded dynamic array with push/pop/swap_remove
//! - [`RingBuffer`] -- Fixed-capacity circular buffer for journals/logs
//! - [`SlotMap`] -- Fixed-slot map with generation counters for safe handles
//! - [`BitSet`] -- Compact bit array for flags and bitmask operations

mod bit_set;
mod fixed_vec;
mod ring_buffer;
mod slot_map;
mod sorted_vec;

pub use bit_set::BitSet;
pub use fixed_vec::FixedVec;
pub use ring_buffer::RingBuffer;
pub use slot_map::SlotMap;
pub use sorted_vec::SortedVec;

mod packed_map;
pub use packed_map::PackedMap;

pub mod journal;
pub use journal::{Journal, JournalReader, JOURNAL_HEADER_SIZE};

pub mod slab;
pub use slab::{bitmap_bytes, Slab, SLAB_HEADER_SIZE};

pub mod compact_tail;
pub use compact_tail::{CompactTail, TailBitSet, TailRing, TailSlab, TailVec};

// ── Zero-copy element invariant (compile-time) ───────────────────────
//
// Every collection here does its slot-offset and capacity math in units
// of `FixedLayout::SIZE`, but the actual element read/write moves
// `size_of::<T>()` bytes (`read_unaligned`/`write_unaligned(ptr as *T)`,
// or a `T::SIZE` copy). Those two counts are equal only by convention:
// `FixedLayout::SIZE` is a hand-written associated const with no
// language-level tie to the real type size. Two soundness holes hide in
// that gap, and neither is reachable by the byte-fuzzing harness below
// because both are properties of the *type parameter*, not the bytes:
//
//   1. A `FixedLayout` impl whose `SIZE` disagrees with `size_of::<T>()`
//      makes the bounds math (which trusts `SIZE`) disagree with the
//      write width (`size_of`), so a last-slot write can run past what
//      the capacity check proved in-bounds — an out-of-bounds write.
//   2. A zero-sized element (`SIZE == 0`) turns `capacity() = len / SIZE`
//      into a divide-by-zero panic.
//
// This const asserts the invariant that closes both. Invoked as
// `const { assert_zero_copy_element::<T>() }` in every constructor, it
// is evaluated per monomorphization at compile time — a mismatched or
// zero-sized element is a build error, at zero runtime cost. For every
// real wire type `SIZE == size_of` and `> 0`, so conforming code is
// unaffected.
#[inline(always)]
pub(crate) const fn assert_zero_copy_element<T: crate::account::Pod + crate::account::FixedLayout>()
{
    // The `SIZE == size_of` half now lives in the trait itself
    // (`FixedLayout::_SIZE_IS_HONEST`); touching it here forces that
    // trait-level proof to be evaluated for `T`. A dishonest `SIZE`
    // override is a build error at this point, for every collection and
    // every element type, without each author re-writing the assertion.
    let () = <T as crate::account::FixedLayout>::_SIZE_IS_HONEST;
    // The trait proof allows a zero-sized honest type (`SIZE == 0`); the
    // collections additionally forbid it, since `capacity() = len / SIZE`
    // divides by it.
    assert!(
        T::SIZE > 0,
        "zero-copy collection element type must not be zero-sized",
    );
}

// ── I13: adversarial-metadata property harness ───────────────────────
//
// Every collection above is a zero-copy overlay on account bytes, and
// account bytes are attacker-writable between instructions: lengths,
// heads, counts, and free lists are all UNTRUSTED INPUT every time a
// handler touches them. The Batch 3 audit found five point instances of
// the same root cause (trusted stored metadata → out-of-bounds access
// or panic); this harness generalizes the fix into a standing property:
//
//   For ARBITRARY buffer contents and ANY sequence of API calls, every
//   collection either succeeds within bounds or returns a clean `Err` —
//   it never panics and never touches memory outside its slice.
//
// Panics surface directly as proptest failures; out-of-bounds via the
// safe-index paths would panic too, and the `unsafe` paths are guarded
// by the bounds established before them (that is exactly what the
// audit fixes pinned). No competing framework ships hardened on-chain
// collections at all, let alone corruption-fuzzed ones.
#[cfg(test)]
mod hostile_metadata_proptests {
    use super::slot_map::SlotKey;
    use super::*;
    use crate::abi::WireU64;
    use proptest::prelude::*;
    use std::vec::Vec;

    // Arbitrary small-ish buffers: large enough to exercise multi-slot
    // layouts, small enough to keep case counts high. Contents are
    // fully arbitrary — headers, counts, and free lists included.
    fn buf_strategy() -> impl Strategy<Value = Vec<u8>> {
        proptest::collection::vec(any::<u8>(), 0..256)
    }

    proptest! {
        #[test]
        fn fixed_vec_survives_hostile_bytes(mut buf in buf_strategy(), idx in 0usize..300) {
            if let Ok(mut v) = FixedVec::<WireU64>::from_bytes(&mut buf) {
                let _ = v.get(idx);
                let _ = v.push(WireU64::new(u64::MAX));
                let _ = v.pop();
                let _ = v.swap_remove(idx);
                v.clear();
            }
        }

        #[test]
        fn sorted_vec_survives_hostile_bytes(mut buf in buf_strategy(), idx in 0usize..300) {
            if let Ok(mut v) = SortedVec::<WireU64>::from_bytes(&mut buf) {
                let _ = v.binary_search(&WireU64::new(42));
                let _ = v.insert(WireU64::new(7));
                let _ = v.remove(idx);
                let _ = v.max();
            }
        }

        #[test]
        fn packed_map_survives_hostile_bytes(mut buf in buf_strategy()) {
            if let Ok(mut m) = PackedMap::<WireU64, WireU64>::from_bytes(&mut buf) {
                let k = WireU64::new(3);
                let _ = m.get(&k);
                let _ = m.insert(k, WireU64::new(9));
                let _ = m.remove(&k);
                m.clear();
            }
        }

        #[test]
        fn ring_buffer_survives_hostile_bytes(mut buf in buf_strategy(), idx in 0usize..300) {
            if let Ok(mut r) = RingBuffer::<WireU64>::from_bytes(&mut buf) {
                let _ = r.push(WireU64::new(1));
                let _ = r.get(idx);
                let _ = r.latest();
                let _ = r.oldest();
            }
        }

        #[test]
        fn journal_survives_hostile_bytes(mut buf in buf_strategy(), idx in 0usize..300) {
            if let Ok(mut j) = Journal::<WireU64>::from_bytes_mut(&mut buf) {
                let _ = j.append(WireU64::new(5));
                let _ = j.read(idx);
                let _ = j.latest();
            }
        }

        #[test]
        fn slab_survives_hostile_bytes(mut buf in buf_strategy(), idx in 0u32..300) {
            if let Ok(mut s) = Slab::<WireU64>::from_bytes_mut(&mut buf) {
                let _ = s.alloc(WireU64::new(8));
                let _ = s.get(idx);
                let _ = s.get_mut(idx);
                let _ = s.set(idx, WireU64::new(11));
                let _ = s.free(idx);
                // A second alloc exercises next-free pointers read from
                // the (arbitrary) slot bytes.
                let _ = s.alloc(WireU64::new(9));
            }
        }

        #[test]
        fn slot_map_survives_hostile_bytes(mut buf in buf_strategy(), idx in 0u32..300, generation in any::<u32>()) {
            if let Ok(mut m) = SlotMap::<WireU64>::from_bytes(&mut buf) {
                let key = SlotKey { index: idx, generation };
                let _ = m.get(key);
                let _ = m.insert(WireU64::new(2));
                let _ = m.remove(key);
            }
        }

        #[test]
        fn bit_set_survives_hostile_bytes(mut buf in buf_strategy(), idx in 0usize..3000) {
            let mut b = BitSet::from_bytes(&mut buf);
            let _ = b.get(idx);
            let _ = b.set(idx);
            let _ = b.toggle(idx);
            let _ = b.check_flags(idx, 0b1010_1010);
            let _ = b.count_ones();
        }
    }
}
