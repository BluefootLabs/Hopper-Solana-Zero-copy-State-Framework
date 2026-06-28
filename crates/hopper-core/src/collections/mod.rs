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
