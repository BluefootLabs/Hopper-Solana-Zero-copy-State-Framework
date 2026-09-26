use hopper_native::{RuntimeAccount, NOT_BORROWED};
use hopper_runtime::{native_boundary::wrap_account_slice, AccountView, Address, ProgramError};
use hopper_solana::{
    constants::{TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID},
    transfer::{TokenTransferOutcome, TokenTransferSnapshot},
};

fn with_accounts(extended: bool, f: impl FnOnce(&[AccountView<'_>])) {
    let len = if extended { 178 } else { 165 };
    let owner = if extended {
        TOKEN_2022_PROGRAM_ID
    } else {
        TOKEN_PROGRAM_ID
    };
    let mut allocations = Vec::new();
    let mut native = Vec::new();
    for (key, amount) in [(1u8, 1000u64), (2, 100)] {
        let mut words = vec![0u64; (RuntimeAccount::SIZE + len).div_ceil(8)];
        let raw = words.as_mut_ptr().cast::<RuntimeAccount>();
        // SAFETY: the complete allocation contains an aligned initialized header
        // and token bytes; allocations outlive the views and all test borrows.
        unsafe {
            raw.write(RuntimeAccount {
                borrow_state: NOT_BORROWED,
                is_signer: 0,
                is_writable: 1,
                executable: 0,
                resize_delta: 0,
                address: hopper_native::Address::new_from_array([key; 32]),
                owner: owner.into(),
                lamports: 1,
                data_len: len as u64,
            });
            let data =
                core::slice::from_raw_parts_mut(raw.cast::<u8>().add(RuntimeAccount::SIZE), len);
            data[..32].fill(7);
            data[32..64].fill(key + 3);
            data[64..72].copy_from_slice(&amount.to_le_bytes());
            data[108] = 1;
            if extended {
                data[165] = 2;
                data[166..168].copy_from_slice(&2u16.to_le_bytes());
                data[168..170].copy_from_slice(&8u16.to_le_bytes());
            }
            native.push(hopper_native::AccountView::new_unchecked(raw));
        }
        allocations.push(words);
    }
    // SAFETY: these fixtures model the native entrypoint's initialized views.
    f(unsafe { wrap_account_slice(&native) });
}

fn amount(account: &AccountView<'_>, n: u64) {
    account.try_borrow_mut().unwrap()[64..72].copy_from_slice(&n.to_le_bytes());
}
const MINT: Address = Address::new_from_array([7; 32]);

#[test]
fn mutable_projection_moves_keep_the_lease_until_drop() {
    with_accounts(false, |a| {
        let guard = a[0].try_borrow_mut().unwrap();
        let guard = guard.slice_from(64);
        let mut guard = guard.slice(0, 8).unwrap();
        guard.copy_from_slice(&900u64.to_le_bytes());
        assert!(a[0].try_borrow().is_err());
        drop(guard);
        assert_eq!(&a[0].try_borrow().unwrap()[64..72], &900u64.to_le_bytes());
        assert!(a[0].try_borrow_mut().unwrap().slice(usize::MAX, 8).is_err());
        assert!(a[0].try_borrow_mut().is_ok());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = a[0].try_borrow_mut().unwrap().slice_from(usize::MAX);
        }));
        assert!(result.is_err());
        assert!(a[0].try_borrow_mut().is_ok());
    });
}

#[test]
fn transfer_policies_cover_exact_fees_shortfalls_reversed_movement_and_extremes() {
    for extended in [false, true] {
        for (debit, credit, minimum, ok) in [
            (100, 100, 100, true),
            (100, 99, 99, true),
            (100, 99, 100, false),
            (100, 0, 0, true),
            (100, 0, 1, false),
            (99, 100, 99, false),
            (101, 100, 99, false),
            (0, 0, 0, false),
            (100, 101, 99, false),
        ] {
            with_accounts(extended, |a| {
                let snap =
                    TokenTransferSnapshot::capture(&a[0], &a[1], &MINT, 100, minimum).unwrap();
                // Capture holds no data borrow: these writes stand in for CPI.
                amount(&a[0], 1000 - debit);
                amount(&a[1], 100 + credit);
                let result = snap.verify();
                if ok {
                    assert_eq!(
                        result.unwrap(),
                        TokenTransferOutcome {
                            debited: debit,
                            credited: credit
                        }
                    );
                } else {
                    assert_eq!(result, Err(ProgramError::InvalidAccountData));
                }
            });
        }
        for (source, destination) in [(1001, 200), (900, 99), (u64::MAX, 200)] {
            with_accounts(extended, |a| {
                let snap = TokenTransferSnapshot::capture(&a[0], &a[1], &MINT, 100, 1).unwrap();
                amount(&a[0], source);
                amount(&a[1], destination);
                assert_eq!(snap.verify(), Err(ProgramError::InvalidAccountData));
            });
        }
    }
}

#[test]
fn capture_rejects_aliases_invalid_policy_funds_mint_and_live_mutable_borrows() {
    with_accounts(false, |a| {
        for (n, min) in [(0, 0), (100, 101)] {
            assert!(matches!(
                TokenTransferSnapshot::capture(&a[0], &a[1], &MINT, n, min),
                Err(ProgramError::InvalidArgument)
            ));
        }
        assert!(matches!(
            TokenTransferSnapshot::capture(&a[0], &a[0], &MINT, 100, 100),
            Err(ProgramError::InvalidArgument)
        ));
        assert!(matches!(
            TokenTransferSnapshot::capture(&a[0], &a[1], &MINT, 1001, 100),
            Err(ProgramError::InsufficientFunds)
        ));
        assert!(matches!(
            TokenTransferSnapshot::capture(
                &a[0],
                &a[1],
                &Address::new_from_array([9; 32]),
                100,
                100
            ),
            Err(ProgramError::InvalidAccountData)
        ));
        let _guard = a[1].try_borrow_mut().unwrap();
        assert!(matches!(
            TokenTransferSnapshot::capture(&a[0], &a[1], &MINT, 100, 100),
            Err(ProgramError::AccountBorrowFailed)
        ));
    });
}

#[test]
fn verify_rechecks_mint_authority_state_shape_owner_and_live_borrows() {
    for extended in [false, true] {
        for field in [0, 32, 108] {
            with_accounts(extended, |a| {
                let snap = TokenTransferSnapshot::capture(&a[0], &a[1], &MINT, 100, 100).unwrap();
                amount(&a[0], 900);
                amount(&a[1], 200);
                a[1].try_borrow_mut().unwrap()[field] = 9;
                assert_eq!(snap.verify(), Err(ProgramError::InvalidAccountData));
            });
        }
        with_accounts(extended, |a| {
            let snap = TokenTransferSnapshot::capture(&a[0], &a[1], &MINT, 100, 100).unwrap();
            // SAFETY: test-owned fixture with no active owner reference.
            unsafe {
                a[1].assign(&Address::new_from_array([9; 32]));
            }
            assert_eq!(snap.verify(), Err(ProgramError::IncorrectProgramId));
        });
        with_accounts(extended, |a| {
            let snap = TokenTransferSnapshot::capture(&a[0], &a[1], &MINT, 100, 100).unwrap();
            let _guard = a[1].try_borrow_mut().unwrap();
            assert_eq!(snap.verify(), Err(ProgramError::AccountBorrowFailed));
        });
    }
    with_accounts(true, |a| {
        let snap = TokenTransferSnapshot::capture(&a[0], &a[1], &MINT, 100, 100).unwrap();
        a[1].try_borrow_mut().unwrap()[165] = 1;
        assert_eq!(snap.verify(), Err(ProgramError::InvalidAccountData));
    });
}
