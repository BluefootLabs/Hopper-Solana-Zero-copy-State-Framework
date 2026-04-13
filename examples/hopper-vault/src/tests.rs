extern crate std;

use {
    hopper::prelude::{TypedAddress, WireU64},
    mollusk_svm::Mollusk,
    solana_account::Account,
    solana_address::Address,
    solana_instruction::{AccountMeta, Instruction},
    std::{println, vec},
};

fn setup(program_id: &Address) -> Mollusk {
    Mollusk::new(program_id, "../../target/deploy/hopper_vault")
}

fn amount_instruction(
    discriminator: u8,
    program_id: Address,
    user: Address,
    user_is_signer: bool,
    vault: Address,
    amount: u64,
) -> Instruction {
    let mut data = vec![discriminator];
    data.extend_from_slice(&amount.to_le_bytes());
    Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(user, user_is_signer),
            AccountMeta::new(vault, false),
        ],
        data,
    }
}

fn seeded_user_account(program_id: &Address, lamports: u64) -> Account {
    Account::new(lamports, 0, program_id)
}

fn seeded_vault_account(program_id: &Address, authority: Address, lamports: u64, balance: u64) -> Account {
    let mut account = Account::new(lamports, crate::Vault::LEN, program_id);
    crate::Vault::write_init_header(&mut account.data).unwrap();
    let vault = crate::Vault::overlay_mut(&mut account.data).unwrap();
    let authority_bytes: &[u8; 32] = authority.as_ref().try_into().unwrap();
    vault.authority = TypedAddress::from_slice(authority_bytes);
    vault.balance = WireU64::new(balance);
    vault.bump = 0;
    account
}

#[test]
fn test_deposit() {
    let program_id = Address::new_unique();
    let mollusk = setup(&program_id);
    let user = Address::new_unique();
    let vault = Address::new_unique();

    let user_before = seeded_user_account(&program_id, 10_000_000_000);
    let vault_before = seeded_vault_account(&program_id, user, 1_000_000_000, 0);

    let deposit_amount = 1_000_000_000u64;
    let result = mollusk.process_instruction(
        &amount_instruction(1, program_id, user, true, vault, deposit_amount),
        &[(user, user_before.clone()), (vault, vault_before.clone())],
    );

    assert!(
        result.program_result.is_ok(),
        "deposit failed: {:?}",
        result.program_result
    );

    let user_after = result.resulting_accounts[0].1.lamports;
    let vault_after = result.resulting_accounts[1].1.lamports;

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
    let program_id = Address::new_unique();
    let mollusk = setup(&program_id);
    let user = Address::new_unique();
    let vault = Address::new_unique();

    let user_before = seeded_user_account(&program_id, 10_000_000_000);
    let vault_before = seeded_vault_account(&program_id, user, 1_000_000_000, 0);

    let deposit_amount = 1_000_000_000u64;
    let deposit_result = mollusk.process_instruction(
        &amount_instruction(1, program_id, user, true, vault, deposit_amount),
        &[(user, user_before.clone()), (vault, vault_before.clone())],
    );

    assert!(
        deposit_result.program_result.is_ok(),
        "deposit failed: {:?}",
        deposit_result.program_result
    );

    let user_after_deposit = deposit_result.resulting_accounts[0].1.clone();
    let vault_after_deposit = deposit_result.resulting_accounts[1].1.clone();

    let withdraw_amount = 500_000_000u64;
    let withdraw_result = mollusk.process_instruction(
        &amount_instruction(2, program_id, user, true, vault, withdraw_amount),
        &[
            (user, user_after_deposit.clone()),
            (vault, vault_after_deposit.clone()),
        ],
    );

    assert!(
        withdraw_result.program_result.is_ok(),
        "withdraw failed: {:?}",
        withdraw_result.program_result
    );

    let user_final = withdraw_result.resulting_accounts[0].1.lamports;
    let vault_final = withdraw_result.resulting_accounts[1].1.lamports;

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
    let program_id = Address::new_unique();
    let mollusk = setup(&program_id);
    let user = Address::new_unique();
    let vault = Address::new_unique();

    let user_before = seeded_user_account(&program_id, 10_000_000_000);
    let vault_before = seeded_vault_account(&program_id, user, 1_000_000_000, 0);

    let deposit_amount = 1_000_000_000u64;
    let deposit_result = mollusk.process_instruction(
        &amount_instruction(1, program_id, user, true, vault, deposit_amount),
        &[(user, user_before.clone()), (vault, vault_before.clone())],
    );

    assert!(
        deposit_result.program_result.is_ok(),
        "deposit failed: {:?}",
        deposit_result.program_result
    );

    let user_after_deposit = deposit_result.resulting_accounts[0].1.clone();
    let vault_after_deposit = deposit_result.resulting_accounts[1].1.clone();

    let withdraw_amount = 500_000_000u64;
    let withdraw_result = mollusk.process_instruction(
        &amount_instruction(2, program_id, user, false, vault, withdraw_amount),
        &[
            (user, user_after_deposit.clone()),
            (vault, vault_after_deposit.clone()),
        ],
    );

    assert!(
        withdraw_result.program_result.is_err(),
        "withdraw without signer unexpectedly succeeded"
    );
    assert_eq!(
        withdraw_result.resulting_accounts[0].1.lamports,
        user_after_deposit.lamports,
        "unsigned withdraw mutated the authority account"
    );
    assert_eq!(
        withdraw_result.resulting_accounts[1].1.lamports,
        vault_after_deposit.lamports,
        "unsigned withdraw mutated the vault account"
    );
}