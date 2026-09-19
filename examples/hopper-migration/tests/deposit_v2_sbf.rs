//! Compiled-SBF regression for `DepositV2` (instruction tag 2).
//!
//! Build the program first with:
//! `cargo build-sbf --manifest-path examples/hopper-migration/Cargo.toml -- --locked`.

use hopper::layout::{write_header, HEADER_LEN};
use hopper_migration::VaultV2;
use hopper_test::LiteSvmHarness;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;

const ELF_PATH_STEM: &str = "../../target/deploy/hopper_migration";
const DEPOSIT_V2_TAG: u8 = 2;

fn harness(program_id: &Pubkey) -> Option<LiteSvmHarness> {
    let harness = LiteSvmHarness::load(program_id, ELF_PATH_STEM);
    if harness.is_none() {
        if std::env::var("HOPPER_REQUIRE_MIGRATION_SBF").as_deref() == Ok("1") {
            panic!("required SBF artifact is missing: {ELF_PATH_STEM}.so");
        }
        eprintln!("SKIPPED: build {ELF_PATH_STEM}.so first");
    }
    harness
}

fn v2_account(program_id: &Pubkey, authority: &Pubkey, balance: u64, lamports: u64) -> Account {
    let mut account = Account::new(lamports, VaultV2::LEN, program_id);
    write_header(
        &mut account.data,
        VaultV2::DISC,
        VaultV2::VERSION,
        &VaultV2::LAYOUT_ID,
    )
    .unwrap();
    account.data[HEADER_LEN..HEADER_LEN + 32].copy_from_slice(authority.as_ref());
    account.data[HEADER_LEN + 32..HEADER_LEN + 40].copy_from_slice(&balance.to_le_bytes());
    account
}

fn deposit_instruction(
    program_id: Pubkey,
    depositor: Pubkey,
    vault: Pubkey,
    amount: u64,
) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(DEPOSIT_V2_TAG);
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction::new_with_bytes(
        program_id,
        &data,
        vec![
            AccountMeta::new(depositor, true),
            AccountMeta::new(vault, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
        ],
    )
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
}

#[test]
fn compiled_deposit_v2_enters_system_program_and_updates_lamports_and_state() {
    let program_id = Pubkey::new_unique();
    let Some(mut svm) = harness(&program_id) else {
        return;
    };
    let depositor = Pubkey::new_unique();
    let vault = Pubkey::new_unique();
    let amount = 125;
    let depositor_before = Account::new(5_000, 0, &Pubkey::default());
    let vault_before = v2_account(&program_id, &depositor, 25, 10_000);
    let accounts = vec![
        (depositor, depositor_before),
        (vault, vault_before),
        LiteSvmHarness::system_program_account(),
    ];

    svm.capture_logs();
    let result = svm.process(
        &deposit_instruction(program_id, depositor, vault, amount),
        &accounts,
    );
    assert!(result.succeeded(), "deposit failed: {:#?}", svm.logs());
    let depositor_after = result.raw().get_account(&depositor).unwrap();
    let vault_after = result.raw().get_account(&vault).unwrap();
    assert_eq!(depositor_after.lamports, 4_875);
    assert_eq!(vault_after.lamports, 10_125);
    assert_eq!(read_u64(&vault_after.data, HEADER_LEN + 32), 150);
    assert_eq!(read_u64(&vault_after.data, HEADER_LEN + 41), amount);
    assert!(
        svm.logs()
            .iter()
            .any(|line| line == "Program 11111111111111111111111111111111 invoke [2]"),
        "DepositV2 must enter the canonical System Program"
    );
}

#[test]
fn compiled_deposit_v2_insufficient_funds_rolls_back_every_account() {
    let program_id = Pubkey::new_unique();
    let Some(svm) = harness(&program_id) else {
        return;
    };
    let depositor = Pubkey::new_unique();
    let vault = Pubkey::new_unique();
    let depositor_before = Account::new(50, 0, &Pubkey::default());
    let vault_before = v2_account(&program_id, &depositor, 25, 10_000);
    let accounts = vec![
        (depositor, depositor_before.clone()),
        (vault, vault_before.clone()),
        LiteSvmHarness::system_program_account(),
    ];

    let result = svm.process(
        &deposit_instruction(program_id, depositor, vault, 125),
        &accounts,
    );
    assert!(!result.succeeded(), "insufficient transfer must fail");
    assert_eq!(
        result.raw().get_account(&depositor),
        Some(&depositor_before)
    );
    assert_eq!(result.raw().get_account(&vault), Some(&vault_before));
}
