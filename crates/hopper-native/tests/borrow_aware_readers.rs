//! The safe readers of account bytes consult the borrow state. A reader that
//! looked at the data underneath a live exclusive borrow would read bytes
//! their holder may be writing; each one refuses instead, and none hands
//! out an untracked reference.

use hopper_native::verify::DataFingerprint;
use hopper_native::{lens, AccountView, Address, ProgramError, RuntimeAccount, NOT_BORROWED};

#[repr(C)]
struct Backing {
    header: RuntimeAccount,
    data: [u8; 64],
}

fn backing() -> Backing {
    let mut data = [0u8; 64];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = i as u8 + 1;
    }
    Backing {
        header: RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: 0,
            is_writable: 1,
            executable: 0,
            resize_delta: 32,
            address: Address::new_from_array([7; 32]),
            owner: Address::new_from_array([0xA1; 32]),
            lamports: 1,
            data_len: 32,
        },
        data,
    }
}

fn view(backing: &mut Backing) -> AccountView<'_> {
    // SAFETY: `Backing` has the loader's header layout followed by
    // initialized bytes, and its `resize_delta` slot holds the entry length.
    unsafe { AccountView::new_unchecked((backing as *mut Backing).cast()) }
}

#[test]
fn header_readers_answer_while_unborrowed_or_shared() {
    let mut a = backing();
    let account = view(&mut a);
    assert_eq!(account.disc(), 1);
    assert_eq!(account.version(), 2);
    assert_eq!(account.layout_id(), Some([5, 6, 7, 8, 9, 10, 11, 12]));

    let shared = account.try_borrow().unwrap();
    assert_eq!(account.disc(), 1);
    assert_eq!(account.version(), 2);
    assert_eq!(account.layout_id(), Some([5, 6, 7, 8, 9, 10, 11, 12]));
    assert_eq!(account.require_disc(1), Ok(()));
    drop(shared);
}

#[test]
fn header_readers_fail_closed_under_an_exclusive_borrow() {
    let mut a = backing();
    let account = view(&mut a);
    let mut exclusive = account.try_borrow_mut().unwrap();
    // The holder is free to write the very bytes the readers would read.
    exclusive[0] = 0x55;
    assert_eq!(account.disc(), 0);
    assert_eq!(account.version(), 0);
    assert_eq!(account.layout_id(), None);
    assert_eq!(
        account.require_disc(0x55),
        Err(ProgramError::InvalidAccountData)
    );
    exclusive[1] = 0x66;
    drop(exclusive);
    // Released: the readers see what was written.
    assert_eq!(account.disc(), 0x55);
    assert_eq!(account.version(), 0x66);
}

/// `layout_id` returns the bytes, not a reference into the account: the
/// value stays what it was when the account is written afterwards.
#[test]
fn layout_id_is_a_copy() {
    let mut a = backing();
    let account = view(&mut a);
    let id = account.layout_id().unwrap();
    account.try_borrow_mut().unwrap()[4..12].fill(0);
    assert_eq!(id, [5, 6, 7, 8, 9, 10, 11, 12]);
    assert_eq!(account.layout_id(), Some([0; 8]));
}

#[test]
fn short_accounts_have_no_header_fields() {
    let mut a = backing();
    a.header.data_len = 0;
    let account = view(&mut a);
    assert_eq!(account.disc(), 0);
    assert_eq!(account.version(), 0);
    assert_eq!(account.layout_id(), None);

    let mut b = backing();
    b.header.data_len = 11;
    let account = view(&mut b);
    assert_eq!(account.disc(), 1);
    assert_eq!(account.layout_id(), None);
}

#[test]
fn by_value_lenses_read_in_bounds_and_refuse_an_exclusive_borrow() {
    let mut a = backing();
    let account = view(&mut a);
    assert_eq!(lens::read_u8(&account, 3), Ok(4));
    assert_eq!(
        lens::read_le_u16(&account, 0),
        Ok(u16::from_le_bytes([1, 2]))
    );
    assert_eq!(
        lens::read_le_u32(&account, 4),
        Ok(u32::from_le_bytes([5, 6, 7, 8]))
    );
    assert_eq!(
        lens::read_le_u64(&account, 24),
        Ok(u64::from_le_bytes([25, 26, 27, 28, 29, 30, 31, 32]))
    );
    // One byte past the end of each width.
    assert_eq!(
        lens::read_u8(&account, 32),
        Err(ProgramError::AccountDataTooSmall)
    );
    assert_eq!(
        lens::read_le_u16(&account, 31),
        Err(ProgramError::AccountDataTooSmall)
    );
    assert_eq!(
        lens::read_le_u32(&account, 29),
        Err(ProgramError::AccountDataTooSmall)
    );
    assert_eq!(
        lens::read_le_u64(&account, 25),
        Err(ProgramError::AccountDataTooSmall)
    );
    assert_eq!(
        lens::read_le_u64(&account, usize::MAX),
        Err(ProgramError::AccountDataTooSmall)
    );

    // A shared borrow does not get in the way; an exclusive one does.
    let shared = account.try_borrow().unwrap();
    assert_eq!(lens::read_u8(&account, 0), Ok(1));
    drop(shared);
    let exclusive = account.try_borrow_mut().unwrap();
    assert_eq!(
        lens::read_u8(&account, 0),
        Err(ProgramError::AccountBorrowFailed)
    );
    assert_eq!(
        lens::read_le_u16(&account, 0),
        Err(ProgramError::AccountBorrowFailed)
    );
    assert_eq!(
        lens::read_le_u32(&account, 0),
        Err(ProgramError::AccountBorrowFailed)
    );
    assert_eq!(
        lens::read_le_u64(&account, 0),
        Err(ProgramError::AccountBorrowFailed)
    );
    drop(exclusive);
    assert_eq!(lens::read_u8(&account, 0), Ok(1));
}

#[test]
fn a_fingerprint_is_taken_under_a_shared_borrow() {
    let mut a = backing();
    let account = view(&mut a);
    let before = DataFingerprint::capture(&account, 32).unwrap();
    assert_eq!(before.verify_unchanged(&account), Ok(()));
    assert!(!account.is_borrowed(), "the capture releases its borrow");

    let mut exclusive = account.try_borrow_mut().unwrap();
    assert_eq!(
        DataFingerprint::capture(&account, 32).err(),
        Some(ProgramError::AccountBorrowFailed)
    );
    assert_eq!(
        before.verify_unchanged(&account),
        Err(ProgramError::AccountBorrowFailed)
    );
    exclusive[9] ^= 1;
    drop(exclusive);
    assert_eq!(
        before.verify_unchanged(&account),
        Err(ProgramError::InvalidAccountData)
    );
    // A length past the data is clamped, and the clamp is part of the match.
    let whole = DataFingerprint::capture(&account, 1_000).unwrap();
    assert_eq!(whole.verify_unchanged(&account), Ok(()));
}
