//! `init_if_needed`'s both-arms behavior, end to end.
//!
//! Anchor-parity `init_if_needed` conditionally creates an account: an
//! EMPTY slot (`data_len() == 0`) is allowed straight through binding, and
//! the field's `init_{field}()` lifecycle helper then runs the full
//! `CreateAccount` CPI lifecycle (fund, allocate, assign, zero-init, write
//! the Hopper layout header) exactly like a plain `init` would. A
//! NONEMPTY slot instead runs the same owner-check + `load::<T>()`
//! layout-header validation an ordinary already-existing field runs — and
//! the lifecycle helper becomes a no-op, so no second `CreateAccount` CPI
//! ever fires against an account that already exists.
//!
//! These tests drive both arms through the hopper-svm host harness:
//!
//! 1. empty     → bind succeeds, the full init CPI lifecycle runs (payer
//!    debited the rent-exempt minimum, the account funded/assigned/
//!    header-written), and the account is immediately usable;
//! 2. present + valid → bind succeeds WITHOUT re-running the create CPI
//!    (the payer's lamports are untouched) and the preexisting state
//!    survives, still mutable;
//! 3. present + wrong owner → bind fails with the exact error a plain
//!    required (non-init) field would produce for the same fixture;
//! 4. present + foreign layout header → same, for a header mismatch;
//! 5. composite v2 lifted the v1 restriction that refused any lifecycle
//!    attribute on a context embedding `#[composite]` — but only for the
//!    OUTER container. This proves the outer side actually runs, not
//!    just expands;
//! 6. pre-funded + unallocated → bind succeeds via `hopper_init!`'s OTHER
//!    create branch (Transfer-shortfall + Allocate + Assign — CreateAccount
//!    refuses an account that already carries lamports): the payer is
//!    debited exactly the shortfall (exactly zero when the slot already
//!    holds the rent-exempt minimum), and the account still ends
//!    allocated, assigned, and header-written.

#![cfg(feature = "proc-macros")]

use hopper::layout::{write_header, HEADER_LEN};
use hopper::prelude::*;
use hopper_svm::{AccountFixture, HopperSvm};

// ── Program under test ─────────────────────────────────────────────

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 95, version = 1)]
pub struct Ledger {
    pub total: WireU64,
}

/// `init_if_needed` context. `ledger` may arrive empty (create it) or
/// already-populated (validate it) — from the caller's perspective bind
/// either succeeds or fails with the same shape of error either way; only
/// whether the CPI-create lifecycle actually fires differs.
#[derive(hopper::Accounts)]
pub struct Upsert<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(init_if_needed, payer = payer, space = Ledger::LEN)]
    pub ledger: Account<'info, Ledger>,

    pub system_program: Program<'info, System>,
}

/// Control: the same shape with `ledger` an ORDINARY required existing
/// account (no lifecycle attribute at all), so the present-but-invalid
/// arms can assert the `init_if_needed` path produces byte-identical
/// errors to the non-lifecycle path.
#[derive(hopper::Accounts)]
pub struct UpsertRequired<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(mut)]
    pub ledger: Account<'info, Ledger>,

    pub system_program: Program<'info, System>,
}

/// A trivial embeddable context used only to prove composite v2's outer
/// composability (see arm 5 below).
#[derive(hopper::Accounts)]
pub struct CheckAuthority<'info> {
    pub authority: Signer<'info>,
}

/// An OUTER container that embeds `#[composite] check` AND declares its
/// own `init_if_needed` field. v1 refused any lifecycle attribute
/// (`init` / `init_if_needed` / `zero` / `close` / `realloc` / `sweep`)
/// on a context embedding `#[composite]` at all; v2 lifted that
/// restriction for the OUTER side only (an INNER embedded context stays
/// restricted — a compile-time `assert!` pinned by expansion tests in
/// `hopper-macros-proc`, not re-tested here).
#[derive(hopper::Accounts)]
pub struct OuterUpsert<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(init_if_needed, payer = payer, space = Ledger::LEN)]
    pub ledger: Account<'info, Ledger>,

    pub system_program: Program<'info, System>,

    #[composite]
    pub check: CheckAuthority<'info>,
}

// ── Handlers ───────────────────────────────────────────────────────

fn upsert_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    let bound = Upsert::bind(&mut ctx)?;
    // Empty slot → runs the full `CreateAccount` CPI lifecycle. Nonempty
    // slot → no-op: the bind-time validator above already proved owner +
    // layout are correct, so there is nothing left for this to do.
    bound.init_ledger()?;
    bound
        .accounts
        .ledger
        .with_mut(|l| l.total.checked_add_assign(1))?;
    Ok(())
}

fn upsert_required_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    let _bound = UpsertRequired::bind(&mut ctx)?;
    Ok(())
}

fn outer_upsert_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    let bound = OuterUpsert::bind(&mut ctx)?;
    bound.init_ledger()?;
    Ok(())
}

// ── Fixtures ───────────────────────────────────────────────────────

const PROGRAM_ID: Address = Address::new_from_array([13u8; 32]);

fn payer_fixture(addr_byte: u8, lamports: u64) -> AccountFixture {
    AccountFixture::new(
        Address::new_from_array([addr_byte; 32]),
        Address::new_from_array([0u8; 32]),
        lamports,
        0,
    )
    .signer()
    .writable()
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

fn system_program_fixture() -> AccountFixture {
    AccountFixture::new(
        Address::new_from_array([0u8; 32]),
        Address::new_from_array([0u8; 32]),
        1,
        0,
    )
    .executable()
}

/// The Solana "brand new account" convention: no data, no lamports.
/// Deliberately a signer too — the System Program's `CreateAccount`
/// requires the account being created to consent (sign) unless a PDA's
/// seeds authorize it instead, matching a fresh client-generated keypair.
fn empty_fixture(addr_byte: u8) -> AccountFixture {
    AccountFixture::new(
        Address::new_from_array([addr_byte; 32]),
        Address::new_from_array([0u8; 32]),
        0,
        0,
    )
    .writable()
    .signer()
}

/// The pre-funded-but-unallocated shape: someone already sent lamports to
/// the address (an airdrop, a prior partial flow) but no data was ever
/// allocated. `CreateAccount` refuses an account that already carries
/// lamports, so `hopper_init!` must take its OTHER branch here —
/// Transfer-shortfall + Allocate + Assign. Writable AND signer: the
/// System Program requires the allocated/assigned account itself to
/// consent (sign), exactly as for `CreateAccount`.
fn prefunded_fixture(addr_byte: u8, lamports: u64) -> AccountFixture {
    AccountFixture::new(
        Address::new_from_array([addr_byte; 32]),
        Address::new_from_array([0u8; 32]),
        lamports,
        0,
    )
    .writable()
    .signer()
}

fn ledger_fixture(addr_byte: u8, value: u64) -> AccountFixture {
    ledger_fixture_owned_by(addr_byte, PROGRAM_ID, value)
}

fn ledger_fixture_owned_by(addr_byte: u8, owner: Address, value: u64) -> AccountFixture {
    let mut data = vec![0u8; Ledger::LEN];
    write_header(&mut data, Ledger::DISC, Ledger::VERSION, &Ledger::LAYOUT_ID).unwrap();
    data[HEADER_LEN..HEADER_LEN + 8].copy_from_slice(&value.to_le_bytes());
    AccountFixture::with_data(
        Address::new_from_array([addr_byte; 32]),
        owner,
        1_057_920,
        data,
    )
    .writable()
}

/// Correct owner, but a foreign layout header (wrong disc) — the
/// nonempty-but-invalid arm that must fail `load::<Ledger>()`.
fn ledger_fixture_foreign_layout(addr_byte: u8) -> AccountFixture {
    let mut data = vec![0u8; Ledger::LEN];
    write_header(
        &mut data,
        Ledger::DISC.wrapping_add(1),
        Ledger::VERSION,
        &Ledger::LAYOUT_ID,
    )
    .unwrap();
    AccountFixture::with_data(
        Address::new_from_array([addr_byte; 32]),
        PROGRAM_ID,
        1_057_920,
        data,
    )
    .writable()
}

fn ledger_total(fixture: &AccountFixture) -> u64 {
    u64::from_le_bytes(fixture.data[HEADER_LEN..HEADER_LEN + 8].try_into().unwrap())
}

// ── Arm 1: empty slot → full init CPI lifecycle ────────────────────

#[test]
fn empty_slot_binds_and_runs_the_full_init_cpi_lifecycle() {
    let accounts = [
        payer_fixture(0x11, 10_000_000_000),
        empty_fixture(0x12),
        system_program_fixture(),
    ];
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, upsert_handler);
    assert!(
        result.program_result.is_ok(),
        "an empty slot must bind and init successfully: {:?}",
        result.program_result
    );

    // The account was actually created: funded, assigned to the program,
    // header written, and the handler's own mutation landed.
    let ledger = &result.resulting_accounts[1];
    assert_eq!(
        ledger.data.len(),
        Ledger::LEN,
        "CreateAccount must allocate exactly `space` bytes"
    );
    assert_eq!(
        ledger.owner, PROGRAM_ID,
        "CreateAccount must assign the program as owner"
    );
    assert!(
        ledger.lamports > 0,
        "CreateAccount must fund the new account"
    );
    assert_eq!(
        ledger_total(ledger),
        1,
        "the account is usable immediately after bind: zero-initialized then mutated by the handler"
    );

    // The Hopper layout header, pinned at the byte level (wire format:
    // disc at byte 0, version at byte 1, layout fingerprint at bytes
    // 4..12) — direct assertions on the init lifecycle's own header
    // write, not merely transitive trust in `with_mut`'s re-validation
    // having accepted it above.
    assert_eq!(
        ledger.data[0],
        Ledger::DISC,
        "header byte 0 must be the layout discriminator"
    );
    assert_eq!(
        ledger.data[1],
        Ledger::VERSION,
        "header byte 1 must be the layout version"
    );
    assert_eq!(
        &ledger.data[4..12],
        &Ledger::LAYOUT_ID,
        "header bytes 4..12 must be the layout fingerprint"
    );

    // The payer paid for it — the create CPI actually moved lamports,
    // it did not merely flip a flag.
    let payer = &result.resulting_accounts[0];
    assert!(
        payer.lamports < 10_000_000_000,
        "the payer must be debited the new account's rent-exempt minimum"
    );
    assert_eq!(payer.lamports + ledger.lamports, 10_000_000_000);
}

// ── Arm 2: present + valid → no re-create, state preserved ─────────

#[test]
fn nonempty_valid_slot_binds_without_recreating_and_preserves_state() {
    let ledger_before = ledger_fixture(0x22, 41);
    let pre_bytes = ledger_before.data.clone();
    let accounts = [
        payer_fixture(0x21, 10_000_000_000),
        ledger_before,
        system_program_fixture(),
    ];
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, upsert_handler);
    assert!(
        result.program_result.is_ok(),
        "a valid nonempty slot must bind successfully: {:?}",
        result.program_result
    );

    // No second CreateAccount CPI fired: the payer's balance is
    // untouched, not merely "not double-charged".
    assert_eq!(
        result.resulting_accounts[0].lamports, 10_000_000_000,
        "init_if_needed must skip the create CPI entirely for a nonempty slot"
    );
    // The account itself was not re-funded either.
    assert_eq!(result.resulting_accounts[1].lamports, 1_057_920);
    // Preexisting state survived, then was mutated by the handler.
    assert_eq!(
        ledger_total(&result.resulting_accounts[1]),
        42,
        "preexisting state must be preserved, then usable by the handler"
    );

    // Structural preservation: the handler mutates exactly ONE range —
    // the 8-byte `total` field at HEADER_LEN..HEADER_LEN + 8 — so every
    // other byte must survive verbatim. The ranges around the mutated
    // field are compared explicitly (not via a value-level spot check) so
    // a future multi-field Ledger cannot hide corruption of an untouched
    // field behind an accidentally-correct `total`.
    let post_bytes = &result.resulting_accounts[1].data;
    assert_eq!(
        post_bytes.len(),
        pre_bytes.len(),
        "binding a nonempty slot must not resize it"
    );
    assert_eq!(
        &post_bytes[..HEADER_LEN],
        &pre_bytes[..HEADER_LEN],
        "the header bytes must survive a nonempty bind byte-for-byte"
    );
    assert_eq!(
        &post_bytes[HEADER_LEN..HEADER_LEN + 8],
        &42u64.to_le_bytes(),
        "the total field is the only mutated range, by exactly the handler's +1"
    );
    assert_eq!(
        &post_bytes[HEADER_LEN + 8..],
        &pre_bytes[HEADER_LEN + 8..],
        "every byte after the mutated total field must survive byte-for-byte \
         (empty today; load-bearing the day Ledger grows a second field)"
    );
}

// ── Arm 3 & 4: present + invalid → byte-identical to a required field ──

fn run_both(
    accounts: &[AccountFixture; 3],
) -> (
    Result<(), hopper::__runtime::ProgramError>,
    Result<(), hopper::__runtime::ProgramError>,
) {
    let under_test = HopperSvm::new()
        .process_instruction(PROGRAM_ID, &[], accounts, upsert_handler)
        .program_result;
    let required = HopperSvm::new()
        .process_instruction(PROGRAM_ID, &[], accounts, upsert_required_handler)
        .program_result;
    (under_test, required)
}

#[test]
fn nonempty_wrong_owner_fails_exactly_like_a_required_mut_account() {
    let accounts = [
        payer_fixture(0x31, 10_000_000_000),
        ledger_fixture_owned_by(0x32, Address::new_from_array([7u8; 32]), 0),
        system_program_fixture(),
    ];
    let (under_test, required) = run_both(&accounts);
    assert!(
        matches!(under_test, Err(ProgramError::IncorrectProgramId)),
        "wrong owner must fail with the exact owner-mismatch error, not a generic catch-all: {under_test:?}"
    );
    assert_eq!(
        under_test, required,
        "init_if_needed's nonempty-branch owner check must fail exactly like an ordinary required field"
    );
}

#[test]
fn nonempty_foreign_layout_fails_exactly_like_a_required_mut_account() {
    let accounts = [
        payer_fixture(0x41, 10_000_000_000),
        ledger_fixture_foreign_layout(0x42),
        system_program_fixture(),
    ];
    let (under_test, required) = run_both(&accounts);
    assert!(
        matches!(under_test, Err(ProgramError::InvalidAccountData)),
        "a foreign layout header must fail with the exact header-mismatch error, not a generic catch-all: {under_test:?}"
    );
    assert_eq!(
        under_test, required,
        "init_if_needed's nonempty-branch load::<T>() check must fail exactly like an ordinary required field"
    );
}

// ── Arm 5: composite v2 composes init_if_needed on the OUTER side ──

#[test]
fn composite_v2_lets_the_outer_container_use_init_if_needed() {
    let accounts = [
        payer_fixture(0x51, 10_000_000_000),
        empty_fixture(0x52),
        system_program_fixture(),
        signer_fixture(0x53), // check.authority
    ];
    let result =
        HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, outer_upsert_handler);
    assert!(
        result.program_result.is_ok(),
        "an outer container embedding #[composite] must still bind and init its own \
         init_if_needed field: {:?}",
        result.program_result
    );
    assert_eq!(result.resulting_accounts[1].data.len(), Ledger::LEN);
}

// ── Arm 6: pre-funded + unallocated → Transfer + Allocate + Assign ──

#[test]
fn prefunded_below_minimum_allocates_assigns_and_debits_exactly_the_shortfall() {
    let rent_min = hopper::hopper_core::check::rent_exempt_min(Ledger::LEN);
    // Strictly between 0 and the rent-exempt minimum, so the branch must
    // BOTH top the account up (Transfer of the shortfall) AND still
    // allocate + assign it.
    let prefunded = rent_min / 2;
    assert!(0 < prefunded && prefunded < rent_min);

    let accounts = [
        payer_fixture(0x61, 10_000_000_000),
        prefunded_fixture(0x62, prefunded),
        system_program_fixture(),
    ];
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, upsert_handler);
    assert!(
        result.program_result.is_ok(),
        "a pre-funded unallocated slot must bind and init successfully: {:?}",
        result.program_result
    );

    // (a) Allocated, assigned, header-written, and immediately usable —
    // the same end state the CreateAccount arm proves, reached through
    // the other branch.
    let ledger = &result.resulting_accounts[1];
    assert_eq!(
        ledger.data.len(),
        Ledger::LEN,
        "Allocate must size the account to exactly `space` bytes"
    );
    assert_eq!(
        ledger.owner, PROGRAM_ID,
        "Assign must hand the account to the program"
    );
    assert_eq!(
        ledger.data[0],
        Ledger::DISC,
        "header byte 0 must be the layout discriminator"
    );
    assert_eq!(
        ledger.data[1],
        Ledger::VERSION,
        "header byte 1 must be the layout version"
    );
    assert_eq!(
        &ledger.data[4..12],
        &Ledger::LAYOUT_ID,
        "header bytes 4..12 must be the layout fingerprint"
    );
    assert_eq!(
        ledger_total(ledger),
        1,
        "the account is usable immediately after bind: zero-initialized then mutated by the handler"
    );

    // (b) The payer was debited EXACTLY the shortfall — this pins the
    // Transfer+Allocate+Assign path as DISTINCT from CreateAccount, which
    // would have debited the full rent-exempt minimum.
    let payer = &result.resulting_accounts[0];
    let shortfall = rent_min - prefunded;
    assert_eq!(
        payer.lamports,
        10_000_000_000 - shortfall,
        "the payer must be debited exactly rent_min - prefunded, not the full minimum"
    );
    assert_eq!(
        ledger.lamports, rent_min,
        "the account must end topped up to exactly the rent-exempt minimum"
    );
    // Lamport conservation: everything in the post-state is exactly what
    // the two fixtures brought in — nothing minted, nothing destroyed.
    assert_eq!(payer.lamports + ledger.lamports, 10_000_000_000 + prefunded);
}

#[test]
fn prefunded_at_minimum_debits_the_payer_nothing_but_still_allocates_and_assigns() {
    let rent_min = hopper::hopper_core::check::rent_exempt_min(Ledger::LEN);

    let accounts = [
        payer_fixture(0x71, 10_000_000_000),
        prefunded_fixture(0x72, rent_min),
        system_program_fixture(),
    ];
    let result = HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, upsert_handler);
    assert!(
        result.program_result.is_ok(),
        "a fully pre-funded unallocated slot must bind and init successfully: {:?}",
        result.program_result
    );

    // No Transfer fires at all: `current_lamports < rent_min` is strictly
    // false at equality, so the payer is debited exactly 0.
    assert_eq!(
        result.resulting_accounts[0].lamports, 10_000_000_000,
        "at/above the minimum the shortfall Transfer must not fire — the payer is debited exactly 0"
    );

    // ... but Allocate + Assign still ran, and the account is usable.
    let ledger = &result.resulting_accounts[1];
    assert_eq!(ledger.data.len(), Ledger::LEN);
    assert_eq!(ledger.owner, PROGRAM_ID);
    assert_eq!(
        ledger.lamports, rent_min,
        "the account keeps exactly its pre-funding — no top-up, no refund"
    );
    assert_eq!(ledger_total(ledger), 1);
}

// ── Published surface: slots, schema, docs ─────────────────────────

#[test]
fn published_surface_names_init_if_needed_and_its_bind_time_contract() {
    assert_eq!(Upsert::ACCOUNT_COUNT, 3);
    assert_eq!(UpsertRequired::ACCOUNT_COUNT, 3);

    let meta = Upsert::SCHEMA_METADATA;
    assert!(
        !meta.accounts[1].optional,
        "init_if_needed is not the Option<...> spelling — it is unconditionally present"
    );
    assert_eq!(meta.accounts[1].kind, "Ledger");
    assert_eq!(meta.accounts[1].layout_ref, "Ledger");
    assert_eq!(
        meta.accounts[1].lifecycle,
        hopper::hopper_schema::accounts::AccountLifecycle::InitIfNeeded
    );
    assert_eq!(
        UpsertRequired::SCHEMA_METADATA.accounts[1].lifecycle,
        hopper::hopper_schema::accounts::AccountLifecycle::Existing
    );

    // The published check description names the exact bind-time
    // contract so auditors see the conditionality without reading source.
    assert!(Upsert::VALIDATION_CHECKS
        .iter()
        .any(|c| c.contains("(ledger) may be empty for init_if_needed")
            && c.contains("owner matches")
            && c.contains("Ledger header is valid")));
    assert!(UpsertRequired::VALIDATION_CHECKS
        .iter()
        .all(|c| !c.contains("may be empty for init_if_needed")));
}
