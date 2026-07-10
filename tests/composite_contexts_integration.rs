//! Composite (nested) account contexts, end to end.
//!
//! Anchor's composite-accounts feature: a context struct embeds another
//! context struct as a `#[composite]` field, and the inner context's
//! account slots flatten into the outer one in declaration order. Hopper
//! reuses the inner context's GENERATED validators/accessors at a
//! flattened base offset (a `const` generic that is `0` at the top level,
//! so the lowering is byte-identical to a non-composite context, and a
//! concrete offset when embedded).
//!
//! These tests drive the real codegen through the hopper-svm host harness,
//! proving:
//!
//! 1. slots flatten — `ACCOUNT_COUNT` sums (outer = leaves + inner count),
//!    and a field declared AFTER the composite resolves to its flattened
//!    slot (a write through it lands in the right account);
//! 2. BOTH validators enforce — a failure in the OUTER's own field
//!    (payer not a signer) AND a failure in an INNER field at its
//!    flattened offset (inner authority not a signer, inner vault wrong
//!    owner) each fail the bind;
//! 3. state writes through the inner bound context persist — writing via
//!    `ctx.check()?.vault_load_mut()` mutates the inner's flattened slot;
//! 4. nested bumps compose — `ctx.bumps().check` is the inner's `Bumps`.

#![cfg(feature = "proc-macros")]

use hopper::layout::{write_header, HEADER_LEN};
use hopper::prelude::*;
use hopper_svm::{AccountFixture, HopperSvm};

// ── Program under test ─────────────────────────────────────────────

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 71, version = 1)]
pub struct Vault {
    pub balance: WireU64,
}

/// The INNER (nested) context. A plain validation context — no lifecycle,
/// args, or advanced options — so it is embeddable. Its two slots are a
/// signer authority and a mutable program-owned vault account.
#[derive(hopper::Accounts)]
pub struct VaultCheck<'info> {
    pub authority: Signer<'info>,

    #[account(mut)]
    pub vault: Account<'info, Vault>,
}

/// The OUTER context. It embeds `VaultCheck` between its own `payer` and a
/// trailing `tail` account, so the flattened slot order is:
///
/// `[0] payer  ·  [1] check.authority  ·  [2] check.vault  ·  [3] tail`
#[derive(hopper::Accounts)]
pub struct Operate<'info> {
    pub payer: Signer<'info>,

    #[composite]
    pub check: VaultCheck<'info>,

    #[account(mut)]
    pub tail: Account<'info, Vault>,
}

// ── Handlers ───────────────────────────────────────────────────────

/// Binds the outer context, writes DISTINCT values into the inner vault
/// (flattened slot 2) and the trailing tail (flattened slot 3) so the test
/// can prove each landed in the correct account.
fn operate_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    let mut bound = Operate::bind(&mut ctx)?;

    // Compile-time proof that the outer Bumps carries the inner's Bumps as
    // a nested field (`OperateBumps { check: VaultCheckBumps, .. }`).
    let _nested_bumps: &VaultCheckBumps = &bound.bumps().check;

    // Write through the INNER bound context (rebased to the flattened
    // offset): the inner vault is at slot 2.
    bound
        .check()?
        .vault_load_mut()?
        .balance
        .checked_add_assign(111)?;

    // Write the trailing outer field (slot 3, AFTER the composite): if the
    // flattening were wrong this would clobber the inner vault instead.
    bound.tail_load_mut()?.balance.checked_add_assign(222)?;

    Ok(())
}

fn operate_handler_entry<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    operate_handler(program_id, accounts, instruction_data)
}

// ── Fixtures ───────────────────────────────────────────────────────

const PROGRAM_ID: Address = Address::new_from_array([9u8; 32]);
const FOREIGN_OWNER: Address = Address::new_from_array([7u8; 32]);

fn vault_fixture(addr_byte: u8) -> AccountFixture {
    vault_fixture_owned_by(addr_byte, PROGRAM_ID)
}

fn vault_fixture_owned_by(addr_byte: u8, owner: Address) -> AccountFixture {
    let mut data = vec![0u8; Vault::LEN];
    write_header(&mut data, Vault::DISC, Vault::VERSION, &Vault::LAYOUT_ID).unwrap();
    AccountFixture::with_data(
        Address::new_from_array([addr_byte; 32]),
        owner,
        1_000_000,
        data,
    )
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

/// A non-signer account at the same address, to flip a slot's signer bit.
fn non_signer_fixture(addr_byte: u8) -> AccountFixture {
    AccountFixture::new(
        Address::new_from_array([addr_byte; 32]),
        Address::new_from_array([0u8; 32]),
        1_000_000,
        0,
    )
}

fn vault_balance(fixture: &AccountFixture) -> u64 {
    u64::from_le_bytes(fixture.data[HEADER_LEN..HEADER_LEN + 8].try_into().unwrap())
}

/// The canonical valid account list, in flattened order.
fn valid_accounts() -> [AccountFixture; 4] {
    [
        signer_fixture(0x41), // [0] payer
        signer_fixture(0x42), // [1] check.authority
        vault_fixture(0x43),  // [2] check.vault
        vault_fixture(0x44),  // [3] tail
    ]
}

// ── Compile-time flattening ────────────────────────────────────────

/// The published schema must describe every FLATTENED slot: leaves as
/// literals, the composite spliced (verbatim) from the inner context's
/// own descriptors — never one opaque row for N slots. The length is
/// additionally const-asserted against `ACCOUNT_COUNT` inside the
/// generated composed array, so a mismatch cannot even compile.
#[test]
fn schema_metadata_splices_the_inner_context_per_slot() {
    let accounts = Operate::SCHEMA_METADATA.accounts;
    assert_eq!(
        accounts.len(),
        Operate::ACCOUNT_COUNT,
        "one descriptor per flattened slot"
    );
    let names: std::vec::Vec<&str> = accounts.iter().map(|a| a.name).collect();
    assert_eq!(
        names,
        ["payer", "authority", "vault", "tail"],
        "leaves in order with the inner context's slots spliced in place"
    );
    // Spliced descriptors carry the INNER context's real constraints,
    // published exactly as leaf contexts publish them: wrapper-derived
    // roles ride `kind` (a `Signer<'info>` field is kind "Signer");
    // the boolean flags reflect explicit `#[account(...)]` attrs.
    assert_eq!(accounts[1].kind, "Signer", "check.authority role via kind");
    assert!(accounts[2].writable, "check.vault is writable");
    assert_eq!(accounts[2].layout_ref, "Vault");
    assert_eq!(accounts[0].kind, "Signer", "payer role via kind");
    assert!(accounts[3].writable, "tail is writable");
    // And the spliced slice is literally the inner context's published
    // descriptor data, not a re-derivation.
    let inner = VaultCheck::SCHEMA_METADATA.accounts;
    assert_eq!(inner.len(), 2);
    assert_eq!(inner[0].name, accounts[1].name);
    assert_eq!(inner[1].layout_ref, accounts[2].layout_ref);
}

#[test]
fn account_count_sums_across_the_composite() {
    assert_eq!(VaultCheck::ACCOUNT_COUNT, 2);
    assert_eq!(
        Operate::ACCOUNT_COUNT,
        4,
        "outer count = 1 payer + VaultCheck::ACCOUNT_COUNT (2) + 1 tail"
    );
    // The inner context is embeddable; the outer container is not.
    assert!(VaultCheck::__HOPPER_EMBEDDABLE);
    assert!(!Operate::__HOPPER_EMBEDDABLE);
}

// ── Arm 1: correct set binds; writes land in the flattened slots ───

#[test]
fn valid_set_binds_and_writes_land_in_the_correct_flattened_slots() {
    let accounts = valid_accounts();
    let result =
        HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, operate_handler_entry);
    assert!(
        result.program_result.is_ok(),
        "a valid composite set must bind: {:?}",
        result.program_result
    );
    // The inner vault (flattened slot 2) received the inner write.
    assert_eq!(
        vault_balance(&result.resulting_accounts[2]),
        111,
        "the write through the inner bound context must persist to slot 2"
    );
    // The trailing field (flattened slot 3) received the outer write — the
    // field AFTER the composite resolved to the correct flattened slot.
    assert_eq!(
        vault_balance(&result.resulting_accounts[3]),
        222,
        "the field after the composite must resolve to flattened slot 3"
    );
    // Neither write bled into the other slot.
    assert_ne!(
        vault_balance(&result.resulting_accounts[2]),
        vault_balance(&result.resulting_accounts[3])
    );
}

// ── Arm 2: the INNER validator enforces at the flattened offset ────

#[test]
fn inner_wrong_owner_vault_fails_bind() {
    let mut accounts = valid_accounts();
    // The inner vault (flattened slot 2) is owned by a foreign program.
    accounts[2] = vault_fixture_owned_by(0x43, FOREIGN_OWNER);
    let result =
        HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, operate_handler_entry);
    assert!(
        result.program_result.is_err(),
        "a wrong-owner inner slot must fail the inner context's owner check"
    );
}

#[test]
fn inner_unsigned_authority_fails_bind() {
    let mut accounts = valid_accounts();
    // The inner authority (flattened slot 1) is not a signer.
    accounts[1] = non_signer_fixture(0x42);
    let result =
        HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, operate_handler_entry);
    assert!(
        result.program_result.is_err(),
        "an unsigned inner authority must fail the inner context's signer check"
    );
}

// ── Arm 3: the OUTER validator enforces its own fields ─────────────

#[test]
fn outer_unsigned_payer_fails_bind() {
    let mut accounts = valid_accounts();
    // The outer payer (slot 0) is not a signer.
    accounts[0] = non_signer_fixture(0x41);
    let result =
        HopperSvm::new().process_instruction(PROGRAM_ID, &[], &accounts, operate_handler_entry);
    assert!(
        result.program_result.is_err(),
        "an unsigned outer payer must fail the outer context's signer check"
    );
}

// ── Arm 4: not enough accounts (flattened require_accounts) ─────────

#[test]
fn too_few_accounts_for_the_flattened_count_fails() {
    let accounts = valid_accounts();
    let result = HopperSvm::new().process_instruction(
        PROGRAM_ID,
        &[],
        &accounts[..3], // one short of the flattened ACCOUNT_COUNT (4)
        operate_handler_entry,
    );
    assert!(
        result.program_result.is_err(),
        "fewer than ACCOUNT_COUNT accounts must fail require_accounts"
    );
}
