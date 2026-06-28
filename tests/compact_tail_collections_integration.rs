//! Compact dynamic accounts with **zero-copy collection tails**.
//!
//! Proves the `[disc:u8][fixed_head][tail_collection]` shape the competitive
//! audit identified as the Quasar-class compact-dynamic story: a 1-byte-header
//! account whose tail is a `TailSlab` / `TailRing` / `TailVec` with O(1),
//! cast-in-place element access (no decode/encode pass per operation).

#![cfg(all(feature = "proc-macros", feature = "collections"))]
#![allow(clippy::assertions_on_constants)]

use hopper::collections::CompactTail;
use hopper::prelude::*;
use hopper::systems::FixedLayout;

/// A compact dynamic head: just an authority. The tail is **program-managed**
/// (a zero-copy collection), declared via the `dynamic` flag.
#[derive(Copy, Clone)]
#[hopper::state(compact, disc = 8, dynamic)]
#[repr(C)]
pub struct Market {
    pub authority: [u8; 32],
}

/// A 48-byte resting order, align-1 so it packs without padding in a tail.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Order {
    owner: [u8; 32],
    price: [u8; 8],
    size: [u8; 8],
}
unsafe impl Zeroable for Order {}
unsafe impl Pod for Order {}
impl FixedLayout for Order {
    const SIZE: usize = 48;
}

/// A 16-byte fill event for the ring.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Fill {
    price: [u8; 8],
    size: [u8; 8],
}
unsafe impl Zeroable for Fill {}
unsafe impl Pod for Fill {}
impl FixedLayout for Fill {
    const SIZE: usize = 16;
}

fn order(owner: u8, price: u64, size: u64) -> Order {
    Order {
        owner: [owner; 32],
        price: price.to_le_bytes(),
        size: size.to_le_bytes(),
    }
}

#[test]
fn market_head_is_one_byte_disc_plus_fixed_head() {
    assert!(Market::HAS_DYNAMIC_TAIL);
    assert_eq!(Market::BODY_SIZE, 32);
    assert_eq!(Market::COMPACT_LEN, 33);
    // The tail region begins at 33 -- right after the 1-byte disc + head, NOT
    // after a 16-byte header.
    assert_eq!(<Market as hopper::account::CompactDynamicLayout>::TAIL_OFFSET, 33);
}

#[test]
fn compact_market_with_tail_slab_of_orders_keeps_stable_indices() {
    // disc(1) + head(32) + slab(capacity 4 of 48-byte Order).
    let space = Market::space_for_tail_slab::<Order>(4);
    let mut data = std::vec![0u8; space];
    data[0] = Market::DISC;

    // Initialize the slab once on the zeroed tail region.
    Market::tail_slab_init::<Order>(&mut data, 4).unwrap();

    let (k0, k1) = {
        let mut bids = Market::tail_slab::<Order>(&mut data).unwrap();
        let k0 = bids.alloc(order(1, 100, 5)).unwrap();
        let k1 = bids.alloc(order(2, 200, 7)).unwrap();
        assert_eq!(bids.count(), 2);
        (k0, k1)
    };

    // Re-overlay, as a later instruction would, and read back by index.
    let mut bids = Market::tail_slab::<Order>(&mut data).unwrap();
    assert_eq!(bids.get(k0).unwrap(), order(1, 100, 5));

    // Freeing k0 leaves k1's index stable -- the slab moat over a plain vec.
    bids.free(k0).unwrap();
    assert!(!bids.is_slot_allocated(k0));
    assert_eq!(bids.get(k1).unwrap(), order(2, 200, 7));
    assert_eq!(bids.count(), 1);
}

#[test]
fn compact_market_with_tail_ring_of_fills_overwrites_oldest() {
    let space = Market::space_for_tail_ring::<Fill>(2);
    let mut data = std::vec![0u8; space];
    data[0] = Market::DISC;

    let mut events = Market::tail_ring::<Fill>(&mut data).unwrap();
    assert_eq!(events.capacity(), 2);
    events
        .push(Fill {
            price: 1u64.to_le_bytes(),
            size: 10u64.to_le_bytes(),
        })
        .unwrap();
    events
        .push(Fill {
            price: 2u64.to_le_bytes(),
            size: 20u64.to_le_bytes(),
        })
        .unwrap();
    // Third push overwrites the oldest entry (ring semantics).
    events
        .push(Fill {
            price: 3u64.to_le_bytes(),
            size: 30u64.to_le_bytes(),
        })
        .unwrap();
    assert_eq!(events.count(), 2);
    // The oldest still in the buffer is now price=2 (price=1 was overwritten).
    let oldest = events.get(0).unwrap();
    assert_eq!(u64::from_le_bytes(oldest.price), 2);
}

#[test]
fn compact_market_with_tail_vec_push_pop() {
    let space = Market::space_for_tail_vec::<Order>(3);
    let mut data = std::vec![0u8; space];
    data[0] = Market::DISC;

    let mut v = Market::tail_vec::<Order>(&mut data).unwrap();
    assert_eq!(v.capacity(), 3);
    v.push(order(9, 1, 1)).unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v.get(0).unwrap(), order(9, 1, 1));
    assert_eq!(v.pop().unwrap(), order(9, 1, 1));
    assert_eq!(v.len(), 0);
}

#[test]
fn fixed_head_and_collection_tail_coexist() {
    let space = Market::space_for_tail_vec::<Order>(2);
    let mut data = std::vec![0u8; space];
    data[0] = Market::DISC;

    // Write the fixed head (authority sits at absolute offset 1, after disc).
    let auth_off = Market::AUTHORITY_ABS_OFFSET as usize;
    data[auth_off..auth_off + 32].copy_from_slice(&[7u8; 32]);

    // Push into the tail vec -- independent of the fixed head bytes.
    {
        let mut v = Market::tail_vec::<Order>(&mut data).unwrap();
        v.push(order(1, 0, 0)).unwrap();
    }

    // The fixed head is intact and zero-copy: overlay it on its sub-slice.
    let head = Market::overlay_body(&data[1..1 + Market::BODY_SIZE]).unwrap();
    assert_eq!(head.authority, [7u8; 32]);

    // The tail is intact too.
    let v = Market::tail_vec::<Order>(&mut data).unwrap();
    assert_eq!(v.len(), 1);
}
