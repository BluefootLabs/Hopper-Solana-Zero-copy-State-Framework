//! Self-CPI events (`event_cpi`), end to end: one attribute + one call.
//!
//! Anchor spells this `#[event_cpi]` on the accounts struct plus
//! `emit_cpi!(MyEvent { .. })`. Hopper spells it
//! `#[hopper::context(event_cpi)]` plus `ctx.emit_event_cpi(&event)`:
//!
//! - the context macro auto-appends the two Anchor-parity trailing
//!   account slots (event-authority PDA + the program's own account) —
//!   `ACCOUNT_COUNT` grows by 2 and both slots are validated at bind;
//! - the bound context gains `emit_event_cpi`, which encodes the wire
//!   format `[0xE0, 0x1E, tag, payload]` and self-invokes with the
//!   event-authority signer seeds (bump captured at bind);
//! - the `#[hopper::program]` dispatcher grows a const-guarded sink on
//!   the reserved `[0xE0, 0x1E]` marker that authenticates each event
//!   (marker + signer; the PDA address pin is on-chain-only because
//!   hosts have no sha256 syscall).
//!
//! These tests drive a real `#[hopper::program]` dispatcher through the
//! hopper-svm host harness. Off-chain there is no ledger to record
//! inner instructions, so the runtime's host CPI emulation validates
//! the self-CPI's address/privilege shape and supplied PDA authority,
//! then captures the would-be inner instruction — program id, authority
//! address, and the EXACT instruction-data bytes — via
//! `cpi_event::take_host_captured_event_cpis()`. The captured bytes are
//! asserted byte-for-byte against the public `encode_event_cpi` encoder
//! and round-tripped through the public `decode_event_cpi` decoder, the
//! same pair indexers use.
//!
//! Wire-size receipt (measured in `emit_one_liner_produces_the_exact_wire_bytes`):
//! Hopper spends 3 bytes of instruction-data overhead per event (2-byte
//! marker + 1-byte tag). Anchor's `emit_cpi!` spends 16 (8-byte
//! `EVENT_IX_TAG_LE` + the event's own 8-byte discriminator) — 13 fewer
//! bytes per event, or 5 fewer counting only the event-identification
//! layer (3 vs one 8-byte hash discriminator).

#![cfg(feature = "proc-macros")]

use hopper::hopper_runtime::cpi_event::{
    decode_event_cpi, encode_event_cpi, take_host_captured_event_cpis, CPI_EVENT_MARKER,
};
use hopper::layout::write_header;
use hopper::prelude::*;
use hopper_svm::{AccountFixture, HopperSvm};

// ── State + event under test ───────────────────────────────────────

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 9, version = 1)]
pub struct Ledger {
    pub total: WireU64,
}

/// The emitted event: 12 payload bytes (8 + 4), alignment-1 Pod.
#[hopper::event(tag = 0x42)]
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Deposited {
    pub amount: WireU64,
    pub memo: [u8; 4],
}

// ── Contexts: opted-in vs control ──────────────────────────────────

/// Opted in: the macro appends `event_authority` + `event_program` as
/// trailing slots and exposes `emit_event_cpi` on the bound context.
#[hopper::context(event_cpi)]
pub struct Deposit {
    #[account(mut)]
    pub vault: Ledger,
}

/// Control: same shape, no opt-in. Appends nothing, exposes nothing.
#[hopper::context]
pub struct PlainDeposit {
    #[account(mut)]
    pub vault: Ledger,
}

// ── Programs: a real dispatcher over each context ──────────────────

#[hopper::program(entrypoint = false)]
mod event_prog {
    use super::*;

    /// The one-liner under test: state change, then `emit_event_cpi`.
    #[instruction(0)]
    fn do_deposit(ctx: Context<Deposit>, amount: u64) -> ProgramResult {
        {
            let mut vault = ctx.vault_load_mut()?;
            let new_total = vault.total.get().wrapping_add(amount);
            vault.total = WireU64::new(new_total);
        }
        ctx.emit_event_cpi(&Deposited {
            amount: WireU64::new(amount),
            memo: *b"e2e!",
        })?;
        Ok(())
    }
}

#[hopper::program(entrypoint = false)]
mod control_prog {
    use super::*;

    /// Same handler shape WITHOUT the option: no emit surface exists,
    /// and the dispatcher's sink guard const-folds to `false`.
    #[instruction(0)]
    fn do_deposit(ctx: Context<PlainDeposit>, amount: u64) -> ProgramResult {
        let mut vault = ctx.vault_load_mut()?;
        let new_total = vault.total.get().wrapping_add(amount);
        vault.total = WireU64::new(new_total);
        Ok(())
    }
}

// ── Fixtures + drivers ─────────────────────────────────────────────

const PROGRAM_ID: [u8; 32] = [9u8; 32];
const AUTHORITY_ADDR: [u8; 32] = [7u8; 32];
const VAULT_ADDR: [u8; 32] = [1u8; 32];

fn vault_fixture(program_id: Address) -> AccountFixture {
    let mut data = vec![0u8; Ledger::LEN];
    write_header(&mut data, Ledger::DISC, Ledger::VERSION, &Ledger::LAYOUT_ID).unwrap();
    AccountFixture::with_data(
        Address::new_from_array(VAULT_ADDR),
        program_id,
        1_000_000,
        data,
    )
    .writable()
}

/// The event-authority fixture. Off-chain no sha256 syscall exists, so
/// the PDA address cannot be derived host-side (bind accepts the slot
/// and reports the placeholder bump; see
/// `cpi_event::HOST_EVENT_AUTHORITY_BUMP`). A real event authority is not a
/// transaction signer: `invoke_signed` grants that privilege to the inner
/// instruction from the generated PDA seeds.
fn authority_fixture(signer: bool) -> AccountFixture {
    let fixture = AccountFixture::new(
        Address::new_from_array(AUTHORITY_ADDR),
        Address::new_from_array([0u8; 32]),
        0,
        0,
    );
    if signer {
        fixture.signer()
    } else {
        fixture
    }
}

/// The program's own account: `bind()` pins its address to
/// `ctx.program_id()`.
fn program_fixture(program_id: Address) -> AccountFixture {
    AccountFixture::new(program_id, Address::new_from_array([0u8; 32]), 0, 0).executable()
}

fn drive_event<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    event_prog::process_instruction(&mut ctx)
}

fn drive_control<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    control_prog::process_instruction(&mut ctx)
}

/// `[disc = 0, amount u64 LE]` — the generated decoder enforces the length.
fn deposit_data(amount: u64) -> [u8; 9] {
    let mut data = [0u8; 9];
    data[1..].copy_from_slice(&amount.to_le_bytes());
    data
}

// ── Compile-time contract ──────────────────────────────────────────

#[test]
fn opt_in_appends_two_trailing_slots_and_advertises_the_const() {
    // The auto-append is real slots: one declared field + two appended.
    assert_eq!(
        Deposit::ACCOUNT_COUNT,
        3,
        "event_cpi must append exactly two trailing account slots"
    );
    assert_eq!(
        PlainDeposit::ACCOUNT_COUNT,
        1,
        "the control context must be untouched"
    );
    assert!(
        Deposit::EVENT_CPI,
        "the opted-in spec must advertise EVENT_CPI = true"
    );
    assert!(
        !PlainDeposit::EVENT_CPI,
        "the control spec must default EVENT_CPI = false"
    );
    // The dispatcher consults exactly these consts (const-folded).
    assert!(event_prog::__HOPPER_EVENT_CPI_ENABLED);
    assert!(!control_prog::__HOPPER_EVENT_CPI_ENABLED);
}

// ── The emit path ──────────────────────────────────────────────────

/// The decisive end-to-end: dispatch a real instruction through the
/// generated dispatcher, let the handler's ONE-LINER emit, capture the
/// inner-instruction data the self-CPI carried, and assert the exact
/// marker+tag+payload bytes round-trip through the public decoder.
#[test]
fn emit_one_liner_produces_the_exact_wire_bytes() {
    let _ = take_host_captured_event_cpis();
    let program_id = Address::new_from_array(PROGRAM_ID);
    let amount: u64 = 5_000;

    let result = HopperSvm::new().process_instruction(
        program_id,
        &deposit_data(amount),
        &[
            vault_fixture(program_id),
            authority_fixture(true),
            program_fixture(program_id),
        ],
        drive_event,
    );
    assert_eq!(result.program_result, Ok(()), "the deposit must succeed");

    // The state change AND the emit both happened.
    let total_off = Ledger::TOTAL_ABS_OFFSET as usize;
    let total = u64::from_le_bytes(
        result.resulting_accounts[0].data[total_off..total_off + 8]
            .try_into()
            .unwrap(),
    );
    assert_eq!(total, amount, "the handler's state change must persist");

    let captured = take_host_captured_event_cpis();
    assert_eq!(
        captured.len(),
        1,
        "exactly one self-CPI event must have been emitted"
    );
    let event = &captured[0];

    // The self-CPI targets the program itself, signed by the
    // context's appended authority slot.
    assert_eq!(event.program_id, program_id, "self-CPI must target self");
    assert_eq!(
        event.authority,
        Address::new_from_array(AUTHORITY_ADDR),
        "the appended event-authority slot must sign the CPI"
    );

    // Exact wire bytes: marker, tag, then the event's Pod payload —
    // byte-compared against the public encoder so producer and
    // consumer are pinned to each other.
    let expected_event = Deposited {
        amount: WireU64::new(amount),
        memo: *b"e2e!",
    };
    let mut expected = [0u8; 3 + core::mem::size_of::<Deposited>()];
    let expected_len = encode_event_cpi(
        Deposited::EVENT_TAG,
        expected_event.as_bytes(),
        &mut expected,
    )
    .unwrap();
    assert_eq!(
        event.data,
        &expected[..expected_len],
        "emitted bytes must round-trip the public encoder byte-for-byte"
    );
    assert_eq!(&event.data[..2], &CPI_EVENT_MARKER, "marker first");
    assert_eq!(event.data[2], 0x42, "declared tag byte third");

    // And the public decoder reads back exactly the tag + payload.
    let (tag, payload) = decode_event_cpi(&event.data).expect("decodable");
    assert_eq!(tag, Deposited::EVENT_TAG);
    assert_eq!(payload, expected_event.as_bytes());

    // The measured size claim: 3 bytes of instruction-data overhead
    // per event (2-byte marker + 1-byte tag). Anchor's emit_cpi wire is
    // 16 bytes of overhead (8-byte EVENT_IX_TAG_LE instruction
    // discriminator + the event's 8-byte discriminator) before the
    // same payload — 13 more per event.
    assert_eq!(
        event.data.len() - core::mem::size_of::<Deposited>(),
        3,
        "instruction-data overhead per event must be exactly 3 bytes"
    );
}

/// The outer event-authority account does not need transaction-signer
/// privilege. Hopper supplies its PDA seeds to the self-CPI; the SVM verifies
/// and grants signer privilege to the inner instruction.
#[test]
fn emit_with_unsigned_outer_pda_authority_uses_supplied_seeds() {
    let _ = take_host_captured_event_cpis();
    let program_id = Address::new_from_array(PROGRAM_ID);

    let result = HopperSvm::new().process_instruction(
        program_id,
        &deposit_data(1),
        &[
            vault_fixture(program_id),
            authority_fixture(false),
            program_fixture(program_id),
        ],
        drive_event,
    );
    assert_eq!(result.program_result, Ok(()));
    assert_eq!(take_host_captured_event_cpis().len(), 1);
}

/// The appended program slot is pinned at bind: passing anything other
/// than the program's own account fails validation before the handler.
#[test]
fn bind_pins_the_appended_program_slot_to_the_program_id() {
    let program_id = Address::new_from_array(PROGRAM_ID);
    let wrong_program =
        AccountFixture::new(Address::new_from_array([8u8; 32]), program_id, 0, 0).executable();

    let result = HopperSvm::new().process_instruction(
        program_id,
        &deposit_data(1),
        &[
            vault_fixture(program_id),
            authority_fixture(true),
            wrong_program,
        ],
        drive_event,
    );
    assert_eq!(
        result.program_result,
        Err(ProgramError::InvalidAccountData),
        "a foreign account in the event_program slot must fail bind"
    );
}

/// The two slots are REQUIRED once opted in: the pre-feature account
/// shape (just the vault) no longer satisfies `require_accounts`.
#[test]
fn opted_in_context_requires_the_two_extra_accounts() {
    let program_id = Address::new_from_array(PROGRAM_ID);
    let result = HopperSvm::new().process_instruction(
        program_id,
        &deposit_data(1),
        &[vault_fixture(program_id)],
        drive_event,
    );
    assert!(
        result.program_result.is_err(),
        "binding without the appended slots must fail"
    );
}

// ── The sink path (the generated [0xE0, 0x1E] arm) ─────────────────

/// The dispatcher routes marker-prefixed data to the generated sink,
/// which authenticates: a signed authority is accepted (host builds
/// stop at the signer check; the PDA address pin is on-chain-only), an
/// unsigned one is refused, and malformed markers never reach it.
#[test]
fn generated_sink_authenticates_marker_instructions() {
    let program_id = Address::new_from_array(PROGRAM_ID);
    let sink_data: &[u8] = &[0xE0, 0x1E, 0x42, 1, 2, 3];

    // Signed authority → the event CPI is accepted (Ok sink).
    let ok = HopperSvm::new().process_instruction(
        program_id,
        sink_data,
        &[authority_fixture(true)],
        drive_event,
    );
    assert_eq!(
        ok.program_result,
        Ok(()),
        "a signed event-authority must satisfy the generated sink"
    );

    // Unsigned authority → forged event, refused.
    let forged = HopperSvm::new().process_instruction(
        program_id,
        sink_data,
        &[authority_fixture(false)],
        drive_event,
    );
    assert_eq!(
        forged.program_result,
        Err(ProgramError::MissingRequiredSignature),
        "an unsigned authority is a forged event and must be refused"
    );

    // Marker byte 0xE0 with a wrong second byte → not an event.
    let wrong_marker = HopperSvm::new().process_instruction(
        program_id,
        &[0xE0, 0x77, 0x42],
        &[authority_fixture(true)],
        drive_event,
    );
    assert_eq!(
        wrong_marker.program_result,
        Err(ProgramError::InvalidInstructionData),
        "a wrong second marker byte must be refused"
    );

    // Marker without a tag byte → truncated, refused.
    let truncated = HopperSvm::new().process_instruction(
        program_id,
        &[0xE0, 0x1E],
        &[authority_fixture(true)],
        drive_event,
    );
    assert_eq!(
        truncated.program_result,
        Err(ProgramError::InvalidInstructionData),
        "marker without a tag byte must be refused"
    );
}

// ── The control program: absent option = pre-feature behavior ──────

/// Without the option nothing changes: the same instruction bytes work
/// against the ORIGINAL account shape, no event is emitted, and the
/// reserved marker stays an unknown discriminator (the sink guard
/// const-folds to false), exactly as before the feature existed.
#[test]
fn control_program_behaves_exactly_as_before() {
    let _ = take_host_captured_event_cpis();
    let program_id = Address::new_from_array(PROGRAM_ID);

    // Pre-feature account shape (just the vault) still binds and runs.
    let result = HopperSvm::new().process_instruction(
        program_id,
        &deposit_data(3),
        &[vault_fixture(program_id)],
        drive_control,
    );
    assert_eq!(
        result.program_result,
        Ok(()),
        "the control handler must run against the original account shape"
    );
    let total_off = Ledger::TOTAL_ABS_OFFSET as usize;
    let total = u64::from_le_bytes(
        result.resulting_accounts[0].data[total_off..total_off + 8]
            .try_into()
            .unwrap(),
    );
    assert_eq!(total, 3, "control state change must persist");
    assert!(
        take_host_captured_event_cpis().is_empty(),
        "the control program must emit no event CPI"
    );

    // The reserved marker is just an unknown discriminator here: the
    // sink guard (`false || PlainDeposit::EVENT_CPI`) const-folds away.
    let marker = HopperSvm::new().process_instruction(
        program_id,
        &[0xE0, 0x1E, 0x42, 1, 2, 3],
        &[authority_fixture(true)],
        drive_control,
    );
    assert_eq!(
        marker.program_result,
        Err(ProgramError::InvalidInstructionData),
        "without the option the marker must stay an unknown discriminator"
    );
}
