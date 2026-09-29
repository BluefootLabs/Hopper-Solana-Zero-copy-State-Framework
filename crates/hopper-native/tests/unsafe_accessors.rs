//! The `unsafe` accessors of `AccountView`, each exercised under the
//! contract its `# Safety` section states, and each compared with the
//! checked API it bypasses. `docs/UNSAFE_INVARIANTS.md` cites these tests
//! by name; `scripts/check-doc-citations.py` fails the gate when a cited
//! test is renamed or removed.

use hopper_native::{AccountView, Address, ProgramError, RuntimeAccount, NOT_BORROWED};

const LEN: usize = 32;

#[repr(C)]
struct Backing {
    header: RuntimeAccount,
    data: [u8; 256],
}

fn backing() -> Backing {
    let mut data = [0u8; 256];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = i as u8;
    }
    Backing {
        header: RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: 0,
            is_writable: 1,
            executable: 0,
            resize_delta: LEN as u32,
            address: Address::new_from_array([7; 32]),
            owner: Address::new_from_array([0xA1; 32]),
            lamports: 1_000,
            data_len: LEN as u64,
        },
        data,
    }
}

fn view(backing: &mut Backing) -> AccountView<'_> {
    // SAFETY: `Backing` has the loader's header layout followed by 256
    // initialized bytes, more than any length these tests set, and its
    // `resize_delta` slot holds the entry length.
    unsafe { AccountView::new_unchecked((backing as *mut Backing).cast()) }
}

#[test]
fn new_unchecked_views_the_header_and_data_it_was_given() {
    let mut a = backing();
    let header: *const RuntimeAccount = &a.header;
    let account = view(&mut a);
    assert_eq!(account.account_ptr(), header);
    assert_eq!(account.address(), &Address::new_from_array([7; 32]));
    assert_eq!(account.lamports(), 1_000);
    assert_eq!(account.data_len(), LEN);
    assert!(account.is_writable() && !account.is_signer() && !account.is_borrowed());
    assert_eq!(&*account.try_borrow().unwrap(), &a_bytes()[..LEN]);
}

fn a_bytes() -> [u8; 256] {
    backing().data
}

#[test]
fn owner_reads_the_header_and_assign_replaces_it() {
    let mut a = backing();
    let account = view(&mut a);
    // SAFETY: nothing assigns or closes the account while the reference
    // is read; it is dropped before `assign`.
    assert_eq!(
        unsafe { account.owner() },
        &Address::new_from_array([0xA1; 32])
    );
    let next = Address::new_from_array([0xB2; 32]);
    // SAFETY: the account is writable and no owner reference is live.
    unsafe { account.assign(&next) };
    // SAFETY: as above.
    assert_eq!(unsafe { account.owner() }, &next);
    // Nothing else in the header moved.
    assert_eq!(account.lamports(), 1_000);
    assert_eq!(account.data_len(), LEN);
    assert_eq!(a.header.owner, next);
}

#[test]
fn borrow_unchecked_is_the_checked_region_and_leaves_the_borrow_state_alone() {
    let mut a = backing();
    let account = view(&mut a);
    {
        // SAFETY: no exclusive borrow is live.
        let raw = unsafe { account.borrow_unchecked() };
        assert_eq!(raw.len(), LEN);
        assert!(
            !account.is_borrowed(),
            "the unchecked borrow is not tracked"
        );
        let checked = account.try_borrow().unwrap();
        assert_eq!(raw.as_ptr(), checked.as_ptr());
        assert_eq!(raw, &*checked);
    }
    {
        // SAFETY: no other borrow is live for the write below.
        let raw = unsafe { account.borrow_unchecked_mut() };
        assert_eq!(raw.len(), LEN);
        raw[3] = 0xEE;
        assert!(!account.is_borrowed());
    }
    assert_eq!(account.try_borrow().unwrap()[3], 0xEE);
    assert_eq!(a.data[3], 0xEE);
    assert_eq!(
        a.data[LEN], LEN as u8,
        "bytes past the length are untouched"
    );
}

#[test]
fn unchecked_segments_address_the_same_bytes_as_the_checked_ones() {
    let mut a = backing();
    let account = view(&mut a);
    for offset in [0u32, 5, (LEN - 8) as u32] {
        let checked = *account.segment_ref::<[u8; 8]>(offset, 8).unwrap();
        // SAFETY: `offset + 8 <= data_len` and no exclusive borrow of the
        // range is live.
        let unchecked = unsafe { account.segment_ref_unchecked::<[u8; 8]>(offset) }.unwrap();
        assert_eq!(*unchecked, checked);
        assert!(!account.is_borrowed(), "unchecked segments are not tracked");
    }
    {
        // SAFETY: the account is writable, `8 + 8 <= data_len`, and no
        // other borrow of the range is live.
        let mut segment = unsafe { account.segment_mut_unchecked::<[u8; 8]>(8) }.unwrap();
        *segment = [0xC3; 8];
    }
    assert_eq!(*account.segment_ref::<[u8; 8]>(8, 8).unwrap(), [0xC3; 8]);
    assert_eq!(a.data[7], 7);
    assert_eq!(a.data[16], 16);
}

/// What the unchecked segment accessors skip, shown on the checked ones:
/// bounds, the declared size, writability, and a live conflicting borrow.
#[test]
fn checked_segments_refuse_what_the_unchecked_contract_assumes() {
    let mut a = backing();
    let account = view(&mut a);
    assert_eq!(
        account.segment_ref::<[u8; 8]>((LEN - 7) as u32, 8).err(),
        Some(ProgramError::AccountDataTooSmall)
    );
    assert_eq!(
        account.segment_ref::<[u8; 8]>(0, 7).err(),
        Some(ProgramError::InvalidArgument)
    );
    assert_eq!(
        account.segment_mut::<[u8; 8]>(u32::MAX - 3, 8).err(),
        Some(ProgramError::ArithmeticOverflow)
    );
    let held = account.try_borrow().unwrap();
    assert_eq!(
        account.segment_mut::<[u8; 8]>(0, 8).err(),
        Some(ProgramError::AccountBorrowFailed)
    );
    drop(held);

    let mut b = backing();
    b.header.is_writable = 0;
    let read_only = view(&mut b);
    assert!(read_only.segment_mut::<[u8; 8]>(0, 8).is_err());
    assert!(read_only.segment_ref::<[u8; 8]>(0, 8).is_ok());
}

/// `raw_ref` and `raw_mut` are `unsafe` for what the type means, not for
/// memory: they go through the checked segment path, so they are bounds
/// checked and borrow tracked.
#[test]
fn raw_ref_and_raw_mut_are_bounds_checked_and_borrow_tracked() {
    let mut a = backing();
    let account = view(&mut a);
    {
        // SAFETY: `[u8; 8]` is valid for any bytes.
        let head = unsafe { account.raw_ref::<[u8; 8]>() }.unwrap();
        assert_eq!(*head, [0, 1, 2, 3, 4, 5, 6, 7]);
        assert!(account.is_borrowed());
        // SAFETY: as above; the call is refused, nothing is aliased.
        assert_eq!(
            unsafe { account.raw_mut::<[u8; 8]>() }.err(),
            Some(ProgramError::AccountBorrowFailed)
        );
    }
    assert!(!account.is_borrowed());
    {
        // SAFETY: no other borrow is live.
        let mut head = unsafe { account.raw_mut::<[u8; 8]>() }.unwrap();
        head[0] = 0x99;
    }
    assert_eq!(a.data[0], 0x99);

    let mut b = backing();
    b.header.data_len = 4;
    let short = view(&mut b);
    // SAFETY: the call is refused before any byte is read.
    assert_eq!(
        unsafe { short.raw_ref::<[u8; 8]>() }.err(),
        Some(ProgramError::AccountDataTooSmall)
    );
}

#[test]
fn resize_unchecked_sets_the_length_and_nothing_else() {
    let mut a = backing();
    let account = view(&mut a);
    // SAFETY: writable, no borrow live, and the growth is far below the
    // permitted increase; the backing store holds 256 initialized bytes.
    unsafe { account.resize_unchecked(LEN + 16) };
    assert_eq!(account.data_len(), LEN + 16);
    let data = account.try_borrow().unwrap();
    assert_eq!(data.len(), LEN + 16);
    // No zero fill: the grown region shows what the buffer already held.
    // The checked `resize` clears it; a caller of the unchecked one must.
    assert_eq!(data[LEN], LEN as u8);
    drop(data);
    // SAFETY: as above, shrinking.
    unsafe { account.resize_unchecked(8) };
    assert_eq!(account.data_len(), 8);
    assert_eq!(account.lamports(), 1_000);
    assert_eq!(a.data[..LEN + 16], a_bytes()[..LEN + 16]);
}

#[test]
fn close_unchecked_clears_lamports_length_and_owner_without_touching_data() {
    let mut a = backing();
    let account = view(&mut a);
    // SAFETY: no borrow of the account is live.
    unsafe { account.close_unchecked() };
    assert_eq!(account.lamports(), 0);
    assert_eq!(account.data_len(), 0);
    // SAFETY: nothing assigns the account while the reference is read.
    assert_eq!(unsafe { account.owner() }, &AccountView::SYSTEM_PROGRAM_ID);
    // The bytes stay: with a zero length they are unreachable through the
    // view, and the runtime discards them with the account.
    assert_eq!(a.data, a_bytes());

    // The checked close refuses a live borrow and zeroes the data.
    let mut b = backing();
    let account = view(&mut b);
    let held = account.try_borrow().unwrap();
    assert_eq!(account.close(), Err(ProgramError::AccountBorrowFailed));
    assert_eq!((account.lamports(), account.data_len()), (1_000, LEN));
    drop(held);
    account.close().unwrap();
    assert_eq!((account.lamports(), account.data_len()), (0, 0));
    assert_eq!(b.data[..LEN], [0u8; LEN]);
    assert_eq!(b.data[LEN], LEN as u8);
}
