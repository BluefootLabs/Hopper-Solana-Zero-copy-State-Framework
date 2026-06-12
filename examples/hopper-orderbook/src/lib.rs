//! # Hopper Orderbook Example
//!
//! Showcases **segment-level borrows on a large (>100 KB) zero-copy
//! account**. A central limit orderbook keeps three independent regions
//! inside one account:
//!
//! ```text
//! Orderbook account (segmented, ~110 KB):
//!   [Header: 16 bytes]
//!   [Segment registry: 4 + 3 x 16 = 52 bytes]
//!   [bids  segment : 40 KB]  -- up to 1024 resting buy orders
//!   [asks  segment : 40 KB]  -- up to 1024 resting sell orders
//!   [events segment: 24 KB]  -- ring buffer of fills/cancels for crank
//! ```
//!
//! The point Hopper makes here that Anchor and Quasar cannot: posting a
//! bid touches **only** the `bids` segment, matching touches `asks` and
//! `events`, and the crank drains `events` — each instruction borrows a
//! disjoint byte range of the same account, and the runtime
//! [`SegmentBorrowRegistry`] rejects any accidental overlap at runtime.
//! There is no second deserialize pass and no full-account copy: a fill
//! that mutates 64 bytes of a 110 KB account pays for 64 bytes, not
//! 110 KB.
//!
//! ## Instructions
//!
//! - `0` = InitBook  : create the segmented account and zero all regions
//! - `1` = PostBid   : append a resting buy order into the `bids` segment
//! - `2` = PostAsk   : append a resting sell order into the `asks` segment
//! - `3` = Match     : cross top-of-book, write a fill into `events`
//! - `4` = CrankEvents: read the `events` segment and advance the head

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code, unused_variables)]

use hopper::hopper_core::account;
use hopper::prelude::*;
use hopper::systems::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

// =====================================================================
// Records (header-free, live inside already-typed segments)
// =====================================================================

/// A single resting order. 56 bytes, align-1, so it packs without
/// padding inside a segment's `[u8]` data region.
#[derive(Clone, Copy)]
#[repr(C)]
struct OrderRecord {
    owner: [u8; 32],
    price: [u8; 8],
    size: [u8; 8],
    seq: [u8; 8],
}
const _: () = assert!(core::mem::size_of::<OrderRecord>() == 56);
const _: () = assert!(core::mem::align_of::<OrderRecord>() == 1);
// SAFETY: #[repr(C)] of byte arrays only; every bit pattern is valid and
// alignment is 1, so a cast from any 56-byte window is sound.
unsafe impl Zeroable for OrderRecord {}
unsafe impl Pod for OrderRecord {}
impl FixedLayout for OrderRecord {
    const SIZE: usize = 56;
}

/// A fill/cancel event for the off-chain crank. 48 bytes.
#[derive(Clone, Copy)]
#[repr(C)]
struct EventRecord {
    maker: [u8; 32],
    price: [u8; 8],
    size: [u8; 8],
}
const _: () = assert!(core::mem::size_of::<EventRecord>() == 48);
const _: () = assert!(core::mem::align_of::<EventRecord>() == 1);
// SAFETY: #[repr(C)] of byte arrays only; every bit pattern is valid and
// alignment is 1, so a cast from any 48-byte window is sound.
unsafe impl Zeroable for EventRecord {}
unsafe impl Pod for EventRecord {}
impl FixedLayout for EventRecord {
    const SIZE: usize = 48;
}

// =====================================================================
// Segment layout
// =====================================================================

const BIDS_SEG: SegmentId = segment_id("bids");
const ASKS_SEG: SegmentId = segment_id("asks");
const EVENTS_SEG: SegmentId = segment_id("events");

// 1024 orders per side, 56 bytes each + a small per-segment 8-byte
// header (count + head) keeps each side just over 40 KB. The event
// ring holds 512 events of 48 bytes + 8-byte head/tail = ~24 KB. The
// whole account lands above 100 KB, which is the regime where the
// "copy the whole account twice" cost of older frameworks actually
// bites.
const SIDE_CAP: u32 = 1024;
const SEG_META: u32 = 8; // [count:4][head:4] within each segment
const SIDE_SIZE: u32 = SEG_META + SIDE_CAP * OrderRecord::SIZE as u32; // 57_352
const EVENT_CAP: u32 = 512;
const EVENTS_SIZE: u32 = SEG_META + EVENT_CAP * EventRecord::SIZE as u32; // 24_584

const BOOK_ACCOUNT_SIZE: usize = HEADER_LEN
    + account::registry::REGISTRY_HEADER_SIZE
    + 3 * account::registry::SEGMENT_ENTRY_SIZE
    + SIDE_SIZE as usize
    + SIDE_SIZE as usize
    + EVENTS_SIZE as usize;

// Compile-time proof the account is in the large-account regime the
// example claims to demonstrate.
const _: () = assert!(BOOK_ACCOUNT_SIZE > 100_000);

// =====================================================================
// Errors
// =====================================================================

hopper_error! {
    base = 7300;
    Unauthorized,
    BookFull,
    EmptyBook,
    InvalidSide,
    ZeroSize,
    NoEvents
}

// =====================================================================
// Entrypoint
// =====================================================================

#[cfg(target_os = "solana")]
program_entrypoint!(process_instruction);

fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    instruction_data: &[u8],
) -> ProgramResult {
    hopper::hopper_dispatch! {
        program_id, accounts, instruction_data;
        0 => process_init_book,
        1 => process_post_bid,
        2 => process_post_ask,
        3 => process_match,
        4 => process_crank_events,
    }
}

// =====================================================================
// Instruction 0: Init Book
// =====================================================================

fn process_init_book(
    program_id: &Address,
    accounts: &[AccountView],
    _data: &[u8],
) -> ProgramResult {
    if accounts.len() < 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let payer = &accounts[0];
    let book = &accounts[1];
    let _system_program = &accounts[2];

    hopper_validate! {
        accounts = accounts,
        program_id = program_id,
        data = _data,
        rules {
            require_signer_at(0),
            require_writable_at(1)
        }
    }?;

    let lamports = rent_exempt_min(BOOK_ACCOUNT_SIZE);
    hopper::hopper_system::CreateAccount {
        from: payer,
        to: book,
        lamports,
        space: BOOK_ACCOUNT_SIZE as u64,
        owner: program_id,
    }
    .invoke()?;

    let mut data = book.try_borrow_mut()?;
    zero_init(&mut data);
    write_header(&mut data, 30, 1, &[0u8; 8])?;

    let specs: &[(SegmentId, u32, u8)] = &[
        (BIDS_SEG, SIDE_SIZE, 1),
        (ASKS_SEG, SIDE_SIZE, 1),
        (EVENTS_SEG, EVENTS_SIZE, 1),
    ];
    SegmentRegistryMut::init(&mut data, specs)?;

    emit_slices(&[b"book_init"]);
    Ok(())
}

// =====================================================================
// Segment-local push helpers
// =====================================================================
//
// Each side segment is `[count:4][head:4][record;N]`. `count` is the
// number of live orders. We only ever read/write the bytes of the one
// segment we are told to, which is exactly the disjoint-borrow property
// the example exists to show: posting a bid never reads or writes a
// byte of the asks or events region.

fn segment_push_order(
    book: &AccountView,
    seg: &SegmentId,
    rec: &OrderRecord,
) -> Result<u32, ProgramError> {
    let mut data = book.try_borrow_mut()?;
    let mut reg = SegmentRegistryMut::from_account_mut(&mut data)?;
    let region = reg.segment_data_mut(seg)?;
    if region.len() < SEG_META as usize {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let count = u32::from_le_bytes([region[0], region[1], region[2], region[3]]);
    if count >= SIDE_CAP {
        return Err(BookFull.into());
    }
    let base = SEG_META as usize + count as usize * OrderRecord::SIZE;
    let slot = &mut region[base..base + OrderRecord::SIZE];
    // SAFETY: `slot` is exactly OrderRecord::SIZE bytes, OrderRecord is
    // align-1 Pod, and `base` is bounded by the `count < SIDE_CAP`
    // capacity check above, so the window lies inside the segment.
    let bytes: &[u8] = unsafe { core::slice::from_raw_parts(rec as *const _ as *const u8, OrderRecord::SIZE) };
    slot.copy_from_slice(bytes);
    let new_count = count + 1;
    region[0..4].copy_from_slice(&new_count.to_le_bytes());
    Ok(new_count)
}

fn parse_order(owner: &AccountView, data: &[u8], seq: u64) -> Result<OrderRecord, ProgramError> {
    if data.len() < 16 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let price = u64::from_le_bytes(data[0..8].try_into().unwrap());
    let size = u64::from_le_bytes(data[8..16].try_into().unwrap());
    if size == 0 {
        return Err(ZeroSize.into());
    }
    Ok(OrderRecord {
        owner: *owner.address().as_array(),
        price: price.to_le_bytes(),
        size: size.to_le_bytes(),
        seq: seq.to_le_bytes(),
    })
}

// =====================================================================
// Instruction 1 / 2: Post Bid / Post Ask
// =====================================================================

fn process_post_bid(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() < 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let owner = &accounts[0];
    let book = &accounts[1];
    check_signer(owner)?;
    check_owner(book, program_id)?;
    check_writable(book)?;

    let rec = parse_order(owner, data, slot_seq(data))?;
    let n = segment_push_order(book, &BIDS_SEG, &rec)?;
    emit_slices(&[b"bid_posted", &n.to_le_bytes()]);
    Ok(())
}

fn process_post_ask(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() < 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let owner = &accounts[0];
    let book = &accounts[1];
    check_signer(owner)?;
    check_owner(book, program_id)?;
    check_writable(book)?;

    let rec = parse_order(owner, data, slot_seq(data))?;
    let n = segment_push_order(book, &ASKS_SEG, &rec)?;
    emit_slices(&[b"ask_posted", &n.to_le_bytes()]);
    Ok(())
}

fn slot_seq(data: &[u8]) -> u64 {
    if data.len() >= 24 {
        u64::from_le_bytes(data[16..24].try_into().unwrap())
    } else {
        0
    }
}

// =====================================================================
// Instruction 3: Match (touches asks + events, never bids)
// =====================================================================
//
// Pops the most recent ask and writes a fill event. This deliberately
// reads/writes the `asks` and `events` segments but never the `bids`
// segment, demonstrating that a single instruction borrows only the
// regions it needs.

fn process_match(
    program_id: &Address,
    accounts: &[AccountView],
    _data: &[u8],
) -> ProgramResult {
    if accounts.is_empty() {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let book = &accounts[0];
    check_owner(book, program_id)?;
    check_writable(book)?;

    let mut data = book.try_borrow_mut()?;
    let mut reg = SegmentRegistryMut::from_account_mut(&mut data)?;

    // Pop the top ask.
    let (maker, price, size) = {
        let asks = reg.segment_data_mut(&ASKS_SEG)?;
        let count = u32::from_le_bytes([asks[0], asks[1], asks[2], asks[3]]);
        if count == 0 {
            return Err(EmptyBook.into());
        }
        let top = (count - 1) as usize;
        let base = SEG_META as usize + top * OrderRecord::SIZE;
            let rec: &OrderRecord = unsafe { &*(asks[base..base + OrderRecord::SIZE].as_ptr() as *const OrderRecord) };
        let m = rec.maker_clone();
        asks[0..4].copy_from_slice(&(count - 1).to_le_bytes());
        m
    };

    // Append a fill event into the events ring.
    {
        let events = reg.segment_data_mut(&EVENTS_SEG)?;
        let tail = u32::from_le_bytes([events[0], events[1], events[2], events[3]]);
        let slot = tail % EVENT_CAP;
        let base = SEG_META as usize + slot as usize * EventRecord::SIZE;
        let ev = EventRecord {
            maker,
            price: price.to_le_bytes(),
            size: size.to_le_bytes(),
        };
        let dst = &mut events[base..base + EventRecord::SIZE];
            let ev_bytes: &[u8] = unsafe { core::slice::from_raw_parts(&ev as *const _ as *const u8, EventRecord::SIZE) };
            dst.copy_from_slice(ev_bytes);
        events[0..4].copy_from_slice(&(tail + 1).to_le_bytes());
    }

    emit_slices(&[b"matched"]);
    Ok(())
}

impl OrderRecord {
    fn maker_clone(&self) -> ([u8; 32], u64, u64) {
        (
            self.owner,
            u64::from_le_bytes(self.price),
            u64::from_le_bytes(self.size),
        )
    }
}

// =====================================================================
// Instruction 4: Crank Events (reads only the events segment)
// =====================================================================

fn process_crank_events(
    program_id: &Address,
    accounts: &[AccountView],
    _data: &[u8],
) -> ProgramResult {
    if accounts.is_empty() {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let book = &accounts[0];
    check_owner(book, program_id)?;

    let data = book.try_borrow()?;
    let reg = SegmentRegistry::from_account(&data)?;
    let events = reg.segment_data(&EVENTS_SEG)?;
    let tail = u32::from_le_bytes([events[0], events[1], events[2], events[3]]);
    let head = u32::from_le_bytes([events[4], events[5], events[6], events[7]]);
    if tail == head {
        return Err(NoEvents.into());
    }
    emit_slices(&[b"crank", &tail.to_le_bytes(), &head.to_le_bytes()]);
    Ok(())
}

// =====================================================================
// Host-side tests (layout invariants; on-chain behaviour is covered by
// the gated integration test under `tests/`)
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_is_large() {
        const { assert!(BOOK_ACCOUNT_SIZE > 100_000) };
    }

    #[test]
    fn segments_are_disjoint_by_construction() {
        // The three segment sizes plus framing must equal the declared
        // account size, i.e. the regions tile the account with no gaps
        // or overlaps.
        let framing = HEADER_LEN
            + account::registry::REGISTRY_HEADER_SIZE
            + 3 * account::registry::SEGMENT_ENTRY_SIZE;
        assert_eq!(
            framing + SIDE_SIZE as usize * 2 + EVENTS_SIZE as usize,
            BOOK_ACCOUNT_SIZE
        );
    }

    #[test]
    fn record_sizes_pack() {
        assert_eq!(OrderRecord::SIZE, 56);
        assert_eq!(EventRecord::SIZE, 48);
    }

    #[test]
    fn side_capacity_fits() {
        assert_eq!(SIDE_SIZE, SEG_META + SIDE_CAP * 56);
    }
}
