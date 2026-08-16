//! Compiled Cicada lifecycle and adversarial-route coverage.
//!
//! Build all three ELFs first with three commands:
//! `cargo build-sbf --manifest-path examples/hopper-cicada/Cargo.toml -- --locked`,
//! `cargo build-sbf --manifest-path examples/hopper-cicada-route-fixture/Cargo.toml -- --locked`,
//! and `cargo build-sbf --manifest-path examples/hopper-cicada-canonical-route-fixture/Cargo.toml -- --locked`.
//! The tests skip when those artifacts are absent, matching Hopper's other
//! compiled-SBF suites.

use std::collections::BTreeMap;

use hopper_cicada::{
    CicadaConfig, ClaimStillActive, ConflictingDuplicateRouteMeta, DestinationLamportsShortfall,
    DestinationTokenPolicyChanged, EmptySettlement, IntentShard, MinimumOutputNotMet,
    ProtectedAccountDelegation, SourceLamportsDecreased, SourceTokenPolicyChanged,
    UnauthorizedInitializer, CONFIG_SEED, INTENTS_PER_SHARD, MAX_ROUTE_ACCOUNTS, SOURCE_LEASE_SEED,
    STATUS_CANCELLED, STATUS_CLAIMED, STATUS_OPEN, STATUS_SETTLED, VAULT_AUTHORITY_SEED,
};
use hopper_test::{HarnessResult, LiteSvmHarness};
use solana_account::Account;
use solana_instruction::{error::InstructionError, AccountMeta, Instruction};
use solana_pubkey::Pubkey;

const CICADA_ELF: &str = "../../target/deploy/hopper_cicada";
const ROUTE_ELF: &str = "../../target/deploy/hopper_cicada_route_fixture";
const CANONICAL_ROUTE_ELF: &str = "../../target/deploy/hopper_cicada_canonical_route_fixture";
const ROUTE_HONEST: u8 = 0xA0;
const ROUTE_MUTATE_POLICY: u8 = 0xA1;
const ROUTE_SPOOF_OUTPUT: u8 = 0xA2;
const ROUTE_DRAIN_SOURCE_LAMPORT: u8 = 0xA3;
const ROUTE_CANONICAL_SWAP: u8 = 0xB0;
const ROUTE_CANONICAL_UNDERPAY: u8 = 0xB1;
const ROUTE_CANONICAL_NO_INPUT: u8 = 0xB2;
const ROUTE_CANONICAL_MUTATE_SOURCE_POLICY: u8 = 0xB4;
const ROUTE_CANONICAL_SUPPLY_NEUTRAL_MINT_BURN: u8 = 0xB5;
const LEGACY_NATIVE_MINT: Pubkey =
    Pubkey::from_str_const("So11111111111111111111111111111111111111112");
const TOKEN_2022_NATIVE_MINT: Pubkey =
    Pubkey::from_str_const("9pan9bMn5HatX4EJdBwg9VgCa7Uz5HL8N1m5D3NdXejP");
const NATIVE_RENT_RESERVE: u64 = 2_039_280;
const TOKEN_IS_NATIVE_OPTION_OFFSET: usize = 109;
const TOKEN_IS_NATIVE_VALUE_OFFSET: usize = TOKEN_IS_NATIVE_OPTION_OFFSET + 4;

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
    input_sink: Pubkey,
    output_reserve: Pubkey,
    dust_donor: Pubkey,
    dust_authority: Pubkey,
    route_program: Pubkey,
    source_lease: Pubkey,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RouteBackend {
    AdversarialCombinedToken,
    CanonicalTokenOnly,
    CanonicalToken2022Only,
    CanonicalRoute,
    CanonicalToken2022Route,
}

impl RouteBackend {
    fn uses_token_2022(self) -> bool {
        matches!(
            self,
            Self::CanonicalToken2022Only | Self::CanonicalToken2022Route
        )
    }

    fn uses_canonical_route(self) -> bool {
        matches!(self, Self::CanonicalRoute | Self::CanonicalToken2022Route)
    }

    fn native_mint(self) -> Pubkey {
        if self.uses_token_2022() {
            TOKEN_2022_NATIVE_MINT
        } else {
            LEGACY_NATIVE_MINT
        }
    }
}

fn report_missing_sbf(path: &str) {
    if std::env::var("HOPPER_REQUIRE_CICADA_SBF").as_deref() == Ok("1") {
        panic!("required Cicada SBF artifact is missing: {path}.so");
    }
    eprintln!("SKIPPED: {path}.so not found");
}

fn harness(backend: RouteBackend) -> Option<(LiteSvmHarness, Pubkey, Pubkey, Pubkey)> {
    let program_id = Pubkey::new_unique();
    let Some(mut svm) = LiteSvmHarness::load(&program_id, CICADA_ELF) else {
        report_missing_sbf(CICADA_ELF);
        return None;
    };
    let token_program = if backend.uses_token_2022() {
        mollusk_svm_programs_token::token2022::ID
    } else {
        mollusk_svm_programs_token::token::ID
    };
    let canonical_token = backend != RouteBackend::AdversarialCombinedToken;
    let route_program = if canonical_token {
        if backend.uses_token_2022() {
            mollusk_svm_programs_token::token2022::add_program(svm.mollusk_mut());
        } else {
            mollusk_svm_programs_token::token::add_program(svm.mollusk_mut());
        }
        Pubkey::new_unique()
    } else {
        // The adversarial fixture intentionally emulates the two SPL Token
        // instructions Cicada invokes and exposes hostile route commands from
        // the same executable. Canonical SPL compatibility is covered by the
        // separate `canonical_token` lane below.
        token_program
    };
    let route_elf = if backend.uses_canonical_route() {
        CANONICAL_ROUTE_ELF
    } else {
        ROUTE_ELF
    };
    if !svm.add_program(&route_program, route_elf) {
        report_missing_sbf(route_elf);
        return None;
    }
    Some((svm, program_id, token_program, route_program))
}

fn system_account() -> Account {
    Account::new(0, 0, &Pubkey::default())
}

fn mint_account(
    token_program: &Pubkey,
    decimals: u8,
    supply: u64,
    mint_authority: Option<&Pubkey>,
) -> Account {
    let mut data = vec![0u8; 82];
    if let Some(authority) = mint_authority {
        data[..4].copy_from_slice(&1u32.to_le_bytes());
        data[4..36].copy_from_slice(&authority.to_bytes());
    }
    data[36..44].copy_from_slice(&supply.to_le_bytes());
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

fn loader_v3_deployment_accounts(
    svm: &LiteSvmHarness,
    program_id: &Pubkey,
    upgrade_authority: &Pubkey,
) -> (Pubkey, Account, Account) {
    let (loaded_program_id, program_account) = svm.own_program_account();
    assert_eq!(loaded_program_id, *program_id);
    let (program_data, _) =
        Pubkey::find_program_address(&[program_id.as_ref()], &program_account.owner);
    assert_eq!(
        &program_account.data[4..36],
        program_data.as_ref(),
        "Mollusk loader-v3 program must point at the canonical ProgramData PDA",
    );

    let mut program_data_bytes = vec![0u8; 45];
    program_data_bytes[..4].copy_from_slice(&3u32.to_le_bytes());
    program_data_bytes[12] = 1;
    program_data_bytes[13..45].copy_from_slice(upgrade_authority.as_ref());
    let program_data_account = Account {
        lamports: 10_000_000,
        data: program_data_bytes,
        owner: program_account.owner,
        executable: false,
        rent_epoch: 0,
    };
    (program_data, program_account, program_data_account)
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

fn native_token_account(
    token_program: &Pubkey,
    mint: &Pubkey,
    owner: &Pubkey,
    amount: u64,
) -> Account {
    let mut account = token_account(token_program, mint, owner, amount);
    account.lamports = NATIVE_RENT_RESERVE
        .checked_add(amount)
        .expect("native fixture lamports");
    account.data[TOKEN_IS_NATIVE_OPTION_OFFSET..TOKEN_IS_NATIVE_VALUE_OFFSET]
        .copy_from_slice(&1u32.to_le_bytes());
    account.data[TOKEN_IS_NATIVE_VALUE_OFFSET..TOKEN_IS_NATIVE_VALUE_OFFSET + 8]
        .copy_from_slice(&NATIVE_RENT_RESERVE.to_le_bytes());
    account
}

fn fixture_token_account(
    token_program: &Pubkey,
    mint: &Pubkey,
    owner: &Pubkey,
    amount: u64,
    native: bool,
) -> Account {
    if native {
        native_token_account(token_program, mint, owner, amount)
    } else {
        token_account(token_program, mint, owner, amount)
    }
}

fn token_amount(account: &Account) -> u64 {
    u64::from_le_bytes(account.data[64..72].try_into().unwrap())
}

fn token_authority(account: &Account) -> Pubkey {
    Pubkey::new_from_array(account.data[32..64].try_into().unwrap())
}

fn token_native_reserve(account: &Account) -> Option<u64> {
    match u32::from_le_bytes(
        account.data[TOKEN_IS_NATIVE_OPTION_OFFSET..TOKEN_IS_NATIVE_VALUE_OFFSET]
            .try_into()
            .unwrap(),
    ) {
        0 => None,
        1 => Some(u64::from_le_bytes(
            account.data[TOKEN_IS_NATIVE_VALUE_OFFSET..TOKEN_IS_NATIVE_VALUE_OFFSET + 8]
                .try_into()
                .unwrap(),
        )),
        option => panic!("invalid native COption tag {option}"),
    }
}

fn account_bytes(account: &Account, offset: u32, size: usize) -> &[u8] {
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

fn setup_open_intent_with_backend(backend: RouteBackend) -> Option<Fixture> {
    setup_open_intent_with_backend_and_native(backend, false)
}

fn setup_open_intent_with_backend_and_native(
    backend: RouteBackend,
    native: bool,
) -> Option<Fixture> {
    let (svm, program_id, token_program, route_program) = harness(backend)?;
    let canonical_token = backend != RouteBackend::AdversarialCombinedToken;
    let owner = Pubkey::new_unique();
    let executor = Pubkey::new_unique();
    let shard = Pubkey::new_unique();
    let source = Pubkey::new_unique();
    let refund = Pubkey::new_unique();
    let destination = Pubkey::new_unique();
    let input_mint = if native {
        backend.native_mint()
    } else {
        Pubkey::new_unique()
    };
    let output_mint = if native {
        input_mint
    } else {
        Pubkey::new_unique()
    };
    let input_sink = Pubkey::new_unique();
    let output_reserve = Pubkey::new_unique();
    let dust_donor = Pubkey::new_unique();
    let dust_authority = Pubkey::new_unique();
    let (config, _) = Pubkey::find_program_address(&[CONFIG_SEED], &program_id);
    let (vault, _) = Pubkey::find_program_address(
        &[VAULT_AUTHORITY_SEED, owner.as_ref(), source.as_ref()],
        &program_id,
    );
    let (source_lease, _) =
        Pubkey::find_program_address(&[SOURCE_LEASE_SEED, source.as_ref()], &program_id);
    let (program_data, program_account, program_data_account) =
        loader_v3_deployment_accounts(&svm, &program_id, &owner);

    let mut bank = Bank::new();
    bank.insert(owner, Account::new(50_000_000_000, 0, &Pubkey::default()));
    bank.insert(executor, Account::new(5_000_000_000, 0, &Pubkey::default()));
    bank.insert(
        dust_authority,
        Account::new(5_000_000_000, 0, &Pubkey::default()),
    );
    bank.insert(config, system_account());
    bank.insert(shard, system_account());
    bank.insert(source_lease, system_account());
    bank.insert(vault, system_account());
    bank.insert(program_id, program_account);
    bank.insert(program_data, program_data_account);
    if native {
        bank.insert(input_mint, mint_account(&token_program, 9, 0, None));
    } else {
        bank.insert(input_mint, mint_account(&token_program, 6, 101, None));
        bank.insert(
            output_mint,
            mint_account(&token_program, 6, 1_000, Some(&executor)),
        );
    }
    bank.insert(
        source,
        fixture_token_account(&token_program, &input_mint, &owner, 100, native),
    );
    bank.insert(
        refund,
        fixture_token_account(&token_program, &input_mint, &owner, 0, native),
    );
    bank.insert(
        destination,
        fixture_token_account(&token_program, &output_mint, &owner, 0, native),
    );
    bank.insert(
        input_sink,
        fixture_token_account(&token_program, &input_mint, &executor, 0, native),
    );
    bank.insert(
        output_reserve,
        fixture_token_account(&token_program, &output_mint, &executor, 1_000, native),
    );
    bank.insert(
        dust_donor,
        fixture_token_account(&token_program, &input_mint, &dust_authority, 1, native),
    );
    bank.insert(
        Pubkey::default(),
        LiteSvmHarness::system_program_account().1,
    );
    let token_program_account = if canonical_token {
        if backend.uses_token_2022() {
            mollusk_svm_programs_token::token2022::account()
        } else {
            mollusk_svm_programs_token::token::account()
        }
    } else {
        LiteSvmHarness::executable_program_account(&token_program).1
    };
    bank.insert(token_program, token_program_account);
    if route_program != token_program {
        bank.insert(
            route_program,
            LiteSvmHarness::executable_program_account(&route_program).1,
        );
    }

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
        input_sink,
        output_reserve,
        dust_donor,
        dust_authority,
        route_program,
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
            AccountMeta::new_readonly(program_id, false),
            AccountMeta::new_readonly(program_data, false),
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
            AccountMeta::new(source, false),
            AccountMeta::new_readonly(vault, false),
            AccountMeta::new_readonly(refund, false),
            AccountMeta::new_readonly(destination, false),
            AccountMeta::new_readonly(input_mint, false),
            AccountMeta::new_readonly(output_mint, false),
            AccountMeta::new_readonly(token_program, false),
            AccountMeta::new_readonly(route_program, false),
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
    assert_eq!(
        token_authority(&fixture.bank[&source]),
        vault,
        "create_intent must atomically move source authority into Cicada custody",
    );
    if canonical_token {
        assert!(
            fixture
                .svm
                .logs()
                .iter()
                .any(|line| line == &format!("Program {token_program} invoke [2]")),
            "create_intent must adopt custody through the canonical token processor",
        );
    }
    Some(fixture)
}

fn setup_open_intent() -> Option<Fixture> {
    setup_open_intent_with_backend(RouteBackend::AdversarialCombinedToken)
}

fn setup_open_intent_with_canonical_token() -> Option<Fixture> {
    setup_open_intent_with_backend(RouteBackend::CanonicalTokenOnly)
}

fn setup_open_intent_with_canonical_token_2022() -> Option<Fixture> {
    setup_open_intent_with_backend(RouteBackend::CanonicalToken2022Only)
}

fn setup_open_intent_with_canonical_route() -> Option<Fixture> {
    setup_open_intent_with_backend(RouteBackend::CanonicalRoute)
}

fn setup_open_intent_with_canonical_token_2022_route() -> Option<Fixture> {
    setup_open_intent_with_backend(RouteBackend::CanonicalToken2022Route)
}

fn setup_open_native_intent() -> Option<Fixture> {
    setup_open_intent_with_backend_and_native(RouteBackend::AdversarialCombinedToken, true)
}

fn setup_open_native_intent_with_canonical_route() -> Option<Fixture> {
    setup_open_intent_with_backend_and_native(RouteBackend::CanonicalRoute, true)
}

fn setup_open_native_intent_with_canonical_token_2022_route() -> Option<Fixture> {
    setup_open_intent_with_backend_and_native(RouteBackend::CanonicalToken2022Route, true)
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
            AccountMeta::new_readonly(f.route_program, false),
            AccountMeta::new(f.source, false),
            AccountMeta::new(f.destination, false),
            AccountMeta::new_readonly(f.vault, false),
        ],
    )
}

fn canonical_execute_ix(f: &Fixture, command: u8, input: u64, output: u64) -> Instruction {
    let mut route_data = vec![command];
    route_data.extend_from_slice(&input.to_le_bytes());
    route_data.extend_from_slice(&output.to_le_bytes());
    route_data.push(f.bank[&f.input_mint].data[44]);
    route_data.push(f.bank[&f.output_mint].data[44]);

    let mut args = Vec::new();
    args.extend_from_slice(&0u16.to_le_bytes());
    args.extend_from_slice(&(route_data.len() as u16).to_le_bytes());
    args.extend_from_slice(&route_data);
    let mut flags = [0u8; MAX_ROUTE_ACCOUNTS];
    flags[..9].copy_from_slice(&[
        1,
        0,
        1,
        2,
        1,
        u8::from(command == ROUTE_CANONICAL_SUPPLY_NEUTRAL_MINT_BURN),
        1,
        2,
        0,
    ]);
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
            AccountMeta::new_readonly(f.route_program, false),
            AccountMeta::new(f.source, false),
            AccountMeta::new_readonly(f.input_mint, false),
            AccountMeta::new(f.input_sink, false),
            AccountMeta::new_readonly(f.vault, false),
            AccountMeta::new(f.output_reserve, false),
            if command == ROUTE_CANONICAL_SUPPLY_NEUTRAL_MINT_BURN {
                AccountMeta::new(f.output_mint, false)
            } else {
                AccountMeta::new_readonly(f.output_mint, false)
            },
            AccountMeta::new(f.destination, false),
            AccountMeta::new_readonly(f.executor, true),
            AccountMeta::new_readonly(f.token_program, false),
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

fn dust_source_through_canonical_token(f: &mut Fixture) {
    let mut data = Vec::with_capacity(10);
    data.push(12); // TransferChecked
    data.extend_from_slice(&1u64.to_le_bytes());
    data.push(f.bank[&f.input_mint].data[44]);
    let instruction = Instruction::new_with_bytes(
        f.token_program,
        &data,
        vec![
            AccountMeta::new(f.dust_donor, false),
            AccountMeta::new_readonly(f.input_mint, false),
            AccountMeta::new(f.source, false),
            AccountMeta::new_readonly(f.dust_authority, true),
        ],
    );
    f.svm.capture_logs();
    let result = process(f, &instruction);
    assert!(
        result.succeeded(),
        "canonical dust transfer failed: {:#?}",
        f.svm.logs(),
    );
    assert_eq!(token_amount(&f.bank[&f.dust_donor]), 0);
    assert_eq!(token_amount(&f.bank[&f.source]), 1);
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
fn compiled_initialize_refuses_non_upgrade_authority_first_caller() {
    let Some((svm, program_id, _, _)) = harness(RouteBackend::AdversarialCombinedToken) else {
        eprintln!("SKIPPED: build Cicada SBF artifacts first");
        return;
    };
    let upgrade_authority = Pubkey::new_unique();
    let attacker = Pubkey::new_unique();
    let (config, _) = Pubkey::find_program_address(&[CONFIG_SEED], &program_id);
    let (program_data, program_account, program_data_account) =
        loader_v3_deployment_accounts(&svm, &program_id, &upgrade_authority);
    let attacker_account = Account::new(50_000_000_000, 0, &Pubkey::default());
    let empty_config = system_account();
    let system_program = LiteSvmHarness::system_program_account().1;

    let mut args = Vec::new();
    args.extend_from_slice(attacker.as_ref());
    args.extend_from_slice(&10u64.to_le_bytes());
    let instruction = ix(
        program_id,
        0,
        &args,
        vec![
            AccountMeta::new(attacker, true),
            AccountMeta::new(config, false),
            AccountMeta::new_readonly(program_id, false),
            AccountMeta::new_readonly(program_data, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
        ],
    );
    let seeds = vec![
        (attacker, attacker_account),
        (config, empty_config.clone()),
        (program_id, program_account),
        (program_data, program_data_account),
        (Pubkey::default(), system_program),
    ];
    let result = svm.process(&instruction, &seeds);
    assert_custom_error(&result, UnauthorizedInitializer::CODE);
    assert_eq!(
        result.raw().get_account(&config).unwrap(),
        &empty_config,
        "a refused first caller must not initialize or fund the config PDA",
    );
}

#[test]
fn compiled_initialize_refuses_finalized_program_without_authority() {
    let Some((svm, program_id, _, _)) = harness(RouteBackend::AdversarialCombinedToken) else {
        eprintln!("SKIPPED: build Cicada SBF artifacts first");
        return;
    };
    let payer = Pubkey::new_unique();
    let (config, _) = Pubkey::find_program_address(&[CONFIG_SEED], &program_id);
    let (program_data, program_account, mut program_data_account) =
        loader_v3_deployment_accounts(&svm, &program_id, &payer);
    program_data_account.data[12] = 0;
    program_data_account.data[13..45].fill(0);
    let payer_account = Account::new(50_000_000_000, 0, &Pubkey::default());
    let empty_config = system_account();
    let system_program = LiteSvmHarness::system_program_account().1;

    let mut args = Vec::new();
    args.extend_from_slice(payer.as_ref());
    args.extend_from_slice(&10u64.to_le_bytes());
    let instruction = ix(
        program_id,
        0,
        &args,
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(config, false),
            AccountMeta::new_readonly(program_id, false),
            AccountMeta::new_readonly(program_data, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
        ],
    );
    let seeds = vec![
        (payer, payer_account),
        (config, empty_config.clone()),
        (program_id, program_account),
        (program_data, program_data_account),
        (Pubkey::default(), system_program),
    ];
    let result = svm.process(&instruction, &seeds);
    assert_custom_error(&result, UnauthorizedInitializer::CODE);
    assert_eq!(
        result.raw().get_account(&config).unwrap(),
        &empty_config,
        "a finalized deployment must not admit a first-caller initializer",
    );
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
fn compiled_canonical_spl_cancel_refund_and_reclaim_round_trip() {
    let Some(mut f) = setup_open_intent_with_canonical_token() else {
        eprintln!("SKIPPED: build Cicada SBF artifacts first");
        return;
    };
    f.svm.capture_logs();

    // This lane registers Mollusk's vendored canonical SPL Token ELF rather
    // than Cicada's adversarial route fixture under the token program id.
    let cancel = cancel_ix(&f);
    let result = process(&mut f, &cancel);
    assert!(
        result.succeeded(),
        "canonical SPL refund failed: {:#?}",
        f.svm.logs()
    );
    assert_eq!(token_amount(&f.bank[&f.source]), 0);
    assert_eq!(token_amount(&f.bank[&f.refund]), 100);
    assert_eq!(
        f.bank[&f.shard].data[IntentShard::STATUSES_ABS_OFFSET as usize],
        STATUS_CANCELLED,
    );
    assert!(
        f.svm
            .logs()
            .iter()
            .any(|line| line == &format!("Program {} invoke [2]", f.token_program)),
        "cancel must invoke the canonical SPL Token processor",
    );

    f.svm.capture_logs();
    let reclaim = reclaim_ix(&f);
    let result = process(&mut f, &reclaim);
    assert!(
        result.succeeded(),
        "canonical SPL authority restore failed: {:#?}",
        f.svm.logs()
    );
    assert!(
        f.svm
            .logs()
            .iter()
            .any(|line| line == &format!("Program {} invoke [2]", f.token_program)),
        "reclaim must invoke the canonical SPL Token processor",
    );
    assert_reclaimed(&f);
}

#[test]
fn compiled_canonical_token_2022_custody_cancel_and_reclaim_round_trip() {
    let Some(mut f) = setup_open_intent_with_canonical_token_2022() else {
        eprintln!("SKIPPED: build Cicada SBF artifacts first");
        return;
    };
    assert_eq!(
        token_authority(&f.bank[&f.source]),
        f.vault,
        "canonical Token-2022 create must adopt the extension-free source",
    );

    f.svm.capture_logs();
    let cancel = cancel_ix(&f);
    let result = process(&mut f, &cancel);
    assert!(
        result.succeeded(),
        "canonical Token-2022 refund failed: {:#?}",
        f.svm.logs(),
    );
    assert_eq!(token_amount(&f.bank[&f.source]), 0);
    assert_eq!(token_amount(&f.bank[&f.refund]), 100);
    assert!(
        f.svm
            .logs()
            .iter()
            .any(|line| line == &format!("Program {} invoke [2]", f.token_program)),
        "cancel must invoke Mollusk's canonical Token-2022 processor",
    );

    f.svm.capture_logs();
    let reclaim = reclaim_ix(&f);
    let result = process(&mut f, &reclaim);
    assert!(
        result.succeeded(),
        "canonical Token-2022 authority restore failed: {:#?}",
        f.svm.logs(),
    );
    assert!(
        f.svm
            .logs()
            .iter()
            .any(|line| line == &format!("Program {} invoke [2]", f.token_program)),
        "reclaim must invoke Mollusk's canonical Token-2022 processor",
    );
    assert_reclaimed(&f);
}

#[test]
fn compiled_canonical_route_executes_two_real_token_legs_refunds_and_reclaims() {
    let Some(mut f) = setup_open_intent_with_canonical_route() else {
        eprintln!("SKIPPED: build all Cicada SBF artifacts first");
        return;
    };

    let claim = claim_ix(&f);
    assert!(process(&mut f, &claim).succeeded(), "claim failed");

    f.svm.capture_logs();
    let execute = canonical_execute_ix(&f, ROUTE_CANONICAL_SWAP, 60, 95);
    let result = process(&mut f, &execute);
    assert!(
        result.succeeded(),
        "canonical route execute failed: {:#?}",
        f.svm.logs(),
    );
    let logs = f.svm.logs();
    assert!(
        logs.iter()
            .any(|line| line == &format!("Program {} invoke [2]", f.route_program)),
        "Cicada must enter the compiled route program: {logs:#?}",
    );
    assert_eq!(
        logs.iter()
            .filter(|line| line == &&format!("Program {} invoke [3]", f.token_program))
            .count(),
        2,
        "both swap legs must enter Mollusk's canonical SPL Token ELF",
    );
    assert_eq!(
        logs.iter()
            .filter(|line| line == &&format!("Program {} invoke [2]", f.token_program))
            .count(),
        1,
        "Cicada must refund the unused source through canonical SPL Token",
    );

    assert_eq!(token_amount(&f.bank[&f.source]), 0);
    assert_eq!(token_amount(&f.bank[&f.input_sink]), 60);
    assert_eq!(token_amount(&f.bank[&f.output_reserve]), 905);
    assert_eq!(token_amount(&f.bank[&f.destination]), 95);
    assert_eq!(token_amount(&f.bank[&f.refund]), 40);
    assert_eq!(
        f.bank[&f.shard].data[IntentShard::STATUSES_ABS_OFFSET as usize],
        STATUS_SETTLED,
    );
    assert_eq!(
        account_u64(&f.bank[&f.shard], IntentShard::SETTLED_INPUTS_ABS_OFFSET),
        60,
    );
    assert_eq!(
        account_u64(&f.bank[&f.shard], IntentShard::SETTLED_OUTPUTS_ABS_OFFSET),
        95,
    );

    f.svm.capture_logs();
    let reclaim = reclaim_ix(&f);
    let result = process(&mut f, &reclaim);
    assert!(
        result.succeeded(),
        "canonical settled reclaim failed: {:#?}",
        f.svm.logs(),
    );
    assert!(
        f.svm
            .logs()
            .iter()
            .any(|line| line == &format!("Program {} invoke [2]", f.token_program)),
        "reclaim must restore source authority through canonical SPL Token",
    );
    assert_reclaimed(&f);
}

#[test]
fn compiled_canonical_token_2022_route_executes_refunds_and_reclaims() {
    let Some(mut f) = setup_open_intent_with_canonical_token_2022_route() else {
        eprintln!("SKIPPED: build all Cicada SBF artifacts first");
        return;
    };

    let claim = claim_ix(&f);
    assert!(process(&mut f, &claim).succeeded(), "claim failed");
    f.svm.capture_logs();
    let execute = canonical_execute_ix(&f, ROUTE_CANONICAL_SWAP, 60, 95);
    let result = process(&mut f, &execute);
    assert!(
        result.succeeded(),
        "canonical Token-2022 route execute failed: {:#?}",
        f.svm.logs(),
    );
    let logs = f.svm.logs();
    assert_eq!(
        logs.iter()
            .filter(|line| line == &&format!("Program {} invoke [3]", f.token_program))
            .count(),
        2,
        "both swap legs must enter Mollusk's canonical Token-2022 ELF",
    );
    assert_eq!(
        logs.iter()
            .filter(|line| line == &&format!("Program {} invoke [2]", f.token_program))
            .count(),
        1,
        "Cicada must refund unused input through canonical Token-2022",
    );
    assert_eq!(token_amount(&f.bank[&f.source]), 0);
    assert_eq!(token_amount(&f.bank[&f.input_sink]), 60);
    assert_eq!(token_amount(&f.bank[&f.output_reserve]), 905);
    assert_eq!(token_amount(&f.bank[&f.destination]), 95);
    assert_eq!(token_amount(&f.bank[&f.refund]), 40);
    assert_eq!(
        f.bank[&f.shard].data[IntentShard::STATUSES_ABS_OFFSET as usize],
        STATUS_SETTLED,
    );

    f.svm.capture_logs();
    let reclaim = reclaim_ix(&f);
    let result = process(&mut f, &reclaim);
    assert!(
        result.succeeded(),
        "canonical Token-2022 reclaim failed: {:#?}",
        f.svm.logs(),
    );
    assert!(
        f.svm
            .logs()
            .iter()
            .any(|line| line == &format!("Program {} invoke [2]", f.token_program)),
        "reclaim must restore authority through canonical Token-2022",
    );
    assert_reclaimed(&f);
}

#[test]
fn compiled_native_lamport_floors_accept_canonical_spl_and_token_2022_routes() {
    let cases = [
        (
            setup_open_native_intent_with_canonical_route as fn() -> Option<Fixture>,
            "SPL Token",
            LEGACY_NATIVE_MINT,
        ),
        (
            setup_open_native_intent_with_canonical_token_2022_route as fn() -> Option<Fixture>,
            "Token-2022",
            TOKEN_2022_NATIVE_MINT,
        ),
    ];

    for (setup, label, expected_mint) in cases {
        let Some(mut f) = setup() else {
            eprintln!("SKIPPED: build all Cicada SBF artifacts first");
            return;
        };
        assert_eq!(f.input_mint, expected_mint, "wrong {label} native mint");
        assert_eq!(f.output_mint, expected_mint, "wrong {label} native mint");

        let claim = claim_ix(&f);
        assert!(process(&mut f, &claim).succeeded(), "{label} claim failed");

        let source_lamports = f.bank[&f.source].lamports;
        let input_sink_lamports = f.bank[&f.input_sink].lamports;
        let output_reserve_lamports = f.bank[&f.output_reserve].lamports;
        let destination_lamports = f.bank[&f.destination].lamports;
        let refund_lamports = f.bank[&f.refund].lamports;
        f.svm.capture_logs();
        let execute = canonical_execute_ix(&f, ROUTE_CANONICAL_SWAP, 60, 95);
        let result = process(&mut f, &execute);
        assert!(
            result.succeeded(),
            "canonical {label} native route failed: {:#?}",
            f.svm.logs(),
        );

        assert_eq!(token_amount(&f.bank[&f.source]), 0);
        assert_eq!(token_amount(&f.bank[&f.input_sink]), 60);
        assert_eq!(token_amount(&f.bank[&f.output_reserve]), 905);
        assert_eq!(token_amount(&f.bank[&f.destination]), 95);
        assert_eq!(token_amount(&f.bank[&f.refund]), 40);
        assert_eq!(f.bank[&f.source].lamports, source_lamports - 100);
        assert_eq!(f.bank[&f.input_sink].lamports, input_sink_lamports + 60);
        assert_eq!(
            f.bank[&f.output_reserve].lamports,
            output_reserve_lamports - 95,
        );
        assert_eq!(f.bank[&f.destination].lamports, destination_lamports + 95,);
        assert_eq!(f.bank[&f.refund].lamports, refund_lamports + 40);
        for address in [
            f.source,
            f.input_sink,
            f.output_reserve,
            f.destination,
            f.refund,
        ] {
            assert_eq!(
                token_native_reserve(&f.bank[&address]),
                Some(NATIVE_RENT_RESERVE),
                "{label} route changed native reserve for {address}",
            );
        }

        let logs = f.svm.logs();
        assert_eq!(
            logs.iter()
                .filter(|line| line == &&format!("Program {} invoke [3]", f.token_program))
                .count(),
            2,
            "both {label} native swap legs must enter the canonical token ELF",
        );
        assert_eq!(
            logs.iter()
                .filter(|line| line == &&format!("Program {} invoke [2]", f.token_program))
                .count(),
            1,
            "the {label} native refund must enter the canonical token ELF",
        );
    }
}

#[test]
fn compiled_native_lamport_shortfall_is_rolled_back() {
    let Some(mut f) = setup_open_native_intent() else {
        eprintln!("SKIPPED: build Cicada SBF artifacts first");
        return;
    };
    f.svm.capture_logs();
    let execute = execute_ix(&f, ROUTE_DRAIN_SOURCE_LAMPORT);
    let before = instruction_snapshot(&f, &execute);
    let result = process(&mut f, &execute);
    let logs = f.svm.logs();

    // The fixture models a close/reinitialize end state: token bytes claim a
    // 95-lamport native credit, but only one lamport actually reaches the
    // destination. Cicada must reject the under-backed account and Solana must
    // roll the nested changes back atomically.
    assert_custom_error(&result, DestinationLamportsShortfall::CODE);
    assert!(
        logs.iter()
            .any(|line| line == &format!("Program {} invoke [2]", f.route_program)),
        "the modeled native shortfall must execute before Cicada rejects it: {logs:#?}",
    );
    assert_instruction_rolled_back(&result, &before);
}

#[test]
fn compiled_settled_dust_cannot_block_reclaim_for_canonical_tokens() {
    let setups = [
        setup_open_intent_with_canonical_route as fn() -> Option<Fixture>,
        setup_open_intent_with_canonical_token_2022_route as fn() -> Option<Fixture>,
    ];

    for setup in setups {
        let Some(mut f) = setup() else {
            eprintln!("SKIPPED: build all Cicada SBF artifacts first");
            return;
        };
        let claim = claim_ix(&f);
        assert!(process(&mut f, &claim).succeeded());
        let execute = canonical_execute_ix(&f, ROUTE_CANONICAL_SWAP, 60, 95);
        let result = process(&mut f, &execute);
        assert!(
            result.succeeded(),
            "canonical settlement failed: {:#?}",
            f.svm.logs(),
        );
        assert_eq!(
            f.bank[&f.shard].data[IntentShard::STATUSES_ABS_OFFSET as usize],
            STATUS_SETTLED,
        );

        dust_source_through_canonical_token(&mut f);
        // Reclaim must remain independent of a post-final refund account.
        f.bank.remove(&f.refund);
        f.svm.capture_logs();
        let reclaim = reclaim_ix(&f);
        let result = process(&mut f, &reclaim);
        assert!(
            result.succeeded(),
            "settled dusty vault reclaim failed: {:#?}",
            f.svm.logs(),
        );
        assert_eq!(
            f.svm
                .logs()
                .iter()
                .filter(|line| line == &&format!("Program {} invoke [2]", f.token_program))
                .count(),
            1,
            "reclaim should restore authority without moving post-final dust",
        );
        assert_eq!(token_amount(&f.bank[&f.source]), 1);
        assert_reclaimed(&f);
    }
}

#[test]
fn compiled_cancelled_dust_cannot_block_reclaim_for_canonical_tokens() {
    let setups = [
        setup_open_intent_with_canonical_token as fn() -> Option<Fixture>,
        setup_open_intent_with_canonical_token_2022 as fn() -> Option<Fixture>,
    ];

    for setup in setups {
        let Some(mut f) = setup() else {
            eprintln!("SKIPPED: build Cicada SBF artifacts first");
            return;
        };
        let cancel = cancel_ix(&f);
        let result = process(&mut f, &cancel);
        assert!(
            result.succeeded(),
            "canonical cancellation failed: {:#?}",
            f.svm.logs(),
        );
        assert_eq!(
            f.bank[&f.shard].data[IntentShard::STATUSES_ABS_OFFSET as usize],
            STATUS_CANCELLED,
        );

        dust_source_through_canonical_token(&mut f);
        // Reclaim must remain independent of a post-final refund account.
        f.bank.remove(&f.refund);
        f.svm.capture_logs();
        let reclaim = reclaim_ix(&f);
        let result = process(&mut f, &reclaim);
        assert!(
            result.succeeded(),
            "cancelled dusty vault reclaim failed: {:#?}",
            f.svm.logs(),
        );
        assert_eq!(
            f.svm
                .logs()
                .iter()
                .filter(|line| line == &&format!("Program {} invoke [2]", f.token_program))
                .count(),
            1,
            "reclaim should restore authority without moving post-final dust",
        );
        assert_eq!(token_amount(&f.bank[&f.source]), 1);
        assert_reclaimed(&f);
    }
}

#[test]
fn compiled_canonical_route_owner_authorized_policy_mutation_rolls_back() {
    let Some(mut f) = setup_open_intent_with_canonical_route() else {
        eprintln!("SKIPPED: build all Cicada SBF artifacts first");
        return;
    };
    f.svm.capture_logs();
    let execute = canonical_execute_ix(&f, ROUTE_CANONICAL_MUTATE_SOURCE_POLICY, 60, 95);
    let before = instruction_snapshot(&f, &execute);
    let result = process(&mut f, &execute);
    assert_custom_error(&result, SourceTokenPolicyChanged::CODE);
    assert_eq!(
        f.svm
            .logs()
            .iter()
            .filter(|line| line == &&format!("Program {} invoke [3]", f.token_program))
            .count(),
        3,
        "two transfers and SetAuthority must all reach canonical SPL Token",
    );
    assert_instruction_rolled_back(&result, &before);
}

#[test]
fn compiled_writable_output_mint_is_rejected_before_route_cpi() {
    let Some(mut f) = setup_open_intent_with_canonical_route() else {
        eprintln!("SKIPPED: build all Cicada SBF artifacts first");
        return;
    };
    f.svm.capture_logs();
    let execute = canonical_execute_ix(&f, ROUTE_CANONICAL_SUPPLY_NEUTRAL_MINT_BURN, 60, 95);
    let before = instruction_snapshot(&f, &execute);
    let result = process(&mut f, &execute);
    assert_custom_error(&result, ProtectedAccountDelegation::CODE);
    assert!(
        !f.svm
            .logs()
            .iter()
            .any(|line| line == &format!("Program {} invoke [2]", f.route_program)),
        "writable mint delegation must be refused before route CPI",
    );
    assert_instruction_rolled_back(&result, &before);
}

#[test]
fn compiled_canonical_route_underpayment_and_zero_input_roll_back() {
    for (command, input, output, code) in [
        (ROUTE_CANONICAL_UNDERPAY, 60, 89, MinimumOutputNotMet::CODE),
        (ROUTE_CANONICAL_NO_INPUT, 0, 95, EmptySettlement::CODE),
    ] {
        let Some(mut f) = setup_open_intent_with_canonical_route() else {
            eprintln!("SKIPPED: build all Cicada SBF artifacts first");
            return;
        };
        let execute = canonical_execute_ix(&f, command, input, output);
        let before = instruction_snapshot(&f, &execute);
        let result = process(&mut f, &execute);
        assert_custom_error(&result, code);
        assert_instruction_rolled_back(&result, &before);
    }
}

#[test]
fn compiled_noncanonical_token_account_shapes_are_rejected() {
    let cases = [
        (
            setup_open_intent_with_canonical_route as fn() -> Option<Fixture>,
            166usize,
        ),
        (
            setup_open_intent_with_canonical_token_2022_route as fn() -> Option<Fixture>,
            355usize,
        ),
    ];

    for (setup, source_len) in cases {
        let Some(mut f) = setup() else {
            eprintln!("SKIPPED: build all Cicada SBF artifacts first");
            return;
        };
        f.bank
            .get_mut(&f.source)
            .expect("source account")
            .data
            .resize(source_len, 0);

        let execute = canonical_execute_ix(&f, ROUTE_CANONICAL_SWAP, 60, 95);
        let before = instruction_snapshot(&f, &execute);
        let result = process(&mut f, &execute);

        assert_eq!(
            result.raw().raw_result,
            Err(InstructionError::InvalidAccountData),
            "noncanonical token-account length {source_len} must fail closed",
        );
        assert_instruction_rolled_back(&result, &before);
    }
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
            .any(|line| line == &format!("Program {} invoke [2]", f.route_program)),
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
fn compiled_hostile_route_lamport_debit_is_rolled_back() {
    let Some(mut f) = setup_open_intent() else {
        eprintln!("SKIPPED: build Cicada SBF artifacts first");
        return;
    };
    f.svm.capture_logs();
    let execute = execute_ix(&f, ROUTE_DRAIN_SOURCE_LAMPORT);
    let before = instruction_snapshot(&f, &execute);
    let result = process(&mut f, &execute);
    let logs = f.svm.logs();

    assert_custom_error(&result, SourceLamportsDecreased::CODE);
    assert!(
        logs.iter()
            .any(|line| line == &format!("Program {} invoke [2]", f.route_program)),
        "the lamport-changing route must execute before Cicada rejects its postcondition: {logs:#?}",
    );
    assert_instruction_rolled_back(&result, &before);
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

#[test]
fn compiled_writable_or_conflicting_duplicate_route_meta_is_rejected() {
    let Some(mut f) = setup_open_intent() else {
        eprintln!("SKIPPED: build Cicada SBF artifacts first");
        return;
    };

    // Repeating the source in the route's first two dynamic positions makes
    // both occurrences writable. Cicada must reject this before the safe
    // deduplicated CPI tier sees a shape that it deliberately cannot execute.
    f.svm.capture_logs();
    let mut writable_duplicate = execute_ix(&f, ROUTE_HONEST);
    let first_remaining = writable_duplicate.accounts.len() - 3;
    writable_duplicate.accounts[first_remaining + 1] = AccountMeta::new(f.source, false);
    let before = instruction_snapshot(&f, &writable_duplicate);
    let result = process(&mut f, &writable_duplicate);
    let logs = f.svm.logs();

    assert_custom_error(&result, ConflictingDuplicateRouteMeta::CODE);
    assert!(
        !logs
            .iter()
            .any(|line| line == &format!("Program {} invoke [2]", f.route_program)),
        "a duplicate writable route must be rejected before route CPI: {logs:#?}",
    );
    assert_instruction_rolled_back(&result, &before);

    // Conflicting read-only/writable flags remain rejected for the separate
    // privilege-union reason: the committed per-position flags would not
    // describe the callee's effective privilege.
    let mut execute = execute_ix(&f, ROUTE_HONEST);
    execute.accounts[first_remaining + 1] = AccountMeta::new_readonly(f.source, false);

    // Execute data is tag, intent index, route-data length, 17-byte route
    // payload, then the fixed-width route flags. The two aliases now name the
    // same Pubkey but request writable and read-only privileges respectively.
    let flags_start = 1 + 2 + 2 + 17;
    execute.data[flags_start + 1] = 0;

    let before = instruction_snapshot(&f, &execute);
    let result = process(&mut f, &execute);

    assert_custom_error(&result, ConflictingDuplicateRouteMeta::CODE);
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
