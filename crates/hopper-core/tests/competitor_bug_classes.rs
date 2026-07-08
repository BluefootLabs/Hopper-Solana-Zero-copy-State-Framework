//! # Competitor-bug-class regression suite (I20) — core lifecycle pins
//!
//! Companion to `hopper-runtime/tests/competitor_bug_classes.rs`. The
//! account self-close class (Quasar issue #240) and the append-migration
//! zeroing guard live behind hopper-core APIs (`account::safe_close*`,
//! `migrate::migrate_append`) that the runtime crate does not re-export,
//! so they are pinned here.
//!
//! Host-side only: accounts are fabricated over owned heap buffers with
//! the same `RuntimeAccount` header layout the Solana loader produces,
//! following the in-module test pattern in `account/lifecycle.rs`.

use hopper_core::account::{
    read_layout_id, read_version, safe_close, safe_close_with_sentinel, write_header,
    CLOSE_SENTINEL,
};
use hopper_core::migrate::migrate_append;
use hopper_native::{
    AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount,
    MAX_PERMITTED_DATA_INCREASE, NOT_BORROWED,
};
use hopper_runtime::{AccountView, Address, ProgramError};

const PROGRAM_BYTES: [u8; 32] = [2; 32];

fn program_id() -> Address {
    Address::new_from_array(PROGRAM_BYTES)
}

/// Fabricate a host-side account over an owned backing buffer.
///
/// The backing includes the loader's `MAX_PERMITTED_DATA_INCREASE`
/// growth reserve so realloc-based paths stay in bounds, and the
/// reserve is pre-filled with `reserve_fill` to simulate whatever stale
/// garbage the heap carried there. The returned `Vec<u64>` (8-aligned) owns the
/// memory and must outlive the `AccountView`.
#[allow(clippy::too_many_arguments)]
fn make_account(
    address_byte: u8,
    owner: [u8; 32],
    is_signer: bool,
    is_writable: bool,
    lamports: u64,
    data: &[u8],
    reserve_fill: u8,
) -> (Vec<u64>, AccountView<'static>) {
    // Word-sized backing: `RuntimeAccount` has u64 fields (align 8), and a
    // `Vec<u8>` allocation only guarantees alignment 1 — writing the header
    // through an under-aligned pointer would be UB by spec even where the
    // system allocator happens to over-align (adversarial review 2026-07-07).
    let total = RuntimeAccount::SIZE + data.len() + MAX_PERMITTED_DATA_INCREASE;
    let mut backing = vec![u64::from_ne_bytes([reserve_fill; 8]); total.div_ceil(8)];
    let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
    // SAFETY: The test owns `backing`, which is sized for one
    // RuntimeAccount header, the payload, and the loader growth reserve;
    // exactly one valid header is written before any view is derived,
    // and the buffer outlives the returned AccountView.
    unsafe {
        raw.write(RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: u8::from(is_signer),
            is_writable: u8::from(is_writable),
            executable: 0,
            resize_delta: 0,
            address: NativeAddress::new_from_array([address_byte; 32]),
            owner: NativeAddress::new_from_array(owner),
            lamports,
            data_len: data.len() as u64,
        });
        let data_ptr = (backing.as_mut_ptr() as *mut u8).add(RuntimeAccount::SIZE);
        core::ptr::copy_nonoverlapping(data.as_ptr(), data_ptr, data.len());
    }
    // SAFETY: `raw` points at the fully initialized RuntimeAccount header
    // written above.
    let backend = unsafe { NativeAccountView::new_unchecked(raw) };
    // SAFETY: hopper-runtime's AccountView is repr(transparent) over the
    // active hopper-native AccountView (compile-time size/align asserts in
    // hopper-runtime's account.rs), so this transmute is a zero-cost
    // identity rewrap.
    let view = unsafe { core::mem::transmute::<NativeAccountView, AccountView>(backend) };
    (backing, view)
}

fn make_program_account(
    address_byte: u8,
    lamports: u64,
    data: &[u8],
) -> (Vec<u64>, AccountView<'static>) {
    make_account(address_byte, PROGRAM_BYTES, true, true, lamports, data, 0)
}

// =====================================================================
// Class 2 — account self-close imbalance
// =====================================================================

/// Bug class: closing an account leaves the instruction unbalanced —
/// lamports vanish or data survives for revival (Quasar issue #240,
/// open 2026-07). Hopper guard:
/// `hopper-core/src/account/lifecycle.rs::safe_close_with_sentinel`
/// (drain via checked add, full data zeroing, revival sentinel). This
/// test pins: after a close, the lamport sum across source and
/// destination is exactly conserved, the source reads zero, every data
/// byte is scrubbed, and byte 0 carries `CLOSE_SENTINEL` so the account
/// cannot be revived as live state.
#[test]
fn close_conserves_lamports_and_scrubs_every_byte() {
    let payload = [0xAB; 32];
    let (_account_backing, account) = make_program_account(1, 10, &payload);
    let (_dest_backing, destination) = make_program_account(2, 5, b"");
    let total_before = account.lamports() + destination.lamports();

    safe_close_with_sentinel(&account, &destination, &program_id()).unwrap();

    // Balanced: nothing minted, nothing burned.
    assert_eq!(account.lamports(), 0);
    assert_eq!(destination.lamports(), 15);
    assert_eq!(account.lamports() + destination.lamports(), total_before);

    // Scrubbed: sentinel at byte 0, zero everywhere else.
    let data = account.try_borrow().unwrap();
    assert_eq!(data.len(), payload.len());
    assert_eq!(data[0], CLOSE_SENTINEL);
    assert!(data[1..].iter().all(|byte| *byte == 0));
}

/// Bug class: close paths that move lamports before validating leave a
/// half-closed account when a later step fails (the imbalance family of
/// Quasar issue #240, open 2026-07). Hopper guard:
/// `hopper-core/src/account/lifecycle.rs::safe_close` /
/// `safe_close_unchecked` fail-closed ordering — writability, ownership,
/// borrow availability, and the checked destination add are all decided
/// before any balance changes. This test pins: on every rejected close
/// (non-writable source, wrong owner, live data borrow, destination
/// lamport overflow) both balances and the account data are exactly as
/// they were.
#[test]
fn rejected_close_moves_nothing_and_scrubs_nothing() {
    let payload = [0xCD; 16];

    // Non-writable source.
    let (_a_backing, account) = make_account(3, PROGRAM_BYTES, true, false, 10, &payload, 0);
    let (_b_backing, destination) = make_program_account(4, 5, b"");
    assert_eq!(
        safe_close(&account, &destination, &program_id()),
        Err(ProgramError::InvalidAccountData)
    );
    assert_eq!(account.lamports(), 10);
    assert_eq!(destination.lamports(), 5);
    assert_eq!(&*account.try_borrow().unwrap(), &payload[..]);

    // Wrong owner.
    let (_c_backing, account) = make_account(5, [9; 32], true, true, 10, &payload, 0);
    let (_d_backing, destination) = make_program_account(6, 5, b"");
    assert_eq!(
        safe_close(&account, &destination, &program_id()),
        Err(ProgramError::IncorrectProgramId)
    );
    assert_eq!(account.lamports(), 10);
    assert_eq!(destination.lamports(), 5);
    assert_eq!(&*account.try_borrow().unwrap(), &payload[..]);

    // Live data borrow: refused before any lamport movement.
    let (_e_backing, account) = make_program_account(7, 10, &payload);
    let (_f_backing, destination) = make_program_account(8, 5, b"");
    {
        let _hold = account.try_borrow().unwrap();
        assert_eq!(
            safe_close(&account, &destination, &program_id()),
            Err(ProgramError::AccountBorrowFailed)
        );
    }
    assert_eq!(account.lamports(), 10);
    assert_eq!(destination.lamports(), 5);
    assert_eq!(&*account.try_borrow().unwrap(), &payload[..]);

    // Destination overflow: the checked add fails before either balance
    // is written, so no lamports are minted or destroyed.
    let (_g_backing, account) = make_program_account(9, 10, &payload);
    let (_h_backing, destination) = make_program_account(10, u64::MAX, b"");
    assert_eq!(
        safe_close(&account, &destination, &program_id()),
        Err(ProgramError::ArithmeticOverflow)
    );
    assert_eq!(account.lamports(), 10);
    assert_eq!(destination.lamports(), u64::MAX);
    assert_eq!(&*account.try_borrow().unwrap(), &payload[..]);
}

/// Bug class: a close whose destination aliases the closed account
/// silently burns the drained lamports inside program scope — the
/// transaction only dies later at the runtime's global lamport-sum
/// check (the self-close shape of Quasar issue #240, open 2026-07).
/// Hopper guard: `lifecycle.rs::safe_close_unchecked` refuses
/// address-aliased source/destination up front (fix landed 2026-07-07,
/// found by this suite's authoring pass). This test pins: an aliased
/// close is rejected before any mutation, through both the checked and
/// sentinel entry points, and the account's balance and data survive.
#[test]
fn aliased_self_close_is_rejected_before_any_mutation() {
    let payload = [0xCD; 16];
    // Two distinct views carrying the SAME address — the on-chain shape
    // of a duplicate account entry passed as both source and destination.
    let (_a_backing, account) = make_program_account(11, 10, &payload);
    let (_b_backing, aliased_destination) = make_program_account(11, 10, &payload);

    assert_eq!(
        safe_close(&account, &aliased_destination, &program_id()),
        Err(ProgramError::InvalidArgument)
    );
    assert_eq!(
        safe_close_with_sentinel(&account, &aliased_destination, &program_id()),
        Err(ProgramError::InvalidArgument)
    );

    // Nothing moved, nothing scrubbed: the balance was not burned and the
    // data was left intact.
    assert_eq!(account.lamports(), 10);
    assert_eq!(aliased_destination.lamports(), 10);
    assert_eq!(&*account.try_borrow().unwrap(), &payload[..]);
}

// =====================================================================
// Class 4 — migration stale state (core append-migration layer)
// =====================================================================

/// Bug class: `Migration::migrate` leaves old fields untouched, so a
/// grown layout exposes stale bytes through the new version's view
/// (Quasar issue #239, open 2026-07). Hopper guard:
/// `hopper-core/src/migrate/mod.rs::migrate_append` — after the realloc
/// it rewrites the header and explicitly zeroes the appended region.
/// This test pins: a V1→V2 append migration that grows the account
/// leaves the surviving V1 payload intact, stamps the new version and
/// layout_id, and serves all-zero bytes in the appended region even when
/// the loader growth reserve was full of garbage beforehand.
#[test]
fn append_migration_zeroes_the_grown_region() {
    const OLD_LAYOUT_ID: [u8; 8] = [0xA1; 8];
    const NEW_LAYOUT_ID: [u8; 8] = [0xB2; 8];
    const OLD_SIZE: usize = 32; // 16-byte header + 16-byte V1 payload
    const NEW_SIZE: usize = 48; // V2 appends 16 bytes

    // V1 account: valid header, recognizable payload, and 0xAA garbage
    // pre-seeded through the growth reserve the realloc will claim.
    let mut v1_data = [0u8; OLD_SIZE];
    write_header(&mut v1_data, 7, 1, &OLD_LAYOUT_ID).unwrap();
    for (i, byte) in v1_data[16..].iter_mut().enumerate() {
        *byte = 0x10 + i as u8;
    }
    let (_account_backing, account) =
        make_account(11, PROGRAM_BYTES, false, true, 0, &v1_data, 0xAA);

    // Rent-funded payer for the realloc delta.
    let rent_needed = hopper_runtime::rent::minimum_balance_live(NEW_SIZE);
    let (_payer_backing, payer) = make_account(12, PROGRAM_BYTES, true, true, rent_needed, b"", 0);

    migrate_append(
        &account,
        &payer,
        &program_id(),
        &OLD_LAYOUT_ID,
        2,
        &NEW_LAYOUT_ID,
        7,
        NEW_SIZE,
    )
    .unwrap();

    assert_eq!(account.data_len(), NEW_SIZE);
    let data = account.try_borrow().unwrap();
    // Header rewritten to the V2 identity.
    assert_eq!(read_version(&data).unwrap(), 2);
    assert_eq!(read_layout_id(&data).unwrap(), NEW_LAYOUT_ID);
    // Surviving V1 payload byte-identical.
    for (i, byte) in data[16..OLD_SIZE].iter().enumerate() {
        assert_eq!(*byte, 0x10 + i as u8);
    }
    // The grown region is all zeros: none of the 0xAA reserve garbage is
    // reachable through the V2 view.
    assert!(data[OLD_SIZE..NEW_SIZE].iter().all(|byte| *byte == 0));
}

/// Bug class: migrations that run against the wrong source shape corrupt
/// state instead of refusing (the stale-state family of Quasar issue
/// #239, open 2026-07). Hopper guard:
/// `hopper-core/src/migrate/mod.rs::migrate_append` preflight — source
/// layout_id, forward-only version, and strict growth are all validated
/// before the realloc. This test pins: a layout_id mismatch, a
/// non-advancing version, and a non-growing size are each rejected with
/// the account bytes and length untouched.
#[test]
fn append_migration_refuses_wrong_source_shape_without_touching_state() {
    const OLD_LAYOUT_ID: [u8; 8] = [0xA1; 8];
    const NEW_LAYOUT_ID: [u8; 8] = [0xB2; 8];

    let mut v1_data = [0u8; 32];
    write_header(&mut v1_data, 7, 1, &OLD_LAYOUT_ID).unwrap();
    v1_data[16] = 0x55;

    let rent_needed = hopper_runtime::rent::minimum_balance_live(48);
    let cases: [(&[u8; 8], u8, usize, ProgramError); 3] = [
        // Wrong source layout_id.
        (&NEW_LAYOUT_ID, 2, 48, ProgramError::InvalidAccountData),
        // Version does not advance.
        (&OLD_LAYOUT_ID, 1, 48, ProgramError::InvalidAccountData),
        // Not a growth.
        (&OLD_LAYOUT_ID, 2, 32, ProgramError::InvalidArgument),
    ];

    for (i, (old_id, new_version, new_size, expected)) in cases.into_iter().enumerate() {
        let (_account_backing, account) =
            make_account(20 + i as u8, PROGRAM_BYTES, false, true, 0, &v1_data, 0xAA);
        let (_payer_backing, payer) =
            make_account(40 + i as u8, PROGRAM_BYTES, true, true, rent_needed, b"", 0);

        assert_eq!(
            migrate_append(
                &account,
                &payer,
                &program_id(),
                old_id,
                new_version,
                &NEW_LAYOUT_ID,
                7,
                new_size,
            ),
            Err(expected)
        );
        assert_eq!(account.data_len(), 32);
        assert_eq!(&*account.try_borrow().unwrap(), &v1_data[..]);
        assert_eq!(payer.lamports(), rent_needed);
    }
}
