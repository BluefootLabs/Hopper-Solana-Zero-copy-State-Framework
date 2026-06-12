extern crate std;

use {
    hopper::{layout, prelude::{Address, WireU64}},
    hopper_svm::{AccountFixture, HopperSvm, ProcessResult},
    std::{println, vec, vec::Vec},
};

fn process_instruction(
    program_id: Address,
    instruction_data: &[u8],
    accounts: &[AccountFixture],
) -> ProcessResult {
    HopperSvm::new().process_instruction(
        program_id,
        instruction_data,
        accounts,
        super::__hopper_process_instruction_vault_program,
    )
}

fn amount_instruction(discriminator: u8, amount: u64) -> Vec<u8> {
    let mut data = vec![discriminator];
    data.extend_from_slice(&amount.to_le_bytes());
    data
}

fn address(seed: u8) -> Address {
    Address::new_from_array([seed; 32])
}

fn system_program() -> Address {
    Address::new_from_array([0; 32])
}

fn seeded_user_account(address: Address, lamports: u64, is_signer: bool) -> AccountFixture {
    let account = AccountFixture::new(address, system_program(), lamports, 0).writable();
    if is_signer {
        account.signer()
    } else {
        account
    }
}

fn seeded_vault_account(
    address: Address,
    program_id: Address,
    authority: Address,
    lamports: u64,
    balance: u64,
) -> AccountFixture {
    let mut data = vec![0; crate::Vault::LEN];
    layout::write_header(
        &mut data,
        crate::Vault::DISC,
        crate::Vault::VERSION,
        &crate::Vault::LAYOUT_ID,
    )
    .unwrap();
    let vault = crate::Vault::overlay_mut(&mut data[layout::HEADER_LEN..]).unwrap();
    vault.authority = authority;
    vault.balance = WireU64::new(balance);
    vault.bump = 0;
    AccountFixture::with_data(address, program_id, lamports, data).writable()
}

#[test]
fn test_deposit() {
    let program_id = address(9);
    let user = address(1);
    let vault = address(2);

    let user_before = seeded_user_account(user, 10_000_000_000, true);
    let vault_before = seeded_vault_account(vault, program_id, user, 1_000_000_000, 0);

    let deposit_amount = 1_000_000_000u64;
    let result = process_instruction(
        program_id,
        &amount_instruction(1, deposit_amount),
        &[user_before.clone(), vault_before.clone()],
    );

    assert!(
        result.program_result.is_ok(),
        "deposit failed: {:?}",
        result.program_result
    );

    let user_after = result.resulting_accounts[0].lamports;
    let vault_after = result.resulting_accounts[1].lamports;

    assert_eq!(
        user_after,
        user_before.lamports - deposit_amount,
        "user lamports after deposit"
    );
    assert_eq!(
        vault_after,
        vault_before.lamports + deposit_amount,
        "vault lamports after deposit"
    );

    println!("  DEPOSIT CU: {}", result.compute_units_consumed);
}

#[test]
fn test_withdraw() {
    let program_id = address(19);
    let user = address(11);
    let vault = address(12);

    let user_before = seeded_user_account(user, 10_000_000_000, true);
    let vault_before = seeded_vault_account(vault, program_id, user, 1_000_000_000, 0);

    let deposit_amount = 1_000_000_000u64;
    let deposit_result = process_instruction(
        program_id,
        &amount_instruction(1, deposit_amount),
        &[user_before.clone(), vault_before.clone()],
    );

    assert!(
        deposit_result.program_result.is_ok(),
        "deposit failed: {:?}",
        deposit_result.program_result
    );

    let user_after_deposit = deposit_result.resulting_accounts[0].clone();
    let vault_after_deposit = deposit_result.resulting_accounts[1].clone();

    let withdraw_amount = 500_000_000u64;
    let withdraw_result = process_instruction(
        program_id,
        &amount_instruction(2, withdraw_amount),
        &[user_after_deposit.clone(), vault_after_deposit.clone()],
    );

    assert!(
        withdraw_result.program_result.is_ok(),
        "withdraw failed: {:?}",
        withdraw_result.program_result
    );

    let user_final = withdraw_result.resulting_accounts[0].lamports;
    let vault_final = withdraw_result.resulting_accounts[1].lamports;

    assert_eq!(
        user_final,
        user_after_deposit.lamports + withdraw_amount,
        "user lamports after withdraw"
    );
    assert_eq!(
        vault_final,
        vault_after_deposit.lamports - withdraw_amount,
        "vault lamports after withdraw"
    );

    println!("  WITHDRAW CU: {}", withdraw_result.compute_units_consumed);
}

#[test]
fn test_withdraw_rejects_unsigned_user() {
    let program_id = address(29);
    let user = address(21);
    let vault = address(22);

    let user_before = seeded_user_account(user, 10_000_000_000, true);
    let vault_before = seeded_vault_account(vault, program_id, user, 1_000_000_000, 0);

    let deposit_amount = 1_000_000_000u64;
    let deposit_result = process_instruction(
        program_id,
        &amount_instruction(1, deposit_amount),
        &[user_before.clone(), vault_before.clone()],
    );

    assert!(
        deposit_result.program_result.is_ok(),
        "deposit failed: {:?}",
        deposit_result.program_result
    );

    let mut user_after_deposit = deposit_result.resulting_accounts[0].clone();
    user_after_deposit.is_signer = false;
    let vault_after_deposit = deposit_result.resulting_accounts[1].clone();

    let withdraw_amount = 500_000_000u64;
    let withdraw_result = process_instruction(
        program_id,
        &amount_instruction(2, withdraw_amount),
        &[user_after_deposit.clone(), vault_after_deposit.clone()],
    );

    assert!(
        withdraw_result.program_result.is_err(),
        "withdraw without signer unexpectedly succeeded"
    );
    assert_eq!(
        withdraw_result.resulting_accounts[0].lamports, user_after_deposit.lamports,
        "unsigned withdraw mutated the authority account"
    );
    assert_eq!(
        withdraw_result.resulting_accounts[1].lamports, vault_after_deposit.lamports,
        "unsigned withdraw mutated the vault account"
    );
}
