//! A deduplicated info list is resolved by address, never by list position.
use crate::{
    cpi::invoke_signed_deduped,
    instruction::{InstructionAccount, InstructionView},
    AccountView, Address, ProgramError,
};
use hopper_native::{RuntimeAccount, NOT_BORROWED};
use std::vec::Vec;

fn account(key: u8, lamports: u64) -> (Vec<u64>, AccountView<'static>) {
    let mut backing = std::vec![0u64; RuntimeAccount::SIZE.div_ceil(8)];
    let raw = backing.as_mut_ptr().cast::<RuntimeAccount>();
    // SAFETY: the word allocation fits the header and remains live in the test.
    unsafe {
        raw.write(RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: 1,
            is_writable: 1,
            executable: 0,
            resize_delta: 0,
            address: Address::new_from_array([key; 32]).into(),
            owner: Address::new_from_array([0; 32]).into(),
            lamports,
            data_len: 0,
        });
    }
    // SAFETY: initialized, aligned header with empty data, kept alive by the test.
    let native = unsafe { hopper_native::AccountView::new_unchecked(raw) };
    (backing, AccountView::from_backend(native))
}

#[test]
fn dedup_system_transfer_resolves_reordered_infos_and_ignores_extras() {
    let (_s, source) = account(1, 100);
    let (_d, destination) = account(2, 20);
    let (_e, extra) = account(3, 50);
    let metas = [
        InstructionAccount::writable_signer(source.address()),
        InstructionAccount::writable(destination.address()),
    ];
    let system = Address::new_from_array([0; 32]);
    let mut data = [0; 12];
    data[0] = 2;
    data[4..].copy_from_slice(&10u64.to_le_bytes());
    let ix = InstructionView {
        program_id: &system,
        accounts: &metas,
        data: &data,
    };
    invoke_signed_deduped::<3>(&ix, &[&extra, &destination, &source], &[]).unwrap();
    assert_eq!(
        (source.lamports(), destination.lamports(), extra.lamports()),
        (90, 30, 50)
    );
}

#[test]
fn dedup_system_transfer_rejects_missing_meta_before_mutation() {
    let (_s, source) = account(1, 100);
    let (_d, destination) = account(2, 20);
    let metas = [
        InstructionAccount::writable_signer(source.address()),
        InstructionAccount::writable(destination.address()),
    ];
    let system = Address::new_from_array([0; 32]);
    let mut data = [0; 12];
    data[0] = 2;
    data[4..].copy_from_slice(&10u64.to_le_bytes());
    let ix = InstructionView {
        program_id: &system,
        accounts: &metas,
        data: &data,
    };
    assert_eq!(
        invoke_signed_deduped::<1>(&ix, &[&source], &[]),
        Err(ProgramError::NotEnoughAccountKeys)
    );
    assert_eq!((source.lamports(), destination.lamports()), (100, 20));
}
