//! # Competitor-bug-class regression suite (I20)
//!
//! Quasar's open tracker carries several unsoundness/correctness classes
//! that Hopper claims structural immunity to. Each test below pins one
//! claim to an actual guard in the runtime so a regression that
//! reintroduces the bug class fails CI instead of shipping. The
//! return-data class is pinned in a separate suite.
//!
//! Host-side only: accounts are fabricated over owned heap buffers with
//! the same `RuntimeAccount` header layout the Solana loader produces,
//! exactly as the in-module tests in `remaining.rs`, `audit.rs`, and
//! `migrate.rs` do.

use hopper_native::{
    AccountView as NativeAccountView, Address as NativeAddress, RuntimeAccount, NOT_BORROWED,
};
use hopper_runtime::remaining::{RemainingAccounts, RemainingError, MAX_REMAINING_ACCOUNTS};
use hopper_runtime::segment_borrow::SegmentBorrowRegistry;
use hopper_runtime::write_policy::{write_policy_violation, WritePolicy, WriteRange};
use hopper_runtime::{
    apply_pending_migrations, AccountAudit, AccountView, Address, FieldInfo, FieldMap,
    HopperHeader, LayoutContract, LayoutMigration, MigrationEdge, ProgramError,
};

const DEFAULT_OWNER: [u8; 32] = [9; 32];

/// Fabricate a host-side account over an owned backing buffer.
///
/// The returned `Vec<u64>` (8-aligned) owns the memory; it must stay alive for as
/// long as the `AccountView` is used (its heap allocation is stable
/// across moves of the `Vec` itself).
fn make_account(
    address: [u8; 32],
    owner: [u8; 32],
    is_signer: bool,
    is_writable: bool,
    lamports: u64,
    data: &[u8],
) -> (Vec<u64>, AccountView<'static>) {
    // Word-sized backing: `RuntimeAccount` has u64 fields (align 8), and a
    // `Vec<u8>` allocation only guarantees alignment 1 — writing the header
    // through an under-aligned pointer would be UB by spec even where the
    // system allocator happens to over-align (adversarial review 2026-07-07).
    let total = RuntimeAccount::SIZE + data.len();
    let mut backing = vec![0u64; total.div_ceil(8)];
    let raw = backing.as_mut_ptr() as *mut RuntimeAccount;
    // SAFETY: The test owns `backing`, which is 8-aligned (Vec<u64>) and
    // sized for one RuntimeAccount header plus `data.len()` payload bytes;
    // exactly one valid header is written before any view is derived, and
    // the buffer outlives the returned AccountView.
    unsafe {
        raw.write(RuntimeAccount {
            borrow_state: NOT_BORROWED,
            is_signer: u8::from(is_signer),
            is_writable: u8::from(is_writable),
            executable: 0,
            resize_delta: 0,
            address: NativeAddress::new_from_array(address),
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
    // account.rs), so this transmute is a zero-cost identity rewrap.
    let view = unsafe { core::mem::transmute::<NativeAccountView, AccountView>(backend) };
    (backing, view)
}

/// Build `count` distinct, non-signer, read-only accounts whose
/// addresses are `[1;32]`, `[2;32]`, ... `[count;32]`.
fn make_distinct_accounts(count: usize) -> (Vec<Vec<u64>>, Vec<AccountView<'static>>) {
    assert!(count < 256, "test helper uses one address byte per account");
    let mut backings = Vec::with_capacity(count);
    let mut views = Vec::with_capacity(count);
    for i in 0..count {
        let (backing, view) =
            make_account([(i + 1) as u8; 32], DEFAULT_OWNER, false, false, 1, b"");
        backings.push(backing);
        views.push(view);
    }
    (backings, views)
}

// =====================================================================
// Class 1 — remaining-accounts capacity honesty
// =====================================================================

/// Bug class: `Remaining<T, N>` advertises capacity `N` but the real cap
/// is `min(N, 64 / COUNT)`, so callers past the silent cap read absent
/// accounts (Quasar issue #242, open 2026-07). Hopper guard:
/// `hopper-runtime/src/remaining.rs::RemainingAccounts::len` /
/// `MAX_REMAINING_ACCOUNTS`. This test pins: the advertised length is
/// always the true slice length in every mode, including the empty
/// boundary, and out-of-range access is an explicit `Ok(None)` rather
/// than a phantom account.
#[test]
fn remaining_advertised_length_is_true_length_at_zero_and_nonzero() {
    // Empty boundary: zero remaining accounts.
    let empty: [AccountView<'static>; 0] = [];
    for view in [
        RemainingAccounts::strict(&empty, &empty),
        RemainingAccounts::passthrough(&empty, &empty),
    ] {
        assert_eq!(view.len(), 0);
        assert!(view.is_empty());
        assert_eq!(view.get(0).unwrap(), None);
        assert!(view.iter().next().is_none());
        assert_eq!(view.as_slice().len(), 0);
    }

    // Non-empty: len() equals the backing slice length exactly, and every
    // advertised index serves the account actually at that slot.
    let (_backings, views) = make_distinct_accounts(5);
    let strict = RemainingAccounts::strict(&[], &views);
    assert_eq!(strict.len(), 5);
    for (i, item) in strict.iter().enumerate() {
        assert_eq!(item.unwrap().address(), views[i].address());
    }
    // One past the end: honest absence, not an error and not a phantom.
    assert_eq!(strict.get(5).unwrap(), None);
}

/// Bug class: remaining-accounts capacity dishonesty (Quasar issue #242,
/// open 2026-07). Hopper guard:
/// `hopper-runtime/src/remaining.rs::MAX_REMAINING_ACCOUNTS` and the
/// explicit `RemainingError::Overflow` in `RemainingIter::next` /
/// `RemainingAccounts::get`. This test pins: the documented cap of 64 is
/// enforced loudly — at exactly 64 accounts every slot is served, at 65
/// the 65th access is a hard `Overflow` error (both modes' iterators),
/// never a silently truncated or aliased view.
#[test]
fn remaining_cap_is_exact_and_loud_at_the_64_boundary() {
    assert_eq!(MAX_REMAINING_ACCOUNTS, 64);

    // Exactly the cap: all 64 slots served, then clean exhaustion.
    let (_backings, views) = make_distinct_accounts(MAX_REMAINING_ACCOUNTS);
    let strict = RemainingAccounts::strict(&[], &views);
    let mut iter = strict.iter();
    for expected in &views {
        assert_eq!(iter.next().unwrap().unwrap().address(), expected.address());
    }
    assert!(iter.next().is_none());

    // One past the cap: 64 slots served, 65th is a hard error, then the
    // iterator terminates deterministically. Same behavior in
    // passthrough mode — the cap is not a strict-mode side effect.
    let (_backings, views) = make_distinct_accounts(MAX_REMAINING_ACCOUNTS + 1);
    for view in [
        RemainingAccounts::strict(&[], &views),
        RemainingAccounts::passthrough(&[], &views),
    ] {
        assert_eq!(view.len(), MAX_REMAINING_ACCOUNTS + 1);
        let mut iter = view.iter();
        for _ in 0..MAX_REMAINING_ACCOUNTS {
            assert!(iter.next().unwrap().is_ok());
        }
        assert_eq!(
            iter.next().unwrap().unwrap_err(),
            ProgramError::from(RemainingError::Overflow)
        );
        assert!(iter.next().is_none());
    }

    // Strict random access at the cap index is the same hard error.
    let strict = RemainingAccounts::strict(&[], &views);
    assert!(strict.get(MAX_REMAINING_ACCOUNTS - 1).unwrap().is_some());
    assert_eq!(
        strict.get(MAX_REMAINING_ACCOUNTS).unwrap_err(),
        ProgramError::from(RemainingError::Overflow)
    );
}

/// Bug class: typed remaining wrappers advertising more capacity than
/// they parse (Quasar issue #242, open 2026-07). Hopper guard:
/// `hopper-runtime/src/remaining.rs::RemainingAccounts::account_views` /
/// `signers` overflow checks. This test pins: a bounded typed set of
/// `N` parses exactly `len` accounts when `len <= N` (each slot the real
/// account), refuses `len > N` with `Overflow`, and never fabricates
/// entries past `len` — at 0, at `N`, and at `N + 1`.
#[test]
fn typed_remaining_sets_parse_exactly_what_they_advertise() {
    // Zero accounts through an N=4 window.
    let empty: [AccountView<'static>; 0] = [];
    let views4 = RemainingAccounts::strict(&empty, &empty)
        .account_views::<4>()
        .unwrap();
    assert_eq!(views4.len(), 0);
    assert!(views4.is_empty());
    assert!(views4.get(0).is_none());

    // Exactly N accounts: every slot served, index N is honest None.
    let (_backings, views) = make_distinct_accounts(4);
    let parsed = RemainingAccounts::strict(&[], &views)
        .account_views::<4>()
        .unwrap();
    assert_eq!(parsed.len(), 4);
    for (i, expected) in views.iter().enumerate() {
        assert_eq!(parsed.get(i).unwrap().address(), expected.address());
    }
    assert!(parsed.get(4).is_none());
    assert_eq!(parsed.iter().count(), 4);

    // N + 1 accounts through an N window: loud overflow, no truncation.
    let (_backings, views) = make_distinct_accounts(5);
    assert_eq!(
        RemainingAccounts::strict(&[], &views)
            .account_views::<4>()
            .err()
            .unwrap(),
        ProgramError::from(RemainingError::Overflow)
    );

    // Same shape for the signer set.
    let mut backings = Vec::new();
    let mut signer_views = Vec::new();
    for i in 0..3usize {
        let (backing, view) =
            make_account([(i + 40) as u8; 32], DEFAULT_OWNER, true, false, 1, b"");
        backings.push(backing);
        signer_views.push(view);
    }
    let signers = RemainingAccounts::strict(&[], &signer_views)
        .signers::<3>()
        .unwrap();
    assert_eq!(signers.len(), 3);
    for (i, expected) in signer_views.iter().enumerate() {
        assert_eq!(signers.get(i).unwrap().key(), expected.address());
    }
    assert!(signers.get(3).is_none());
    assert_eq!(
        RemainingAccounts::strict(&[], &signer_views)
            .signers::<2>()
            .err()
            .unwrap(),
        ProgramError::from(RemainingError::Overflow)
    );
}

/// Bug class: capacity dishonesty in sequential/lazy remaining parsers
/// (Quasar issue #242, open 2026-07). Hopper guard:
/// `hopper-runtime/src/remaining.rs::RemainingTyped` bookkeeping and
/// `RemainingLazy::at` bounds check. This test pins: `consumed` plus
/// `remaining_len` always equals the true tail length, consuming past
/// the end is `NotEnoughAccountKeys` (never a stale re-serve), group
/// splits cannot overrun the tail, and lazy access one past the end is
/// rejected.
#[test]
fn typed_and_lazy_parsers_report_and_enforce_true_capacity() {
    let (_backings, views) = make_distinct_accounts(3);
    let accounts = RemainingAccounts::strict(&[], &views);

    let mut typed = accounts.typed();
    assert_eq!(typed.consumed(), 0);
    assert_eq!(typed.remaining_len(), 3);
    for (i, expected) in views.iter().enumerate() {
        let account = typed.next_account().unwrap();
        assert_eq!(account.address(), expected.address());
        assert_eq!(typed.consumed(), i + 1);
        assert_eq!(typed.remaining_len(), 3 - (i + 1));
    }
    assert!(typed.is_empty());
    assert!(typed.assert_empty().is_ok());
    // Past the end: hard error, and the cursor stays exhausted.
    assert_eq!(
        typed.next_account().unwrap_err(),
        ProgramError::NotEnoughAccountKeys
    );
    assert_eq!(typed.consumed(), 3);

    // A group larger than the unconsumed tail is refused up front.
    let mut typed = accounts.typed();
    assert_eq!(
        typed.take_group(4).err().unwrap(),
        ProgramError::NotEnoughAccountKeys
    );
    // A group of exactly the tail length consumes it fully.
    let mut group = typed.take_group(3).unwrap();
    assert_eq!(group.remaining_len(), 3);
    for expected in &views {
        assert_eq!(group.next_account().unwrap().address(), expected.address());
    }
    assert!(group.assert_empty().is_ok());
    assert!(typed.is_empty());

    // Lazy access: valid indexes serve the true slot, len is honest,
    // one-past-the-end is rejected.
    let lazy = accounts.lazy();
    assert_eq!(lazy.len(), 3);
    assert_eq!(lazy.at(2).unwrap().account().address(), views[2].address());
    assert_eq!(
        lazy.at(3).err().unwrap(),
        ProgramError::NotEnoughAccountKeys
    );
}

// =====================================================================
// Class 3 — duplicate-account aliasing
// =====================================================================

/// Bug class: raw-handler footgun where `ptr::read`-aliased AccountViews
/// allow two unchecked mutable borrows of the same account (Quasar raw
/// handlers, open 2026-07). Hopper guard:
/// `hopper-runtime/src/audit.rs::AccountAudit::require_unique_writable`
/// / `require_unique_signers` / `require_all_unique` — the exact
/// implementations behind `Context::require_unique_*_accounts`. This
/// test pins: an instruction account slice containing the same address
/// twice is rejected whenever a duplicate is writable or claims a
/// signer role, including the close-shaped alias where one account is
/// passed as both source and lamport destination.
#[test]
fn duplicate_accounts_in_an_instruction_slice_are_rejected() {
    // Writable alias: the double-mutable-borrow setup.
    let (_a_backing, first) = make_account([7; 32], DEFAULT_OWNER, false, true, 1, b"");
    let (_b_backing, second) = make_account([7; 32], DEFAULT_OWNER, false, false, 1, b"");
    let accounts = [first, second];
    let audit = AccountAudit::new(&accounts);
    let duplicate = audit.first_duplicate_writable().unwrap();
    assert_eq!(duplicate.first_index, 0);
    assert_eq!(duplicate.second_index, 1);
    assert_eq!(duplicate.address, Address::new_from_array([7; 32]));
    assert_eq!(
        audit.require_unique_writable(),
        Err(ProgramError::InvalidArgument)
    );
    assert_eq!(
        audit.require_all_unique(),
        Err(ProgramError::InvalidArgument)
    );

    // Signer alias: one key smuggled into two signer roles.
    let (_c_backing, first) = make_account([8; 32], DEFAULT_OWNER, true, false, 1, b"");
    let (_d_backing, second) = make_account([8; 32], DEFAULT_OWNER, false, false, 1, b"");
    let accounts = [first, second];
    let audit = AccountAudit::new(&accounts);
    assert_eq!(
        audit.require_unique_signers(),
        Err(ProgramError::InvalidArgument)
    );

    // Distinct accounts sail through every audit.
    let (_e_backing, first) = make_account([10; 32], DEFAULT_OWNER, true, true, 1, b"");
    let (_f_backing, second) = make_account([11; 32], DEFAULT_OWNER, true, true, 1, b"");
    let accounts = [first, second];
    let audit = AccountAudit::new(&accounts);
    assert_eq!(audit.require_all_unique(), Ok(()));
    assert_eq!(audit.require_unique_writable(), Ok(()));
    assert_eq!(audit.require_unique_signers(), Ok(()));
    assert!(audit.first_duplicate().is_none());
}

/// Bug class: unchecked mutable aliasing of one account's bytes through
/// two live views (Quasar raw handlers, open 2026-07). Hopper guard:
/// `hopper-runtime/src/segment_borrow.rs::SegmentBorrowRegistry::register`
/// conflict scan (full-address identity, not fingerprint). This test
/// pins: two overlapping mutable segment borrows on the same account are
/// rejected with `AccountBorrowFailed`, write/read overlap is rejected
/// in both registration orders, while shared reads, disjoint writes,
/// same-range borrows on *different* accounts, and re-registration
/// after release all remain legal.
#[test]
fn overlapping_mutable_segment_borrows_on_one_account_are_rejected() {
    let vault = Address::new_from_array([21; 32]);
    let pool = Address::new_from_array([22; 32]);

    // Write/write overlap: rejected.
    let mut registry = SegmentBorrowRegistry::new();
    registry.register_write(&vault, 0, 16).unwrap();
    assert_eq!(
        registry.register_write(&vault, 8, 16).unwrap_err(),
        ProgramError::AccountBorrowFailed
    );

    // Read/write overlap: rejected in both orders.
    let mut registry = SegmentBorrowRegistry::new();
    registry.register_read(&vault, 0, 8).unwrap();
    assert_eq!(
        registry.register_write(&vault, 4, 8).unwrap_err(),
        ProgramError::AccountBorrowFailed
    );
    let mut registry = SegmentBorrowRegistry::new();
    registry.register_write(&vault, 0, 8).unwrap();
    assert_eq!(
        registry.register_read(&vault, 4, 8).unwrap_err(),
        ProgramError::AccountBorrowFailed
    );

    // Legal shapes stay legal: shared reads, disjoint writes, and the
    // same byte range on a different account.
    let mut registry = SegmentBorrowRegistry::new();
    registry.register_read(&vault, 0, 8).unwrap();
    registry.register_read(&vault, 0, 8).unwrap();
    registry.register_write(&vault, 8, 8).unwrap();
    registry.register_write(&pool, 8, 8).unwrap();

    // RAII release actually frees the range for a new writer.
    let mut registry = SegmentBorrowRegistry::new();
    let borrow = registry.register_leased_write(&vault, 0, 8).unwrap();
    assert_eq!(
        registry.register_write(&vault, 0, 8).unwrap_err(),
        ProgramError::AccountBorrowFailed
    );
    assert!(registry.release(&borrow));
    registry.register_write(&vault, 0, 8).unwrap();
}

// =====================================================================
// Class 4 — migration stale state
// =====================================================================

/// Body layout used by the migration stale-state pin below. V1 wrote
/// only the first 4 body bytes; V2 (SCHEMA_EPOCH = 2) claims the next 4
/// reserved bytes, so the 1→2 migrator must scrub them.
#[repr(C)]
#[derive(Clone, Copy)]
struct CounterV2 {
    v: [u8; 8],
}
// SAFETY: repr(C) single byte-array field — every bit pattern is valid,
// alignment 1, no padding.
unsafe impl hopper_runtime::Zeroable for CounterV2 {}
// SAFETY: as above.
unsafe impl hopper_runtime::Pod for CounterV2 {}
// SAFETY: test-local layout upholding the sealed overlay contract.
unsafe impl hopper_runtime::__sealed::HopperZeroCopySealed for CounterV2 {}
impl FieldMap for CounterV2 {
    const FIELDS: &'static [FieldInfo] = &[FieldInfo::new("v", HopperHeader::SIZE, 8)];
}
impl LayoutContract for CounterV2 {
    const DISC: u8 = 77;
    const VERSION: u8 = 1;
    const LAYOUT_ID: [u8; 8] = [0x77; 8];
    const SIZE: usize = HopperHeader::SIZE + core::mem::size_of::<Self>();
    const SCHEMA_EPOCH: u32 = 2;
}

fn grow_v1_to_v2(body: &mut [u8]) -> Result<(), ProgramError> {
    if body.len() < 8 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    // V2 claims body bytes 4..8. Whatever the account carried there
    // (reserved padding, stale garbage) must be rewritten, never exposed
    // through the V2 view.
    for byte in &mut body[4..8] {
        *byte = 0;
    }
    Ok(())
}

impl LayoutMigration for CounterV2 {
    const MIGRATIONS: &'static [MigrationEdge] = &[MigrationEdge {
        from_epoch: 1,
        to_epoch: 2,
        migrator: grow_v1_to_v2,
    }];
}

/// Build an epoch-1 account whose entire body is stale garbage (0xAA)
/// except the 4 V1 payload bytes, with the header epoch field stamped 1.
fn make_v1_counter_account(address_byte: u8) -> (Vec<u64>, AccountView<'static>) {
    let mut data = [0xAAu8; HopperHeader::SIZE + 8];
    // Header bytes 12..16 carry schema_epoch (u32 LE) per layout.rs.
    data[12..16].copy_from_slice(&1u32.to_le_bytes());
    // V1 payload: body bytes 0..4.
    data[HopperHeader::SIZE..HopperHeader::SIZE + 4].copy_from_slice(&[1, 2, 3, 4]);
    make_account([address_byte; 32], DEFAULT_OWNER, false, true, 1, &data)
}

/// Bug class: `Migration::migrate` leaves old fields untouched, so a
/// grown layout exposes stale prior-version bytes through the new view
/// (Quasar issue #239, open 2026-07). Hopper guard:
/// `hopper-runtime/src/migrate.rs::apply_pending_migrations` +
/// `MigrationEdge` — the migrator receives the full mutable body and the
/// runtime stamps the header epoch only after the edge succeeded. This
/// test pins: after a 1→2 edge that grows the layout, the newly claimed
/// region carries migrator-written bytes (no 0xAA stale garbage leaks
/// into the V2 view), the surviving V1 payload is intact, the header
/// epoch reads 2, and re-running the chain is a no-op.
#[test]
fn migration_edge_that_grows_the_layout_leaves_no_stale_bytes() {
    let (_backing, account) = make_v1_counter_account(30);

    let applied =
        apply_pending_migrations::<CounterV2>(&account, &Address::new_from_array(DEFAULT_OWNER), 1)
            .unwrap();
    assert_eq!(applied, 1);

    {
        let data = account.try_borrow().unwrap();
        // Header epoch atomically stamped to the edge's to_epoch.
        assert_eq!(&data[12..16], &2u32.to_le_bytes());
        // Surviving V1 payload untouched.
        assert_eq!(
            &data[HopperHeader::SIZE..HopperHeader::SIZE + 4],
            &[1, 2, 3, 4]
        );
        // Newly claimed region rewritten by the migrator: no stale 0xAA
        // byte is reachable through the V2 body.
        assert_eq!(
            &data[HopperHeader::SIZE + 4..HopperHeader::SIZE + 8],
            &[0, 0, 0, 0]
        );
    }

    // Idempotence: the account is now at the target epoch, so the chain
    // applies zero edges and rewrites nothing.
    assert_eq!(
        apply_pending_migrations::<CounterV2>(&account, &Address::new_from_array(DEFAULT_OWNER), 2),
        Ok(0)
    );

    // From-the-future accounts are refused outright, never "migrated".
    assert_eq!(
        apply_pending_migrations::<CounterV2>(&account, &Address::new_from_array(DEFAULT_OWNER), 3),
        Err(ProgramError::InvalidAccountData)
    );
}

/// Second layout for the atomicity pin: same shape as [`CounterV2`] but
/// its only edge fails after partially writing the body.
#[repr(C)]
#[derive(Clone, Copy)]
struct PoisonV2 {
    v: [u8; 8],
}
// SAFETY: repr(C) single byte-array field — every bit pattern is valid,
// alignment 1, no padding.
unsafe impl hopper_runtime::Zeroable for PoisonV2 {}
// SAFETY: as above.
unsafe impl hopper_runtime::Pod for PoisonV2 {}
// SAFETY: test-local layout upholding the sealed overlay contract.
unsafe impl hopper_runtime::__sealed::HopperZeroCopySealed for PoisonV2 {}
impl FieldMap for PoisonV2 {
    const FIELDS: &'static [FieldInfo] = &[FieldInfo::new("v", HopperHeader::SIZE, 8)];
}
impl LayoutContract for PoisonV2 {
    const DISC: u8 = 78;
    const VERSION: u8 = 1;
    const LAYOUT_ID: [u8; 8] = [0x78; 8];
    const SIZE: usize = HopperHeader::SIZE + core::mem::size_of::<Self>();
    const SCHEMA_EPOCH: u32 = 2;
}

fn failing_migrator(body: &mut [u8]) -> Result<(), ProgramError> {
    // Simulate a migrator that dirties the body and then errors.
    if let Some(first) = body.first_mut() {
        *first = 0xEE;
    }
    Err(ProgramError::Custom(0xDEAD))
}

impl LayoutMigration for PoisonV2 {
    const MIGRATIONS: &'static [MigrationEdge] = &[MigrationEdge {
        from_epoch: 1,
        to_epoch: 2,
        migrator: failing_migrator,
    }];
}

/// Bug class: a failed migration that still advances the version marker
/// leaves half-migrated state masquerading as the new epoch (the
/// stale-state family of Quasar issue #239, open 2026-07). Hopper
/// guard: `hopper-runtime/src/migrate.rs::apply_pending_migrations`
/// step ordering — the header's `schema_epoch` is bumped only after the
/// edge's migrator returned `Ok`. This test pins: when the migrator
/// errors, the error propagates verbatim and the header epoch still
/// reads the old value, so the account can never be mistaken for a
/// completed V2.
#[test]
fn failed_migration_edge_never_advances_the_schema_epoch() {
    let mut data = [0u8; HopperHeader::SIZE + 8];
    data[12..16].copy_from_slice(&1u32.to_le_bytes());
    let (_backing, account) = make_account([31; 32], DEFAULT_OWNER, false, true, 1, &data);

    assert_eq!(
        apply_pending_migrations::<PoisonV2>(&account, &Address::new_from_array(DEFAULT_OWNER), 1),
        Err(ProgramError::Custom(0xDEAD))
    );

    let data = account.try_borrow().unwrap();
    // The epoch was NOT stamped: under Solana's transaction-abort
    // semantics the propagated error rolls the body back, and until then
    // no typed V2 load can succeed against the old epoch.
    assert_eq!(&data[12..16], &1u32.to_le_bytes());
}

// =====================================================================
// Anchor coarse-borrow bug classes (BLD-ABUG)
//
// Anchor's borrow tracker is account-index granular: a 256-bit MUT_MASK
// (`[u64; 4]`, one bit per account entry) records *which* accounts an
// instruction may mutate, never *which bytes*. Two recurring open bug
// classes in the anchor-next tracker follow directly from that
// coarseness. Hopper's byte-range segment ledger and `strict_writes`
// write-policy make classes (i) and (ii) structurally harder; the tests
// below pin the exact host-reachable guard for each and state the level
// it is pinned at. (Realloc edge class (iii) lives behind hopper-core
// APIs and is pinned in the companion core suite.)
// =====================================================================

/// Anchor bug class: read-only-account-gets-mutated — a handler or CPI
/// writes an account the context declared read-only, and it goes
/// undetected because the coarse 256-bit MUT_MASK (`[u64; 4]`) tracks
/// account *indices*, not byte ranges: an account read-only *for this
/// instruction* is frequently transaction-writable for another, so its
/// mask bit says nothing about the intended write-set (their coarse
/// 256-bit MUT_MASK cannot distinguish; anchor-next open issues).
/// Hopper guard: `hopper-runtime/src/write_policy.rs::WritePolicy::check_write`,
/// installed on the `Context` by `set_write_policy` and consulted at
/// every write acquire in `context.rs::Context::check_write_policy`
/// (`load_mut`, the `segment_mut*` family, and the raw escape hatches).
/// This test pins: the byte-range write-set rejects (a) a write to an
/// account absent from the set, (b) a write to an undeclared byte range
/// of an account that *is* partially writable, and (c) every write under
/// an empty policy (a machine-checked read-only instruction) — the first
/// two being distinctions the account-index mask cannot represent.
/// Pinned at host level (the const policy decision; the `Context` wiring
/// that calls it is exercised in `context.rs::write_policy_tests`).
#[test]
fn strict_writes_rejects_writes_outside_the_declared_byte_range_set() {
    // A representative declared write-set: the vault (instruction account
    // 1) may be written only in its balance field `[16, 24)`; account 2
    // is wholly writable. Account 0 — an authority/config the instruction
    // only reads — appears nowhere in the set.
    static POLICY: WritePolicy =
        WritePolicy::new(&[WriteRange::new(1, 16, 8), WriteRange::whole_account(2)]);

    // (a) Read-only-by-omission: account 0 carries no WriteRange, so every
    // write to it is refused, the indexed error naming account 0. This is
    // exactly the case Anchor's mask permits when index 0 happens to be
    // transaction-writable for an unrelated reason.
    assert_eq!(POLICY.check_write(0, 0, 8), Err(write_policy_violation(0)));
    assert_eq!(POLICY.check_write(0, 0, 1), Err(write_policy_violation(0)));

    // (b) Right account, undeclared range: the vault is partially
    // writable, but a write to any byte outside `[16, 24)` is refused. The
    // account-index mask sees only "account 1 is writable" and cannot
    // express this sub-account boundary.
    assert!(POLICY.check_write(1, 16, 8).is_ok());
    assert_eq!(POLICY.check_write(1, 0, 8), Err(write_policy_violation(1)));
    assert_eq!(POLICY.check_write(1, 24, 8), Err(write_policy_violation(1)));
    // A write straddling the declared range and adjacent bytes is refused
    // as a whole — there is no partial acceptance of the in-range prefix.
    assert!(POLICY.check_write(1, 20, 8).is_err());

    // The wholly-writable account still accepts any range: byte-range
    // tracking is a refinement of the write-set, not a blanket denial.
    assert!(POLICY.check_write(2, 0, 8).is_ok());
    assert!(POLICY.check_write(2, 4096, 1024).is_ok());

    // (c) An empty policy is the fully read-only contract: no byte on any
    // account may be written. Anchor has no account-index encoding for
    // "writable at the transaction level but read-only in this handler".
    static READ_ONLY: WritePolicy = WritePolicy::new(&[]);
    assert_eq!(
        READ_ONLY.check_write(0, 0, 1),
        Err(write_policy_violation(0))
    );
    assert!(READ_ONLY.check_write(1, 16, 8).is_err());
}

/// Anchor bug class: stale-account-view-after-CPI — a borrowed data view
/// is used after a CPI that could have reallocated or mutated the same
/// account, because the account-granular borrow model neither ties the
/// borrow to a byte range nor re-checks it across the CPI boundary (their
/// coarse 256-bit MUT_MASK cannot distinguish; anchor-next open issues).
/// Hopper guard:
/// `hopper-runtime/src/segment_borrow.rs::SegmentBorrowRegistry::register`
/// (byte-range conflict scan, full-address identity) and the
/// account-level borrow byte behind
/// `hopper-native::AccountView::try_borrow`/`try_borrow_mut`. This test
/// pins, at host level: while a write lease over a byte range is live,
/// any conflicting acquire over those bytes — the exact borrow a
/// CPI-passing helper or a later reader would take — is rejected with
/// `AccountBorrowFailed`; the conflict is byte-range precise (a disjoint
/// range is still allowed); the block lifts only when the lease is
/// released, so a view can never silently outlive the borrow that
/// authorized it; and the whole-account borrow byte enforces the same at
/// account granularity. The on-chain effect of a CPI *reallocating* the
/// account (its data pointer moving under a held view) is not reachable
/// host-side and is noted as an on-chain follow-up.
#[test]
fn a_live_segment_borrow_blocks_the_access_a_stale_view_would_need() {
    let vault = Address::new_from_array([51; 32]);

    // A live write lease over `[0, 32)` models a mutable view held across
    // a mutating operation. A read acquire over the overlapping `[16, 24)`
    // — a stale view being consumed while the write is in flight — is
    // rejected. Anchor's account-index bit does not track the range, so it
    // cannot observe this overlap at all.
    let mut registry = SegmentBorrowRegistry::new();
    let lease = registry.register_leased_write(&vault, 0, 32).unwrap();
    assert_eq!(
        registry.register_read(&vault, 16, 8).unwrap_err(),
        ProgramError::AccountBorrowFailed
    );

    // Byte-range precision: a disjoint range is not a view of the written
    // bytes, so it is admitted even while the write is live — Hopper does
    // not over-block, the way an account-granular exclusive lock would.
    registry.register_read(&vault, 32, 8).unwrap();

    // The block is scoped to the lease: once it releases, the range is
    // free for a fresh writer. Anchor's mask bit, by contrast, stays set
    // for the whole instruction.
    assert!(registry.release(&lease));
    registry.register_write(&vault, 0, 32).unwrap();

    // Account-level analog: a live mutable whole-account borrow blocks the
    // re-borrow a CPI (or a second view) would need to read the same
    // account's data. Solana already provides this coarse guard; Hopper
    // keeps it *and* adds the byte-range ledger above.
    let (_backing, account) = make_account([52; 32], DEFAULT_OWNER, false, true, 1, b"payload!");
    let live_view = account.try_borrow_mut().unwrap();
    assert_eq!(
        account.try_borrow().unwrap_err(),
        ProgramError::AccountBorrowFailed
    );
    drop(live_view);
    assert!(account.try_borrow().is_ok());
}

// =====================================================================
// Anchor v2 Slab bug classes (anchor-next, fixed May–June 2026)
//
// anchor-next's lang-v2 now ships `Slab<H, T>`, its first zero-copy
// collection (`lang-v2/src/accounts/slab.rs`). Two bug classes were
// found and fixed in it: #4616 (read aliases during mutable borrows,
// fixed 2026-06-02) is pinned here; #4603 (stale bytes past a shrunken
// tail, fixed 2026-05-27) is pinned in the hopper-core companion suite
// where the realloc lifecycle APIs live.
// =====================================================================

/// Anchor v2 bug class: Slab read alias during a mutable borrow —
/// anchor-next #4616 ("v2: Prevent Slab read aliases during mutable
/// borrows", fixed 2026-06-02). Before the fix, `Slab::load` /
/// `load_mut` cached a typed header pointer into account data without
/// marking pinocchio's per-account `borrow_state`, so a *copied*
/// `AccountView` could take a safe `try_borrow_mut()` over the same
/// bytes and alias the Slab's `&H` / `&mut H`; the fix retrofits manual
/// borrow-state marking into each Slab constructor. Hopper guards (both
/// centralized, not per-wrapper retrofits):
/// `hopper-runtime/src/segment_borrow.rs::SegmentBorrowRegistry::register`
/// — every typed segment acquire (`account.rs::segment_ref` /
/// `segment_mut`) registers a byte-range lease and conflicting acquires
/// are refused — and the `borrow_state` byte in
/// `hopper-native/src/raw_account.rs`, which every view copy shares
/// because it lives in the account header itself. At the typed-guard
/// level the alias is additionally unrepresentable in safe code: both
/// `segment_ref` and `segment_mut` take `&mut SegmentBorrowRegistry`,
/// so rustc refuses two live guards from one registry. This test pins
/// the dynamic guards: a shared byte-range acquire overlapping a live
/// mutable segment lease fails through the ledger, and while a real
/// `segment_mut` guard is live every whole-account borrow a copied view
/// could attempt is refused, lifting only when the guard drops.
#[test]
fn anchor_4616_shared_read_alias_during_live_mutable_borrow_is_refused() {
    // Ledger level: a live mutable lease over the Slab-shaped header
    // region [8, 40) — the `&mut H` a Slab hands out — refuses any
    // overlapping shared acquire, in the exact-overlap and
    // partial-overlap shapes, while a disjoint tail read stays legal.
    let slab = Address::new_from_array([61; 32]);
    let mut registry = SegmentBorrowRegistry::new();
    let lease = registry.register_leased_write(&slab, 8, 32).unwrap();
    assert_eq!(
        registry.register_read(&slab, 8, 32).unwrap_err(),
        ProgramError::AccountBorrowFailed
    );
    assert_eq!(
        registry.register_read(&slab, 32, 16).unwrap_err(),
        ProgramError::AccountBorrowFailed
    );
    registry.register_read(&slab, 40, 8).unwrap();
    // The refusal is tied to the lease lifetime, exactly like a borrow.
    assert!(registry.release(&lease));
    registry.register_read(&slab, 8, 32).unwrap();

    // Accessor level: while a real typed mutable segment guard is live,
    // the whole-account borrows a copied AccountView would take (the
    // #4616 attack surface) are refused through the shared borrow byte.
    let (_backing, account) = make_account([62; 32], DEFAULT_OWNER, false, true, 1, &[0u8; 16]);
    let mut registry = SegmentBorrowRegistry::new();
    {
        let mut header = account.segment_mut::<[u8; 8]>(&mut registry, 0, 8).unwrap();
        *header = [0xA5; 8];
        assert_eq!(
            account.try_borrow().unwrap_err(),
            ProgramError::AccountBorrowFailed
        );
        assert_eq!(
            account.try_borrow_mut().unwrap_err(),
            ProgramError::AccountBorrowFailed
        );
    }
    // Dropping the guard is a full release: reads work and observe the
    // committed write.
    let data = account.try_borrow().unwrap();
    assert_eq!(&data[..8], &[0xA5; 8]);
}
