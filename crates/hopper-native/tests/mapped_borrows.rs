use hopper_native::{
    borrow::{Ref, RefMut},
    AccountView, Address, ProgramError, RuntimeAccount, NOT_BORROWED,
};

#[repr(C)]
struct Backing {
    header: RuntimeAccount,
    data: [u8; 16],
}
fn backing() -> Backing {
    Backing {
        header: RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: 0,
            is_writable: 1,
            executable: 0,
            resize_delta: 16,
            address: Address::new_from_array([1; 32]),
            owner: Address::new_from_array([2; 32]),
            lamports: 100,
            data_len: 16,
        },
        data: [4; 16],
    }
}
fn view(backing: &mut Backing) -> AccountView<'_> {
    // SAFETY: repr(C) backing has the loader header followed by its 16 data bytes.
    unsafe { AccountView::new_unchecked(&mut backing.header) }
}

#[test]
fn shared_projection_and_failed_selection_keep_the_lease() {
    let mut backing = backing();
    let account = view(&mut backing);
    let mapped = Ref::map(account.try_borrow().unwrap(), |bytes| &bytes[4..8]);
    assert_eq!(&*mapped, &[4; 4]);
    assert_eq!(
        account.check_borrow_mut(),
        Err(ProgramError::AccountBorrowFailed)
    );
    let original = match Ref::filter_map(mapped, |bytes| bytes.get(9)) {
        Err(v) => v,
        Ok(_) => panic!("missing field accepted"),
    };
    assert_eq!(original.len(), 4);
    drop(original);
    assert!(account.check_borrow_mut().is_ok());
}

#[test]
fn mutable_projection_preserves_exclusivity_and_error_returns_original() {
    let mut backing = backing();
    let account = view(&mut backing);
    let original = account.try_borrow_mut().unwrap();
    let (original, error) = match RefMut::try_map(original, |bytes| bytes.get_mut(99).ok_or(7)) {
        Err(v) => v,
        Ok(_) => panic!("missing field accepted"),
    };
    assert_eq!(error, 7);
    assert_eq!(
        account.check_borrow(),
        Err(ProgramError::AccountBorrowFailed)
    );
    let mut mapped = RefMut::map(original, |bytes| &mut bytes[4..8]);
    mapped.copy_from_slice(&[1, 2, 3, 4]);
    assert_eq!(
        account.check_borrow_mut(),
        Err(ProgramError::AccountBorrowFailed)
    );
    drop(mapped);
    assert_eq!(&account.try_borrow().unwrap()[4..8], &[1, 2, 3, 4]);
}

#[test]
fn unwinding_a_mapping_closure_releases_the_borrow() {
    let mut backing = backing();
    let account = view(&mut backing);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        RefMut::map::<u8>(account.try_borrow_mut().unwrap(), |_| {
            panic!("mapping failed")
        });
    }));
    assert!(result.is_err());
    assert!(account.check_borrow_mut().is_ok());
}
