#![cfg(feature = "proc-macros")]

use hopper::address::Address;

hopper::declare_program!(
    demo_manifest,
    "tests/fixtures/declare_program_manifest.json"
);

#[test]
fn declare_program_builds_borrowed_instruction_parts() {
    let program_id = Address::new([9u8; 32]);
    let accounts = demo_manifest::DepositAccounts {
        authority: Address::new([1u8; 32]),
        vault: Address::new([2u8; 32]),
    };
    let args = demo_manifest::DepositArgs {
        amount: 1_000_000,
        memo: *b"memo",
        authority_key: Address::new([3u8; 32]),
    };

    let built = demo_manifest::deposit(&program_id, &accounts, args);
    assert_eq!(built.data[0], 7);
    assert_eq!(
        u64::from_le_bytes(built.data[1..9].try_into().unwrap()),
        1_000_000
    );
    assert_eq!(&built.data[9..13], b"memo");
    assert_eq!(&built.data[13..45], &[3u8; 32]);

    assert_eq!(built.accounts.len(), 2);
    assert!(built.accounts[0].is_signer);
    assert!(!built.accounts[0].is_writable);
    assert_eq!(built.accounts[0].address, &accounts.authority);
    assert!(built.accounts[1].is_writable);
    assert_eq!(built.accounts[1].address, &accounts.vault);

    let view = built.view();
    assert_eq!(view.program_id, &program_id);
    assert_eq!(view.data.len(), 45);
    assert_eq!(view.accounts.len(), 2);

    assert_eq!(demo_manifest::DEPOSIT_SPEC.account_count, 2);
    assert_eq!(demo_manifest::DEPOSIT_SPEC.args_size, 44);
    assert_eq!(demo_manifest::DEPOSIT_ACCOUNT_SPECS[1].resolver, "pda");
    assert_eq!(demo_manifest::DEPOSIT_ACCOUNT_SPECS[1].lifecycle, "init");
    assert_eq!(demo_manifest::DEPOSIT_ACCOUNT_SPECS[1].payer, "authority");
    assert_eq!(
        demo_manifest::DEPOSIT_ACCOUNT_SPECS[1].expected_owner,
        "DemoProgram"
    );
    assert_eq!(demo_manifest::DEPOSIT_EFFECT_SPECS.len(), 3);
    assert_eq!(demo_manifest::DEPOSIT_EFFECT_SPECS[2].kind, "emits_receipt");
}
