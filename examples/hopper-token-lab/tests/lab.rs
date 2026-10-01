//! Compiled (SBF) run of the token lab against Mollusk's SPL Token and
//! Token-2022 programs. Build `hopper_token_lab.so` first
//! (`cargo build-sbf` in this directory); the test skips without it.

use std::collections::BTreeMap;

use hopper_test::{HarnessResult, LiteSvmHarness};
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

const ELF_PATH_STEM: &str = "../../target/deploy/hopper_token_lab";

struct Fixture {
    svm: LiteSvmHarness,
    program_id: Pubkey,
    payer: Pubkey,
    bank: BTreeMap<Pubkey, Account>,
}

fn setup() -> Option<Fixture> {
    let program_id = Pubkey::new_unique();
    let path = std::env::var("HOPPER_TOKEN_LAB_SBF").unwrap_or_else(|_| ELF_PATH_STEM.into());
    let Some(mut svm) = LiteSvmHarness::load(&program_id, &path) else {
        assert!(
            std::env::var_os("HOPPER_TOKEN_LAB_SBF").is_none(),
            "explicit SBF fixture missing: {path}"
        );
        eprintln!("SKIPPED: build {ELF_PATH_STEM}.so first");
        return None;
    };
    mollusk_svm_programs_token::token::add_program(svm.mollusk_mut());
    mollusk_svm_programs_token::token2022::add_program(svm.mollusk_mut());
    let payer = Pubkey::new_unique();
    let mut bank = BTreeMap::new();
    bank.insert(payer, Account::new(1_000_000_000, 0, &Pubkey::default()));
    bank.insert(
        mollusk_svm_programs_token::token::ID,
        mollusk_svm_programs_token::token::account(),
    );
    bank.insert(
        mollusk_svm_programs_token::token2022::ID,
        mollusk_svm_programs_token::token2022::account(),
    );
    let (system, system_account) = LiteSvmHarness::system_program_account();
    bank.insert(system, system_account);
    Some(Fixture {
        svm,
        program_id,
        payer,
        bank,
    })
}

fn process(f: &mut Fixture, tag: u8, payload: &[u8], accounts: Vec<AccountMeta>) -> HarnessResult {
    let mut data = vec![tag];
    data.extend_from_slice(payload);
    let instruction = Instruction::new_with_bytes(f.program_id, &data, accounts);
    let mut seeds: Vec<(Pubkey, Account)> = Vec::new();
    for meta in &instruction.accounts {
        if seeds.iter().any(|(k, _)| k == &meta.pubkey) {
            continue;
        }
        let account = f
            .bank
            .get(&meta.pubkey)
            .cloned()
            .unwrap_or_else(|| Account::new(0, 0, &Pubkey::default()));
        seeds.push((meta.pubkey, account));
    }
    f.svm.capture_logs();
    let result = f.svm.process(&instruction, &seeds);
    if result.succeeded() {
        for (key, account) in &result.raw().resulting_accounts {
            f.bank.insert(*key, account.clone());
        }
    }
    result
}

fn create_mint(f: &mut Fixture, token: Pubkey) -> Pubkey {
    let mint = Pubkey::new_unique();
    let payer = f.payer;
    let result = process(
        f,
        0,
        &[],
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(mint, true),
            AccountMeta::new_readonly(Pubkey::default(), false),
            AccountMeta::new_readonly(token, false),
        ],
    );
    assert!(
        result.succeeded(),
        "create_mint on {token} failed: {:#?}",
        f.svm.logs()
    );
    let account = &f.bank[&mint];
    assert_eq!(account.owner, token);
    assert_eq!(account.data.len(), 82);
    assert_eq!(account.data[44], 6);
    mint
}

fn immutable_account(f: &mut Fixture, token: Pubkey, mint: Pubkey) -> Pubkey {
    let account = Pubkey::new_unique();
    let payer = f.payer;
    let result = process(
        f,
        2,
        &[],
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(account, true),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(payer, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
            AccountMeta::new_readonly(token, false),
        ],
    );
    assert!(
        result.succeeded(),
        "immutable_account on {token} failed: {:#?}",
        f.svm.logs()
    );
    let data = &f.bank[&account].data;
    assert_eq!(&data[..32], mint.as_ref());
    assert_eq!(&data[32..64], payer.as_ref());
    if token == mollusk_svm_programs_token::token2022::ID {
        assert_eq!(data.len(), 170);
        assert_eq!(data[165], 2);
        assert_eq!(&data[166..170], &[7, 0, 0, 0]);
    } else {
        assert_eq!(data.len(), 165);
    }
    account
}

#[test]
fn lanes_on_both_programs() {
    let Some(mut f) = setup() else { return };
    let payer = f.payer;
    for token in [
        mollusk_svm_programs_token::token::ID,
        mollusk_svm_programs_token::token2022::ID,
    ] {
        let mint = create_mint(&mut f, token);
        let a = immutable_account(&mut f, token, mint);
        let b = immutable_account(&mut f, token, mint);

        let mut payload = 1_000_000u64.to_le_bytes().to_vec();
        payload.push(6);
        let result = process(
            &mut f,
            3,
            &payload,
            vec![
                AccountMeta::new(mint, false),
                AccountMeta::new(a, false),
                AccountMeta::new_readonly(payer, true),
                AccountMeta::new_readonly(token, false),
            ],
        );
        assert!(result.succeeded(), "mint_to failed: {:#?}", f.svm.logs());
        assert_eq!(&f.bank[&a].data[64..72], &1_000_000u64.to_le_bytes());

        let before = f.bank[&a].clone();
        let rejected = process(
            &mut f,
            16,
            &payload,
            vec![
                AccountMeta::new(mint, false),
                AccountMeta::new(a, false),
                AccountMeta::new_readonly(payer, true),
                AccountMeta::new_readonly(token, false),
            ],
        );
        assert!(
            !rejected.succeeded(),
            "batched self-transfer must fail before a CPI"
        );
        assert_eq!(
            format!("{:?}", rejected.raw().program_result),
            "Failure(AccountBorrowFailed)"
        );
        assert!(!f
            .svm
            .logs()
            .iter()
            .any(|line| line == &format!("Program {token} invoke [2]")));
        assert_eq!(f.bank[&a], before);

        let mut payload = 1_234_567u64.to_le_bytes().to_vec();
        let result = process(
            &mut f,
            5,
            &payload,
            vec![
                AccountMeta::new_readonly(mint, false),
                AccountMeta::new_readonly(token, false),
            ],
        );
        assert!(
            result.succeeded(),
            "ui round trip failed: {:#?}",
            f.svm.logs()
        );
        let returned = result.raw().return_data.clone();
        assert_eq!(&returned[..8], &1_234_567u64.to_le_bytes());
        assert_eq!(&returned[8..], b"1.234567");

        payload = 250_000u64.to_le_bytes().to_vec();
        payload.push(6);
        let result = process(
            &mut f,
            4,
            &payload,
            vec![
                AccountMeta::new(a, false),
                AccountMeta::new_readonly(mint, false),
                AccountMeta::new(b, false),
                AccountMeta::new_readonly(payer, true),
                AccountMeta::new_readonly(token, false),
            ],
        );
        // Mollusk's SPL Token is the p-token build, which has `Batch`; its
        // Token-2022 refuses discriminator 255 with InvalidInstruction (0xc).
        if token == mollusk_svm_programs_token::token::ID {
            assert!(result.succeeded(), "batch failed: {:#?}", f.svm.logs());
            let invocations = f
                .svm
                .logs()
                .iter()
                .filter(|line| *line == &format!("Program {token} invoke [2]"))
                .count();
            assert_eq!(invocations, 1, "two transfers must be one token CPI");
            assert_eq!(&f.bank[&a].data[64..72], &1_000_000u64.to_le_bytes());
            assert_eq!(&f.bank[&b].data[64..72], &0u64.to_le_bytes());
        } else {
            assert!(
                !result.succeeded(),
                "Token-2022 accepted Batch: {:#?}",
                f.svm.logs()
            );
            eprintln!("batch on {token}: refused, logs={:#?}", f.svm.logs());
        }

        let multisig = Pubkey::new_unique();
        let result = process(
            &mut f,
            7,
            &[1],
            vec![
                AccountMeta::new(payer, true),
                AccountMeta::new(multisig, true),
                AccountMeta::new_readonly(payer, false),
                AccountMeta::new_readonly(a, false),
                AccountMeta::new_readonly(Pubkey::default(), false),
                AccountMeta::new_readonly(token, false),
            ],
        );
        assert!(result.succeeded(), "multisig failed: {:#?}", f.svm.logs());
        let data = &f.bank[&multisig].data;
        assert_eq!(data.len(), 355);
        assert_eq!(&data[..3], &[1, 2, 1]);
    }
}

#[test]
fn hook_wire_validation_in_compiled_program() {
    let Some(mut f) = setup() else { return };
    let token = mollusk_svm_programs_token::token2022::ID;
    let mint = create_mint(&mut f, token);
    let mut valid = [0u8; 51];
    valid[..8].copy_from_slice(&[105, 37, 101, 197, 75, 251, 102, 26]);
    valid[8..12].copy_from_slice(&39u32.to_le_bytes());
    valid[12..16].copy_from_slice(&1u32.to_le_bytes());
    valid[17..49].copy_from_slice(mint.as_ref());
    let mut pda_wire = valid;
    pda_wire[16] = 1;
    pda_wire[17..49].fill(0);
    pda_wire[17..19].copy_from_slice(&[3, 0]);
    let derived = process(
        &mut f,
        17,
        &pda_wire,
        vec![
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(token, false),
        ],
    );
    assert!(derived.succeeded(), "{:#?}", f.svm.logs());
    assert_eq!(
        derived.raw().return_data,
        Pubkey::find_program_address(&[mint.as_ref()], &token)
            .0
            .to_bytes()
    );
    let mut cases = vec![(valid, None)];
    let mut bad = valid;
    bad[..8].fill(9);
    cases.push((bad, Some(6700)));
    let mut bad = valid;
    bad[8..12].copy_from_slice(&4u32.to_le_bytes());
    cases.push((bad, Some(6702)));
    let mut bad = valid;
    bad[8..12].copy_from_slice(&40u32.to_le_bytes());
    cases.push((bad, Some(6701)));
    let mut bad = valid;
    bad[16] = 3;
    bad[17..49].fill(0);
    cases.push((bad, Some(6703)));
    let mut bad = valid;
    bad[16] = 1;
    for seed in bad[17..49].chunks_exact_mut(2) {
        seed.copy_from_slice(&[3, 0]);
    }
    cases.push((bad, Some(6704)));
    let mut bad = valid;
    bad[16] = 1;
    bad[17..49].fill(0);
    bad[17..20].copy_from_slice(&[2, 0, 33]);
    cases.push((bad, Some(6703)));
    for (wire, expected) in cases {
        let result = process(
            &mut f,
            17,
            &wire,
            vec![
                AccountMeta::new_readonly(mint, false),
                AccountMeta::new_readonly(token, false),
            ],
        );
        if let Some(code) = expected {
            assert_eq!(
                format!("{:?}", result.raw().program_result),
                format!("Failure(Custom({code}))")
            );
        } else {
            assert!(result.succeeded(), "{:#?}", f.svm.logs());
            assert_eq!(result.raw().return_data, mint.to_bytes());
        }
    }
}
