//! Optional accounts (Anchor ≥ 0.26 `Option<...>` parity), end to end.
//!
//! Anchor's optional-account calling convention, matched exactly so
//! ported clients work unchanged: an ABSENT optional account is passed
//! as THE PROGRAM'S OWN ID in that account slot. At bind time, a slot
//! whose address equals the executing program id binds `None` and skips
//! every check attached to that field; any other address takes the full
//! inner-wrapper path (all checks) and binds `Some(inner)`.
//!
//! These tests drive both arms through the hopper-svm harness:
//!
//! 1. absent  → handler sees `None`, succeeds, and the absent slot needs
//!    to satisfy ZERO constraints (not writable, not a signer, wrong
//!    owner — nothing is checked);
//! 2. present + valid → handler sees `Some`, with every declared check
//!    enforced and the wrapper fully usable (typed mutation);
//! 3. present + invalid (wrong owner / not writable / unsigned) → bind
//!    fails with byte-identical errors to a control context whose same
//!    fields are required.

#![cfg(feature = "proc-macros")]

use hopper::layout::{write_header, HEADER_LEN};
use hopper::prelude::*;
use hopper_svm::{AccountFixture, HopperSvm};

// ── Program under test ─────────────────────────────────────────────

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 61, version = 1)]
pub struct TipJar {
    pub total: WireU64,
}

/// Optional-account context. `referral` (a mutable layout account) and
/// `bonus_authority` (a signer) are conditionally present, the way an
/// optional referral / fee account is in real programs.
#[derive(hopper::Accounts)]
pub struct Tip<'info> {
    pub authority: Signer<'info>,

    #[account(mut)]
    pub jar: Account<'info, TipJar>,

    #[account(mut)]
    pub referral: Option<Account<'info, TipJar>>,

    pub bonus_authority: Option<Signer<'info>>,
}

/// Control: the same shape with both fields REQUIRED, so the
/// present-but-invalid arms can assert the optional path produces
/// byte-identical errors to the non-optional path.
#[derive(hopper::Accounts)]
pub struct TipRequired<'info> {
    pub authority: Signer<'info>,

    #[account(mut)]
    pub jar: Account<'info, TipJar>,

    #[account(mut)]
    pub referral: Account<'info, TipJar>,

    pub bonus_authority: Signer<'info>,
}

// ── Handlers ───────────────────────────────────────────────────────

/// data[0] / data[1] carry the caller's expectation for referral /
/// bonus_authority presence so the handler can assert the bound
/// `Option` mirrors the slot address exactly.
fn tip_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    let bound = Tip::bind(&mut ctx)?;

    let expect_referral = instruction_data[0] == 1;
    let expect_bonus = instruction_data[1] == 1;
    assert_eq!(
        bound.accounts.referral.is_some(),
        expect_referral,
        "referral presence must mirror the slot address"
    );
    assert_eq!(
        bound.accounts.bonus_authority.is_some(),
        expect_bonus,
        "bonus_authority presence must mirror the slot address"
    );
    // The presence-aware accessor (the facade-less spelling) agrees
    // with the typed facade.
    assert_eq!(bound.referral_account_opt()?.is_some(), expect_referral);
    assert_eq!(bound.bonus_authority_account_opt()?.is_some(), expect_bonus);

    // Required account: always usable.
    bound
        .accounts
        .jar
        .with_mut(|jar| jar.total.checked_add_assign(10))?;

    // Present optional: a fully-validated typed wrapper.
    if let Some(referral) = &bound.accounts.referral {
        referral.with_mut(|r| r.total.checked_add_assign(1))?;
    }
    if let Some(bonus) = &bound.accounts.bonus_authority {
        assert_ne!(
            bonus.key(),
            program_id,
            "a present optional can never carry the program id"
        );
    }
    Ok(())
}

fn tip_required_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    let _bound = TipRequired::bind(&mut ctx)?;
    Ok(())
}

// ── Fixtures ───────────────────────────────────────────────────────

const PROGRAM_ID: Address = Address::new_from_array([9u8; 32]);

fn jar_fixture(addr_byte: u8) -> AccountFixture {
    jar_fixture_owned_by(addr_byte, PROGRAM_ID)
}

fn jar_fixture_owned_by(addr_byte: u8, owner: Address) -> AccountFixture {
    let mut data = vec![0u8; TipJar::LEN];
    write_header(&mut data, TipJar::DISC, TipJar::VERSION, &TipJar::LAYOUT_ID).unwrap();
    AccountFixture::with_data(
        Address::new_from_array([addr_byte; 32]),
        owner,
        1_000_000,
        data,
    )
    .writable()
}

/// The Anchor absence convention: the slot carries the executing
/// program's own id. Deliberately NOT writable, NOT a signer, empty,
/// and owned by nobody in particular — an absent optional must satisfy
/// ZERO constraints because none of its checks run.
fn absent_fixture() -> AccountFixture {
    AccountFixture::new(PROGRAM_ID, Address::new_from_array([0u8; 32]), 1, 0)
}

fn signer_fixture(addr_byte: u8) -> AccountFixture {
    AccountFixture::new(
        Address::new_from_array([addr_byte; 32]),
        Address::new_from_array([0u8; 32]),
        1_000_000,
        0,
    )
    .signer()
}

fn jar_total(fixture: &AccountFixture) -> u64 {
    u64::from_le_bytes(fixture.data[HEADER_LEN..HEADER_LEN + 8].try_into().unwrap())
}

// ── Arm 1: absent → None, zero checks ──────────────────────────────

#[test]
fn absent_optionals_bind_none_and_perform_zero_checks() {
    // The absent slots violate every constraint their fields declare
    // when present (`mut` referral is not writable; `bonus_authority`
    // is not a signer) — binding succeeds anyway because absence skips
    // the checks entirely.
    let accounts = [
        signer_fixture(0x21),
        jar_fixture(0x22),
        absent_fixture(),
        absent_fixture(),
    ];
    let result = HopperSvm::new().process_instruction(
        PROGRAM_ID,
        &[0, 0], // handler must see None / None
        &accounts,
        tip_handler,
    );
    assert!(
        result.program_result.is_ok(),
        "absent optionals must bind None and succeed: {:?}",
        result.program_result
    );
    // The required jar was mutated; the absent slots were not touched.
    assert_eq!(jar_total(&result.resulting_accounts[1]), 10);
    assert_eq!(result.resulting_accounts[2], accounts[2]);
    assert_eq!(result.resulting_accounts[3], accounts[3]);
}

/// The same program-id-in-slot accounts fed to the REQUIRED control
/// context fail its checks — absence semantics apply only to fields
/// declared `Option<..>`.
#[test]
fn program_id_slot_in_a_required_context_is_not_special() {
    let accounts = [
        signer_fixture(0x31),
        jar_fixture(0x32),
        absent_fixture(),
        absent_fixture(),
    ];
    let result =
        HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, tip_required_handler);
    assert!(
        result.program_result.is_err(),
        "a required field must still run its checks against a program-id slot"
    );
}

// ── Arm 2: present + valid → Some, all checks, fully usable ────────

#[test]
fn present_optionals_bind_some_with_checks_enforced_and_typed_access() {
    let accounts = [
        signer_fixture(0x41),
        jar_fixture(0x42),
        jar_fixture(0x43),    // present, valid, writable
        signer_fixture(0x44), // present, signed
    ];
    let result = HopperSvm::new().process_instruction(
        PROGRAM_ID,
        &[1, 1], // handler must see Some / Some
        &accounts,
        tip_handler,
    );
    assert!(
        result.program_result.is_ok(),
        "valid present optionals must bind Some: {:?}",
        result.program_result
    );
    // Both the required jar and the present optional referral were
    // mutated through their typed wrappers.
    assert_eq!(jar_total(&result.resulting_accounts[1]), 10);
    assert_eq!(jar_total(&result.resulting_accounts[2]), 1);
}

// ── Arm 3: present + invalid → same error as the required form ─────

fn run_both(
    accounts: &[AccountFixture; 4],
) -> (
    Result<(), hopper::__runtime::ProgramError>,
    Result<(), hopper::__runtime::ProgramError>,
) {
    let optional = HopperSvm::new()
        .process_instruction(PROGRAM_ID, &[1, 1], accounts, tip_handler)
        .program_result;
    let required = HopperSvm::new()
        .process_instruction(PROGRAM_ID, &[1, 1], accounts, tip_required_handler)
        .program_result;
    (optional, required)
}

#[test]
fn present_optional_with_wrong_owner_fails_exactly_like_required() {
    let accounts = [
        signer_fixture(0x51),
        jar_fixture(0x52),
        // Present (address != program id) but owned by a foreign
        // program: the gated owner check must fire.
        jar_fixture_owned_by(0x53, Address::new_from_array([7u8; 32])),
        signer_fixture(0x54),
    ];
    let (optional, required) = run_both(&accounts);
    assert!(optional.is_err(), "wrong owner must fail the optional path");
    assert_eq!(
        optional, required,
        "a present optional must fail with the exact error the required form produces"
    );
}

#[test]
fn present_optional_not_writable_fails_exactly_like_required() {
    let mut referral = jar_fixture(0x63);
    referral.is_writable = false; // valid account, missing the `mut` grant
    let accounts = [
        signer_fixture(0x61),
        jar_fixture(0x62),
        referral,
        signer_fixture(0x64),
    ];
    let (optional, required) = run_both(&accounts);
    assert!(
        optional.is_err(),
        "a present optional `mut` account must be writable"
    );
    assert_eq!(optional, required);
}

#[test]
fn present_optional_signer_unsigned_fails_exactly_like_required() {
    let mut bonus = signer_fixture(0x74);
    bonus.is_signer = false; // present but did not sign
    let accounts = [
        signer_fixture(0x71),
        jar_fixture(0x72),
        jar_fixture(0x73),
        bonus,
    ];
    let (optional, required) = run_both(&accounts);
    assert!(
        optional.is_err(),
        "a present optional Signer must have signed"
    );
    assert_eq!(optional, required);
}

// ── Published surface: slots, schema, docs ─────────────────────────

#[test]
fn optional_slots_still_count_and_schema_publishes_optionality() {
    // An absent optional still occupies its account slot (Anchor
    // convention without `allow-missing-optionals`).
    assert_eq!(Tip::ACCOUNT_COUNT, 4);
    assert_eq!(TipRequired::ACCOUNT_COUNT, 4);

    let meta = Tip::SCHEMA_METADATA;
    assert!(!meta.accounts[0].optional);
    assert!(!meta.accounts[1].optional);
    assert!(meta.accounts[2].optional);
    assert!(meta.accounts[3].optional);
    // Optionality publishes the INNER role, not the literal `Option`.
    assert_eq!(meta.accounts[2].kind, "TipJar");
    assert_eq!(meta.accounts[2].layout_ref, "TipJar");
    assert_eq!(meta.accounts[3].kind, "Signer");
    assert!(TipRequired::SCHEMA_METADATA
        .accounts
        .iter()
        .all(|a| !a.optional));

    // The published check docs name the skip contract for optional
    // fields so auditors see the conditionality without reading source.
    assert!(Tip::VALIDATION_CHECKS
        .iter()
        .any(|c| c.contains("(referral) is optional")));
    assert!(Tip::VALIDATION_CHECKS
        .iter()
        .any(|c| c.contains("(bonus_authority) is optional")));
    assert!(TipRequired::VALIDATION_CHECKS
        .iter()
        .all(|c| !c.contains("is optional")));
}
