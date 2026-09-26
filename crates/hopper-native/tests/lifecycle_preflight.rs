use hopper_native::{batch, AccountView, Address, ProgramError, RuntimeAccount, NOT_BORROWED};

#[repr(C)]
struct Backing {
    header: RuntimeAccount,
    data: [u8; 256],
}

fn backing(tag: u8) -> Backing {
    Backing {
        header: RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: 1,
            is_writable: 1,
            executable: 0,
            resize_delta: 16,
            address: Address::new_from_array([tag; 32]),
            owner: Address::new_from_array([0xA1; 32]),
            lamports: 1000,
            data_len: 16,
        },
        data: [tag; 256],
    }
}

fn view(backing: &mut Backing) -> AccountView<'_> {
    // SAFETY: Backing has the loader header layout, initialized original length,
    // and sufficient initialized storage for every resize exercised below.
    unsafe { AccountView::new_unchecked(&mut backing.header) }
}

#[test]
fn close_refusal_is_unchanged_even_when_the_error_is_caught() {
    let mut a = backing(1);
    let mut b = backing(2);
    let source = view(&mut a);
    let destination = view(&mut b);
    let borrowed = source.try_borrow().unwrap();
    assert_eq!(
        batch::close_and_transfer(&source, &destination),
        Err(ProgramError::AccountBorrowFailed)
    );
    assert_eq!(
        (source.lamports(), destination.lamports(), source.data_len()),
        (1000, 1000, 16)
    );
    assert_eq!(&*borrowed, &[1; 16]);
}

#[test]
fn self_transfer_preserves_balance_and_self_close_is_rejected() {
    let mut a = backing(1);
    let source = view(&mut a);
    let alias = source.clone();
    batch::transfer_lamports(&source, &alias, 100).unwrap();
    assert_eq!(source.lamports(), 1000);
    assert_eq!(
        batch::transfer_lamports(&source, &alias, 1001),
        Err(ProgramError::InsufficientFunds)
    );
    assert_eq!(
        batch::close_and_transfer(&source, &alias),
        Err(ProgramError::InvalidArgument)
    );
    assert_eq!(source.lamports(), 1000);
    assert_eq!(source.data_len(), 16);
}

#[test]
fn read_only_recipient_and_overflow_leave_both_accounts_unchanged() {
    for (writable, balance, error) in [
        (0, 1000, ProgramError::Immutable),
        (1, u64::MAX, ProgramError::ArithmeticOverflow),
    ] {
        let mut a = backing(1);
        let mut b = backing(2);
        b.header.is_writable = writable;
        b.header.lamports = balance;
        let source = view(&mut a);
        let destination = view(&mut b);
        assert_eq!(batch::close_and_transfer(&source, &destination), Err(error));
        assert_eq!(
            (source.lamports(), destination.lamports(), source.data_len()),
            (1000, balance, 16)
        );
        assert_eq!(&*source.try_borrow().unwrap(), &[1; 16]);
    }
}

#[test]
fn underfunded_resize_cannot_use_the_target_as_its_own_payer() {
    let mut a = backing(1);
    let source = view(&mut a);
    let rent = hopper_native::sysvar::Rent {
        lamports_per_byte_year: 5,
        exemption_threshold: 2.0,
        burn_percent: 50,
    };
    assert_eq!(
        batch::realloc_checked_with(&rent, &source, 32, Some(&source)),
        Err(ProgramError::InvalidArgument)
    );
    assert_eq!((source.lamports(), source.data_len()), (1000, 16));
    assert_eq!(&*source.try_borrow().unwrap(), &[1; 16]);
}
