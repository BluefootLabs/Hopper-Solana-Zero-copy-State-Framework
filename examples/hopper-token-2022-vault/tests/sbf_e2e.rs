//! Compiled-SBF lifecycle proof against Mollusk's canonical Token-2022 and
//! Associated Token Account program ELFs.
//!
//! Build first with:
//! `cargo build-sbf --manifest-path examples/hopper-token-2022-vault/Cargo.toml -- --locked`.

use std::collections::BTreeMap;

use hopper::layout::HEADER_LEN;
use hopper_test::{HarnessResult, LiteSvmHarness};
use hopper_token_2022_vault::RewardVault;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

const ELF_PATH_STEM: &str = "../../target/deploy/hopper_token_2022_vault";
const INIT_TAG: u8 = 0;
const PREPARE_TAG: u8 = 1;
const MINT_TAG: u8 = 2;
const SWEEP_TAG: u8 = 3;
const MINTED_TOTAL_OFFSET: usize = HEADER_LEN + 32 + 32 + 32;
const SWEPT_TOTAL_OFFSET: usize = MINTED_TOTAL_OFFSET + 8;

type Bank = BTreeMap<Pubkey, Account>;

struct Fixture {
    svm: LiteSvmHarness,
    program_id: Pubkey,
    payer: Pubkey,
    authority: Pubkey,
    attacker: Pubkey,
    state: Pubkey,
    mint: Pubkey,
    vault_ata: Pubkey,
    destination: Pubkey,
    bank: Bank,
}

fn token_2022_mint(authority: &Pubkey) -> Account {
    let mut data = vec![0u8; 82];
    data[..4].copy_from_slice(&1u32.to_le_bytes());
    data[4..36].copy_from_slice(authority.as_ref());
    data[44] = 6;
    data[45] = 1;
    Account {
        lamports: 10_000_000,
        data,
        owner: mollusk_svm_programs_token::token2022::ID,
        executable: false,
        rent_epoch: 0,
    }
}

fn token_2022_account(mint: &Pubkey, owner: &Pubkey, amount: u64) -> Account {
    let mut data = vec![0u8; 165];
    data[..32].copy_from_slice(mint.as_ref());
    data[32..64].copy_from_slice(owner.as_ref());
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data[108] = 1;
    Account {
        lamports: 10_000_000,
        data,
        owner: mollusk_svm_programs_token::token2022::ID,
        executable: false,
        rent_epoch: 0,
    }
}

fn associated_address(wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            wallet.as_ref(),
            mollusk_svm_programs_token::token2022::ID.as_ref(),
            mint.as_ref(),
        ],
        &mollusk_svm_programs_token::associated_token::ID,
    )
    .0
}

fn instruction(
    program_id: Pubkey,
    tag: u8,
    payload: &[u8],
    accounts: Vec<AccountMeta>,
) -> Instruction {
    let mut data = Vec::with_capacity(1 + payload.len());
    data.push(tag);
    data.extend_from_slice(payload);
    Instruction::new_with_bytes(program_id, &data, accounts)
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
    fixture.svm.capture_logs();
    let result = fixture.svm.process(instruction, &seeds);
    if result.succeeded() {
        for (key, account) in &result.raw().resulting_accounts {
            fixture.bank.insert(*key, account.clone());
        }
    }
    result
}

fn setup() -> Option<Fixture> {
    let program_id = Pubkey::new_unique();
    let Some(mut svm) = LiteSvmHarness::load(&program_id, ELF_PATH_STEM) else {
        if std::env::var("HOPPER_REQUIRE_TOKEN_2022_VAULT_SBF").as_deref() == Ok("1") {
            panic!("required SBF artifact is missing: {ELF_PATH_STEM}.so");
        }
        eprintln!("SKIPPED: build {ELF_PATH_STEM}.so first");
        return None;
    };
    mollusk_svm_programs_token::token2022::add_program(svm.mollusk_mut());
    mollusk_svm_programs_token::associated_token::add_program(svm.mollusk_mut());

    let payer = Pubkey::new_unique();
    let authority = Pubkey::new_unique();
    let attacker = Pubkey::new_unique();
    let state = Pubkey::new_unique();
    let mint = Pubkey::new_unique();
    let vault_ata = associated_address(&authority, &mint);
    let destination = Pubkey::new_unique();
    let mut bank = Bank::new();
    bank.insert(payer, Account::new(1_000_000_000, 0, &Pubkey::default()));
    bank.insert(authority, Account::new(1_000_000, 0, &Pubkey::default()));
    bank.insert(attacker, Account::new(1_000_000, 0, &Pubkey::default()));
    bank.insert(state, Account::new(0, 0, &Pubkey::default()));
    bank.insert(mint, token_2022_mint(&authority));
    bank.insert(vault_ata, Account::new(0, 0, &Pubkey::default()));
    bank.insert(
        destination,
        token_2022_account(&mint, &Pubkey::new_unique(), 0),
    );
    bank.insert(
        mollusk_svm_programs_token::token2022::ID,
        mollusk_svm_programs_token::token2022::account(),
    );
    bank.insert(
        mollusk_svm_programs_token::associated_token::ID,
        mollusk_svm_programs_token::associated_token::account(),
    );
    bank.insert(
        LiteSvmHarness::system_program_account().0,
        LiteSvmHarness::system_program_account().1,
    );

    let mut fixture = Fixture {
        svm,
        program_id,
        payer,
        authority,
        attacker,
        state,
        mint,
        vault_ata,
        destination,
        bank,
    };
    let init = instruction(
        program_id,
        INIT_TAG,
        &[],
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(state, true),
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new_readonly(Pubkey::default(), false),
        ],
    );
    let result = process(&mut fixture, &init);
    assert!(result.succeeded(), "init failed: {:#?}", fixture.svm.logs());
    assert_eq!(fixture.bank[&state].data.len(), RewardVault::LEN);
    Some(fixture)
}

fn prepare_ix(f: &Fixture, authority: Pubkey, mint: Pubkey, ata: Pubkey) -> Instruction {
    instruction(
        f.program_id,
        PREPARE_TAG,
        &[],
        vec![
            AccountMeta::new(f.payer, true),
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new(f.state, false),
            AccountMeta::new(ata, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
            AccountMeta::new_readonly(mollusk_svm_programs_token::token2022::ID, false),
            AccountMeta::new_readonly(mollusk_svm_programs_token::associated_token::ID, false),
        ],
    )
}

fn read_u64(account: &Account, offset: usize) -> u64 {
    u64::from_le_bytes(account.data[offset..offset + 8].try_into().unwrap())
}

fn token_amount(account: &Account) -> u64 {
    read_u64(account, 64)
}

#[test]
fn canonical_token_2022_lifecycle_preserves_authority_and_binding() {
    let Some(mut f) = setup() else {
        return;
    };

    let attacker_ata = associated_address(&f.attacker, &f.mint);
    f.bank
        .insert(attacker_ata, Account::new(0, 0, &Pubkey::default()));
    let state_before = f.bank[&f.state].clone();
    let unauthorized = prepare_ix(&f, f.attacker, f.mint, attacker_ata);
    let result = process(&mut f, &unauthorized);
    assert!(
        !result.succeeded(),
        "stored authority must gate PrepareVaultAta"
    );
    assert_eq!(result.raw().get_account(&f.state), Some(&state_before));
    assert_eq!(result.raw().get_account(&attacker_ata).unwrap().lamports, 0);
    assert!(
        !f.svm.logs().iter().any(|line| {
            line == &format!(
                "Program {} invoke [2]",
                mollusk_svm_programs_token::associated_token::ID
            )
        }),
        "unauthorized prepare must fail before ATA CPI"
    );

    let prepare = prepare_ix(&f, f.authority, f.mint, f.vault_ata);
    let result = process(&mut f, &prepare);
    assert!(result.succeeded(), "prepare failed: {:#?}", f.svm.logs());
    assert!(f.svm.logs().iter().any(|line| {
        line == &format!(
            "Program {} invoke [2]",
            mollusk_svm_programs_token::associated_token::ID
        )
    }));
    let state = &f.bank[&f.state].data;
    assert_eq!(&state[HEADER_LEN..HEADER_LEN + 32], f.authority.as_ref());
    assert_eq!(&state[HEADER_LEN + 32..HEADER_LEN + 64], f.mint.as_ref());
    assert_eq!(
        &state[HEADER_LEN + 64..HEADER_LEN + 96],
        f.vault_ata.as_ref()
    );

    let second_mint = Pubkey::new_unique();
    let second_ata = associated_address(&f.authority, &second_mint);
    f.bank.insert(second_mint, token_2022_mint(&f.authority));
    f.bank
        .insert(second_ata, Account::new(0, 0, &Pubkey::default()));
    let bound_state = f.bank[&f.state].clone();
    let rebind = prepare_ix(&f, f.authority, second_mint, second_ata);
    let result = process(&mut f, &rebind);
    assert!(
        !result.succeeded(),
        "an established vault cannot be rebound"
    );
    assert_eq!(result.raw().get_account(&f.state), Some(&bound_state));
    assert_eq!(result.raw().get_account(&second_ata).unwrap().lamports, 0);
    assert!(
        !f.svm.logs().iter().any(|line| {
            line == &format!(
                "Program {} invoke [2]",
                mollusk_svm_programs_token::associated_token::ID
            )
        }),
        "rebind must fail before ATA CPI"
    );

    let unauthorized_mint = instruction(
        f.program_id,
        MINT_TAG,
        &10u64.to_le_bytes(),
        vec![
            AccountMeta::new_readonly(f.attacker, true),
            AccountMeta::new(f.state, false),
            AccountMeta::new(f.vault_ata, false),
            AccountMeta::new(f.mint, false),
            AccountMeta::new_readonly(mollusk_svm_programs_token::token2022::ID, false),
        ],
    );
    let state_before = f.bank[&f.state].clone();
    let vault_before = f.bank[&f.vault_ata].clone();
    let mint_before = f.bank[&f.mint].clone();
    let result = process(&mut f, &unauthorized_mint);
    assert!(
        !result.succeeded(),
        "stored authority must gate MintRewards"
    );
    assert_eq!(result.raw().get_account(&f.state), Some(&state_before));
    assert_eq!(result.raw().get_account(&f.vault_ata), Some(&vault_before));
    assert_eq!(result.raw().get_account(&f.mint), Some(&mint_before));
    assert!(
        !f.svm.logs().iter().any(|line| {
            line == &format!(
                "Program {} invoke [2]",
                mollusk_svm_programs_token::token2022::ID
            )
        }),
        "unauthorized mint must fail before Token-2022 CPI"
    );

    let mint = instruction(
        f.program_id,
        MINT_TAG,
        &10u64.to_le_bytes(),
        vec![
            AccountMeta::new_readonly(f.authority, true),
            AccountMeta::new(f.state, false),
            AccountMeta::new(f.vault_ata, false),
            AccountMeta::new(f.mint, false),
            AccountMeta::new_readonly(mollusk_svm_programs_token::token2022::ID, false),
        ],
    );
    let result = process(&mut f, &mint);
    assert!(result.succeeded(), "mint failed: {:#?}", f.svm.logs());
    assert_eq!(token_amount(&f.bank[&f.vault_ata]), 10);
    assert_eq!(read_u64(&f.bank[&f.state], MINTED_TOTAL_OFFSET), 10);
    assert!(f.svm.logs().iter().any(|line| {
        line == &format!(
            "Program {} invoke [2]",
            mollusk_svm_programs_token::token2022::ID
        )
    }));

    let sweep = instruction(
        f.program_id,
        SWEEP_TAG,
        &4u64.to_le_bytes(),
        vec![
            AccountMeta::new_readonly(f.authority, true),
            AccountMeta::new(f.state, false),
            AccountMeta::new(f.vault_ata, false),
            AccountMeta::new(f.destination, false),
            AccountMeta::new_readonly(f.mint, false),
            AccountMeta::new_readonly(mollusk_svm_programs_token::token2022::ID, false),
        ],
    );
    let result = process(&mut f, &sweep);
    assert!(result.succeeded(), "sweep failed: {:#?}", f.svm.logs());
    assert_eq!(token_amount(&f.bank[&f.vault_ata]), 6);
    assert_eq!(token_amount(&f.bank[&f.destination]), 4);
    assert_eq!(read_u64(&f.bank[&f.state], SWEPT_TOTAL_OFFSET), 4);
    assert!(f.svm.logs().iter().any(|line| {
        line == &format!(
            "Program {} invoke [2]",
            mollusk_svm_programs_token::token2022::ID
        )
    }));
}
