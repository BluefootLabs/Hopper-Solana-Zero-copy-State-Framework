//! The projection primitives and the cross-program lenses, each against an
//! account with known bytes: what they return, what they refuse, and what
//! they do to the borrow state. `audit/UNSAFE_MAP.md` lists these sites as
//! reached from here.

use hopper_native::lens;
use hopper_native::project::{
    project, project_hopper, project_hopper_mut, project_mut, project_safe, project_safe_mut,
    project_slice, HOPPER_HEADER_LEN,
};
use hopper_native::wire::LeU64;
use hopper_native::{AccountView, Address, ProgramError, RuntimeAccount, NOT_BORROWED};

const LEN: usize = 64;
const DISC: u8 = 9;

#[repr(C, align(8))]
struct Backing {
    header: RuntimeAccount,
    data: [u8; 256],
}

/// An account of `len` bytes: a Hopper header with discriminator [`DISC`],
/// then a body where byte `i` holds `i`.
fn backing(len: usize) -> Backing {
    let mut data = [0u8; 256];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = i as u8;
    }
    data[0] = DISC;
    Backing {
        header: RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: 1,
            is_writable: 1,
            executable: 0,
            resize_delta: len as u32,
            address: Address::new_from_array([7; 32]),
            owner: Address::new_from_array([0xA1; 32]),
            lamports: 1_000,
            data_len: len as u64,
        },
        data,
    }
}

fn view(backing: &mut Backing) -> AccountView<'_> {
    // SAFETY: `Backing` has the loader's header layout followed by 256
    // initialized bytes, more than any length these tests set.
    unsafe { AccountView::new_unchecked((backing as *mut Backing).cast()) }
}

fn body(at: usize) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = (at + i) as u8;
    }
    bytes
}

#[test]
fn the_hopper_body_starts_after_sixteen_bytes() {
    assert_eq!(HOPPER_HEADER_LEN, 16);
    let mut a = backing(LEN);
    let account = view(&mut a);

    let projected = project_hopper::<[u8; 32]>(&account, DISC).unwrap();
    assert_eq!(*projected, body(16));
    // The header's layout id and reserved bytes (4..16) are not part of it.
    assert_eq!(projected[0], 16);
    drop(projected);

    assert_eq!(
        project_hopper::<[u8; 32]>(&account, DISC + 1).unwrap_err(),
        ProgramError::InvalidAccountData
    );
    // 64 bytes of body do not fit behind a 16-byte header in 64 bytes.
    assert_eq!(
        project_hopper::<[u8; 64]>(&account, DISC).unwrap_err(),
        ProgramError::AccountDataTooSmall
    );
}

#[test]
fn project_hopper_mut_writes_the_body_and_leaves_the_header() {
    let mut a = backing(LEN);
    {
        let account = view(&mut a);
        // SAFETY: nothing else refers to the account's data in this block.
        let body = unsafe { project_hopper_mut::<[u8; 32]>(&account, DISC) }.unwrap();
        *body = [0xEE; 32];
        // SAFETY: as above; `body` is no longer used.
        let wrong = unsafe { project_hopper_mut::<[u8; 32]>(&account, DISC + 1) };
        assert_eq!(wrong.unwrap_err(), ProgramError::InvalidAccountData);
    }
    assert_eq!(a.data[0], DISC);
    for i in 1..16 {
        assert_eq!(a.data[i], i as u8, "header byte {i}");
    }
    assert!(a.data[16..48].iter().all(|b| *b == 0xEE));
    assert_eq!(a.data[48], 48);
}

#[test]
fn project_mut_checks_bounds_disc_and_alignment() {
    let mut a = backing(LEN);
    let account = view(&mut a);

    // SAFETY: each projection is dropped before the next is made and
    // nothing else refers to the data.
    unsafe {
        *project_mut::<u8>(&account, 63, None).unwrap() = 0x42;
        assert_eq!(
            project_mut::<u8>(&account, 64, None).unwrap_err(),
            ProgramError::AccountDataTooSmall
        );
        assert_eq!(
            project_mut::<[u8; 32]>(&account, 33, None).unwrap_err(),
            ProgramError::AccountDataTooSmall
        );
        assert_eq!(
            project_mut::<u8>(&account, usize::MAX, None).unwrap_err(),
            ProgramError::AccountDataTooSmall
        );
        assert_eq!(
            project_mut::<u8>(&account, 0, Some(DISC + 1)).unwrap_err(),
            ProgramError::InvalidAccountData
        );
        // The data starts on an 8-byte boundary, so offset 8 is aligned for
        // a `u64` and offset 9 is not.
        *project_mut::<u64>(&account, 8, Some(DISC)).unwrap() = u64::MAX;
        assert_eq!(
            project_mut::<u64>(&account, 9, None).unwrap_err(),
            ProgramError::InvalidAccountData
        );
        *project_safe_mut::<u16>(&account, 32, Some(DISC)).unwrap() = 0xBEEF;
    }
    assert_eq!(*project_safe::<u8>(&account, 63, None).unwrap(), 0x42);
    assert_eq!(*project::<u64>(&account, 8, None).unwrap(), u64::MAX);
    assert_eq!(*project::<u16>(&account, 32, None).unwrap(), 0xBEEF);
    // `project_mut` does not take a borrow; the account is still free.
    assert!(account.try_borrow_mut().is_ok());
}

#[test]
fn project_mut_on_an_empty_account_has_no_discriminator_to_match() {
    #[derive(Clone, Copy, Debug)]
    struct Nothing;
    // SAFETY: a type with no bytes has no bit pattern to get wrong.
    unsafe impl hopper_native::Projectable for Nothing {}

    let mut a = backing(0);
    let account = view(&mut a);
    // SAFETY: nothing else refers to the data.
    let result = unsafe { project_mut::<Nothing>(&account, 0, Some(DISC)) };
    assert_eq!(result.unwrap_err(), ProgramError::InvalidAccountData);
}

#[test]
fn project_slice_counts_in_elements() {
    let mut a = backing(LEN);
    let account = view(&mut a);

    let words = project_slice::<u16>(&account, 16, 4).unwrap();
    assert_eq!(words.len(), 4);
    assert_eq!(words[0], u16::from_ne_bytes([16, 17]));
    assert_eq!(words[3], u16::from_ne_bytes([22, 23]));
    // The slice is a shared borrow.
    assert!(account.try_borrow_mut().is_err());
    drop(words);
    assert!(account.try_borrow_mut().is_ok());

    assert_eq!(project_slice::<u8>(&account, 64, 0).unwrap().len(), 0);
    assert_eq!(project_slice::<u16>(&account, 0, 32).unwrap().len(), 32);
    assert_eq!(
        project_slice::<u16>(&account, 0, 33).unwrap_err(),
        ProgramError::AccountDataTooSmall
    );
    assert_eq!(
        project_slice::<u16>(&account, 17, 2).unwrap_err(),
        ProgramError::InvalidAccountData
    );
    // A count whose byte length overflows is refused before any pointer
    // arithmetic.
    assert_eq!(
        project_slice::<u64>(&account, 0, usize::MAX).unwrap_err(),
        ProgramError::ArithmeticOverflow
    );
    assert_eq!(
        project_slice::<u8>(&account, 1, usize::MAX).unwrap_err(),
        ProgramError::AccountDataTooSmall
    );
    assert!(account.try_borrow_mut().is_ok());
}

#[test]
fn lenses_read_what_is_there_and_hold_a_shared_borrow() {
    let mut a = backing(LEN);
    let account = view(&mut a);

    let address = lens::read_address(&account, 16).unwrap();
    assert_eq!(address.as_array(), &body(16));
    let bytes = lens::read_bytes(&account, 60, 4).unwrap();
    assert_eq!(&*bytes, &[60, 61, 62, 63]);
    // An odd offset: the wire type has alignment 1.
    let value = lens::read_field_pod::<LeU64>(&account, 17).unwrap();
    assert_eq!(
        value.get(),
        u64::from_le_bytes([17, 18, 19, 20, 21, 22, 23, 24])
    );
    let array = lens::read_field_pod::<[u8; 4]>(&account, 60).unwrap();
    assert_eq!(*array, [60, 61, 62, 63]);

    // Four shared borrows are live; an exclusive one has to wait.
    assert!(account.try_borrow_mut().is_err());
    drop((address, bytes, value, array));
    assert!(account.try_borrow_mut().is_ok());
}

#[test]
fn lenses_refuse_what_is_out_of_bounds_without_leaking_a_borrow() {
    let mut a = backing(LEN);
    let account = view(&mut a);

    assert!(lens::read_address(&account, 32).is_ok());
    for offset in [33, 64, usize::MAX, usize::MAX - 31] {
        assert_eq!(
            lens::read_address(&account, offset).unwrap_err(),
            ProgramError::AccountDataTooSmall,
            "address at {offset}"
        );
    }
    assert_eq!(lens::read_bytes(&account, 64, 0).unwrap().len(), 0);
    for (offset, len) in [(64, 1), (0, 65), (1, usize::MAX), (usize::MAX, 1)] {
        assert_eq!(
            lens::read_bytes(&account, offset, len).unwrap_err(),
            ProgramError::AccountDataTooSmall,
            "{len} bytes at {offset}"
        );
    }
    assert!(lens::read_field_pod::<LeU64>(&account, 56).is_ok());
    assert_eq!(
        lens::read_field_pod::<LeU64>(&account, 57).unwrap_err(),
        ProgramError::AccountDataTooSmall
    );
    assert_eq!(
        lens::read_field_pod::<LeU64>(&account, usize::MAX).unwrap_err(),
        ProgramError::ArithmeticOverflow
    );
    assert!(account.try_borrow_mut().is_ok());
}

#[test]
fn lenses_wait_for_an_exclusive_borrow() {
    let mut a = backing(LEN);
    let account = view(&mut a);
    {
        let _held = account.try_borrow_mut().unwrap();
        assert_eq!(
            lens::read_address(&account, 0).unwrap_err(),
            ProgramError::AccountBorrowFailed
        );
        assert_eq!(
            lens::read_bytes(&account, 0, 8).unwrap_err(),
            ProgramError::AccountBorrowFailed
        );
        assert_eq!(
            lens::read_field_pod::<LeU64>(&account, 0).unwrap_err(),
            ProgramError::AccountBorrowFailed
        );
    }
    assert!(lens::read_address(&account, 0).is_ok());
}

#[test]
fn header_word_packs_the_four_flag_bytes() {
    let mut a = backing(LEN);
    let account = view(&mut a);
    // borrow_state, is_signer, is_writable, executable: low byte first.
    assert_eq!(
        account.header_word(),
        u32::from_le_bytes([NOT_BORROWED, 1, 1, 0])
    );
    let held = account.try_borrow().unwrap();
    // One shared borrow: the count starts at 1.
    assert_eq!(account.header_word() & 0xFF, 1);
    assert_eq!(account.header_word() >> 8, 0x0101);
    drop(held);
    assert_eq!(account.header_word() & 0xFF, NOT_BORROWED as u32);
}
