//! Lazy migration at bind (`migrate(from = Old, with = transform)`),
//! end to end.
//!
//! The macro half of the typed cross-version migration story: a context
//! field declared `#[account(mut, migrate(from = CounterV1, with = f))]`
//! turns EVERY instruction that binds the context into a migration crank.
//! `bind()` probes the slot for a fully-valid OLD-layout header and, only
//! then, runs `hopper::migration::migrate_layout::<Old, New, _>` — the
//! committed runtime half (typed transform, in-place, header re-stamped
//! LAST with flags preserved) — BEFORE any validator runs. `validate()`
//! (the read-only standalone surface) accepts EITHER version without
//! writing a byte.
//!
//! These tests drive the real codegen through the hopper-svm host harness
//! (same conventions as `composite_contexts_integration.rs`), proving:
//!
//! 1. a V1-seeded account binds, migrates in place, and the handler reads
//!    the transformed V2 values; the post-state header is fully V2
//!    (disc / version / layout_id / schema-epoch) with account flags
//!    preserved;
//! 2. a V2-seeded account binds untouched (byte-identical before/after);
//! 3. a foreign-layout account fails bind with the normal V2 validation
//!    error, unchanged (and is not written);
//! 4. standalone `validate()` accepts BOTH V1- and V2-seeded sets without
//!    modifying either, and still rejects foreign layouts;
//! 5. would-bind parity for undersized V1 accounts: an allocation the V2
//!    shape does not fit is rejected by BOTH surfaces (resizing is
//!    `realloc`'s job, done before migrating).
//!
//! The compile-error surface (mut required, lifecycle/Option/composite
//! combos, same-type source, malformed spellings) lives in the macro
//! crate's expansion tests (`hopper-derive`, `context::migrate_attr_tests`),
//! not here.

#![cfg(feature = "proc-macros")]

use hopper::layout::{write_header, HEADER_LEN};
use hopper::prelude::*;
use hopper_svm::{AccountFixture, HopperSvm};

// ── Program under test ─────────────────────────────────────────────

/// Version 1: a narrow counter plus a legacy blob that V2 drops.
#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 88, version = 1)]
pub struct CounterV1 {
    pub hits: WireU32,
    pub legacy: WireU32,
}

/// Version 2: the counter widens to u64 and a marker field appears.
/// Same disc, higher version, wider body (12 > 8) — the fixture
/// allocation is therefore sized for V2.
#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 88, version = 2)]
pub struct CounterV2 {
    pub hits: WireU64,
    pub boosted: WireU32,
}

/// A different account kind entirely (foreign disc): must keep failing
/// the migrate field's validation exactly like before the feature.
#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 89, version = 1)]
pub struct Intruder {
    pub junk: WireU64,
}

/// The typed transform: widen the counter (+1000 so a spurious re-run
/// would be visible in the value) and set the marker. `legacy` is
/// dropped — the runtime zero-fills the V2 span before the transform,
/// so nothing of it can leak through.
fn v1_to_v2(old: &CounterV1, new: &mut CounterV2) -> Result<(), ProgramError> {
    new.hits = WireU64::new(old.hits.get() as u64 + 1000);
    new.boosted = WireU32::new(1);
    Ok(())
}

/// Every instruction binding this context is a lazy migration crank.
#[derive(hopper::Accounts)]
pub struct Touch<'info> {
    pub authority: Signer<'info>,

    #[account(mut, migrate(from = CounterV1, with = v1_to_v2))]
    pub counter: Account<'info, CounterV2>,
}

// ── Handlers ───────────────────────────────────────────────────────

/// Binds (migrating if needed) and verifies the handler observes the
/// canonical V2 values through the typed accessor.
fn touch_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    let bound = Touch::bind(&mut ctx)?;
    let counter = bound.counter_load()?;
    if counter.hits.get() != 1007 {
        return Err(ProgramError::Custom(0xBAD1));
    }
    if counter.boosted.get() != 1 {
        return Err(ProgramError::Custom(0xBAD2));
    }
    Ok(())
}

/// The read-only standalone surface: "would bind accept this set?"
/// with zero writes.
fn validate_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let ctx = Context::new(program_id, accounts, instruction_data);
    Touch::validate(&ctx)
}

// ── Fixtures ───────────────────────────────────────────────────────

const PROGRAM_ID: Address = Address::new_from_array([9u8; 32]);

/// Account-state flags seeded on every counter fixture: the migration
/// must carry them across the header re-stamp (flags are account state,
/// not layout identity).
const SEEDED_FLAGS: u16 = 0x0304;

fn signer_fixture(addr_byte: u8) -> AccountFixture {
    AccountFixture::new(
        Address::new_from_array([addr_byte; 32]),
        Address::new_from_array([0u8; 32]),
        1_000_000,
        0,
    )
    .signer()
}

/// A non-signer account at the same address, to flip the signer bit.
fn non_signer_fixture(addr_byte: u8) -> AccountFixture {
    AccountFixture::new(
        Address::new_from_array([addr_byte; 32]),
        Address::new_from_array([0u8; 32]),
        1_000_000,
        0,
    )
}

/// A V1-headed counter (hits = 7, legacy = 0xDEADBEEF) in an allocation
/// of `data_len` bytes. `CounterV2::LEN` by default — the V2 shape must
/// FIT the allocation because in-place migration never resizes.
fn v1_fixture_with_len(addr_byte: u8, data_len: usize) -> AccountFixture {
    let mut data = vec![0u8; data_len];
    write_header(
        &mut data,
        CounterV1::DISC,
        CounterV1::VERSION,
        &CounterV1::LAYOUT_ID,
    )
    .unwrap();
    data[2..4].copy_from_slice(&SEEDED_FLAGS.to_le_bytes());
    data[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&7u32.to_le_bytes());
    data[HEADER_LEN + 4..HEADER_LEN + 8].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
    AccountFixture::with_data(
        Address::new_from_array([addr_byte; 32]),
        PROGRAM_ID,
        1_000_000,
        data,
    )
    .writable()
}

fn v1_fixture(addr_byte: u8) -> AccountFixture {
    v1_fixture_with_len(addr_byte, CounterV2::LEN)
}

/// An already-migrated counter: the exact byte image `v1_fixture` would
/// have after the crank (hits widened +1000, marker set, V2 header,
/// seeded flags preserved).
fn v2_fixture(addr_byte: u8) -> AccountFixture {
    let mut data = vec![0u8; CounterV2::LEN];
    write_header(
        &mut data,
        CounterV2::DISC,
        CounterV2::VERSION,
        &CounterV2::LAYOUT_ID,
    )
    .unwrap();
    data[2..4].copy_from_slice(&SEEDED_FLAGS.to_le_bytes());
    data[HEADER_LEN..HEADER_LEN + 8].copy_from_slice(&1007u64.to_le_bytes());
    data[HEADER_LEN + 8..HEADER_LEN + 12].copy_from_slice(&1u32.to_le_bytes());
    AccountFixture::with_data(
        Address::new_from_array([addr_byte; 32]),
        PROGRAM_ID,
        1_000_000,
        data,
    )
    .writable()
}

/// A foreign account kind in the counter slot (valid Intruder header).
fn foreign_fixture(addr_byte: u8) -> AccountFixture {
    let mut data = vec![0u8; CounterV2::LEN];
    write_header(
        &mut data,
        Intruder::DISC,
        Intruder::VERSION,
        &Intruder::LAYOUT_ID,
    )
    .unwrap();
    AccountFixture::with_data(
        Address::new_from_array([addr_byte; 32]),
        PROGRAM_ID,
        1_000_000,
        data,
    )
    .writable()
}

fn accounts_with_counter(counter: AccountFixture) -> [AccountFixture; 2] {
    [signer_fixture(0x51), counter]
}

// ── Arm 1: V1 binds, migrates in place, handler sees V2 ────────────

#[test]
fn v1_account_binds_migrates_and_handler_reads_the_transformed_values() {
    let accounts = accounts_with_counter(v1_fixture(0x52));
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, touch_handler);
    assert!(
        result.program_result.is_ok(),
        "a V1-seeded account must migrate at bind and satisfy the handler: {:?}",
        result.program_result
    );

    let data = &result.resulting_accounts[1].data;
    // Header: the full NEW identity, stamped by the runtime after the
    // transform succeeded.
    assert_eq!(data[0], CounterV2::DISC, "disc unchanged (same kind)");
    assert_eq!(data[1], CounterV2::VERSION, "version advanced to 2");
    assert_eq!(
        &data[4..12],
        &CounterV2::LAYOUT_ID,
        "layout fingerprint re-stamped to V2"
    );
    assert_eq!(
        &data[12..16],
        &1u32.to_le_bytes(),
        "schema epoch stamped from the V2 layout (default epoch 1)"
    );
    // Flags are account state, not layout identity: preserved verbatim.
    assert_eq!(
        u16::from_le_bytes([data[2], data[3]]),
        SEEDED_FLAGS,
        "header flags must survive the re-stamp"
    );
    // Body: widened counter, set marker, and nothing left of `legacy`
    // (the zeroed V2 span is the deterministic default for the byte the
    // transform did not set).
    assert_eq!(
        &data[HEADER_LEN..HEADER_LEN + 8],
        &1007u64.to_le_bytes(),
        "hits widened through the typed transform (7 + 1000)"
    );
    assert_eq!(
        &data[HEADER_LEN + 8..HEADER_LEN + 12],
        &1u32.to_le_bytes(),
        "marker set by the transform"
    );
}

// ── Arm 2: V2 binds untouched ──────────────────────────────────────

#[test]
fn v2_account_binds_without_a_single_byte_written() {
    let accounts = accounts_with_counter(v2_fixture(0x53));
    let before = accounts[1].data.clone();
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, touch_handler);
    assert!(
        result.program_result.is_ok(),
        "an already-current account must bind normally: {:?}",
        result.program_result
    );
    assert_eq!(
        result.resulting_accounts[1].data, before,
        "a V2 account must skip the migrate pre-step (byte-identical data)"
    );
}

// ── Arm 3: foreign layouts keep the normal error ───────────────────

#[test]
fn foreign_layout_fails_bind_with_the_normal_error_and_is_not_written() {
    let accounts = accounts_with_counter(foreign_fixture(0x54));
    let before = accounts[1].data.clone();
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, touch_handler);
    assert_eq!(
        result.program_result,
        Err(ProgramError::InvalidAccountData),
        "neither-version accounts must surface the normal V2 validation error, unchanged"
    );
    assert_eq!(
        result.resulting_accounts[1].data, before,
        "a refused account must not be written"
    );
}

/// The pre-migration checks are unchanged too: an unsigned authority
/// still fails bind (after the crank — the write rolls back with the
/// failed instruction under transaction-abort semantics).
#[test]
fn unsigned_authority_still_fails_bind() {
    let accounts = [non_signer_fixture(0x51), v2_fixture(0x55)];
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, touch_handler);
    assert!(
        result.program_result.is_err(),
        "signer validation is unchanged by the migrate attribute"
    );
}

// ── Arm 4: standalone validate() accepts EITHER version, read-only ──

#[test]
fn standalone_validate_accepts_both_versions_without_writing() {
    for (label, counter) in [("V1", v1_fixture(0x56)), ("V2", v2_fixture(0x57))] {
        let accounts = accounts_with_counter(counter);
        let before = accounts[1].data.clone();
        let result =
            HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, validate_handler);
        assert!(
            result.program_result.is_ok(),
            "validate() must accept a {label}-seeded set: {:?}",
            result.program_result
        );
        assert_eq!(
            result.resulting_accounts[1].data, before,
            "validate() is read-only and must not modify the {label} account"
        );
    }
}

#[test]
fn standalone_validate_still_rejects_foreign_layouts() {
    let accounts = accounts_with_counter(foreign_fixture(0x58));
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, validate_handler);
    assert_eq!(
        result.program_result,
        Err(ProgramError::InvalidAccountData),
        "either-version acceptance must not widen beyond the two declared layouts"
    );
}

// ── Arm 5: would-bind parity for undersized V1 allocations ──────────

/// In-place migration never resizes: a V1 account whose allocation the
/// V2 shape does not fit is rejected by BOTH surfaces (bind refuses
/// before writing; validate mirrors the same acceptance set), so
/// `validate()` never green-lights a set `bind()` would refuse.
#[test]
fn undersized_v1_allocation_is_rejected_by_both_surfaces() {
    // Sized for V1 exactly (24 bytes); V2 needs 28.
    let bind_accounts = accounts_with_counter(v1_fixture_with_len(0x59, CounterV1::LEN));
    let before = bind_accounts[1].data.clone();
    let bind_result =
        HopperSvm::new().process_instruction(PROGRAM_ID, &[], &bind_accounts, touch_handler);
    assert!(
        bind_result.program_result.is_err(),
        "bind must refuse an allocation the V2 shape does not fit (realloc first)"
    );
    assert_eq!(
        bind_result.resulting_accounts[1].data, before,
        "the refusal must land before any byte is written"
    );

    let validate_accounts = accounts_with_counter(v1_fixture_with_len(0x5A, CounterV1::LEN));
    let validate_result =
        HopperSvm::new().process_instruction(PROGRAM_ID, &[], &validate_accounts, validate_handler);
    assert!(
        validate_result.program_result.is_err(),
        "validate() must mirror bind's acceptance set for undersized V1 accounts"
    );
}
