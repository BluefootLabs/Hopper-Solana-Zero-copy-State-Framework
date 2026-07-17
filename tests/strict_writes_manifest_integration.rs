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

/// BLD-MUT: a mutation-complete context — both dimensions declared.
/// `ledger` may write only its `balance` field (data dimension);
/// `fee_sink` is a declared lamport credit target (lamport dimension);
/// `config` and `authority` may be mutated in NEITHER dimension.
#[hopper::context(strict_writes, lamports(fee_sink))]
pub struct Payout {
    #[account(mut(balance))]
    pub ledger: Ledger,

    pub fee_sink: AccountView<'static>,

    pub config: AccountView<'static>,

    #[signer]
    pub authority: AccountView<'static>,
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
        remaining_accounts: None,
        capabilities: &[],
        policy_pack: "",
        receipt_expected: false,
        strict_writes: Credit::STRICT_WRITES,
        write_ranges: Credit::WRITE_RANGES,
        parametric_write_ranges: Credit::PARAMETRIC_WRITE_RANGES,
        mutation_complete: Credit::MUTATION_COMPLETE,
        lamport_accounts: Credit::LAMPORT_ACCOUNTS,
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
    // Credit is strict but declared no lamport dimension: the manifest
    // must NOT claim mutation completeness.
    assert!(!json.contains("mutationComplete"));
}

// ── BLD-MUT: lamport dimension + mutation completeness ─────────────

static PAYOUT_ACCOUNTS: [AccountEntry; 4] = [
    AccountEntry {
        name: "ledger",
        writable: true,
        signer: false,
        layout_ref: "Ledger",
        seeds: &[],
    },
    AccountEntry {
        name: "fee_sink",
        writable: true,
        signer: false,
        layout_ref: "",
        seeds: &[],
    },
    // Over-declared by the client: the context mutates config in
    // neither dimension, so demotion may trim exactly this flag.
    AccountEntry {
        name: "config",
        writable: true,
        signer: false,
        layout_ref: "",
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

static PAYOUT_IX: InstructionDescriptor = InstructionDescriptor {
    name: "payout",
    tag: 1,
    args: &[],
    accounts: &PAYOUT_ACCOUNTS,
    remaining_accounts: None,
    capabilities: &[],
    policy_pack: "",
    receipt_expected: false,
    strict_writes: Payout::STRICT_WRITES,
    write_ranges: Payout::WRITE_RANGES,
    parametric_write_ranges: Payout::PARAMETRIC_WRITE_RANGES,
    mutation_complete: Payout::MUTATION_COMPLETE,
    lamport_accounts: Payout::LAMPORT_ACCOUNTS,
    cu_estimate: 0,
};

#[test]
fn lamports_context_publishes_mutation_complete_consts() {
    // Both dimensions declared: complete. The lamport set is exactly
    // the explicit `lamports(fee_sink)` (index 1); the `mut(balance)`
    // field is field-granular, not whole-account, so it carries no
    // implied lamport permission.
    assert!(Payout::STRICT_WRITES);
    assert!(Payout::MUTATION_COMPLETE);
    assert_eq!(Payout::LAMPORT_ACCOUNTS, &[1u8]);
    assert!(Payout::SCHEMA_METADATA.mutation_complete);
    assert_eq!(Payout::SCHEMA_METADATA.lamport_accounts, &[1u8]);

    // A bare strict_writes context stays honest: data dimension only,
    // lamports undeclared, NOT mutation-complete.
    assert!(Credit::STRICT_WRITES);
    assert!(!Credit::MUTATION_COMPLETE);
    assert!(Credit::LAMPORT_ACCOUNTS.is_empty());
    assert!(!Credit::SCHEMA_METADATA.mutation_complete);

    // Non-strict: nothing declared, nothing claimed.
    assert!(!Inspect::MUTATION_COMPLETE);
    assert!(!Inspect::SCHEMA_METADATA.mutation_complete);
}

#[test]
fn manifest_json_publishes_mutation_complete_for_lamports_context() {
    static INSTRUCTIONS: [InstructionDescriptor; 1] = [PAYOUT_IX];
    static CONTEXTS: [ContextDescriptor; 1] = [Payout::SCHEMA_METADATA];
    let manifest = ProgramManifest {
        name: "bld_mut_it",
        version: "0.0.1",
        description: "BLD-MUT mutation-complete publication fixture",
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
    assert!(json.contains("\"mutationComplete\": true"));
    assert!(json.contains("\"lamportAccounts\": [1]"));
}

#[test]
fn effective_writable_demotes_exactly_the_untouched_account() {
    // ledger: data range declared — stays writable.
    assert!(PAYOUT_IX.effective_writable(0, true));
    // fee_sink: lamport permission, zero data ranges — stays writable
    // (demoting it would break the declared lamport credit on chain).
    assert!(PAYOUT_IX.effective_writable(1, true));
    // config: neither dimension — the over-declared flag is demoted.
    assert!(!PAYOUT_IX.effective_writable(2, true));
    // authority: read-only is never promoted.
    assert!(!PAYOUT_IX.effective_writable(3, false));

    // The same account list under the INCOMPLETE Credit-style contract
    // must pass through unchanged: no demotion without both dimensions.
    static INCOMPLETE_IX: InstructionDescriptor = InstructionDescriptor {
        mutation_complete: false,
        lamport_accounts: &[],
        ..PAYOUT_IX
    };
    assert!(INCOMPLETE_IX.effective_writable(2, true));
}

fn payout_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    use hopper_runtime::write_policy::write_policy_violation;

    let mut ctx = Context::new(program_id, accounts, instruction_data);
    let bound = Payout::bind(&mut ctx)?;

    // While the bound context (and therefore the lamport gate) is live:
    // the declared lamport target accepts a credit...
    let sink_before = accounts[1].lamports();
    accounts[1].try_set_lamports(sink_before + 7)?;
    assert_eq!(accounts[1].lamports(), sink_before + 7);

    // ...and every undeclared account's lamport mutation is refused
    // with the indexed policy error, balances untouched.
    let ledger_before = accounts[0].lamports();
    assert_eq!(
        accounts[0].try_set_lamports(0),
        Err(write_policy_violation(0))
    );
    assert_eq!(accounts[0].lamports(), ledger_before);
    assert_eq!(
        accounts[2].try_set_lamports(0),
        Err(write_policy_violation(2))
    );
    assert_eq!(
        accounts[3].try_set_lamports(0),
        Err(write_policy_violation(3))
    );

    // Dropping the bound scope uninstalls the gate: lamport access
    // returns to the ungoverned (pre-BLD-MUT) contract.
    drop(bound);
    accounts[2].try_set_lamports(accounts[2].lamports())?;
    Ok(())
}

#[test]
fn runtime_gate_refuses_undeclared_lamport_mutation_for_bound_scope() {
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
    let fee_sink = AccountFixture::new(
        Address::new_from_array([2u8; 32]),
        Address::new_from_array([0u8; 32]),
        1_000_000,
        0,
    )
    .writable();
    let config = AccountFixture::new(
        Address::new_from_array([3u8; 32]),
        Address::new_from_array([0u8; 32]),
        1_000_000,
        0,
    )
    .writable();
    let authority = AccountFixture::new(
        Address::new_from_array([4u8; 32]),
        Address::new_from_array([0u8; 32]),
        1_000_000,
        0,
    )
    .signer();

    let result = HopperSvm::new().process_instruction(
        program_id,
        &[],
        &[ledger, fee_sink, config, authority],
        payout_handler,
    );
    assert!(
        result.program_result.is_ok(),
        "bound-scope lamport gate behavior failed: {:?}",
        result.program_result
    );
}

// ── BLD-MUT: `sweep = target` end-to-end under the gate ─────────────
//
// The generated sweep helper drains through the runtime lamport funnel
// (`try_set_lamports`), so a mutation-complete context must imply both
// sweep roles or its own helper would be refused by its own gate.
// (This surface previously could not compile at all — the emission
// called a nonexistent `try_borrow_mut_lamports` — so this test is the
// first executable proof of the sweep contract.)

/// `lamports()` (empty explicit list) is valid: ONLY the implied
/// lifecycle roles — here the sweep source + target — may move
/// lamports.
#[hopper::context(strict_writes, lamports())]
pub struct SweepFees {
    #[account(sweep = treasury)]
    pub fees: AccountView<'static>,

    pub treasury: AccountView<'static>,

    #[signer]
    pub authority: AccountView<'static>,
}

#[test]
fn sweep_context_implies_exactly_the_sweep_roles() {
    assert!(SweepFees::STRICT_WRITES);
    assert!(SweepFees::MUTATION_COMPLETE);
    // fees (0): sweep source (sweep implies `mut`); treasury (1):
    // sweep target. authority (2) gets nothing.
    assert_eq!(SweepFees::LAMPORT_ACCOUNTS, &[0u8, 1u8]);
    assert_eq!(SweepFees::SCHEMA_METADATA.lamport_accounts, &[0u8, 1u8]);
}

fn sweep_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    use hopper_runtime::write_policy::write_policy_violation;

    let mut ctx = Context::new(program_id, accounts, instruction_data);
    let bound = SweepFees::bind(&mut ctx)?;

    let fees_before = accounts[0].lamports();
    let treasury_before = accounts[1].lamports();
    assert!(fees_before > 0, "fixture must fund the swept account");

    // The gate is live (lamports() declared): the swept transfer is
    // admitted because source + target are the implied permission set.
    let swept = bound.sweep_fees()?;
    assert_eq!(swept, fees_before);
    assert_eq!(accounts[0].lamports(), 0);
    assert_eq!(accounts[1].lamports(), treasury_before + fees_before);

    // Sweeping an already-empty slot is a no-op returning 0.
    assert_eq!(bound.sweep_fees()?, 0);

    // The sweep succeeded because of the implication, not because the
    // gate is loose: an account outside the implied set stays refused.
    assert_eq!(
        accounts[2].try_set_lamports(0),
        Err(write_policy_violation(2))
    );
    Ok(())
}

#[test]
fn sweep_helper_executes_end_to_end_under_the_lamport_gate() {
    let program_id = Address::new_from_array([9u8; 32]);

    let fees =
        AccountFixture::new(Address::new_from_array([5u8; 32]), program_id, 250_000, 0).writable();
    let treasury = AccountFixture::new(
        Address::new_from_array([6u8; 32]),
        Address::new_from_array([0u8; 32]),
        1_000_000,
        0,
    )
    .writable();
    let authority = AccountFixture::new(
        Address::new_from_array([7u8; 32]),
        Address::new_from_array([0u8; 32]),
        1_000_000,
        0,
    )
    .signer();

    let result = HopperSvm::new().process_instruction(
        program_id,
        &[],
        &[fees, treasury, authority],
        sweep_handler,
    );
    assert!(
        result.program_result.is_ok(),
        "sweep under the lamport gate failed: {:?}",
        result.program_result
    );
}

// ── BLD-MUT: the gated transfer on the bound context ────────────────
//
// A mutation-complete context exposes `ctx.transfer_lamports(..)`,
// delegating to `hopper_runtime::transfer_lamports` — the substrate
// helper's arithmetic run through the gated funnel. These tests prove
// the generated method end-to-end: declared→declared moves exact
// balances; a transfer touching an undeclared account is refused with
// the indexed policy error BEFORE any balance changes, in both
// orderings; and the free function is reachable via the prelude.

#[hopper::context(strict_writes, lamports(payer_slot, fee_sink))]
pub struct Toll {
    pub payer_slot: AccountView<'static>,

    pub fee_sink: AccountView<'static>,

    pub config: AccountView<'static>,

    #[signer]
    pub authority: AccountView<'static>,
}

#[test]
fn toll_context_declares_exactly_the_transfer_roles() {
    assert!(Toll::STRICT_WRITES);
    assert!(Toll::MUTATION_COMPLETE);
    assert_eq!(Toll::LAMPORT_ACCOUNTS, &[0u8, 1u8]);
}

fn toll_handler<'info>(
    program_id: &'info Address,
    accounts: &'info [AccountView<'info>],
    instruction_data: &'info [u8],
) -> ProgramResult {
    use hopper_runtime::write_policy::write_policy_violation;

    let mut ctx = Context::new(program_id, accounts, instruction_data);
    let bound = Toll::bind(&mut ctx)?;

    let payer_before = accounts[0].lamports();
    let sink_before = accounts[1].lamports();
    let config_before = accounts[2].lamports();

    // Declared -> declared: admitted by the gate, balances move exactly.
    bound.transfer_lamports(
        bound.payer_slot_account()?,
        bound.fee_sink_account()?,
        40_000,
    )?;
    assert_eq!(accounts[0].lamports(), payer_before - 40_000);
    assert_eq!(accounts[1].lamports(), sink_before + 40_000);

    // From-undeclared: refused with the indexed policy error before any
    // balance changes.
    assert_eq!(
        bound.transfer_lamports(bound.config_account()?, bound.fee_sink_account()?, 1),
        Err(write_policy_violation(2))
    );
    // To-undeclared: same refusal shape — and crucially the DECLARED
    // debit side must not have been half-applied.
    assert_eq!(
        bound.transfer_lamports(bound.payer_slot_account()?, bound.config_account()?, 1),
        Err(write_policy_violation(2))
    );
    assert_eq!(accounts[0].lamports(), payer_before - 40_000);
    assert_eq!(accounts[1].lamports(), sink_before + 40_000);
    assert_eq!(accounts[2].lamports(), config_before);

    // The prelude free function is the same gated funnel (item 4:
    // `use hopper::prelude::*` reaches it).
    transfer_lamports(&accounts[1], &accounts[0], 40_000)?;
    assert_eq!(accounts[0].lamports(), payer_before);
    assert_eq!(accounts[1].lamports(), sink_before);

    // Gated self-transfer on a declared account: balance-checked net
    // zero.
    bound.transfer_lamports(
        bound.payer_slot_account()?,
        bound.payer_slot_account()?,
        payer_before,
    )?;
    assert_eq!(accounts[0].lamports(), payer_before);

    Ok(())
}

#[test]
fn bound_context_transfer_lamports_is_gated_end_to_end() {
    let program_id = Address::new_from_array([9u8; 32]);

    let payer_slot =
        AccountFixture::new(Address::new_from_array([10u8; 32]), program_id, 500_000, 0).writable();
    let fee_sink = AccountFixture::new(
        Address::new_from_array([11u8; 32]),
        Address::new_from_array([0u8; 32]),
        1_000_000,
        0,
    )
    .writable();
    let config = AccountFixture::new(
        Address::new_from_array([12u8; 32]),
        Address::new_from_array([0u8; 32]),
        1_000_000,
        0,
    )
    .writable();
    let authority = AccountFixture::new(
        Address::new_from_array([13u8; 32]),
        Address::new_from_array([0u8; 32]),
        1_000_000,
        0,
    )
    .signer();

    let result = HopperSvm::new().process_instruction(
        program_id,
        &[],
        &[payer_slot, fee_sink, config, authority],
        toll_handler,
    );
    assert!(
        result.program_result.is_ok(),
        "gated bound-context transfer failed: {:?}",
        result.program_result
    );
}
