//! Compact-tail bridge: overlay Hopper's zero-copy collections on the tail of
//! a compact dynamic account.
//!
//! A `#[hopper::state(compact, disc = N, dynamic)]` layout has the wire shape
//! `[disc:u8][fixed_head][tail]` -- no 16-byte header and no length prefix. The
//! `tail` region (everything after the fixed head) can be overlaid with any of
//! Hopper's zero-copy collections for O(1), cast-in-place element access. That
//! is the `[disc][fixed_head][tail_collection]` shape the competitive audit
//! flagged as the Quasar-class compact-dynamic story -- order books, event
//! rings, registries, slabs -- now first-class on a 1-byte header.
//!
//! The collections are re-exported here under the `Tail*` names used across the
//! audit and the Quasar comparison:
//!
//! | Tail alias       | Backing collection | Tail wire shape                  |
//! |------------------|--------------------|----------------------------------|
//! | [`TailVec<T>`]   | [`FixedVec`]       | `[len:u32][T; cap]`              |
//! | [`TailRing<T>`]  | [`RingBuffer`]     | `[head:u32][count:u32][T; cap]`  |
//! | [`TailSlab<T>`]  | [`Slab`]           | `[hdr:16][bitmap][T; cap]`       |
//! | [`TailBitSet`]   | [`BitSet`]         | packed bits                      |
//!
//! ```ignore
//! #[hopper::state(compact, disc = 8, dynamic)]
//! #[repr(C)]
//! struct Market { base_mint: TypedAddress<Mint>, quote_mint: TypedAddress<Mint> }
//!
//! // Allocate: disc + head + a 1024-order slab.
//! let space = Market::space_for_tail_slab::<Order>(1024);
//!
//! // Append a bid -- O(1), cast in place, no decode pass.
//! let mut data = account.try_borrow_mut()?;
//! let mut bids = Market::tail_slab::<Order>(&mut data)?;
//! let slot = bids.insert(order)?;
//! ```

use hopper_runtime::error::ProgramError;
use hopper_runtime::CompactDynamicLayout;

use crate::account::{FixedLayout, Pod};

use super::slab::{bitmap_bytes, SLAB_HEADER_SIZE};
use super::{BitSet, FixedVec, RingBuffer, Slab};

/// Growable zero-copy vector tail: `[len:u32][T; cap]`. Alias for [`FixedVec`].
pub type TailVec<'a, T> = FixedVec<'a, T>;
/// Ring-buffer tail (overwrites the oldest entry when full):
/// `[head:u32][count:u32][T; cap]`. Alias for [`RingBuffer`].
pub type TailRing<'a, T> = RingBuffer<'a, T>;
/// Slab tail with an occupancy bitmap and free list, for stable indices and
/// O(1) insert/remove. Alias for [`Slab`].
pub type TailSlab<'a, T> = Slab<'a, T>;
/// Packed-bit tail. Alias for [`BitSet`].
pub type TailBitSet<'a> = BitSet<'a>;

/// [`FixedVec`] header bytes (`len: u32`).
const VEC_HEADER: usize = 4;
/// [`RingBuffer`] header bytes (`head: u32` + `count: u32`).
const RING_HEADER: usize = 8;

/// Overlay Hopper's zero-copy collections on the tail region of a compact
/// dynamic account (`[disc][fixed_head][tail]`).
///
/// Blanket-implemented for every [`CompactDynamicLayout`], so any
/// `#[hopper::state(compact, dynamic)]` layout gets these helpers for free. The
/// tail region is the account bytes from
/// [`TAIL_OFFSET`](CompactDynamicLayout::TAIL_OFFSET) (the fixed compact
/// length) onward.
pub trait CompactTail: CompactDynamicLayout {
    /// Borrow the tail region of `data` (the bytes after the fixed head).
    #[inline]
    fn tail_region(data: &[u8]) -> Result<&[u8], ProgramError> {
        data.get(Self::TAIL_OFFSET..)
            .ok_or(ProgramError::AccountDataTooSmall)
    }

    /// Mutably borrow the tail region of `data`.
    #[inline]
    fn tail_region_mut(data: &mut [u8]) -> Result<&mut [u8], ProgramError> {
        let start = Self::TAIL_OFFSET;
        if data.len() < start {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(&mut data[start..])
    }

    /// Overlay a [`TailVec<E>`] on the tail region.
    #[inline]
    fn tail_vec<'d, E: Pod + FixedLayout>(
        data: &'d mut [u8],
    ) -> Result<TailVec<'d, E>, ProgramError> {
        FixedVec::from_bytes(Self::tail_region_mut(data)?)
    }

    /// Overlay a [`TailRing<E>`] on the tail region.
    #[inline]
    fn tail_ring<'d, E: Pod + FixedLayout>(
        data: &'d mut [u8],
    ) -> Result<TailRing<'d, E>, ProgramError> {
        RingBuffer::from_bytes(Self::tail_region_mut(data)?)
    }

    /// Overlay a [`TailSlab<E>`] on the tail region. The slab must already be
    /// initialized; call [`tail_slab_init`](Self::tail_slab_init) once on a
    /// fresh account first.
    #[inline]
    fn tail_slab<'d, E: Pod + FixedLayout>(
        data: &'d mut [u8],
    ) -> Result<TailSlab<'d, E>, ProgramError> {
        Slab::from_bytes_mut(Self::tail_region_mut(data)?)
    }

    /// Initialize a [`TailSlab<E>`] in the tail region with `capacity` slots
    /// (sets up the free list and clears the occupancy bitmap). Call once on a
    /// freshly-allocated, zeroed account.
    #[inline]
    fn tail_slab_init<E: Pod + FixedLayout>(
        data: &mut [u8],
        capacity: usize,
    ) -> Result<(), ProgramError> {
        Slab::<E>::init(Self::tail_region_mut(data)?, capacity)
    }

    /// Overlay a [`TailBitSet`] on the tail region.
    #[inline]
    fn tail_bitset(data: &mut [u8]) -> Result<TailBitSet<'_>, ProgramError> {
        Ok(BitSet::from_bytes(Self::tail_region_mut(data)?))
    }

    /// Account size for a compact account whose tail is a [`TailVec<E>`] with
    /// `capacity` elements: disc + fixed head + 4-byte length + `cap * E::SIZE`.
    #[inline]
    fn space_for_tail_vec<E: FixedLayout>(capacity: usize) -> usize {
        Self::TAIL_OFFSET + VEC_HEADER + capacity * E::SIZE
    }

    /// Account size for a compact account whose tail is a [`TailRing<E>`].
    #[inline]
    fn space_for_tail_ring<E: FixedLayout>(capacity: usize) -> usize {
        Self::TAIL_OFFSET + RING_HEADER + capacity * E::SIZE
    }

    /// Account size for a compact account whose tail is a [`TailSlab<E>`]:
    /// disc + fixed head + slab header + occupancy bitmap + `cap * E::SIZE`.
    #[inline]
    fn space_for_tail_slab<E: FixedLayout>(capacity: usize) -> usize {
        Self::TAIL_OFFSET + SLAB_HEADER_SIZE + bitmap_bytes(capacity) + capacity * E::SIZE
    }
}

impl<T: CompactDynamicLayout> CompactTail for T {}
