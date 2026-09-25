//! A segmented order-storage demonstration, not a trading venue.
//!
//! A 139,356-byte account stores bids, asks, and a bounded event ring.
//! After validating its header and registry, handlers borrow only the
//! counters and records they use through `SegmentBorrowRegistry` guards.
//! Solana still locks and loads the account as a whole; touched bytes are
//! not a transaction-fee or compute-unit formula.
//!
//! Orders are uncollateralized records. Opcode 3 requires the latest ask's
//! maker to sign, removes that ask, and records a demonstration event. It
//! does not cross bids, transfer tokens, settle a trade, or enforce priority.
//! Opcode 4 permissionlessly logs one queued event and advances its head.

#![cfg_attr(target_os = "solana", no_std)]
#![allow(dead_code)]

use hopper::hopper_core::account;
use hopper::prelude::*;
use hopper::systems::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

#[derive(Clone, Copy)]
#[repr(C)]
struct OrderRecord {
    owner: [u8; 32],
    price: [u8; 8],
    size: [u8; 8],
    /// Caller-supplied label, not a trusted ordering or replay counter.
    seq: [u8; 8],
}
const _: () = assert!(core::mem::size_of::<OrderRecord>() == 56);
const _: () = assert!(core::mem::align_of::<OrderRecord>() == 1);
// SAFETY: repr(C), byte-array fields, alignment one, no padding or invalid bits.
unsafe impl Zeroable for OrderRecord {}
// SAFETY: repr(C), byte-array fields, alignment one, no padding or invalid bits.
unsafe impl Pod for OrderRecord {}
impl FixedLayout for OrderRecord {
    const SIZE: usize = 56;
}

/// An ask removed by its maker; this does not attest to a trade or payment.
#[derive(Clone, Copy)]
#[repr(C)]
struct EventRecord {
    maker: [u8; 32],
    price: [u8; 8],
    size: [u8; 8],
}
const _: () = assert!(core::mem::size_of::<EventRecord>() == 48);
const _: () = assert!(core::mem::align_of::<EventRecord>() == 1);
// SAFETY: repr(C), byte-array fields, alignment one, no padding or invalid bits.
unsafe impl Zeroable for EventRecord {}
// SAFETY: repr(C), byte-array fields, alignment one, no padding or invalid bits.
unsafe impl Pod for EventRecord {}
impl FixedLayout for EventRecord {
    const SIZE: usize = 48;
}

const BIDS_SEG: SegmentId = segment_id("bids");
const ASKS_SEG: SegmentId = segment_id("asks");
const EVENTS_SEG: SegmentId = segment_id("events");
const BOOK_LAYOUT: [u8; 8] = *b"OBDEMO01";
const SIDE_CAP: u32 = 1024;
const SEG_META: u32 = 8;
const SIDE_SIZE: u32 = SEG_META + SIDE_CAP * OrderRecord::SIZE as u32;
const EVENT_CAP: u32 = 512;
const EVENTS_SIZE: u32 = SEG_META + EVENT_CAP * EventRecord::SIZE as u32;
const PREFIX_SIZE: usize = HEADER_LEN
    + account::registry::REGISTRY_HEADER_SIZE
    + 3 * account::registry::SEGMENT_ENTRY_SIZE;
const BIDS_OFFSET: u32 = PREFIX_SIZE as u32;
const ASKS_OFFSET: u32 = BIDS_OFFSET + SIDE_SIZE;
const EVENTS_OFFSET: u32 = ASKS_OFFSET + SIDE_SIZE;
pub const BOOK_ACCOUNT_SIZE: usize = EVENTS_OFFSET as usize + EVENTS_SIZE as usize;
const _: () = assert!(BOOK_ACCOUNT_SIZE == 139_356);

hopper_error! {
    base = 7300;
    Unauthorized,
    BookFull,
    EmptyBook,
    InvalidSide,
    ZeroSize,
    NoEvents,
    ZeroPrice,
    EventQueueFull
}

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
        3 => process_record_ask_event,
        4 => process_crank_events,
    }
}

fn require_empty(data: &[u8]) -> ProgramResult {
    if data.is_empty() {
        Ok(())
    } else {
        Err(ProgramError::InvalidInstructionData)
    }
}

/// Initialize a zeroed, already allocated account. Its size exceeds Solana's
/// single-CPI growth limit, so allocation belongs in a preceding top-level
/// System instruction in the same transaction. Both payer and book sign.
fn process_init_book(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    require_empty(data)?;
    if accounts.len() != 3 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    check_signer(&accounts[0])?;
    let book = &accounts[1];
    check_signer(book)?;
    check_writable(book)?;
    check_owner(book, program_id)?;
    if accounts[2].address().as_array() != &[0; 32] {
        return Err(ProgramError::IncorrectProgramId);
    }
    if book.data_len() != BOOK_ACCOUNT_SIZE {
        return Err(ProgramError::InvalidAccountData);
    }
    // Never clear existing state: even a zero header with dirty trailing data
    // is refused. The book signature also prevents initialization squatting.
    {
        let bytes = book.try_borrow()?;
        // Compare bounded blocks so the SBF compiler can use word loads.
        // A byte-at-a-time scan of this large account exceeds the ordinary
        // instruction budget. The remainder check still covers every byte.
        let mut blocks = bytes.chunks_exact(32);
        if blocks.any(|block| block != [0; 32]) || blocks.remainder().iter().any(|byte| *byte != 0)
        {
            return Err(ProgramError::AccountAlreadyInitialized);
        }
    }
    let mut borrows = SegmentBorrowRegistry::new();
    let mut prefix = book.segment_mut::<[u8; PREFIX_SIZE]>(&mut borrows, 0, PREFIX_SIZE as u32)?;
    write_header(&mut prefix[..], 30, 1, &BOOK_LAYOUT)?;
    SegmentRegistryMut::init(
        &mut prefix[..],
        &[
            (BIDS_SEG, SIDE_SIZE, 1),
            (ASKS_SEG, SIDE_SIZE, 1),
            (EVENTS_SEG, EVENTS_SIZE, 1),
        ],
    )?;
    emit_slices(&[b"book_init"]);
    Ok(())
}

/// Authenticate this example's exact immutable layout before using offsets.
/// Registry lookup alone does not validate the account's type or disjointness.
fn validate_book(book: &AccountView, program_id: &Address) -> ProgramResult {
    check_owner(book, program_id)?;
    check_writable(book)?;
    if book.data_len() != BOOK_ACCOUNT_SIZE {
        return Err(ProgramError::InvalidAccountData);
    }
    let mut borrows = SegmentBorrowRegistry::new();
    let prefix = book.segment_ref::<[u8; PREFIX_SIZE]>(&mut borrows, 0, PREFIX_SIZE as u32)?;
    let mut expected_header = [0; HEADER_LEN];
    write_header(&mut expected_header, 30, 1, &BOOK_LAYOUT)?;
    if prefix[..HEADER_LEN] != expected_header || prefix[HEADER_LEN..HEADER_LEN + 4] != [3, 0, 0, 0]
    {
        return Err(ProgramError::InvalidAccountData);
    }
    let registry = SegmentRegistry::from_account(&prefix[..])?;
    for (i, (id, offset, size)) in [
        (BIDS_SEG, BIDS_OFFSET, SIDE_SIZE),
        (ASKS_SEG, ASKS_OFFSET, SIDE_SIZE),
        (EVENTS_SEG, EVENTS_OFFSET, EVENTS_SIZE),
    ]
    .iter()
    .enumerate()
    {
        let entry = registry.entry(i)?;
        if entry.id != *id
            || entry.offset() != *offset
            || entry.size() != *size
            || entry.flags() != 0
            || entry.version != 1
            || entry._reserved != 0
        {
            return Err(ProgramError::InvalidAccountData);
        }
    }
    Ok(())
}

fn side_count(book: &AccountView, offset: u32) -> Result<u32, ProgramError> {
    let mut borrows = SegmentBorrowRegistry::new();
    let meta = book.segment_ref::<[u8; 8]>(&mut borrows, offset, SEG_META)?;
    let count = u32::from_le_bytes(meta[..4].try_into().unwrap());
    if count > SIDE_CAP || meta[4..] != [0; 4] {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(count)
}

fn event_cursors(book: &AccountView) -> Result<(u32, u32), ProgramError> {
    let mut borrows = SegmentBorrowRegistry::new();
    let meta = book.segment_ref::<[u8; 8]>(&mut borrows, EVENTS_OFFSET, SEG_META)?;
    let tail = u32::from_le_bytes(meta[..4].try_into().unwrap());
    let head = u32::from_le_bytes(meta[4..].try_into().unwrap());
    if tail.wrapping_sub(head) > EVENT_CAP {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok((tail, head))
}

fn parse_order(owner: &AccountView, data: &[u8]) -> Result<OrderRecord, ProgramError> {
    if data.len() != 16 && data.len() != 24 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let price: [u8; 8] = data[..8].try_into().unwrap();
    let size: [u8; 8] = data[8..16].try_into().unwrap();
    if u64::from_le_bytes(price) == 0 {
        return Err(ZeroPrice.into());
    }
    if u64::from_le_bytes(size) == 0 {
        return Err(ZeroSize.into());
    }
    Ok(OrderRecord {
        owner: *owner.address().as_array(),
        price,
        size,
        seq: if data.len() == 24 {
            data[16..24].try_into().unwrap()
        } else {
            [0; 8]
        },
    })
}

fn process_post(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
    offset: u32,
) -> ProgramResult {
    if accounts.len() != 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let owner = &accounts[0];
    let book = &accounts[1];
    check_signer(owner)?;
    validate_book(book, program_id)?;
    let rec = parse_order(owner, data)?;
    let count = side_count(book, offset)?;
    if count == SIDE_CAP {
        return Err(BookFull.into());
    }
    let mut borrows = SegmentBorrowRegistry::new();
    *book.segment_mut::<OrderRecord>(
        &mut borrows,
        offset + SEG_META + count * OrderRecord::SIZE as u32,
        OrderRecord::SIZE as u32,
    )? = rec;
    *book.segment_mut::<[u8; 4]>(&mut borrows, offset, 4)? = (count + 1).to_le_bytes();
    emit_slices(&[
        if offset == BIDS_OFFSET {
            b"bid_posted"
        } else {
            b"ask_posted"
        },
        &(count + 1).to_le_bytes(),
    ]);
    Ok(())
}

fn process_post_bid(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    process_post(program_id, accounts, data, BIDS_OFFSET)
}

fn process_post_ask(program_id: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    process_post(program_id, accounts, data, ASKS_OFFSET)
}

/// Remove the newest ask only with its maker's signature. The event queue
/// must have room before any state is changed. No trade occurs here.
fn process_record_ask_event(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    require_empty(data)?;
    if accounts.len() != 2 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let maker = &accounts[0];
    let book = &accounts[1];
    check_signer(maker)?;
    validate_book(book, program_id)?;
    let count = side_count(book, ASKS_OFFSET)?;
    if count == 0 {
        return Err(EmptyBook.into());
    }
    let (tail, head) = event_cursors(book)?;
    if tail.wrapping_sub(head) == EVENT_CAP {
        return Err(EventQueueFull.into());
    }
    let mut borrows = SegmentBorrowRegistry::new();
    let rec = *book.segment_ref::<OrderRecord>(
        &mut borrows,
        ASKS_OFFSET + SEG_META + (count - 1) * OrderRecord::SIZE as u32,
        OrderRecord::SIZE as u32,
    )?;
    if rec.owner != *maker.address().as_array() {
        return Err(Unauthorized.into());
    }
    if u64::from_le_bytes(rec.price) == 0 || u64::from_le_bytes(rec.size) == 0 {
        return Err(ProgramError::InvalidAccountData);
    }
    *book.segment_mut::<EventRecord>(
        &mut borrows,
        EVENTS_OFFSET + SEG_META + (tail % EVENT_CAP) * EventRecord::SIZE as u32,
        EventRecord::SIZE as u32,
    )? = EventRecord {
        maker: rec.owner,
        price: rec.price,
        size: rec.size,
    };
    // Wrapping sequence numbers are intentional. Capacity is a power of two
    // dividing 2^32, so the slot index remains continuous across wraparound.
    *book.segment_mut::<[u8; 4]>(&mut borrows, EVENTS_OFFSET, 4)? =
        tail.wrapping_add(1).to_le_bytes();
    *book.segment_mut::<[u8; 4]>(&mut borrows, ASKS_OFFSET, 4)? = (count - 1).to_le_bytes();
    emit_slices(&[b"ask_event"]);
    Ok(())
}

/// Permissionless one-event drain. The event is logged before advancing the
/// head; old slot bytes remain until reused. Logs may be truncated and are
/// not a durable or authenticated settlement receipt.
fn process_crank_events(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    require_empty(data)?;
    if accounts.len() != 1 {
        return Err(ProgramError::NotEnoughAccountKeys);
    }
    let book = &accounts[0];
    validate_book(book, program_id)?;
    let (tail, head) = event_cursors(book)?;
    if tail == head {
        return Err(NoEvents.into());
    }
    let mut borrows = SegmentBorrowRegistry::new();
    {
        let event = book.segment_ref::<EventRecord>(
            &mut borrows,
            EVENTS_OFFSET + SEG_META + (head % EVENT_CAP) * EventRecord::SIZE as u32,
            EventRecord::SIZE as u32,
        )?;
        emit_slices(&[
            b"ask_event_drain",
            &head.to_le_bytes(),
            &event.maker,
            &event.price,
            &event.size,
        ]);
    }
    *book.segment_mut::<[u8; 4]>(&mut borrows, EVENTS_OFFSET + 4, 4)? =
        head.wrapping_add(1).to_le_bytes();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hopper::hopper_runtime::__hopper_native::{
        AccountView as NativeView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
    };
    use std::vec::Vec;

    // Host borrow tracking keys accounts by address, including across test
    // threads. Give each synthetic account a distinct non-system key.
    static NEXT_ADDRESS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);

    const PROGRAM: Address = Address::new_from_array([9; 32]);

    struct TestAccount {
        backing: Vec<u64>,
    }

    impl TestAccount {
        fn new(address: u8, size: usize, signer: bool, writable: bool) -> Self {
            let address = if address == 0 {
                0
            } else {
                NEXT_ADDRESS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            };
            let mut backing = vec![0u64; (RuntimeAccount::SIZE + size).div_ceil(8)];
            // SAFETY: The owned word-aligned backing covers the header and data.
            // Every header field is initialized before constructing a view.
            unsafe {
                (backing.as_mut_ptr() as *mut RuntimeAccount).write(RuntimeAccount {
                    borrow_state: NOT_BORROWED,
                    is_signer: u8::from(signer),
                    is_writable: u8::from(writable),
                    executable: 0,
                    resize_delta: 0,
                    address: NativeAddress::new_from_array([address; 32]),
                    owner: NativeAddress::new_from_array([9; 32]),
                    lamports: 1_000_000_000,
                    data_len: size as u64,
                });
            }
            Self { backing }
        }

        fn view(&mut self) -> AccountView<'_> {
            // SAFETY: The initialized backing remains alive and exclusively
            // borrowed for the returned view's lifetime; it is never resized.
            let backend = unsafe {
                NativeView::new_unchecked(self.backing.as_mut_ptr() as *mut RuntimeAccount)
            };
            // SAFETY: AccountView is repr(transparent) over NativeView, with
            // identical lifetimes. This mirrors Hopper's entrypoint bridge.
            unsafe { core::mem::transmute::<NativeView<'_>, AccountView<'_>>(backend) }
        }
    }

    fn init(book: &AccountView) -> ProgramResult {
        let mut payer = TestAccount::new(1, 0, true, true);
        let mut system = TestAccount::new(0, 0, false, false);
        process_init_book(&PROGRAM, &[payer.view(), book.clone(), system.view()], &[])
    }

    fn order_data() -> Vec<u8> {
        [100u64.to_le_bytes(), 5u64.to_le_bytes(), 7u64.to_le_bytes()].concat()
    }

    fn post(book: &AccountView, maker: &AccountView, ask: bool) -> ProgramResult {
        process_post(
            &PROGRAM,
            &[maker.clone(), book.clone()],
            &order_data(),
            if ask { ASKS_OFFSET } else { BIDS_OFFSET },
        )
    }

    fn bytes(book: &AccountView) -> Vec<u8> {
        book.try_borrow().unwrap().to_vec()
    }

    fn write_u32(book: &AccountView, offset: u32, value: u32) {
        book.try_borrow_mut().unwrap()[offset as usize..offset as usize + 4]
            .copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn initialization_requires_book_signature_and_never_resets_state() {
        let mut book = TestAccount::new(2, BOOK_ACCOUNT_SIZE, false, true);
        let view = book.view();
        assert_eq!(init(&view), Err(ProgramError::MissingRequiredSignature));
        assert!(bytes(&view).iter().all(|b| *b == 0));
        let mut book = TestAccount::new(2, BOOK_ACCOUNT_SIZE, true, true);
        let view = book.view();
        init(&view).unwrap();
        let before = bytes(&view);
        assert_eq!(init(&view), Err(ProgramError::AccountAlreadyInitialized));
        assert_eq!(bytes(&view), before);
    }

    #[test]
    fn initialization_refuses_dirty_body_even_with_zero_header() {
        let mut book = TestAccount::new(2, BOOK_ACCOUNT_SIZE, true, true);
        let view = book.view();
        for offset in [
            16,
            31,
            32,
            63,
            64,
            BOOK_ACCOUNT_SIZE - 5,
            BOOK_ACCOUNT_SIZE - 1,
        ] {
            view.try_borrow_mut().unwrap()[offset] = 1;
            let before = bytes(&view);
            assert_eq!(init(&view), Err(ProgramError::AccountAlreadyInitialized));
            assert_eq!(bytes(&view), before);
            view.try_borrow_mut().unwrap()[offset] = 0;
        }
    }

    #[test]
    fn posting_changes_only_selected_count_and_record() {
        let mut book = TestAccount::new(2, BOOK_ACCOUNT_SIZE, true, true);
        let view = book.view();
        init(&view).unwrap();
        let mut maker = TestAccount::new(3, 0, true, false);
        let maker = maker.view();
        for (ask, offset) in [(false, BIDS_OFFSET), (true, ASKS_OFFSET)] {
            let before = bytes(&view);
            post(&view, &maker, ask).unwrap();
            let after = bytes(&view);
            for i in 0..BOOK_ACCOUNT_SIZE {
                if !(offset as usize..offset as usize + 4).contains(&i)
                    && !(offset as usize + 8..offset as usize + 64).contains(&i)
                {
                    assert_eq!(before[i], after[i], "unexpected change at {i}");
                }
            }
            assert_eq!(side_count(&view, offset).unwrap(), 1);
            assert_eq!(
                &after[offset as usize + 8..offset as usize + 40],
                maker.address().as_array()
            );
        }
    }

    #[test]
    fn ask_removal_requires_its_maker_and_drain_advances_once() {
        let mut book = TestAccount::new(2, BOOK_ACCOUNT_SIZE, true, true);
        let view = book.view();
        init(&view).unwrap();
        let mut maker = TestAccount::new(3, 0, true, false);
        let maker = maker.view();
        let mut stranger = TestAccount::new(4, 0, true, false);
        post(&view, &maker, true).unwrap();
        let before = bytes(&view);
        assert_eq!(
            process_record_ask_event(&PROGRAM, &[stranger.view(), view.clone()], &[]),
            Err(Unauthorized.into())
        );
        assert_eq!(bytes(&view), before);
        process_record_ask_event(&PROGRAM, &[maker, view.clone()], &[]).unwrap();
        assert_eq!(side_count(&view, ASKS_OFFSET).unwrap(), 0);
        assert_eq!(event_cursors(&view).unwrap(), (1, 0));
        let before = bytes(&view);
        process_crank_events(&PROGRAM, core::slice::from_ref(&view), &[]).unwrap();
        assert_eq!(event_cursors(&view).unwrap(), (1, 1));
        let after = bytes(&view);
        assert_eq!(
            &before[..EVENTS_OFFSET as usize + 4],
            &after[..EVENTS_OFFSET as usize + 4]
        );
        assert_eq!(
            &before[EVENTS_OFFSET as usize + 8..],
            &after[EVENTS_OFFSET as usize + 8..]
        );
        assert_eq!(
            process_crank_events(&PROGRAM, core::slice::from_ref(&view), &[]),
            Err(NoEvents.into())
        );
        assert_eq!(bytes(&view), after);
    }

    #[test]
    fn full_event_queue_preserves_ask_and_unread_events() {
        let mut book = TestAccount::new(2, BOOK_ACCOUNT_SIZE, true, true);
        let view = book.view();
        init(&view).unwrap();
        let mut maker = TestAccount::new(3, 0, true, false);
        let maker = maker.view();
        post(&view, &maker, true).unwrap();
        write_u32(&view, EVENTS_OFFSET, EVENT_CAP);
        let before = bytes(&view);
        assert_eq!(
            process_record_ask_event(&PROGRAM, &[maker, view.clone()], &[]),
            Err(EventQueueFull.into())
        );
        assert_eq!(bytes(&view), before);
    }

    #[test]
    fn event_queue_wraparound_preserves_slot_order() {
        let mut book = TestAccount::new(2, BOOK_ACCOUNT_SIZE, true, true);
        let view = book.view();
        init(&view).unwrap();
        let mut maker = TestAccount::new(3, 0, true, false);
        let maker = maker.view();
        post(&view, &maker, true).unwrap();
        write_u32(&view, EVENTS_OFFSET, u32::MAX);
        write_u32(&view, EVENTS_OFFSET + 4, u32::MAX);
        process_record_ask_event(&PROGRAM, &[maker.clone(), view.clone()], &[]).unwrap();
        assert_eq!(event_cursors(&view).unwrap(), (0, u32::MAX));
        let start =
            (EVENTS_OFFSET + SEG_META + (EVENT_CAP - 1) * EventRecord::SIZE as u32) as usize;
        assert_eq!(&bytes(&view)[start..start + 32], maker.address().as_array());
        process_crank_events(&PROGRAM, core::slice::from_ref(&view), &[]).unwrap();
        assert_eq!(event_cursors(&view).unwrap(), (0, 0));
    }

    #[test]
    fn malformed_layout_and_counters_are_refused_without_mutation() {
        let mut book = TestAccount::new(2, BOOK_ACCOUNT_SIZE, true, true);
        let view = book.view();
        init(&view).unwrap();
        let pristine = bytes(&view);
        let mut maker = TestAccount::new(3, 0, true, false);
        let maker = maker.view();
        for (offset, value) in [
            (0, 0),
            (24, 0),
            (BIDS_OFFSET, SIDE_CAP + 1),
            (BIDS_OFFSET + 4, 1),
        ] {
            view.try_borrow_mut().unwrap().copy_from_slice(&pristine);
            write_u32(&view, offset, value);
            let before = bytes(&view);
            assert_eq!(
                post(&view, &maker, false),
                Err(ProgramError::InvalidAccountData)
            );
            assert_eq!(bytes(&view), before);
        }
        view.try_borrow_mut().unwrap().copy_from_slice(&pristine);
        write_u32(&view, EVENTS_OFFSET, EVENT_CAP + 1);
        let before = bytes(&view);
        assert_eq!(
            process_crank_events(&PROGRAM, core::slice::from_ref(&view), &[]),
            Err(ProgramError::InvalidAccountData)
        );
        assert_eq!(bytes(&view), before);
    }

    #[test]
    fn invalid_order_input_and_missing_signer_do_not_mutate() {
        let mut book = TestAccount::new(2, BOOK_ACCOUNT_SIZE, true, true);
        let view = book.view();
        init(&view).unwrap();
        let mut maker = TestAccount::new(3, 0, true, false);
        let maker = maker.view();
        let before = bytes(&view);
        for data in [
            vec![],
            vec![0; 17],
            vec![0; 25],
            [0u64.to_le_bytes(), 5u64.to_le_bytes()].concat(),
            [5u64.to_le_bytes(), 0u64.to_le_bytes()].concat(),
        ] {
            assert!(process_post_bid(&PROGRAM, &[maker.clone(), view.clone()], &data).is_err());
            assert_eq!(bytes(&view), before);
        }
        let mut unsigned = TestAccount::new(3, 0, false, false);
        assert_eq!(
            post(&view, &unsigned.view(), false),
            Err(ProgramError::MissingRequiredSignature)
        );
        assert_eq!(bytes(&view), before);
    }
}
