//! Epoch-chain healing at bind (`#[account(epoch_migrate)]`), end to end.
//!
//! The final wire for the schema-epoch machinery: a layout declares its
//! target epoch (`#[hopper::state(schema_epoch = 3)]`) and its in-place
//! edges (`#[hopper::migrate]` + `layout_migrations!`), and a context
//! field declared `#[account(mut, epoch_migrate)]` turns every
//! instruction that binds the context into an epoch-healing crank:
//! `bind()` probes the header with the SHARED acceptance predicate
//! (`validate_header_for_epoch_migration` — exact identity, lagging
//! epoch only) and runs `apply_pending_migrations` before any validator,
//! so validators and the handler see the healed account. `validate()`
//! (the read-only surface) accepts exactly the same set.
//!
//! Proves:
//! 1. an epoch-1 account binds, walks BOTH edges (1→2 adds 100, 2→3
//!    XORs a marker), and the handler reads the healed body; the header
//!    epoch reads 3;
//! 2. an already-current (epoch-3) account binds byte-identical;
//! 3. a FUTURE-epoch (4) account fails bind with the layout's own
//!    validation error and is not written — never "migrated" down;
//! 4. standalone `validate()` accepts the stale-epoch set read-only and
//!    still rejects the future-epoch set (would-bind parity).

#![cfg(feature = "proc-macros")]

use hopper::__runtime::ProgramError;
use hopper::layout::{write_header_with_epoch, HEADER_LEN};
use hopper::prelude::*;
use hopper_svm::{AccountFixture, HopperSvm};

// ── Program under test ─────────────────────────────────────────────

/// One layout version whose BODY meaning evolved across three schema
/// epochs. The shape never changes (epoch edges cannot resize); the
/// interpretation does.
#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 91, version = 1, schema_epoch = 3)]
pub struct EpochCounter {
    pub hits: WireU64,
}

/// Epoch 1 → 2: re-base the counter (+100).
#[hopper::migrate(from = 1, to = 2)]
pub fn counter_epoch_1_to_2(body: &mut [u8]) -> Result<(), ProgramError> {
    if body.len() < 8 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&body[0..8]);
    let new = u64::from_le_bytes(bytes).wrapping_add(100);
    body[0..8].copy_from_slice(&new.to_le_bytes());
    Ok(())
}

/// Epoch 2 → 3: flip the marker bit into the counter's top byte.
#[hopper::migrate(from = 2, to = 3)]
pub fn counter_epoch_2_to_3(body: &mut [u8]) -> Result<(), ProgramError> {
    if body.len() < 8 {
        return Err(ProgramError::AccountDataTooSmall);
    }
    body[7] ^= 0x80;
    Ok(())
}

hopper::layout_migrations! {
    EpochCounter = [COUNTER_EPOCH_1_TO_2_EDGE, COUNTER_EPOCH_2_TO_3_EDGE],
}

/// Every instruction binding this context heals the counter's epoch.
#[derive(hopper::Accounts)]
pub struct Heal<'info> {
    pub authority: Signer<'info>,

    #[account(mut, epoch_migrate)]
    pub counter: Account<'info, EpochCounter>,
}

// ── Handlers ───────────────────────────────────────────────────────

/// The canonical healed value: seeded 7, +100 (edge 1→2), top byte
/// XOR 0x80 (edge 2→3).
const HEALED: u64 = (7 + 100) ^ (0x80u64 << 56);

fn heal_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    let bound = Heal::bind(&mut ctx)?;
    let counter = bound.counter_load()?;
    if counter.hits.get() != HEALED {
        return Err(ProgramError::Custom(0xBAD1));
    }
    Ok(())
}

fn validate_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let ctx = Context::new(program_id, accounts, instruction_data);
    Heal::validate(&ctx)
}

// ── Fixtures ───────────────────────────────────────────────────────

const PROGRAM_ID: Address = Address::new_from_array([9u8; 32]);

fn signer_fixture(addr_byte: u8) -> AccountFixture {
    AccountFixture::new(
        Address::new_from_array([addr_byte; 32]),
        Address::new_from_array([0u8; 32]),
        1_000_000,
        0,
    )
    .signer()
}

/// A counter seeded with `hits = 7` at the given stored schema epoch.
fn counter_at_epoch(addr_byte: u8, epoch: u32) -> AccountFixture {
    let mut data = vec![0u8; EpochCounter::LEN];
    write_header_with_epoch(
        &mut data,
        EpochCounter::DISC,
        EpochCounter::VERSION,
        &EpochCounter::LAYOUT_ID,
        epoch,
    )
    .unwrap();
    data[HEADER_LEN..HEADER_LEN + 8].copy_from_slice(&7u64.to_le_bytes());
    AccountFixture::with_data(
        Address::new_from_array([addr_byte; 32]),
        PROGRAM_ID,
        1_000_000,
        data,
    )
    .writable()
}

/// The exact byte image of an epoch-1 counter AFTER the crank.
fn healed_fixture(addr_byte: u8) -> AccountFixture {
    let mut data = vec![0u8; EpochCounter::LEN];
    write_header_with_epoch(
        &mut data,
        EpochCounter::DISC,
        EpochCounter::VERSION,
        &EpochCounter::LAYOUT_ID,
        3,
    )
    .unwrap();
    data[HEADER_LEN..HEADER_LEN + 8].copy_from_slice(&HEALED.to_le_bytes());
    AccountFixture::with_data(
        Address::new_from_array([addr_byte; 32]),
        PROGRAM_ID,
        1_000_000,
        data,
    )
    .writable()
}

// ── Arm 1: stale epoch heals at bind ───────────────────────────────

#[test]
fn stale_epoch_account_heals_through_both_edges_at_bind() {
    let accounts = [signer_fixture(0x61), counter_at_epoch(0x62, 1)];
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, heal_handler);
    assert!(
        result.program_result.is_ok(),
        "an epoch-1 account must heal at bind and satisfy the handler: {:?}",
        result.program_result
    );
    let data = &result.resulting_accounts[1].data;
    assert_eq!(
        &data[12..16],
        &3u32.to_le_bytes(),
        "header epoch stamped to the layout's target (3)"
    );
    assert_eq!(
        &data[HEADER_LEN..HEADER_LEN + 8],
        &HEALED.to_le_bytes(),
        "both edges ran, in order"
    );
}

// ── Arm 2: already-current binds untouched ─────────────────────────

#[test]
fn current_epoch_account_binds_without_a_single_byte_written() {
    let accounts = [signer_fixture(0x61), healed_fixture(0x63)];
    let before = accounts[1].data.clone();
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, heal_handler);
    assert!(
        result.program_result.is_ok(),
        "a current-epoch account must bind normally: {:?}",
        result.program_result
    );
    assert_eq!(
        result.resulting_accounts[1].data, before,
        "a current-epoch account must skip the crank (byte-identical)"
    );
}

// ── Arm 3: future epochs are refused, never migrated down ──────────

#[test]
fn future_epoch_account_fails_bind_and_is_not_written() {
    let accounts = [signer_fixture(0x61), counter_at_epoch(0x64, 4)];
    let before = accounts[1].data.clone();
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, heal_handler);
    assert!(
        result.program_result.is_err(),
        "a from-the-future epoch must fail the layout's own validation"
    );
    assert_eq!(
        result.resulting_accounts[1].data, before,
        "a refused account must not be written"
    );
}

// ── Arm 4: validate() mirrors bind's acceptance set, read-only ─────

#[test]
fn standalone_validate_accepts_stale_epochs_read_only_and_rejects_future() {
    // Stale (healable): accepted without writing a byte.
    let accounts = [signer_fixture(0x61), counter_at_epoch(0x65, 1)];
    let before = accounts[1].data.clone();
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, validate_handler);
    assert!(
        result.program_result.is_ok(),
        "validate() must accept a stale-epoch set bind can heal: {:?}",
        result.program_result
    );
    assert_eq!(
        result.resulting_accounts[1].data, before,
        "validate() is read-only"
    );

    // Future: rejected by both surfaces.
    let accounts = [signer_fixture(0x61), counter_at_epoch(0x66, 4)];
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, validate_handler);
    assert!(
        result.program_result.is_err(),
        "validate() must reject a future-epoch set exactly as bind does"
    );
}
