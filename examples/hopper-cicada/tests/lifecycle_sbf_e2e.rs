//! Compiled Cicada lifecycle and adversarial-route coverage.
//!
//! Build both ELFs first with two commands:
//! `cargo build-sbf -- -p hopper-cicada` and
//! `cargo build-sbf -- -p hopper-cicada-route-fixture`.
//! The tests skip when those artifacts are absent, matching Hopper's other
//! compiled-SBF suites.

use std::collections::BTreeMap;

use hopper_cicada::{
    CicadaConfig, ClaimStillActive, DestinationTokenPolicyChanged, EmptySettlement, IntentShard,
    ProtectedAccountDelegation, CONFIG_SEED, INTENTS_PER_SHARD, MAX_ROUTE_ACCOUNTS,
    SOURCE_LEASE_SEED, STATUS_CANCELLED, STATUS_CLAIMED, STATUS_OPEN, STATUS_SETTLED,
    VAULT_AUTHORITY_SEED,
};
use hopper_test::{HarnessResult, LiteSvmHarness};
use solana_account::Account;
use solana_instruction::{error::InstructionError, AccountMeta, Instruction};
use solana_pubkey::Pubkey;

const CICADA_ELF: &str = "../../target/deploy/hopper_cicada";
const ROUTE_ELF: &str = "../../target/deploy/hopper_cicada_route_fixture";
const ROUTE_HONEST: u8 = 0xA0;
const ROUTE_MUTATE_POLICY: u8 = 0xA1;
const ROUTE_SPOOF_OUTPUT: u8 = 0xA2;

type Bank = BTreeMap<Pubkey, Account>;

struct Fixture {
    svm: LiteSvmHarness,
    bank: Bank,
    program_id: Pubkey,
    token_program: Pubkey,
    owner: Pubkey,
    executor: Pubkey,
    config: Pubkey,
    shard: Pubkey,
    source: Pubkey,
    vault: Pubkey,
    refund: Pubkey,
    destination: Pubkey,
    input_mint: Pubkey,
    output_mint: Pubkey,
    source_lease: Pubkey,
}

fn harness() -> Option<(LiteSvmHarness, Pubkey, Pubkey)> {
    let program_id = Pubkey::new_unique();
    let mut svm = LiteSvmHarness::load(&program_id, CICADA_ELF)?;
    let token_program =
        Pubkey::new_from_array(*hopper::hopper_runtime::token::TOKEN_PROGRAM_ID.as_array());
    if !svm.add_program(&token_program, ROUTE_ELF) {
        eprintln!("SKIPPED: {ROUTE_ELF}.so not found");
        return None;
    }
    Some((svm, program_id, token_program))
}

fn system_account() -> Account {
    Account::new(0, 0, &Pubkey::default())
}

fn mint_account(token_program: &Pubkey, decimals: u8) -> Account {
    let mut data = vec![0u8; 82];
    data[44] = decimals;
    data[45] = 1;
    Account {
        lamports: 10_000_000,
        data,
        owner: *token_program,
        executable: false,
        rent_epoch: 0,
    }
}

fn token_account(token_program: &Pubkey, mint: &Pubkey, owner: &Pubkey, amount: u64) -> Account {
    let mut data = vec![0u8; 165];
    data[..32].copy_from_slice(&mint.to_bytes());
    data[32..64].copy_from_slice(&owner.to_bytes());
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data[108] = 1; // AccountState::Initialized
    Account {
        lamports: 10_000_000,
        data,
        owner: *token_program,
        executable: false,
        rent_epoch: 0,
    }
}

fn token_amount(account: &Account) -> u64 {
    u64::from_le_bytes(account.data[64..72].try_into().unwrap())
}

fn token_authority(account: &Account) -> Pubkey {
    Pubkey::new_from_array(account.data[32..64].try_into().unwrap())
}

fn account_bytes<'a>(account: &'a Account, offset: u32, size: usize) -> &'a [u8] {
    &account.data[offset as usize..offset as usize + size]
}

fn account_u64(account: &Account, offset: u32) -> u64 {
    u64::from_le_bytes(account_bytes(account, offset, 8).try_into().unwrap())
}

fn instruction_snapshot(fixture: &Fixture, instruction: &Instruction) -> Bank {
    instruction
        .accounts
        .iter()
        .map(|meta| {
            (
                meta.pubkey,
                fixture
                    .bank
                    .get(&meta.pubkey)
                    .unwrap_or_else(|| panic!("missing fixture account {}", meta.pubkey))
                    .clone(),
            )
        })
        .collect()
}

/// A failed Solana instruction is atomic across the entire account envelope.
/// Assert against Mollusk's returned accounts, not only `Fixture::bank`: the
/// latter deliberately commits results only on success and would otherwise
/// make a rollback assertion tautological.
fn assert_instruction_rolled_back(result: &HarnessResult, before: &Bank) {
    assert!(
        !result.succeeded(),
        "rollback assertion needs a failed result"
    );
    for (key, expected) in before {
        let actual = result
            .raw()
            .get_account(key)
            .unwrap_or_else(|| panic!("result omitted instruction account {key}"));
        assert_eq!(actual, expected, "failed instruction changed account {key}");
    }
}

fn assert_custom_error(result: &HarnessResult, code: u32) {
    assert_eq!(
        result.raw().raw_result,
        Err(InstructionError::Custom(code)),
        "unexpected compiled-program refusal",
    );
}

fn process(fixture: &mut Fixture, instruction: &Instruction) -> HarnessResult {
    let mut seeds = Vec::new();
    for meta in &instruction.accounts {
        if seeds
            .iter()
            .any(|(key, _): &(Pubkey, Account)| key == &meta.pubkey)
        {
            continue;
        }
        seeds.push((
            meta.pubkey,
            fixture
                .bank
                .get(&meta.pubkey)
                .unwrap_or_else(|| panic!("missing fixture account {}", meta.pubkey))
                .clone(),
        ));
    }
    let result = fixture.svm.process(instruction, &seeds);
    if result.succeeded() {
        for (key, account) in &result.raw().resulting_accounts {
            fixture.bank.insert(*key, account.clone());
        }
    }
    result
}

fn ix(program_id: Pubkey, tag: u8, data: &[u8], accounts: Vec<AccountMeta>) -> Instruction {
    let mut bytes = Vec::with_capacity(1 + data.len());
    bytes.push(tag);
    bytes.extend_from_slice(data);
    Instruction::new_with_bytes(program_id, &bytes, accounts)
}

fn setup_open_intent() -> Option<Fixture> {
    let (svm, program_id, token_program) = harness()?;
    let owner = Pubkey::new_unique();
    let executor = Pubkey::new_unique();
    let shard = Pubkey::new_unique();
    let source = Pubkey::new_unique();
    let refund = Pubkey::new_unique();
    let destination = Pubkey::new_unique();
    let input_mint = Pubkey::new_unique();
    let output_mint = Pubkey::new_unique();
    let (config, _) = Pubkey::find_program_address(&[CONFIG_SEED], &program_id);
    let (vault, _) = Pubkey::find_program_address(
        &[VAULT_AUTHORITY_SEED, owner.as_ref(), source.as_ref()],
        &program_id,
    );
    let (source_lease, _) =
        Pubkey::find_program_address(&[SOURCE_LEASE_SEED, source.as_ref()], &program_id);

    let mut bank = Bank::new();
    bank.insert(owner, Account::new(50_000_000_000, 0, &Pubkey::default()));
    bank.insert(executor, Account::new(5_000_000_000, 0, &Pubkey::default()));
    bank.insert(config, system_account());
    bank.insert(shard, system_account());
    bank.insert(source_lease, system_account());
    bank.insert(vault, system_account());
    bank.insert(input_mint, mint_account(&token_program, 6));
    bank.insert(output_mint, mint_account(&token_program, 6));
    bank.insert(
        source,
        token_account(&token_program, &input_mint, &vault, 100),
    );
    bank.insert(
        refund,
        token_account(&token_program, &input_mint, &owner, 0),
    );
    bank.insert(
        destination,
        token_account(&token_program, &output_mint, &owner, 0),
    );
    bank.insert(
        Pubkey::default(),
        LiteSvmHarness::system_program_account().1,
    );
    bank.insert(
        token_program,
        LiteSvmHarness::executable_program_account(&token_program).1,
    );

    let mut fixture = Fixture {
        svm,
        bank,
        program_id,
        token_program,
        owner,
        executor,
        config,
        shard,
        source,
        vault,
        refund,
        destination,
        input_mint,
        output_mint,
        source_lease,
    };
    fixture.svm.capture_logs();

    let mut init_config = Vec::new();
    init_config.extend_from_slice(owner.as_ref());
    init_config.extend_from_slice(&10u64.to_le_bytes());
    let instruction = ix(
        program_id,
        0,
        &init_config,
        vec![
            AccountMeta::new(owner, true),
            AccountMeta::new(config, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
        ],
    );
    let result = process(&mut fixture, &instruction);
    assert!(
        result.succeeded(),
        "initialize_config failed: {:#?}",
        fixture.svm.logs(),
    );

    let instruction = ix(
        program_id,
        1,
        &7u32.to_le_bytes(),
        vec![
            AccountMeta::new(owner, true),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new(shard, true),
            AccountMeta::new_readonly(Pubkey::default(), false),
        ],
    );
    let result = process(&mut fixture, &instruction);
    assert!(
        result.succeeded(),
        "initialize_shard failed: {:#?}",
        fixture.svm.logs(),
    );

    let mut create = Vec::new();
    create.extend_from_slice(&100u64.to_le_bytes());
    create.extend_from_slice(&90u64.to_le_bytes());
    create.extend_from_slice(&1_000_000u64.to_le_bytes());
    create.extend_from_slice(executor.as_ref());
    create.push(1); // ROUTE_MODE_PROGRAM
    create.extend_from_slice(&[0u8; 32]);
    let instruction = ix(
        program_id,
        2,
        &create,
        vec![
            AccountMeta::new(owner, true),
            AccountMeta::new_readonly(config, false),
            AccountMeta::new(shard, false),
            AccountMeta::new_readonly(source, false),
            AccountMeta::new_readonly(vault, false),
            AccountMeta::new_readonly(refund, false),
            AccountMeta::new_readonly(destination, false),
            AccountMeta::new_readonly(input_mint, false),
            AccountMeta::new_readonly(output_mint, false),
            AccountMeta::new_readonly(token_program, false),
            AccountMeta::new(source_lease, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
        ],
    );
    let result = process(&mut fixture, &instruction);
    assert!(
        result.succeeded(),
        "create_intent failed: {:#?}",
        fixture.svm.logs(),
    );
    assert_eq!(
        fixture.bank[&shard].data[IntentShard::STATUSES_ABS_OFFSET as usize],
        STATUS_OPEN
    );
    Some(fixture)
}

fn claim_ix(f: &Fixture) -> Instruction {
    let mut args = Vec::new();
    args.extend_from_slice(&0u16.to_le_bytes());
    args.extend_from_slice(&5u64.to_le_bytes());
    ix(
        f.program_id,
        3,
        &args,
        vec![
            AccountMeta::new_readonly(f.executor, true),
            AccountMeta::new_readonly(f.config, false),
            AccountMeta::new(f.shard, false),
        ],
    )
}

fn execute_ix(f: &Fixture, command: u8) -> Instruction {
    let mut route_data = vec![command];
    route_data.extend_from_slice(&60u64.to_le_bytes());
    route_data.extend_from_slice(&95u64.to_le_bytes());
    let mut args = Vec::new();
    args.extend_from_slice(&0u16.to_le_bytes());
    args.extend_from_slice(&(route_data.len() as u16).to_le_bytes());
    args.extend_from_slice(&route_data);
    let mut flags = [0u8; MAX_ROUTE_ACCOUNTS];
    flags[0] = 1;
    flags[1] = 1;
    flags[2] = 2;
    args.extend_from_slice(&flags);
    ix(
        f.program_id,
        6,
        &args,
        vec![
            AccountMeta::new_readonly(f.executor, true),
            AccountMeta::new_readonly(f.config, false),
            AccountMeta::new(f.shard, false),
            AccountMeta::new_readonly(f.owner, false),
            AccountMeta::new(f.source, false),
            AccountMeta::new_readonly(f.vault, false),
            AccountMeta::new(f.refund, false),
            AccountMeta::new(f.destination, false),
            AccountMeta::new_readonly(f.input_mint, false),
            AccountMeta::new_readonly(f.output_mint, false),
            AccountMeta::new_readonly(f.token_program, false),
            AccountMeta::new_readonly(f.token_program, false),
            AccountMeta::new(f.source, false),
            AccountMeta::new(f.destination, false),
            AccountMeta::new_readonly(f.vault, false),
        ],
    )
}

fn release_ix(f: &Fixture) -> Instruction {
    ix(
        f.program_id,
        4,
        &0u16.to_le_bytes(),
        vec![
            AccountMeta::new_readonly(f.config, false),
            AccountMeta::new(f.shard, false),
        ],
    )
}

fn cancel_ix(f: &Fixture) -> Instruction {
    ix(
        f.program_id,
        5,
        &0u16.to_le_bytes(),
        vec![
            AccountMeta::new_readonly(f.owner, true),
            AccountMeta::new_readonly(f.config, false),
            AccountMeta::new(f.shard, false),
            AccountMeta::new(f.source, false),
            AccountMeta::new_readonly(f.vault, false),
            AccountMeta::new(f.refund, false),
            AccountMeta::new_readonly(f.input_mint, false),
            AccountMeta::new_readonly(f.token_program, false),
        ],
    )
}

fn reclaim_ix(f: &Fixture) -> Instruction {
    ix(
        f.program_id,
        7,
        &0u16.to_le_bytes(),
        vec![
            AccountMeta::new(f.owner, true),
            AccountMeta::new_readonly(f.config, false),
            AccountMeta::new(f.shard, false),
            AccountMeta::new(f.source, false),
            AccountMeta::new_readonly(f.vault, false),
            AccountMeta::new_readonly(f.token_program, false),
            AccountMeta::new(f.source_lease, false),
        ],
    )
}

fn assert_slot_zeroed(shard: &Account) {
    for (offset, size) in [
        (IntentShard::OWNERS_ABS_OFFSET, 32),
        (IntentShard::SOURCE_TOKENS_ABS_OFFSET, 32),
        (IntentShard::VAULT_AUTHORITIES_ABS_OFFSET, 32),
        (IntentShard::REFUND_TOKENS_ABS_OFFSET, 32),
        (IntentShard::DESTINATION_TOKENS_ABS_OFFSET, 32),
        (IntentShard::INPUT_MINTS_ABS_OFFSET, 32),
        (IntentShard::OUTPUT_MINTS_ABS_OFFSET, 32),
        (IntentShard::MAX_INPUTS_ABS_OFFSET, 8),
        (IntentShard::MIN_OUTPUTS_ABS_OFFSET, 8),
        (IntentShard::EXPIRIES_ABS_OFFSET, 8),
        (IntentShard::ALLOWED_EXECUTORS_ABS_OFFSET, 32),
        (IntentShard::ROUTE_PROGRAMS_ABS_OFFSET, 32),
        (IntentShard::ROUTE_COMMITMENTS_ABS_OFFSET, 32),
        (IntentShard::ROUTE_MODES_ABS_OFFSET, 1),
        (IntentShard::SEQUENCES_ABS_OFFSET, 8),
        (IntentShard::STATUSES_ABS_OFFSET, 1),
        (IntentShard::CLAIMANTS_ABS_OFFSET, 32),
        (IntentShard::CLAIM_EXPIRIES_ABS_OFFSET, 8),
        (IntentShard::SETTLED_INPUTS_ABS_OFFSET, 8),
        (IntentShard::SETTLED_OUTPUTS_ABS_OFFSET, 8),
        (IntentShard::SETTLEMENT_HASHES_ABS_OFFSET, 32),
        (IntentShard::REVISIONS_ABS_OFFSET, 8),
    ] {
        assert!(
            account_bytes(shard, offset, size)
                .iter()
                .all(|byte| *byte == 0),
            "reclaim left slot-zero data at offset {offset}",
        );
    }
}

fn assert_reclaimed(f: &Fixture) {
    assert_eq!(token_authority(&f.bank[&f.source]), f.owner);
    assert_eq!(f.bank[&f.source_lease].lamports, 0);
    assert_eq!(f.bank[&f.source_lease].data[0], 0xFF); // CLOSE_SENTINEL
    assert!(f.bank[&f.source_lease].data[1..]
        .iter()
        .all(|byte| *byte == 0));
    assert_eq!(
        u16::from_le_bytes(
            account_bytes(&f.bank[&f.shard], IntentShard::OCCUPIED_COUNT_ABS_OFFSET, 2)
                .try_into()
                .unwrap(),
        ),
        0,
    );
    assert_eq!(
        u32::from_le_bytes(
            account_bytes(&f.bank[&f.shard], IntentShard::OCCUPIED_ABS_OFFSET, 4)
                .try_into()
                .unwrap(),
        ),
        0,
    );
    assert_slot_zeroed(&f.bank[&f.shard]);
}

#[test]
fn compiled_full_lifecycle_initializes_creates_claims_executes_and_reclaims() {
    let Some(mut f) = setup_open_intent() else {
        eprintln!("SKIPPED: build Cicada SBF artifacts first");
        return;
    };

    let claim = claim_ix(&f);
    assert!(process(&mut f, &claim).succeeded(), "claim failed");
    assert_eq!(
        f.bank[&f.shard].data[IntentShard::STATUSES_ABS_OFFSET as usize],
        STATUS_CLAIMED
    );
    assert_eq!(
        account_bytes(&f.bank[&f.shard], IntentShard::CLAIMANTS_ABS_OFFSET, 32),
        f.executor.as_ref(),
    );
    assert_eq!(
        account_u64(&f.bank[&f.shard], IntentShard::CLAIM_EXPIRIES_ABS_OFFSET),
        5,
    );
    assert_eq!(
        account_u64(&f.bank[&f.shard], IntentShard::REVISIONS_ABS_OFFSET),
        2,
    );

    let execute = execute_ix(&f, ROUTE_HONEST);
    let result = process(&mut f, &execute);
    assert!(result.succeeded(), "execute failed: {:#?}", f.svm.logs());
    assert_eq!(
        f.bank[&f.shard].data[IntentShard::STATUSES_ABS_OFFSET as usize],
        STATUS_SETTLED
    );
    assert_eq!(token_amount(&f.bank[&f.source]), 0);
    assert_eq!(token_amount(&f.bank[&f.refund]), 40);
    assert_eq!(token_amount(&f.bank[&f.destination]), 95);
    assert_eq!(
        account_u64(&f.bank[&f.shard], IntentShard::SETTLED_INPUTS_ABS_OFFSET),
        60,
    );
    assert_eq!(
        account_u64(&f.bank[&f.shard], IntentShard::SETTLED_OUTPUTS_ABS_OFFSET),
        95,
    );
    assert_eq!(
        account_u64(&f.bank[&f.shard], IntentShard::CLAIM_EXPIRIES_ABS_OFFSET),
        0,
    );
    assert_eq!(
        account_u64(&f.bank[&f.shard], IntentShard::REVISIONS_ABS_OFFSET),
        3,
    );
    assert!(account_bytes(
        &f.bank[&f.shard],
        IntentShard::SETTLEMENT_HASHES_ABS_OFFSET,
        32,
    )
    .iter()
    .any(|byte| *byte != 0));

    let reclaim = reclaim_ix(&f);
    let result = process(&mut f, &reclaim);
    assert!(result.succeeded(), "reclaim failed: {:#?}", f.svm.logs());
    assert_reclaimed(&f);
    assert_eq!(
        IntentShard::STATUSES_ELEMENT_COUNT as usize,
        INTENTS_PER_SHARD
    );
}

#[test]
fn compiled_claim_release_cancel_and_reclaim_lifecycle() {
    let Some(mut f) = setup_open_intent() else {
        eprintln!("SKIPPED: build Cicada SBF artifacts first");
        return;
    };

    let claim = claim_ix(&f);
    assert!(process(&mut f, &claim).succeeded(), "claim failed");
    assert_eq!(
        f.bank[&f.shard].data[IntentShard::STATUSES_ABS_OFFSET as usize],
        STATUS_CLAIMED,
    );

    // An active reservation cannot be cleared early. Prove both the typed
    // rejection and transaction-wide rollback from the compiled program.
    let release = release_ix(&f);
    let before_release = instruction_snapshot(&f, &release);
    let result = process(&mut f, &release);
    assert_custom_error(&result, ClaimStillActive::CODE);
    assert_instruction_rolled_back(&result, &before_release);

    // The claim was created at slot 0 with a five-slot lease. Hopper must use
    // the real Clock sysvar in the SBF frame before releasing it at slot 6.
    f.svm.mollusk_mut().warp_to_slot(6);
    let result = process(&mut f, &release);
    assert!(
        result.succeeded(),
        "expired release failed: {:#?}",
        f.svm.logs()
    );
    assert_eq!(
        f.bank[&f.shard].data[IntentShard::STATUSES_ABS_OFFSET as usize],
        STATUS_OPEN,
    );
    assert!(
        account_bytes(&f.bank[&f.shard], IntentShard::CLAIMANTS_ABS_OFFSET, 32)
            .iter()
            .all(|byte| *byte == 0)
    );
    assert_eq!(
        account_u64(&f.bank[&f.shard], IntentShard::CLAIM_EXPIRIES_ABS_OFFSET),
        0,
    );
    assert_eq!(
        account_u64(&f.bank[&f.shard], IntentShard::REVISIONS_ABS_OFFSET),
        3,
    );

    let cancel = cancel_ix(&f);
    let result = process(&mut f, &cancel);
    assert!(result.succeeded(), "cancel failed: {:#?}", f.svm.logs());
    assert_eq!(
        f.bank[&f.shard].data[IntentShard::STATUSES_ABS_OFFSET as usize],
        STATUS_CANCELLED,
    );
    assert_eq!(token_amount(&f.bank[&f.source]), 0);
    assert_eq!(token_amount(&f.bank[&f.refund]), 100);
    assert_eq!(token_amount(&f.bank[&f.destination]), 0);
    assert_eq!(
        account_u64(&f.bank[&f.shard], IntentShard::REVISIONS_ABS_OFFSET),
        4,
    );

    let owner_lamports = f.bank[&f.owner].lamports;
    let lease_lamports = f.bank[&f.source_lease].lamports;
    let reclaim = reclaim_ix(&f);
    let result = process(&mut f, &reclaim);
    assert!(
        result.succeeded(),
        "cancelled reclaim failed: {:#?}",
        f.svm.logs(),
    );
    assert_eq!(
        f.bank[&f.owner].lamports,
        owner_lamports + lease_lamports,
        "closing the source lease must refund its rent to the owner",
    );
    assert_reclaimed(&f);
}

#[test]
fn compiled_hostile_route_policy_mutation_is_rolled_back() {
    let Some(mut f) = setup_open_intent() else {
        eprintln!("SKIPPED: build Cicada SBF artifacts first");
        return;
    };
    f.svm.capture_logs();
    let execute = execute_ix(&f, ROUTE_MUTATE_POLICY);
    let before = instruction_snapshot(&f, &execute);
    let result = process(&mut f, &execute);
    let logs = f.svm.logs();
    assert_custom_error(&result, DestinationTokenPolicyChanged::CODE);
    assert!(
        logs.iter()
            .any(|line| line == &format!("Program {} invoke [2]", f.token_program)),
        "the hostile route must actually execute in a nested SBF frame: {logs:#?}",
    );
    assert_instruction_rolled_back(&result, &before);

    // A refused callee cannot poison the account bank or consume the intent.
    // The identical intent remains executable through an honest route.
    let honest = execute_ix(&f, ROUTE_HONEST);
    let result = process(&mut f, &honest);
    assert!(
        result.succeeded(),
        "honest retry failed: {:#?}",
        f.svm.logs()
    );
    assert_eq!(token_amount(&f.bank[&f.source]), 0);
    assert_eq!(token_amount(&f.bank[&f.refund]), 40);
    assert_eq!(token_amount(&f.bank[&f.destination]), 95);
    assert_eq!(
        f.bank[&f.shard].data[IntentShard::STATUSES_ABS_OFFSET as usize],
        STATUS_SETTLED,
    );
}

#[test]
fn compiled_hostile_route_cannot_spoof_output_without_spending_input() {
    let Some(mut f) = setup_open_intent() else {
        eprintln!("SKIPPED: build Cicada SBF artifacts first");
        return;
    };

    f.svm.capture_logs();
    let execute = execute_ix(&f, ROUTE_SPOOF_OUTPUT);
    let before = instruction_snapshot(&f, &execute);
    let result = process(&mut f, &execute);

    assert_custom_error(&result, EmptySettlement::CODE);
    assert_instruction_rolled_back(&result, &before);
}

#[test]
fn compiled_hostile_route_cannot_delegate_cicada_state() {
    let Some(mut f) = setup_open_intent() else {
        eprintln!("SKIPPED: build Cicada SBF artifacts first");
        return;
    };

    let mut execute = execute_ix(&f, ROUTE_HONEST);
    // Replace the route's first dynamic account with the shard itself while
    // retaining the writable route flag. The duplicate transaction meta is a
    // realistic account-smuggling attempt; Cicada must reject it before CPI.
    let first_remaining = execute.accounts.len() - 3;
    execute.accounts[first_remaining] = AccountMeta::new(f.shard, false);
    let before = instruction_snapshot(&f, &execute);
    let result = process(&mut f, &execute);

    assert_custom_error(&result, ProtectedAccountDelegation::CODE);
    assert_instruction_rolled_back(&result, &before);
}

/// `bump = stored` behavioral proof on compiled SBF: corrupting the
/// `#[bump]`-marked byte in `CicadaConfig` makes every stored-bump context
/// refuse to bind (the one-hash verify no longer derives the account's own
/// address), and restoring the byte heals the same instruction — pinning
/// the refusal to exactly the stored canonical bump.
#[test]
fn compiled_tampered_stored_bump_byte_refuses_the_bind() {
    let Some(mut f) = setup_open_intent() else {
        eprintln!("SKIPPED: build Cicada SBF artifacts first");
        return;
    };
    let bump_off = CicadaConfig::CANONICAL_BUMP_ABS_OFFSET as usize;

    // Tamper: the stored byte no longer derives the config address.
    let real = f.bank.get(&f.config).unwrap().data[bump_off];
    f.bank.get_mut(&f.config).unwrap().data[bump_off] = real.wrapping_add(1);
    let claim = claim_ix(&f);
    let result = process(&mut f, &claim);
    assert!(
        !result.succeeded(),
        "a tampered stored bump must refuse the claim bind: {:#?}",
        f.svm.logs(),
    );

    // Heal: byte restored, the identical instruction binds and claims.
    f.bank.get_mut(&f.config).unwrap().data[bump_off] = real;
    let claim = claim_ix(&f);
    let result = process(&mut f, &claim);
    assert!(
        result.succeeded(),
        "the restored canonical bump must claim: {:#?}",
        f.svm.logs(),
    );
}
