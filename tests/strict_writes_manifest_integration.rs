//! BLD-I24: `strict_writes` write-sets are scheduler-legible with zero
//! manual authoring.
//!
//! The `#[hopper::context(strict_writes)]` macro compiles the `mut(seg)`
//! declarations into ONE shared const that backs three surfaces at once:
//!
//! 1. the runtime `WritePolicy` installed by `bind()` (enforcement),
//! 2. the `WRITE_RANGES` / `STRICT_WRITES` associated consts (authoring),
//! 3. `SCHEMA_METADATA.write_ranges` (manifest/IDL publication).
//!
//! These tests assert declared-vs-published-vs-enforced consistency: the
//! published ranges byte-match the declared field offsets/sizes AND the
//! ranges the runtime `WritePolicy` actually enforces after `bind()`.

#![cfg(feature = "proc-macros")]

use hopper::hopper_schema::accounts::ContextDescriptor;
use hopper::hopper_schema::codama::ManifestJson;
use hopper::hopper_schema::{AccountEntry, InstructionDescriptor, ProgramManifest};
use hopper::layout::{write_header, HEADER_LEN};
use hopper::prelude::*;
use hopper_runtime::write_policy::WriteRange;
use hopper_svm::{AccountFixture, HopperSvm};

// ── Program under test ─────────────────────────────────────────────

#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 42, version = 1)]
pub struct Ledger {
    pub balance: WireU64,
    pub pending_rewards: WireU64,
}

/// Strict-writes context: only `ledger.balance` is declared mutable, so
/// the write surface is exactly one 8-byte range at
/// `HEADER_LEN + Ledger::BALANCE_OFFSET` on account 0.
#[hopper::context(strict_writes)]
pub struct Credit {
    #[account(mut(balance))]
    pub ledger: Ledger,

    #[signer]
    pub authority: AccountView<'static>,
}

/// Control: a non-strict context publishes an empty, no-authority set.
#[hopper::context]
pub struct Inspect {
    pub ledger: Ledger,
}

fn expected_ranges() -> [WriteRange; 1] {
    [WriteRange::new(
        0,
        HEADER_LEN as u32 + Ledger::BALANCE_OFFSET,
        core::mem::size_of::<WireU64>() as u32,
    )]
}

// ── Declared vs published ──────────────────────────────────────────

#[test]
fn published_write_ranges_byte_match_declared_field_offsets() {
    assert!(Credit::STRICT_WRITES);
    assert_eq!(Credit::WRITE_RANGES, &expected_ranges());

    // The schema metadata publishes the SAME set (same shared const),
    // so a manifest consumer sees exactly the declared byte surface.
    let meta: ContextDescriptor = Credit::SCHEMA_METADATA;
    assert!(meta.strict_writes);
    assert_eq!(meta.write_ranges, Credit::WRITE_RANGES);
    assert_eq!(meta.write_ranges, &expected_ranges());

    // Sanity: the range really is the declared field, not a recompute.
    assert_eq!(meta.write_ranges[0].offset, 16); // HEADER_LEN + offset 0
    assert_eq!(meta.write_ranges[0].size, 8); // size_of::<WireU64>()
    assert_eq!(
        HEADER_LEN as u32 + Ledger::PENDING_REWARDS_OFFSET,
        24,
        "undeclared sibling field lies outside the published range"
    );
}

#[test]
fn non_strict_context_publishes_empty_no_authority_set() {
    assert!(!Inspect::STRICT_WRITES);
    assert!(Inspect::WRITE_RANGES.is_empty());
    assert!(!Inspect::SCHEMA_METADATA.strict_writes);
    assert!(Inspect::SCHEMA_METADATA.write_ranges.is_empty());
}

// ── Published vs runtime-enforced ──────────────────────────────────

fn credit_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    let mut ctx = Context::new(program_id, accounts, instruction_data);
    {
        // Bind installs the strict-writes WritePolicy on the raw context.
        let _bound = Credit::bind(&mut ctx)?;
    }

    // The installed policy's ranges are the very slice the schema
    // metadata publishes: declared == published == enforced.
    let policy = ctx
        .write_policy()
        .expect("strict_writes bind() must install a WritePolicy");
    assert_eq!(policy.allows, Credit::SCHEMA_METADATA.write_ranges);
    assert_eq!(policy.allows, Credit::WRITE_RANGES);

    // Behavioral proof the published ranges are what is enforced: the
    // declared `balance` range is writable, the undeclared sibling
    // `pending_rewards` range is refused at acquisition time.
    let balance_off = HEADER_LEN as u32 + Ledger::BALANCE_OFFSET;
    let pending_off = HEADER_LEN as u32 + Ledger::PENDING_REWARDS_OFFSET;
    assert!(ctx.segment_mut::<[u8; 8]>(0, balance_off).is_ok());
    assert!(ctx.segment_mut::<[u8; 8]>(0, pending_off).is_err());
    Ok(())
}

#[test]
fn runtime_write_policy_enforces_exactly_the_published_ranges() {
    let program_id = Address::new_from_array([9u8; 32]);

    let mut ledger_data = vec![0u8; Ledger::LEN];
    write_header(
        &mut ledger_data,
        Ledger::DISC,
        Ledger::VERSION,
        &Ledger::LAYOUT_ID,
    )
    .unwrap();
    let ledger = AccountFixture::with_data(
        Address::new_from_array([1u8; 32]),
        program_id,
        1_000_000,
        ledger_data,
    )
    .writable();
    let authority = AccountFixture::new(
        Address::new_from_array([2u8; 32]),
        Address::new_from_array([0u8; 32]),
        1_000_000,
        0,
    )
    .signer();

    let result =
        HopperSvm::new().process_instruction(program_id, &[], &[ledger, authority], credit_handler);
    assert!(
        result.program_result.is_ok(),
        "bind + policy inspection failed: {:?}",
        result.program_result
    );
}

// ── Manifest publication (zero manual authoring) ───────────────────

#[test]
fn manifest_json_publishes_the_context_write_set() {
    // The instruction descriptor is wired straight from the macro's
    // generated consts. no hand-authored offsets anywhere.
    static CREDIT_ACCOUNTS: [AccountEntry; 2] = [
        AccountEntry {
            name: "ledger",
            writable: true,
            signer: false,
            layout_ref: "Ledger",
            seeds: &[],
        },
        AccountEntry {
            name: "authority",
            writable: false,
            signer: true,
            layout_ref: "",
            seeds: &[],
        },
    ];
    static CREDIT_IX: InstructionDescriptor = InstructionDescriptor {
        name: "credit",
        tag: 0,
        args: &[],
        accounts: &CREDIT_ACCOUNTS,
        capabilities: &[],
        policy_pack: "",
        receipt_expected: false,
        strict_writes: Credit::STRICT_WRITES,
        write_ranges: Credit::WRITE_RANGES,
        cu_estimate: 0,
    };
    static INSTRUCTIONS: [InstructionDescriptor; 1] = [CREDIT_IX];
    static CONTEXTS: [ContextDescriptor; 1] = [Credit::SCHEMA_METADATA];
    let manifest = ProgramManifest {
        name: "strict_writes_it",
        version: "0.0.1",
        description: "BLD-I24 declared-vs-published consistency fixture",
        layouts: &[],
        layout_metadata: &[],
        instructions: &INSTRUCTIONS,
        events: &[],
        policies: &[],
        compatibility_pairs: &[],
        tooling_hints: &[],
        contexts: &CONTEXTS,
    };

    let json = format!("{}", ManifestJson(&manifest));
    assert!(json.contains("\"strictWrites\": true"));
    assert!(json.contains("\"account\": \"ledger\""));
    let expected = expected_ranges()[0];
    assert!(json.contains(&format!(
        "\"accountIndex\": {}, \"offset\": {}, \"size\": {}",
        expected.account_index, expected.offset, expected.size
    )));
}
