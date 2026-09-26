//! Regression coverage for specialized CPI helpers that bypass the generic
//! instruction builder but must retain its borrow/privilege preflight.

use hopper_native::system::{CreateAccount, CreateAccountWithSeed, TransferWithSeed};
use hopper_native::token::Transfer as TokenTransfer;
use hopper_native::{AccountView, Address, ProgramError, RuntimeAccount, NOT_BORROWED};

#[repr(C)]
struct Backing {
    header: RuntimeAccount,
    data: [u8; 16],
}

fn backing(tag: u8, writable: bool) -> Backing {
    Backing {
        header: RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: 1,
            is_writable: u8::from(writable),
            executable: 0,
            resize_delta: 16,
            address: Address::new_from_array([tag; 32]),
            owner: Address::new_from_array([0xA1; 32]),
            lamports: 1_000,
            data_len: 16,
        },
        data: [tag; 16],
    }
}

fn assert_unchanged(account: &AccountView<'_>, lamports: u64, len: usize, first: u8) {
    assert_eq!(account.lamports(), lamports);
    assert_eq!(account.data_len(), len);
    assert_eq!(account.try_borrow().unwrap()[0], first);
}

#[test]
fn specialized_unsigned_signers_fail_locally_but_pdas_reach_runtime_validation() {
    use hopper_native::instruction::{Seed, Signer};
    let mut from_backing = backing(10, true);
    from_backing.header.is_signer = 0;
    let mut to_backing = backing(11, true);
    to_backing.header.is_signer = 0;
    let from = unsafe { AccountView::new_unchecked(&mut from_backing.header) };
    let to = unsafe { AccountView::new_unchecked(&mut to_backing.header) };
    let transfer = hopper_native::system::Transfer {
        from: &from,
        to: &to,
        lamports: 1,
    };
    assert_eq!(
        transfer.invoke(),
        Err(ProgramError::MissingRequiredSignature)
    );
    let token = TokenTransfer {
        from: &from,
        to: &to,
        authority: &from,
        amount: 1,
    };
    assert_eq!(token.invoke(), Err(ProgramError::MissingRequiredSignature));
    let seed = [Seed::from(&b"authority"[..])];
    let signers = [Signer::from(&seed[..])];
    // Host invoke is a no-op: this proves deferral, NOT valid PDA derivation.
    transfer.invoke_signed(&signers).unwrap();
    token.invoke_signed(&signers).unwrap();
    assert_unchanged(&from, 1000, 16, 10);
    assert_unchanged(&to, 1000, 16, 11);
}

#[test]
fn system_create_account_rejects_live_borrows_before_invoke_without_mutation() {
    let mut from_backing = backing(1, true);
    let mut to_backing = backing(2, true);
    let from = unsafe { AccountView::new_unchecked(&mut from_backing.header) };
    let to = unsafe { AccountView::new_unchecked(&mut to_backing.header) };
    let owner = Address::new_from_array([7; 32]);

    let from_shared = from.try_borrow().unwrap();
    assert_eq!(
        CreateAccount {
            from: &from,
            to: &to,
            lamports: 10,
            space: 8,
            owner: &owner,
        }
        .invoke(),
        Err(ProgramError::AccountBorrowFailed)
    );
    assert_eq!(from.lamports(), 1_000);
    assert_eq!(to.lamports(), 1_000);
    assert_eq!(from.data_len(), 16);
    assert_eq!(to.data_len(), 16);
    assert_eq!(from_shared[0], 1);
    drop(from_shared);

    let to_exclusive = to.try_borrow_mut().unwrap();
    assert_eq!(
        CreateAccount {
            from: &from,
            to: &to,
            lamports: 10,
            space: 8,
            owner: &owner,
        }
        .invoke(),
        Err(ProgramError::AccountBorrowFailed)
    );
    assert_eq!(to_exclusive[0], 2);
    drop(to_exclusive);

    // The host syscall is a no-op, but successful preflight proves both
    // writable accounts become invocable once their guards are gone.
    CreateAccount {
        from: &from,
        to: &to,
        lamports: 10,
        space: 8,
        owner: &owner,
    }
    .invoke()
    .unwrap();
    assert_unchanged(&from, 1_000, 16, 1);
    assert_unchanged(&to, 1_000, 16, 2);
}

#[test]
fn system_seeded_helpers_distinguish_readonly_and_writable_metas() {
    let mut from_backing = backing(3, true);
    let mut to_backing = backing(4, true);
    let mut base_backing = backing(5, true);
    let from = unsafe { AccountView::new_unchecked(&mut from_backing.header) };
    let to = unsafe { AccountView::new_unchecked(&mut to_backing.header) };
    let base = unsafe { AccountView::new_unchecked(&mut base_backing.header) };
    let owner = Address::new_from_array([8; 32]);

    // `base` is a read-only meta, so a shared borrow is compatible.
    let base_shared = base.try_borrow().unwrap();
    CreateAccountWithSeed {
        from: &from,
        to: &to,
        base: &base,
        seed: b"seed",
        lamports: 10,
        space: 8,
        owner: &owner,
    }
    .invoke()
    .unwrap();
    assert_eq!(base_shared[0], 5);
    drop(base_shared);

    // A mutable borrow conflicts even with a read-only CPI meta.
    let base_exclusive = base.try_borrow_mut().unwrap();
    assert_eq!(
        CreateAccountWithSeed {
            from: &from,
            to: &to,
            base: &base,
            seed: b"seed",
            lamports: 10,
            space: 8,
            owner: &owner,
        }
        .invoke(),
        Err(ProgramError::AccountBorrowFailed)
    );
    assert_eq!(base_exclusive[0], 5);
    drop(base_exclusive);

    // TransferWithSeed's writable destination is index 2; pin the high mask
    // bit so a future account-order edit cannot silently skip it.
    let to_shared = to.try_borrow().unwrap();
    assert_eq!(
        TransferWithSeed {
            from: &from,
            base: &base,
            to: &to,
            lamports: 1,
            from_seed: b"seed",
            from_owner: &owner,
        }
        .invoke(),
        Err(ProgramError::AccountBorrowFailed)
    );
    assert_eq!(to_shared[0], 4);
    drop(to_shared);

    assert_unchanged(&from, 1_000, 16, 3);
    assert_unchanged(&to, 1_000, 16, 4);
    assert_unchanged(&base, 1_000, 16, 5);
}

#[test]
fn token_transfer_preflights_each_meta_and_outer_writable_privilege() {
    let mut from_backing = backing(6, true);
    let mut to_backing = backing(7, true);
    let mut authority_backing = backing(8, true);
    let from = unsafe { AccountView::new_unchecked(&mut from_backing.header) };
    let to = unsafe { AccountView::new_unchecked(&mut to_backing.header) };
    let authority = unsafe { AccountView::new_unchecked(&mut authority_backing.header) };

    // A shared borrow of the read-only authority is compatible.
    let authority_shared = authority.try_borrow().unwrap();
    TokenTransfer {
        from: &from,
        to: &to,
        authority: &authority,
        amount: 1,
    }
    .invoke()
    .unwrap();
    assert_eq!(authority_shared[0], 8);
    drop(authority_shared);

    let authority_exclusive = authority.try_borrow_mut().unwrap();
    assert_eq!(
        TokenTransfer {
            from: &from,
            to: &to,
            authority: &authority,
            amount: 1,
        }
        .invoke(),
        Err(ProgramError::AccountBorrowFailed)
    );
    assert_eq!(authority_exclusive[0], 8);
    drop(authority_exclusive);

    let from_shared = from.try_borrow().unwrap();
    assert_eq!(
        TokenTransfer {
            from: &from,
            to: &to,
            authority: &authority,
            amount: 1,
        }
        .invoke(),
        Err(ProgramError::AccountBorrowFailed)
    );
    assert_eq!(from_shared[0], 6);
    drop(from_shared);

    // Writable CPI metas must also carry writable outer privilege, and the
    // failure must occur before any account state changes.
    let to_raw = to.account_ptr() as *mut RuntimeAccount;
    unsafe {
        (*to_raw).is_writable = 0;
    }
    assert_eq!(
        TokenTransfer {
            from: &from,
            to: &to,
            authority: &authority,
            amount: 1,
        }
        .invoke(),
        Err(ProgramError::Immutable)
    );
    assert_unchanged(&from, 1_000, 16, 6);
    assert_unchanged(&to, 1_000, 16, 7);
    assert_unchanged(&authority, 1_000, 16, 8);
}
