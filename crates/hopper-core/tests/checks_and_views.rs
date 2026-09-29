//! The address checks, byte views, and collection accessors that form a
//! reference out of raw bytes, each against known bytes: what they return
//! and what they refuse. `audit/UNSAFE_MAP.md` lists these sites as reached
//! from here.

use hopper_core::abi::{FieldRef, TypedAddress, WireU32, WireU64};
use hopper_core::account::{SegmentTableMut, SliceCursor, SEGMENT_DESC_SIZE};
use hopper_core::check::guards::require_authority;
use hopper_core::check::{check_has_one, check_owner_multi};
use hopper_core::collections::{bitmap_bytes, FixedVec, Slab, SLAB_HEADER_SIZE};
use hopper_core::event::{emit_event, emit_event_tagged};
use hopper_native::{
    AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
};
use hopper_runtime::{AccountView, Address, ProgramError};

const KEY: [u8; 32] = [7; 32];
const OWNER: [u8; 32] = [0xA1; 32];

fn account(key: [u8; 32], owner: [u8; 32], is_signer: bool) -> (Vec<u64>, AccountView<'static>) {
    let mut backing = vec![0u64; (RuntimeAccount::SIZE + 64).div_ceil(8)];
    let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
    // SAFETY: `backing` is word-aligned and holds one header plus 64 bytes;
    // one valid header is written before a view is made.
    unsafe {
        raw.write(RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: u8::from(is_signer),
            is_writable: 1,
            executable: 0,
            resize_delta: 0,
            address: NativeAddress::new_from_array(key),
            owner: NativeAddress::new_from_array(owner),
            lamports: 1,
            data_len: 0,
        });
    }
    // SAFETY: `raw` points at the header written above, and the caller
    // keeps `backing` alive next to the view.
    let backend = unsafe { NativeAccountView::new_unchecked(raw) };
    // SAFETY: the runtime's `AccountView` is `repr(transparent)` over the
    // native one.
    let view = unsafe { core::mem::transmute::<NativeAccountView, AccountView>(backend) };
    (backing, view)
}

/// `KEY` with one byte changed.
fn near(at: usize) -> [u8; 32] {
    let mut key = KEY;
    key[at] ^= 1;
    key
}

#[test]
fn has_one_compares_all_thirty_two_bytes() {
    let (_backing, view) = account(KEY, OWNER, false);
    assert_eq!(check_has_one(&KEY, &view), Ok(()));
    // A comparison that read fewer than 32 bytes, or read them from the
    // wrong place, would accept one of these.
    for at in 0..32 {
        assert_eq!(
            check_has_one(&near(at), &view),
            Err(ProgramError::InvalidAccountData),
            "byte {at}"
        );
    }
}

#[test]
fn require_authority_wants_the_signature_and_the_address() {
    let (_signer_backing, signer) = account(KEY, OWNER, true);
    let (_other_backing, not_signer) = account(KEY, OWNER, false);

    assert_eq!(require_authority(&signer, &KEY), Ok(()));
    assert_eq!(
        require_authority(&not_signer, &KEY),
        Err(ProgramError::MissingRequiredSignature)
    );
    for at in [0, 15, 31] {
        assert_eq!(
            require_authority(&signer, &near(at)),
            Err(ProgramError::InvalidAccountData),
            "byte {at}"
        );
    }
}

#[test]
fn owner_multi_answers_with_the_index_that_matched() {
    let (_backing, view) = account(KEY, OWNER, false);
    let token = Address::new_from_array([1; 32]);
    let token_2022 = Address::new_from_array(OWNER);
    let almost = Address::new_from_array({
        let mut owner = OWNER;
        owner[31] ^= 1;
        owner
    });

    assert_eq!(check_owner_multi(&view, &[&token, &token_2022]), Ok(1));
    assert_eq!(check_owner_multi(&view, &[&token_2022, &token]), Ok(0));
    // The first match wins.
    assert_eq!(check_owner_multi(&view, &[&token_2022, &token_2022]), Ok(0));
    assert_eq!(
        check_owner_multi(&view, &[&token, &almost]),
        Err(ProgramError::IncorrectProgramId)
    );
    assert_eq!(
        check_owner_multi(&view, &[]),
        Err(ProgramError::IncorrectProgramId)
    );
}

#[test]
fn typed_address_matches_the_account_it_names() {
    struct Vault;
    let (_backing, view) = account(KEY, OWNER, false);

    assert!(TypedAddress::<Vault>::new(KEY).eq_account(&view));
    assert!(TypedAddress::<Vault>::from_account(&view).eq_account(&view));
    for at in 0..32 {
        assert!(
            !TypedAddress::<Vault>::new(near(at)).eq_account(&view),
            "byte {at}"
        );
    }
    assert!(TypedAddress::<Vault>::new(KEY)
        .require_eq_account(&view)
        .is_ok());
    assert!(TypedAddress::<Vault>::new(near(3))
        .require_eq_account(&view)
        .is_err());
}

#[test]
fn field_ref_gives_an_address_only_when_it_has_thirty_two_bytes() {
    let mut bytes = [0u8; 40];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = i as u8;
    }
    let address = FieldRef::new(&bytes[4..]).as_address().unwrap();
    assert_eq!(address[0], 4);
    assert_eq!(address[31], 35);
    assert!(core::ptr::eq(address.as_ptr(), bytes[4..].as_ptr()));

    assert!(FieldRef::new(&bytes[..32]).as_address().is_ok());
    assert_eq!(
        FieldRef::new(&bytes[..31]).as_address().unwrap_err(),
        ProgramError::InvalidAccountData
    );
    assert_eq!(
        FieldRef::new(&[]).as_address().unwrap_err(),
        ProgramError::InvalidAccountData
    );
}

#[test]
fn the_cursor_reads_an_address_where_it_stands_and_moves_past_it() {
    let mut bytes = [0u8; 70];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = i as u8;
    }
    let mut cursor = SliceCursor::new(&bytes);
    assert_eq!(cursor.read_u8().unwrap(), 0);
    let first = cursor.read_address().unwrap();
    assert_eq!((first[0], first[31]), (1, 32));
    let second = cursor.read_address().unwrap();
    assert_eq!((second[0], second[31]), (33, 64));
    assert_eq!(cursor.remaining(), 5);
    // Five bytes are not an address, and the refusal moves nothing.
    assert_eq!(
        cursor.read_address().unwrap_err(),
        ProgramError::InvalidAccountData
    );
    assert_eq!(cursor.remaining(), 5);
    assert_eq!(cursor.read_u8().unwrap(), 65);
}

#[test]
fn a_descriptor_is_reached_by_index_inside_the_table() {
    let mut table = [0u8; 3 * SEGMENT_DESC_SIZE + 5];
    {
        let mut segments = SegmentTableMut::from_bytes_mut(&mut table, 3).unwrap();
        segments.descriptor_mut(2).unwrap().set_offset(0xAABB_CCDD);
        segments.descriptor_mut(0).unwrap().set_count(0x1122);
        assert_eq!(
            segments.descriptor_mut(3).err(),
            Some(ProgramError::InvalidArgument)
        );
        assert_eq!(
            segments.descriptor_mut(usize::MAX).err(),
            Some(ProgramError::InvalidArgument)
        );
    }
    // Descriptor 2 starts at byte 24 and its offset is its first field;
    // descriptor 0 holds its count after the 4-byte offset.
    assert_eq!(
        &table[2 * SEGMENT_DESC_SIZE..2 * SEGMENT_DESC_SIZE + 4],
        &0xAABB_CCDDu32.to_le_bytes()
    );
    assert_eq!(&table[4..6], &0x1122u16.to_le_bytes());
    // The five bytes after the table are not part of any descriptor.
    assert_eq!(&table[3 * SEGMENT_DESC_SIZE..], &[0; 5]);

    // A table that claims more descriptors than its bytes hold is refused.
    let mut short = [0u8; 2 * SEGMENT_DESC_SIZE];
    assert!(SegmentTableMut::from_bytes_mut(&mut short, 3).is_err());
}

#[test]
fn fixed_vec_get_ref_points_into_the_buffer() {
    let mut bytes = [0u8; 4 + 3 * 8];
    let start = bytes.as_ptr() as usize;
    let mut values = FixedVec::<WireU64>::from_bytes(&mut bytes).unwrap();
    values.push(WireU64::new(11)).unwrap();
    values.push(WireU64::new(22)).unwrap();

    assert_eq!(values.get_ref(0).unwrap().get(), 11);
    assert_eq!(values.get_ref(1).unwrap().get(), 22);
    assert_eq!(
        values.get_ref(1).unwrap() as *const WireU64 as usize,
        start + 4 + 8
    );
    // Index 2 is inside the capacity and outside the length.
    assert_eq!(values.get_ref(2).err(), Some(ProgramError::InvalidArgument));
    assert_eq!(
        values.get_ref(usize::MAX).err(),
        Some(ProgramError::InvalidArgument)
    );

    // A stored length the bytes cannot hold is refused at the door, so no
    // accessor ever trusts it.
    let mut forged = [0u8; 4 + 8];
    forged[..4].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(
        FixedVec::<WireU64>::from_bytes(&mut forged).err(),
        Some(ProgramError::InvalidAccountData)
    );
}

#[test]
fn slab_get_ref_reads_only_allocated_slots() {
    const CAPACITY: usize = 4;
    let mut bytes = vec![0u8; SLAB_HEADER_SIZE + bitmap_bytes(CAPACITY) + CAPACITY * 4];
    Slab::<WireU32>::init(&mut bytes, CAPACITY).unwrap();
    let mut slab = Slab::<WireU32>::from_bytes_mut(&mut bytes).unwrap();

    let first = slab.alloc(WireU32::new(101)).unwrap();
    let second = slab.alloc(WireU32::new(202)).unwrap();
    assert_eq!(slab.get_ref(first).unwrap().get(), 101);
    assert_eq!(slab.get_ref(second).unwrap().get(), 202);

    slab.free(first).unwrap();
    // A freed slot holds a free-list link, not a value.
    assert_eq!(
        slab.get_ref(first).err(),
        Some(ProgramError::InvalidArgument)
    );
    assert_eq!(slab.get_ref(second).unwrap().get(), 202);
    for index in [2, 3, CAPACITY as u32, u32::MAX] {
        assert_eq!(
            slab.get_ref(index).err(),
            Some(ProgramError::InvalidArgument),
            "slot {index}"
        );
    }
}

#[test]
fn events_accept_any_fixed_layout_value() {
    // On the host the log syscall is a no-op; what runs here is the view of
    // the value as bytes, which must not read past it.
    let value = WireU64::new(0x0102_0304_0506_0708);
    assert_eq!(emit_event(&value), Ok(()));
    assert_eq!(emit_event_tagged(9, &value), Ok(()));
    assert_eq!(emit_event_tagged(0, &TypedAddress::<()>::new(KEY)), Ok(()));
}
