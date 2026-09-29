//! The confidential-transfer flow end to end, under Mollusk. This program's
//! instructions call Hopper's builders; the builders call Token-2022; and
//! Token-2022 checks each proof against what the ZK ElGamal proof program
//! verified. The keys, ciphertexts, and proofs are made here with
//! `solana-zk-sdk` and `spl-token-confidential-transfer-proof-generation`,
//! the crates a wallet uses.
//!
//! Token-2022 is the program mainnet runs: `fixtures/token_2022_v11.0.0.so`
//! was dumped from mainnet-beta on 2026-09-29 (see `fixtures/README.md`),
//! and the test refuses a file with another hash. Mollusk's own bundled
//! Token-2022 is v7.0.0, whose ciphertext operations are compiled out: it
//! answers `Deposit` with `InvalidInstructionData`.
//!
//! Public clusters cannot run this today: the proof program is disabled on
//! mainnet, testnet, and devnet. Mollusk runs it with every feature active.
//!
//! Build first, from `examples/hopper-confidential-lab`: `cargo build-sbf`.
//! A test whose artifact is missing prints `SKIPPED` and passes.

use std::collections::BTreeMap;

use bytemuck::Pod;
use hopper_confidential_lab::{
    CONFIDENTIAL_MINT_LEN, DECIMALS, DISABLE_CONFIDENTIAL, DISABLE_NON_CONFIDENTIAL,
    ENABLE_CONFIDENTIAL, ENABLE_NON_CONFIDENTIAL,
};
use hopper_test::LiteSvmHarness;
use sha2::Digest;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction};
use solana_pubkey::Pubkey;
use solana_zk_elgamal_proof_interface::instruction::{ContextStateInfo, ProofInstruction};
use solana_zk_elgamal_proof_interface::proof_data::{
    BatchedGroupedCiphertext2HandlesValidityProofContext,
    BatchedGroupedCiphertext3HandlesValidityProofContext, BatchedRangeProofContext,
    CiphertextCommitmentEqualityProofContext, PercentageWithCapProofContext,
    PubkeyValidityProofContext, ZkProofData,
};
use solana_zk_elgamal_proof_interface::state::ProofContextState;
use solana_zk_sdk::encryption::auth_encryption::{AeCiphertext, AeKey};
use solana_zk_sdk::encryption::elgamal::{ElGamalCiphertext, ElGamalKeypair};
use solana_zk_sdk::zk_elgamal_proof_program::{
    build_pubkey_validity_proof_data, build_zero_ciphertext_proof_data,
};
use spl_token_2022_interface::extension::confidential_transfer::{
    ConfidentialTransferAccount, ConfidentialTransferMint,
};
use spl_token_2022_interface::extension::confidential_transfer_fee::ConfidentialTransferFeeAmount;
use spl_token_2022_interface::extension::{
    BaseStateWithExtensions, ExtensionType, StateWithExtensions,
};
use spl_token_2022_interface::state::{Account as TokenAccount, Mint};
use spl_token_confidential_transfer_proof_generation::transfer::transfer_split_proof_data;
use spl_token_confidential_transfer_proof_generation::transfer_with_fee::transfer_with_fee_split_proof_data;
use spl_token_confidential_transfer_proof_generation::withdraw::withdraw_proof_data;

const ELF: &str = "../../target/deploy/hopper_confidential_lab";
const TOKEN_2022: Pubkey = mollusk_svm_programs_token::token2022::ID;
const TOKEN_2022_FIXTURE: &str = "fixtures/token_2022_v11.0.0.so";
const TOKEN_2022_FIXTURE_SHA256: &str =
    "0999dbf708971e723b08d1caafc988826a59c6001ed6dc02260da07defbe1469";
const INSTRUCTIONS_SYSVAR: Pubkey =
    Pubkey::from_str_const("Sysvar1nstructions1111111111111111111111111");
const MAX_PENDING_CREDITS: u64 = 65_536;

/// A holder: the account's signing key, its ElGamal keypair, and its
/// authenticated-encryption key.
struct Holder {
    signer: Pubkey,
    elgamal: ElGamalKeypair,
    ae: AeKey,
}

impl Holder {
    fn new() -> Self {
        Self {
            signer: Pubkey::new_unique(),
            elgamal: ElGamalKeypair::new_rand(),
            ae: AeKey::new_rand(),
        }
    }
}

struct Outcome {
    ok: bool,
    error: String,
    units: u64,
    logs: Vec<String>,
}

struct Lab {
    svm: LiteSvmHarness,
    program_id: Pubkey,
    payer: Pubkey,
    bank: BTreeMap<Pubkey, Account>,
}

impl Lab {
    fn new() -> Option<Self> {
        let program_id = Pubkey::new_unique();
        let Some(mut svm) = LiteSvmHarness::load(&program_id, ELF) else {
            eprintln!("SKIPPED: build {ELF}.so first");
            return None;
        };
        // The pinned mainnet build, or the file `CONFIDENTIAL_LAB_TOKEN_2022`
        // names (a dump of what another cluster runs).
        let (path, pinned) = match std::env::var("CONFIDENTIAL_LAB_TOKEN_2022") {
            Ok(path) => (path, false),
            Err(_) => (TOKEN_2022_FIXTURE.to_string(), true),
        };
        let elf = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let digest: String = sha2::Sha256::digest(&elf)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if pinned {
            assert_eq!(
                digest, TOKEN_2022_FIXTURE_SHA256,
                "{path} is not the pinned build"
            );
        }
        println!(
            "CONFIDENTIAL_LAB Token-2022 {path}, {} bytes, sha256 {digest}",
            elf.len()
        );
        svm.mollusk_mut().add_program_with_loader_and_elf(
            &TOKEN_2022,
            &mollusk_svm::program::loader_keys::LOADER_V3,
            &elf,
        );
        let payer = Pubkey::new_unique();
        let mut bank = BTreeMap::new();
        bank.insert(
            payer,
            Account::new(1_000_000_000_000, 0, &Pubkey::default()),
        );
        bank.insert(TOKEN_2022, mollusk_svm_programs_token::token2022::account());
        let (system, system_account) = LiteSvmHarness::system_program_account();
        bank.insert(system, system_account);
        Some(Self {
            svm,
            program_id,
            payer,
            bank,
        })
    }

    /// Run `instructions` as one transaction over the bank and keep the
    /// accounts it wrote when it succeeds. The Instructions sysvar is left
    /// out of the account list so Mollusk builds the real one.
    fn send(&mut self, instructions: &[Instruction]) -> Outcome {
        let mut accounts: Vec<(Pubkey, Account)> = Vec::new();
        for instruction in instructions {
            for key in instruction.accounts.iter().map(|m| m.pubkey) {
                if key == INSTRUCTIONS_SYSVAR || accounts.iter().any(|(k, _)| *k == key) {
                    continue;
                }
                let account = self
                    .bank
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| Account::new(0, 0, &Pubkey::default()));
                accounts.push((key, account));
            }
        }
        self.svm.capture_logs();
        let payer = self.payer;
        let result = self.svm.mollusk_mut().process_transaction_instructions(
            instructions,
            &accounts,
            Some(&payer),
        );
        let ok = result.raw_result.is_ok();
        if ok {
            for (key, account) in result.resulting_accounts {
                self.bank.insert(key, account);
            }
        }
        Outcome {
            ok,
            error: format!("{:?}", result.raw_result),
            units: result.compute_units_consumed,
            logs: self.svm.logs(),
        }
    }

    fn expect(&mut self, what: &str, instructions: &[Instruction]) -> Outcome {
        let outcome = self.send(instructions);
        assert!(outcome.ok, "{what}: {}\n{:#?}", outcome.error, outcome.logs);
        println!("CONFIDENTIAL_LAB {what}: {} CU", outcome.units);
        outcome
    }

    fn lab(&self, tag: u8, args: &[u8], accounts: Vec<AccountMeta>) -> Instruction {
        let mut data = vec![tag];
        data.extend_from_slice(args);
        Instruction::new_with_bytes(self.program_id, &data, accounts)
    }

    /// Verify `proof` with the proof program into a fresh context-state
    /// account the payer controls, and return that account.
    fn verify<T, U>(&mut self, what: &str, instruction: ProofInstruction, proof: &T) -> Pubkey
    where
        T: Pod + ZkProofData<U>,
        U: Pod,
    {
        let context = Pubkey::new_unique();
        let size = core::mem::size_of::<ProofContextState<U>>();
        let lamports = self.svm.mollusk_mut().sysvars.rent.minimum_balance(size);
        self.bank.insert(
            context,
            Account::new(lamports, size, &solana_zk_elgamal_proof_interface::id()),
        );
        let payer = self.payer;
        let ix = instruction.encode_verify_proof(
            Some(ContextStateInfo {
                context_state_account: &context,
                context_state_authority: &payer,
            }),
            proof,
        );
        self.expect(what, &[ix]);
        context
    }

    fn state(&self, account: &Pubkey) -> ConfidentialTransferAccount {
        let data = &self.bank[account].data;
        let state = StateWithExtensions::<TokenAccount>::unpack(data).unwrap();
        *state
            .get_extension::<ConfidentialTransferAccount>()
            .unwrap()
    }

    fn public_amount(&self, account: &Pubkey) -> u64 {
        StateWithExtensions::<TokenAccount>::unpack(&self.bank[account].data)
            .unwrap()
            .base
            .amount
    }

    fn mint_state(&self, mint: &Pubkey) -> ConfidentialTransferMint {
        let state = StateWithExtensions::<Mint>::unpack(&self.bank[mint].data).unwrap();
        *state.get_extension::<ConfidentialTransferMint>().unwrap()
    }
}

fn ae_bytes(key: &AeKey, amount: u64) -> [u8; 36] {
    key.encrypt(amount).to_bytes()
}

fn decryptable(state: &ConfidentialTransferAccount, key: &AeKey) -> Option<u64> {
    AeCiphertext::from_bytes(&state.decryptable_available_balance.0)?.decrypt(key)
}

fn elgamal(bytes: &[u8; 64]) -> ElGamalCiphertext {
    ElGamalCiphertext::from_bytes(bytes).unwrap()
}

/// The arguments of `open_account`: the pending-credit cap, the owner's
/// encryption of zero, where the proof is, and one more extension to make
/// room for (zero for none).
fn open_args(holder: &Holder, proof_offset: i8, extra_extension: u16) -> Vec<u8> {
    let mut args = MAX_PENDING_CREDITS.to_le_bytes().to_vec();
    args.extend_from_slice(&ae_bytes(&holder.ae, 0));
    args.push(proof_offset as u8);
    args.extend_from_slice(&extra_extension.to_le_bytes());
    args
}

/// Open and configure an account for `holder`, the proof verified into a
/// context-state account first.
fn open_account(lab: &mut Lab, mint: Pubkey, holder: &Holder, extra_extension: u16) -> Pubkey {
    let proof = build_pubkey_validity_proof_data(&holder.elgamal).unwrap();
    let context = lab.verify::<_, PubkeyValidityProofContext>(
        "verify pubkey validity",
        ProofInstruction::VerifyPubkeyValidity,
        &proof,
    );
    let account = Pubkey::new_unique();
    let payer = lab.payer;
    let open = lab.lab(
        2,
        &open_args(holder, 0, extra_extension),
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(account, true),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(holder.signer, true),
            AccountMeta::new_readonly(context, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    lab.expect("open_account", &[open]);
    account
}

/// Mint `amount` to `account`, deposit it, and apply it.
fn fund(lab: &mut Lab, mint: Pubkey, account: Pubkey, holder: &Holder, amount: u64) {
    let payer = lab.payer;
    let mint_to = spl_token_2022_interface::instruction::mint_to_checked(
        &TOKEN_2022,
        &mint,
        &account,
        &payer,
        &[],
        amount,
        DECIMALS,
    )
    .unwrap();
    lab.expect("mint_to (setup)", &[mint_to]);
    let mut args = amount.to_le_bytes().to_vec();
    args.push(DECIMALS);
    let deposit = lab.lab(
        4,
        &args,
        vec![
            AccountMeta::new(account, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(holder.signer, true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    lab.expect("deposit", &[deposit]);
    let pending = u64::from(lab.state(&account).pending_balance_credit_counter);
    let mut args = pending.to_le_bytes().to_vec();
    args.extend_from_slice(&ae_bytes(&holder.ae, amount));
    let apply = lab.lab(
        5,
        &args,
        vec![
            AccountMeta::new(account, false),
            AccountMeta::new_readonly(holder.signer, true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    lab.expect("apply_pending", &[apply]);
}

#[test]
fn the_confidential_flow_runs_through_hoppers_builders() {
    let Some(mut lab) = Lab::new() else { return };
    let payer = lab.payer;
    let alice = Holder::new();
    let bob = Holder::new();
    let auditor = ElGamalKeypair::new_rand();
    let auditor_key = auditor.pubkey().to_bytes();

    // The mint: confidential transfers on, accounts need approval, an
    // auditor who can read every amount.
    let mint = Pubkey::new_unique();
    let mut args = vec![0u8];
    args.extend_from_slice(&auditor_key);
    let create = lab.lab(
        0,
        &args,
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(mint, true),
            AccountMeta::new_readonly(Pubkey::default(), false),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    lab.expect("create_mint", &[create]);
    assert_eq!(
        CONFIDENTIAL_MINT_LEN,
        ExtensionType::try_calculate_account_len::<Mint>(&[
            ExtensionType::ConfidentialTransferMint
        ])
        .unwrap()
    );
    assert_eq!(lab.bank[&mint].data.len(), CONFIDENTIAL_MINT_LEN);
    let config = lab.mint_state(&mint);
    assert!(!bool::from(config.auto_approve_new_accounts));
    // The extension's bytes after the authority and the flag are the key.
    assert_eq!(
        &lab.bank[&mint].data[CONFIDENTIAL_MINT_LEN - 32..],
        &auditor_key
    );

    // Alice's account, configured with her key's validity proof taken from
    // a context-state account.
    let proof = build_pubkey_validity_proof_data(&alice.elgamal).unwrap();
    let context = lab.verify::<_, PubkeyValidityProofContext>(
        "verify pubkey validity",
        ProofInstruction::VerifyPubkeyValidity,
        &proof,
    );
    let alice_account = Pubkey::new_unique();
    let open = lab.lab(
        2,
        &open_args(&alice, 0, 0),
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(alice_account, true),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(alice.signer, true),
            AccountMeta::new_readonly(context, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    lab.expect("open_account, proof in a context account", &[open]);
    let state = lab.state(&alice_account);
    assert_eq!(state.elgamal_pubkey.0, alice.elgamal.pubkey().to_bytes());
    assert!(!bool::from(state.approved), "the mint asks for approval");
    assert_eq!(decryptable(&state, &alice.ae), Some(0));

    // The confidential-transfer authority approves her.
    let approve = lab.lab(
        3,
        &[],
        vec![
            AccountMeta::new(alice_account, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(payer, true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    lab.expect("approve", &[approve]);
    assert!(bool::from(lab.state(&alice_account).approved));

    // From now on new accounts are approved when configured.
    let mut args = vec![1u8];
    args.extend_from_slice(&auditor_key);
    let update = lab.lab(
        1,
        &args,
        vec![
            AccountMeta::new(mint, false),
            AccountMeta::new_readonly(payer, true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    lab.expect("update_mint", &[update]);
    assert!(bool::from(lab.mint_state(&mint).auto_approve_new_accounts));

    // Carol's account, configured from her ElGamal registry: the account
    // starts at its base size, Token-2022 grows it and the payer funds the
    // growth, and Carol does not sign.
    let carol = Holder::new();
    let registry_program = spl_elgamal_registry_interface::id();
    let registry = spl_elgamal_registry_interface::get_elgamal_registry_address(
        &carol.signer,
        &registry_program,
    );
    let mut data = carol.signer.to_bytes().to_vec();
    data.extend_from_slice(&carol.elgamal.pubkey().to_bytes());
    let lamports = lab
        .svm
        .mollusk_mut()
        .sysvars
        .rent
        .minimum_balance(data.len());
    lab.bank.insert(
        registry,
        Account {
            lamports,
            data,
            owner: registry_program,
            executable: false,
            rent_epoch: 0,
        },
    );
    let carol_account = Pubkey::new_unique();
    let open = lab.lab(
        10,
        &[],
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(carol_account, true),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(carol.signer, false),
            AccountMeta::new_readonly(registry, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    lab.expect("open_from_registry", &[open]);
    let state = lab.state(&carol_account);
    assert_eq!(state.elgamal_pubkey.0, carol.elgamal.pubkey().to_bytes());
    assert!(bool::from(state.approved));
    // A registry for someone else configures nothing.
    let dave = Holder::new();
    let dave_account = Pubkey::new_unique();
    let open = lab.lab(
        10,
        &[],
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(dave_account, true),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(dave.signer, false),
            AccountMeta::new_readonly(registry, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    assert!(
        !lab.send(&[open]).ok,
        "Carol's registry configured Dave's account"
    );

    // Bob's account, configured with the proof in the instruction before it
    // in the same transaction: the builder passes the Instructions sysvar
    // and offset -1.
    let proof = build_pubkey_validity_proof_data(&bob.elgamal).unwrap();
    let verify = ProofInstruction::VerifyPubkeyValidity.encode_verify_proof(None, &proof);
    let bob_account = Pubkey::new_unique();
    let open = lab.lab(
        2,
        &open_args(&bob, -1, 0),
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(bob_account, true),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(bob.signer, true),
            AccountMeta::new_readonly(INSTRUCTIONS_SYSVAR, false),
            AccountMeta::new_readonly(Pubkey::default(), false),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    lab.expect("open_account, proof by instruction offset", &[verify, open]);
    let state = lab.state(&bob_account);
    assert_eq!(state.elgamal_pubkey.0, bob.elgamal.pubkey().to_bytes());
    assert!(bool::from(state.approved), "approved on configuration");

    // A public balance for Alice.
    let mint_to = spl_token_2022_interface::instruction::mint_to_checked(
        &TOKEN_2022,
        &mint,
        &alice_account,
        &payer,
        &[],
        1_000_000,
        DECIMALS,
    )
    .unwrap();
    lab.expect("mint_to (setup)", &[mint_to]);

    // Deposit 700,000 into her pending confidential balance.
    let mut args = 700_000u64.to_le_bytes().to_vec();
    args.push(DECIMALS);
    let deposit = lab.lab(
        4,
        &args,
        vec![
            AccountMeta::new(alice_account, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(alice.signer, true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    lab.expect("deposit", &[deposit]);
    assert_eq!(lab.public_amount(&alice_account), 300_000);
    let state = lab.state(&alice_account);
    assert_eq!(u64::from(state.pending_balance_credit_counter), 1);
    // The pending balance is split in 16 low and 32 high bits, each an
    // ElGamal ciphertext under her key.
    let lo = elgamal(&state.pending_balance_lo.0).decrypt_u32(alice.elgamal.secret());
    let hi = elgamal(&state.pending_balance_hi.0).decrypt_u32(alice.elgamal.secret());
    assert_eq!((lo, hi), (Some(700_000 & 0xffff), Some(700_000 >> 16)));

    // Apply it.
    let mut args = 1u64.to_le_bytes().to_vec();
    args.extend_from_slice(&ae_bytes(&alice.ae, 700_000));
    let apply = lab.lab(
        5,
        &args,
        vec![
            AccountMeta::new(alice_account, false),
            AccountMeta::new_readonly(alice.signer, true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    lab.expect("apply_pending", &[apply]);
    let state = lab.state(&alice_account);
    assert_eq!(decryptable(&state, &alice.ae), Some(700_000));

    // Withdraw 200,000 with an equality and a range proof, both verified
    // into context-state accounts.
    let withdraw_proofs = withdraw_proof_data(
        &elgamal(&state.available_balance.0),
        700_000,
        200_000,
        &alice.elgamal,
    )
    .unwrap();
    let equality = lab.verify::<_, CiphertextCommitmentEqualityProofContext>(
        "verify equality",
        ProofInstruction::VerifyCiphertextCommitmentEquality,
        &withdraw_proofs.equality_proof_data,
    );
    let range = lab.verify::<_, BatchedRangeProofContext>(
        "verify range u64",
        ProofInstruction::VerifyBatchedRangeProofU64,
        &withdraw_proofs.range_proof_data,
    );
    let withdraw_accounts = |equality: Pubkey, range: Pubkey| {
        vec![
            AccountMeta::new(alice_account, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(equality, false),
            AccountMeta::new_readonly(range, false),
            AccountMeta::new_readonly(alice.signer, true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ]
    };
    let withdraw_args = |amount: u64, remaining: u64| {
        let mut args = amount.to_le_bytes().to_vec();
        args.push(DECIMALS);
        args.extend_from_slice(&ae_bytes(&alice.ae, remaining));
        args.extend_from_slice(&[0, 0]);
        args
    };
    let withdraw = lab.lab(
        6,
        &withdraw_args(200_000, 500_000),
        withdraw_accounts(equality, range),
    );
    lab.expect("withdraw", std::slice::from_ref(&withdraw));
    assert_eq!(lab.public_amount(&alice_account), 500_000);
    assert_eq!(
        decryptable(&lab.state(&alice_account), &alice.ae),
        Some(500_000)
    );

    // The same proofs again: they describe a balance the account no longer
    // holds, and Token-2022 refuses them.
    let replay = lab.send(&[withdraw]);
    assert!(!replay.ok, "a replayed withdraw proof was accepted");
    assert_eq!(lab.public_amount(&alice_account), 500_000);

    // Transfer 123,456 to Bob with three proofs. The auditor's ciphertexts
    // ride in the instruction data.
    let state = lab.state(&alice_account);
    let transfer_proofs = transfer_split_proof_data(
        &elgamal(&state.available_balance.0),
        &AeCiphertext::from_bytes(&state.decryptable_available_balance.0).unwrap(),
        123_456,
        &alice.elgamal,
        &alice.ae,
        bob.elgamal.pubkey(),
        Some(auditor.pubkey()),
    )
    .unwrap();
    let equality = lab.verify::<_, CiphertextCommitmentEqualityProofContext>(
        "verify equality",
        ProofInstruction::VerifyCiphertextCommitmentEquality,
        &transfer_proofs.equality_proof_data,
    );
    let validity = lab.verify::<_, BatchedGroupedCiphertext3HandlesValidityProofContext>(
        "verify ciphertext validity",
        ProofInstruction::VerifyBatchedGroupedCiphertext3HandlesValidity,
        &transfer_proofs
            .ciphertext_validity_proof_data_with_ciphertext
            .proof_data,
    );
    let range = lab.verify::<_, BatchedRangeProofContext>(
        "verify range u128",
        ProofInstruction::VerifyBatchedRangeProofU128,
        &transfer_proofs.range_proof_data,
    );
    let auditor_lo = transfer_proofs
        .ciphertext_validity_proof_data_with_ciphertext
        .ciphertext_lo
        .0;
    let auditor_hi = transfer_proofs
        .ciphertext_validity_proof_data_with_ciphertext
        .ciphertext_hi
        .0;
    let mut args = ae_bytes(&alice.ae, 500_000 - 123_456).to_vec();
    args.extend_from_slice(&auditor_lo);
    args.extend_from_slice(&auditor_hi);
    args.extend_from_slice(&[0, 0, 0]);
    let transfer = lab.lab(
        7,
        &args,
        vec![
            AccountMeta::new(alice_account, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new(bob_account, false),
            AccountMeta::new_readonly(equality, false),
            AccountMeta::new_readonly(validity, false),
            AccountMeta::new_readonly(range, false),
            AccountMeta::new_readonly(alice.signer, true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    lab.expect("transfer", &[transfer]);
    assert_eq!(
        decryptable(&lab.state(&alice_account), &alice.ae),
        Some(500_000 - 123_456)
    );
    let state = lab.state(&bob_account);
    assert_eq!(u64::from(state.pending_balance_credit_counter), 1);
    let lo = elgamal(&state.pending_balance_lo.0).decrypt_u32(bob.elgamal.secret());
    let hi = elgamal(&state.pending_balance_hi.0).decrypt_u32(bob.elgamal.secret());
    assert_eq!((lo, hi), (Some(123_456 & 0xffff), Some(123_456 >> 16)));
    // The auditor reads the amount from what the builder carried.
    let lo = elgamal(&auditor_lo).decrypt_u32(auditor.secret());
    let hi = elgamal(&auditor_hi).decrypt_u32(auditor.secret());
    assert_eq!((lo, hi), (Some(123_456 & 0xffff), Some(123_456 >> 16)));

    // Bob applies what he received.
    let mut args = 1u64.to_le_bytes().to_vec();
    args.extend_from_slice(&ae_bytes(&bob.ae, 123_456));
    let apply = lab.lab(
        5,
        &args,
        vec![
            AccountMeta::new(bob_account, false),
            AccountMeta::new_readonly(bob.signer, true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    lab.expect("apply_pending (bob)", &[apply]);
    assert_eq!(
        decryptable(&lab.state(&bob_account), &bob.ae),
        Some(123_456)
    );

    // The credit toggles, each checked in the account and, for public
    // credits, by what Token-2022 then refuses.
    let toggle = |lab: &Lab, which: u8| {
        lab.lab(
            8,
            &[which],
            vec![
                AccountMeta::new(bob_account, false),
                AccountMeta::new_readonly(bob.signer, true),
                AccountMeta::new_readonly(TOKEN_2022, false),
            ],
        )
    };
    let public_transfer = spl_token_2022_interface::instruction::transfer_checked(
        &TOKEN_2022,
        &alice_account,
        &mint,
        &bob_account,
        &alice.signer,
        &[],
        1,
        DECIMALS,
    )
    .unwrap();
    let ix = toggle(&lab, DISABLE_CONFIDENTIAL);
    lab.expect("disable confidential credits", &[ix]);
    assert!(!bool::from(
        lab.state(&bob_account).allow_confidential_credits
    ));
    let ix = toggle(&lab, ENABLE_CONFIDENTIAL);
    lab.expect("enable confidential credits", &[ix]);
    assert!(bool::from(
        lab.state(&bob_account).allow_confidential_credits
    ));
    let ix = toggle(&lab, DISABLE_NON_CONFIDENTIAL);
    lab.expect("disable non-confidential credits", &[ix]);
    assert!(!bool::from(
        lab.state(&bob_account).allow_non_confidential_credits
    ));
    assert!(
        !lab.send(std::slice::from_ref(&public_transfer)).ok,
        "a public transfer reached an account that refuses them"
    );
    let ix = toggle(&lab, ENABLE_NON_CONFIDENTIAL);
    lab.expect("enable non-confidential credits", &[ix]);
    lab.expect("public transfer (setup)", &[public_transfer]);
    assert_eq!(lab.public_amount(&bob_account), 1);

    // Alice withdraws everything left, then proves the balance is zero
    // with the proof in the same transaction.
    let state = lab.state(&alice_account);
    let rest = 500_000 - 123_456;
    let proofs = withdraw_proof_data(
        &elgamal(&state.available_balance.0),
        rest,
        rest,
        &alice.elgamal,
    )
    .unwrap();
    let equality = lab.verify::<_, CiphertextCommitmentEqualityProofContext>(
        "verify equality",
        ProofInstruction::VerifyCiphertextCommitmentEquality,
        &proofs.equality_proof_data,
    );
    let range = lab.verify::<_, BatchedRangeProofContext>(
        "verify range u64",
        ProofInstruction::VerifyBatchedRangeProofU64,
        &proofs.range_proof_data,
    );
    let withdraw = lab.lab(
        6,
        &withdraw_args(rest, 0),
        withdraw_accounts(equality, range),
    );
    lab.expect("withdraw the rest", &[withdraw]);
    assert_eq!(lab.public_amount(&alice_account), 500_000 - 1 + rest);

    let state = lab.state(&alice_account);
    let zero =
        build_zero_ciphertext_proof_data(&alice.elgamal, &elgamal(&state.available_balance.0))
            .unwrap();
    let verify = ProofInstruction::VerifyZeroCiphertext.encode_verify_proof(None, &zero);
    let empty = lab.lab(
        9,
        &[(-1i8) as u8],
        vec![
            AccountMeta::new(alice_account, false),
            AccountMeta::new_readonly(INSTRUCTIONS_SYSVAR, false),
            AccountMeta::new_readonly(alice.signer, true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    lab.expect("empty, proof by instruction offset", &[verify, empty]);
}

#[test]
fn a_transfer_on_a_fee_mint_carries_five_proofs() {
    let Some(mut lab) = Lab::new() else { return };
    let payer = lab.payer;
    let alice = Holder::new();
    let bob = Holder::new();
    let auditor = ElGamalKeypair::new_rand();
    let withheld_authority = ElGamalKeypair::new_rand();
    const FEE_BASIS_POINTS: u16 = 100;
    const MAXIMUM_FEE: u64 = 5_000;

    // The fee mint is set up with the canonical constructors; what is under
    // test is the transfer.
    let mint = Pubkey::new_unique();
    let size = ExtensionType::try_calculate_account_len::<Mint>(&[
        ExtensionType::TransferFeeConfig,
        ExtensionType::ConfidentialTransferMint,
        ExtensionType::ConfidentialTransferFeeConfig,
    ])
    .unwrap();
    let lamports = lab.svm.mollusk_mut().sysvars.rent.minimum_balance(size);
    lab.bank
        .insert(mint, Account::new(lamports, size, &TOKEN_2022));
    use spl_token_2022_interface::extension::{
        confidential_transfer, confidential_transfer_fee, transfer_fee,
    };
    let setup = [
        transfer_fee::instruction::initialize_transfer_fee_config(
            &TOKEN_2022,
            &mint,
            Some(&payer),
            Some(&payer),
            FEE_BASIS_POINTS,
            MAXIMUM_FEE,
        )
        .unwrap(),
        confidential_transfer::instruction::initialize_mint(
            &TOKEN_2022,
            &mint,
            Some(payer),
            true,
            Some((*auditor.pubkey()).into()),
        )
        .unwrap(),
        confidential_transfer_fee::instruction::initialize_confidential_transfer_fee_config(
            &TOKEN_2022,
            &mint,
            Some(payer),
            &(*withheld_authority.pubkey()).into(),
        )
        .unwrap(),
        spl_token_2022_interface::instruction::initialize_mint2(
            &TOKEN_2022,
            &mint,
            &payer,
            None,
            DECIMALS,
        )
        .unwrap(),
    ];
    lab.expect("fee mint (setup)", &setup);

    // `GetAccountDataSize` makes room for the transfer-fee amount and the
    // confidential state, not for the confidential fee amount that
    // `ConfigureAccount` adds on a fee mint: ask for it by name.
    let fee_amount = u16::from(ExtensionType::ConfidentialTransferFeeAmount);
    let alice_account = open_account(&mut lab, mint, &alice, fee_amount);
    let bob_account = open_account(&mut lab, mint, &bob, fee_amount);
    fund(&mut lab, mint, alice_account, &alice, 500_000);

    let amount = 100_000u64;
    let fee = (amount * FEE_BASIS_POINTS as u64)
        .div_ceil(10_000)
        .min(MAXIMUM_FEE);
    let state = lab.state(&alice_account);
    let proofs = transfer_with_fee_split_proof_data(
        &elgamal(&state.available_balance.0),
        &AeCiphertext::from_bytes(&state.decryptable_available_balance.0).unwrap(),
        amount,
        &alice.elgamal,
        &alice.ae,
        bob.elgamal.pubkey(),
        Some(auditor.pubkey()),
        withheld_authority.pubkey(),
        FEE_BASIS_POINTS,
        MAXIMUM_FEE,
    )
    .unwrap();
    let equality = lab.verify::<_, CiphertextCommitmentEqualityProofContext>(
        "verify equality",
        ProofInstruction::VerifyCiphertextCommitmentEquality,
        &proofs.equality_proof_data,
    );
    let amount_validity = lab.verify::<_, BatchedGroupedCiphertext3HandlesValidityProofContext>(
        "verify amount validity",
        ProofInstruction::VerifyBatchedGroupedCiphertext3HandlesValidity,
        &proofs
            .transfer_amount_ciphertext_validity_proof_data_with_ciphertext
            .proof_data,
    );
    let fee_sigma = lab.verify::<_, PercentageWithCapProofContext>(
        "verify fee sigma",
        ProofInstruction::VerifyPercentageWithCap,
        &proofs.percentage_with_cap_proof_data,
    );
    let fee_validity = lab.verify::<_, BatchedGroupedCiphertext2HandlesValidityProofContext>(
        "verify fee validity",
        ProofInstruction::VerifyBatchedGroupedCiphertext2HandlesValidity,
        &proofs.fee_ciphertext_validity_proof_data,
    );
    let range = lab.verify::<_, BatchedRangeProofContext>(
        "verify range u256",
        ProofInstruction::VerifyBatchedRangeProofU256,
        &proofs.range_proof_data,
    );
    let with_ciphertext = &proofs.transfer_amount_ciphertext_validity_proof_data_with_ciphertext;
    let mut args = ae_bytes(&alice.ae, 500_000 - amount).to_vec();
    args.extend_from_slice(&with_ciphertext.ciphertext_lo.0);
    args.extend_from_slice(&with_ciphertext.ciphertext_hi.0);
    args.extend_from_slice(&[0; 5]);
    let transfer = lab.lab(
        11,
        &args,
        vec![
            AccountMeta::new(alice_account, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new(bob_account, false),
            AccountMeta::new_readonly(equality, false),
            AccountMeta::new_readonly(amount_validity, false),
            AccountMeta::new_readonly(fee_sigma, false),
            AccountMeta::new_readonly(fee_validity, false),
            AccountMeta::new_readonly(range, false),
            AccountMeta::new_readonly(alice.signer, true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    lab.expect("transfer_with_fee", &[transfer]);
    assert_eq!(
        decryptable(&lab.state(&alice_account), &alice.ae),
        Some(500_000 - amount)
    );

    // Bob is credited the amount less the fee, and the fee is withheld in
    // his account under the withdraw authority's key.
    let state = lab.state(&bob_account);
    let lo = elgamal(&state.pending_balance_lo.0).decrypt_u32(bob.elgamal.secret());
    let hi = elgamal(&state.pending_balance_hi.0).decrypt_u32(bob.elgamal.secret());
    let credited = lo.unwrap() + (hi.unwrap() << 16);
    assert_eq!(credited, amount - fee, "{lo:?} {hi:?}");
    let data = &lab.bank[&bob_account].data;
    let account = StateWithExtensions::<TokenAccount>::unpack(data).unwrap();
    let withheld = account
        .get_extension::<ConfidentialTransferFeeAmount>()
        .unwrap();
    assert_eq!(
        elgamal(&withheld.withheld_amount.0).decrypt_u32(withheld_authority.secret()),
        Some(fee)
    );
    println!("CONFIDENTIAL_LAB fee withheld: {fee} of {amount}");
}
