extern crate std;
use {
    alloc::vec,
    quasar_escrow_client::*,
    quasar_svm::{Account, Instruction, Pubkey, QuasarSvm},
    spl_token_interface::state::{Account as TokenAccount, AccountState, Mint},
    std::println,
};

fn setup() -> QuasarSvm {
    let elf = std::fs::read("../../target/deploy/quasar_escrow.so").unwrap();
    QuasarSvm::new()
        .with_program(&crate::ID, &elf)
        .with_token_program()
}

fn signer(address: Pubkey) -> Account {
    quasar_svm::token::create_keyed_system_account(&address, 1_000_000_000)
}

fn empty(address: Pubkey) -> Account {
    Account {
        address,
        lamports: 0,
        data: vec![],
        owner: quasar_svm::system_program::ID,
        executable: false,
    }
}

fn mint(address: Pubkey, authority: Pubkey) -> Account {
    quasar_svm::token::create_keyed_mint_account(
        &address,
        &Mint {
            mint_authority: Some(authority).into(),
            supply: 1_000_000_000,
            decimals: 9,
            is_initialized: true,
            freeze_authority: None.into(),
        },
    )
}

fn token(address: Pubkey, mint: Pubkey, owner: Pubkey, amount: u64) -> Account {
    quasar_svm::token::create_keyed_token_account(
        &address,
        &TokenAccount {
            mint,
            owner,
            amount,
            state: AccountState::Initialized,
            ..TokenAccount::default()
        },
    )
}

fn escrow_account(
    address: Pubkey,
    maker: Pubkey,
    mint_a: Pubkey,
    mint_b: Pubkey,
    maker_ta_b: Pubkey,
    receive: u64,
    bump: u8,
) -> Account {
    let escrow = Escrow {
        maker,
        mint_a,
        mint_b,
        maker_ta_b,
        receive,
        bump,
    };
    Account {
        address,
        lamports: 2_000_000,
        data: wincode::serialize(&escrow).unwrap(),
        owner: crate::ID,
        executable: false,
    }
}

/// Mark specific account indices as signers on an instruction.
fn with_signers(mut ix: Instruction, indices: &[usize]) -> Instruction {
    for &i in indices {
        ix.accounts[i].is_signer = true;
    }
    ix
}

#[test]
fn test_make_cu() {
    let mut svm = setup();

    let token_program = quasar_svm::SPL_TOKEN_PROGRAM_ID;
    let system_program = quasar_svm::system_program::ID;
    let maker = Pubkey::new_unique();
    let mint_a = Pubkey::new_unique();
    let mint_b = Pubkey::new_unique();
    let maker_ta_a = Pubkey::new_unique();
    let maker_ta_b = Pubkey::new_unique();
    let vault_ta_a = Pubkey::new_unique();
    let (escrow, escrow_bump) =
        Pubkey::find_program_address(&[b"escrow", maker.as_ref()], &crate::ID);
    let rent = quasar_svm::solana_sdk_ids::sysvar::rent::ID;

    let instruction = with_signers(
        MakeInstruction {
            maker,
            escrow,
            mint_a,
            mint_b,
            maker_ta_a,
            maker_ta_b,
            vault_ta_a,
            rent,
            token_program,
            system_program,
            deposit: 1337,
            receive: 1337,
        }
        .into(),
        &[5, 6], // maker_ta_b, vault_ta_a as signers for create_account CPI
    );

    let result = svm.process_instruction(
        &instruction,
        &[
            signer(maker),
            empty(escrow),
            mint(mint_a, maker),
            mint(mint_b, maker),
            token(maker_ta_a, mint_a, maker, 1_000_000),
            empty(maker_ta_b),
            empty(vault_ta_a),
        ],
    );

    assert!(result.is_ok(), "make failed: {:?}", result.raw_result);

    // Verify escrow state
    let escrow_data = &result.account(&escrow).unwrap().data;
    assert_eq!(escrow_data[0], 1, "discriminator");
    assert_eq!(&escrow_data[1..33], maker.as_ref(), "maker");
    assert_eq!(&escrow_data[129..137], &1337u64.to_le_bytes(), "receive");
    assert_eq!(escrow_data[137], escrow_bump, "bump");

    println!("  MAKE CU: {}", result.compute_units_consumed);
}

#[test]
fn test_take_cu() {
    let mut svm = setup();

    let token_program = quasar_svm::SPL_TOKEN_PROGRAM_ID;
    let system_program = quasar_svm::system_program::ID;
    let maker = Pubkey::new_unique();
    let taker = Pubkey::new_unique();
    let mint_a = Pubkey::new_unique();
    let mint_b = Pubkey::new_unique();
    let taker_ta_a = Pubkey::new_unique();
    let taker_ta_b = Pubkey::new_unique();
    let maker_ta_b = Pubkey::new_unique();
    let vault_ta_a = Pubkey::new_unique();
    let (escrow, escrow_bump) =
        Pubkey::find_program_address(&[b"escrow", maker.as_ref()], &crate::ID);
    let rent = quasar_svm::solana_sdk_ids::sysvar::rent::ID;

    let instruction = with_signers(
        TakeInstruction {
            taker,
            escrow,
            maker,
            mint_a,
            mint_b,
            taker_ta_a,
            taker_ta_b,
            maker_ta_b,
            vault_ta_a,
            rent,
            token_program,
            system_program,
        }
        .into(),
        &[5, 7], // taker_ta_a, maker_ta_b as signers for create_account CPI
    );

    let result = svm.process_instruction(
        &instruction,
        &[
            signer(taker),
            escrow_account(escrow, maker, mint_a, mint_b, maker_ta_b, 1337, escrow_bump),
            signer(maker),
            mint(mint_a, maker),
            mint(mint_b, maker),
            empty(taker_ta_a),
            token(taker_ta_b, mint_b, taker, 10_000),
            empty(maker_ta_b),
            token(vault_ta_a, mint_a, escrow, 1337),
        ],
    );

    assert!(result.is_ok(), "take failed: {:?}", result.raw_result);
    println!("  TAKE CU: {}", result.compute_units_consumed);
}

#[test]
fn test_refund_cu() {
    let mut svm = setup();

    let token_program = quasar_svm::SPL_TOKEN_PROGRAM_ID;
    let system_program = quasar_svm::system_program::ID;
    let maker = Pubkey::new_unique();
    let mint_a = Pubkey::new_unique();
    let mint_b = Pubkey::new_unique();
    let maker_ta_a = Pubkey::new_unique();
    let maker_ta_b = Pubkey::new_unique();
    let vault_ta_a = Pubkey::new_unique();
    let (escrow, escrow_bump) =
        Pubkey::find_program_address(&[b"escrow", maker.as_ref()], &crate::ID);
    let rent = quasar_svm::solana_sdk_ids::sysvar::rent::ID;

    let instruction = with_signers(
        RefundInstruction {
            maker,
            escrow,
            mint_a,
            maker_ta_a,
            vault_ta_a,
            rent,
            token_program,
            system_program,
        }
        .into(),
        &[3], // maker_ta_a as signer for create_account CPI
    );

    let result = svm.process_instruction(
        &instruction,
        &[
            signer(maker),
            escrow_account(escrow, maker, mint_a, mint_b, maker_ta_b, 1337, escrow_bump),
            mint(mint_a, maker),
            empty(maker_ta_a),
            token(vault_ta_a, mint_a, escrow, 1337),
        ],
    );

    assert!(result.is_ok(), "refund failed: {:?}", result.raw_result);
    println!("  REFUND CU: {}", result.compute_units_consumed);
}

// ---------------------------------------------------------------------------
// init_if_needed: pre-existing token accounts
// ---------------------------------------------------------------------------

#[test]
fn test_make_existing_token_accounts() {
    let mut svm = setup();

    let token_program = quasar_svm::SPL_TOKEN_PROGRAM_ID;
    let system_program = quasar_svm::system_program::ID;
    let maker = Pubkey::new_unique();
    let mint_a = Pubkey::new_unique();
    let mint_b = Pubkey::new_unique();
    let maker_ta_a = Pubkey::new_unique();
    let maker_ta_b = Pubkey::new_unique();
    let vault_ta_a = Pubkey::new_unique();
    let (escrow, _) = Pubkey::find_program_address(&[b"escrow", maker.as_ref()], &crate::ID);
    let rent = quasar_svm::solana_sdk_ids::sysvar::rent::ID;

    let instruction: Instruction = MakeInstruction {
        maker,
        escrow,
        mint_a,
        mint_b,
        maker_ta_a,
        maker_ta_b,
        vault_ta_a,
        rent,
        token_program,
        system_program,
        deposit: 1337,
        receive: 1337,
    }
    .into();

    let result = svm.process_instruction(
        &instruction,
        &[
            signer(maker),
            empty(escrow),
            mint(mint_a, maker),
            mint(mint_b, maker),
            token(maker_ta_a, mint_a, maker, 1_000_000),
            token(maker_ta_b, mint_b, maker, 0),
            token(vault_ta_a, mint_a, escrow, 0),
        ],
    );

    assert!(
        result.is_ok(),
        "make with existing token accounts failed: {:?}",
        result.raw_result
    );
    println!(
        "  make with existing token accounts: OK (CU: {})",
        result.compute_units_consumed
    );
}

#[test]
fn test_make_existing_maker_ta_b_wrong_mint() {
    let mut svm = setup();

    let token_program = quasar_svm::SPL_TOKEN_PROGRAM_ID;
    let system_program = quasar_svm::system_program::ID;
    let maker = Pubkey::new_unique();
    let mint_a = Pubkey::new_unique();
    let mint_b = Pubkey::new_unique();
    let maker_ta_a = Pubkey::new_unique();
    let maker_ta_b = Pubkey::new_unique();
    let vault_ta_a = Pubkey::new_unique();
    let (escrow, _) = Pubkey::find_program_address(&[b"escrow", maker.as_ref()], &crate::ID);
    let rent = quasar_svm::solana_sdk_ids::sysvar::rent::ID;

    let instruction: Instruction = MakeInstruction {
        maker,
        escrow,
        mint_a,
        mint_b,
        maker_ta_a,
        maker_ta_b,
        vault_ta_a,
        rent,
        token_program,
        system_program,
        deposit: 1337,
        receive: 1337,
    }
    .into();

    let result = svm.process_instruction(
        &instruction,
        &[
            signer(maker),
            empty(escrow),
            mint(mint_a, maker),
            mint(mint_b, maker),
            token(maker_ta_a, mint_a, maker, 1_000_000),
            token(maker_ta_b, mint_a, maker, 0), // wrong mint
            token(vault_ta_a, mint_a, escrow, 0),
        ],
    );

    assert!(
        result.is_err(),
        "make should fail with wrong mint on maker_ta_b"
    );
}

#[test]
fn test_make_existing_maker_ta_b_wrong_owner() {
    let mut svm = setup();

    let token_program = quasar_svm::SPL_TOKEN_PROGRAM_ID;
    let system_program = quasar_svm::system_program::ID;
    let maker = Pubkey::new_unique();
    let mint_a = Pubkey::new_unique();
    let mint_b = Pubkey::new_unique();
    let maker_ta_a = Pubkey::new_unique();
    let maker_ta_b = Pubkey::new_unique();
    let vault_ta_a = Pubkey::new_unique();
    let wrong_owner = Pubkey::new_unique();
    let (escrow, _) = Pubkey::find_program_address(&[b"escrow", maker.as_ref()], &crate::ID);
    let rent = quasar_svm::solana_sdk_ids::sysvar::rent::ID;

    let instruction: Instruction = MakeInstruction {
        maker,
        escrow,
        mint_a,
        mint_b,
        maker_ta_a,
        maker_ta_b,
        vault_ta_a,
        rent,
        token_program,
        system_program,
        deposit: 1337,
        receive: 1337,
    }
    .into();

    let result = svm.process_instruction(
        &instruction,
        &[
            signer(maker),
            empty(escrow),
            mint(mint_a, maker),
            mint(mint_b, maker),
            token(maker_ta_a, mint_a, maker, 1_000_000),
            token(maker_ta_b, mint_b, wrong_owner, 0), // wrong owner
            token(vault_ta_a, mint_a, escrow, 0),
        ],
    );

    assert!(
        result.is_err(),
        "make should fail with wrong owner on maker_ta_b"
    );
}

#[test]
fn test_take_existing_token_accounts() {
    let mut svm = setup();

    let token_program = quasar_svm::SPL_TOKEN_PROGRAM_ID;
    let system_program = quasar_svm::system_program::ID;
    let maker = Pubkey::new_unique();
    let taker = Pubkey::new_unique();
    let mint_a = Pubkey::new_unique();
    let mint_b = Pubkey::new_unique();
    let taker_ta_a = Pubkey::new_unique();
    let taker_ta_b = Pubkey::new_unique();
    let maker_ta_b = Pubkey::new_unique();
    let vault_ta_a = Pubkey::new_unique();
    let (escrow, escrow_bump) =
        Pubkey::find_program_address(&[b"escrow", maker.as_ref()], &crate::ID);
    let rent = quasar_svm::solana_sdk_ids::sysvar::rent::ID;

    let instruction: Instruction = TakeInstruction {
        taker,
        escrow,
        maker,
        mint_a,
        mint_b,
        taker_ta_a,
        taker_ta_b,
        maker_ta_b,
        vault_ta_a,
        rent,
        token_program,
        system_program,
    }
    .into();

    let result = svm.process_instruction(
        &instruction,
        &[
            signer(taker),
            escrow_account(escrow, maker, mint_a, mint_b, maker_ta_b, 1337, escrow_bump),
            signer(maker),
            mint(mint_a, maker),
            mint(mint_b, maker),
            token(taker_ta_a, mint_a, taker, 0),
            token(taker_ta_b, mint_b, taker, 10_000),
            token(maker_ta_b, mint_b, maker, 500),
            token(vault_ta_a, mint_a, escrow, 1337),
        ],
    );

    assert!(
        result.is_ok(),
        "take with existing token accounts failed: {:?}",
        result.raw_result
    );
    println!(
        "  take with existing token accounts: OK (CU: {})",
        result.compute_units_consumed
    );
}

#[test]
fn test_refund_existing_maker_ta_a() {
    let mut svm = setup();

    let token_program = quasar_svm::SPL_TOKEN_PROGRAM_ID;
    let system_program = quasar_svm::system_program::ID;
    let maker = Pubkey::new_unique();
    let mint_a = Pubkey::new_unique();
    let mint_b = Pubkey::new_unique();
    let maker_ta_a = Pubkey::new_unique();
    let maker_ta_b = Pubkey::new_unique();
    let vault_ta_a = Pubkey::new_unique();
    let (escrow, escrow_bump) =
        Pubkey::find_program_address(&[b"escrow", maker.as_ref()], &crate::ID);
    let rent = quasar_svm::solana_sdk_ids::sysvar::rent::ID;

    let instruction: Instruction = RefundInstruction {
        maker,
        escrow,
        mint_a,
        maker_ta_a,
        vault_ta_a,
        rent,
        token_program,
        system_program,
    }
    .into();

    let result = svm.process_instruction(
        &instruction,
        &[
            signer(maker),
            escrow_account(escrow, maker, mint_a, mint_b, maker_ta_b, 1337, escrow_bump),
            mint(mint_a, maker),
            token(maker_ta_a, mint_a, maker, 5_000),
            token(vault_ta_a, mint_a, escrow, 1337),
        ],
    );

    assert!(
        result.is_ok(),
        "refund with existing maker_ta_a failed: {:?}",
        result.raw_result
    );
    println!(
        "  refund with existing maker_ta_a: OK (CU: {})",
        result.compute_units_consumed
    );
}
