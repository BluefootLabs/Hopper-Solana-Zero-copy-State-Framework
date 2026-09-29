//! Runs the confidential lab's whole flow on a public cluster, with the
//! proofs made here the way a wallet makes them.
//!
//! Every confidential-transfer instruction goes through the lab program,
//! and so through Hopper's builders, to the Token-2022 the cluster runs.
//! Every proof is verified by the cluster's ZK ElGamal proof program: into a
//! context-state account, in the same transaction by instruction offset,
//! or, for the u128 and u256 range proofs that do not fit in a transaction
//! with the compute-budget instruction they need, from an SPL Record
//! account. After each step the runner reads the accounts back
//! and decrypts what landed.
//!
//! ```text
//! cargo run --release -p hopper-confidential-lab-runner -- \
//!   --program <lab program id> --elf target/deploy/hopper_confidential_lab.so \
//!   --payer <keypair> --rpc https://api.devnet.solana.com \
//!   --out target/hopper/confidential-flow-devnet
//! ```
//!
//! It spends SOL on rent and fees. The context-state and record accounts
//! are closed at the end and their rent comes back to the payer.

use std::{
    env, fs,
    num::NonZeroI8,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

use bytemuck::Pod;
use hopper_confidential_lab::{
    CONFIDENTIAL_MINT_LEN, DECIMALS, DISABLE_CONFIDENTIAL, DISABLE_NON_CONFIDENTIAL,
    ENABLE_CONFIDENTIAL, ENABLE_NON_CONFIDENTIAL,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use solana_client::{
    rpc_client::RpcClient, rpc_config::RpcSendTransactionConfig, rpc_request::RpcRequest,
};
use solana_commitment_config::CommitmentConfig;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{read_keypair_file, Keypair};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction as system;
use solana_transaction::Transaction;
use solana_zk_elgamal_proof_interface::{
    instruction::{close_context_state, ContextStateInfo, ProofInstruction},
    proof_data::{
        BatchedGroupedCiphertext2HandlesValidityProofContext,
        BatchedGroupedCiphertext3HandlesValidityProofContext, BatchedRangeProofContext,
        CiphertextCommitmentEqualityProofContext, PercentageWithCapProofContext,
        PubkeyValidityProofContext, ZkProofData,
    },
    state::ProofContextState,
};
use solana_zk_sdk::{
    encryption::{
        auth_encryption::{AeCiphertext, AeKey},
        elgamal::{ElGamalCiphertext, ElGamalKeypair},
    },
    zk_elgamal_proof_program::{
        build_pubkey_validity_proof_data, build_zero_ciphertext_proof_data,
    },
};
use spl_token_2022_interface::{
    extension::{
        confidential_transfer::{ConfidentialTransferAccount, ConfidentialTransferMint},
        confidential_transfer_fee::ConfidentialTransferFeeAmount,
        BaseStateWithExtensions, ExtensionType, StateWithExtensions,
    },
    state::{Account as TokenAccount, Mint},
};
use spl_token_confidential_transfer_proof_extraction::instruction::ProofLocation;
use spl_token_confidential_transfer_proof_generation::{
    transfer::transfer_split_proof_data, transfer_with_fee::transfer_with_fee_split_proof_data,
    withdraw::withdraw_proof_data,
};

const TOKEN_2022: Pubkey = Pubkey::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
const SYSTEM: Pubkey = Pubkey::from_str_const("11111111111111111111111111111111");
const INSTRUCTIONS_SYSVAR: Pubkey =
    Pubkey::from_str_const("Sysvar1nstructions1111111111111111111111111");
const RECORD_PROGRAM: Pubkey =
    Pubkey::from_str_const("recr1L3PCGKLbckBqMNcJhuuyU1zgo8nBhfLVsJNwr5");
/// SPL Record's header: a version byte and the authority.
const RECORD_HEADER: usize = 33;
const RECORD_CHUNK: usize = 800;
const MAX_PENDING_CREDITS: u64 = 65_536;
/// Proof verification is a builtin instruction, whose default compute
/// allowance is too small for most proofs.
const PROOF_UNITS: u32 = 400_000;
const FEE_BASIS_POINTS: u16 = 100;
const MAXIMUM_FEE: u64 = 5_000;

struct Holder {
    signer: Keypair,
    elgamal: ElGamalKeypair,
    ae: AeKey,
}

impl Holder {
    fn new() -> Self {
        Self {
            signer: Keypair::new(),
            elgamal: ElGamalKeypair::new_rand(),
            ae: AeKey::new_rand(),
        }
    }
}

struct Runner {
    client: RpcClient,
    payer: Keypair,
    program: Pubkey,
    out: PathBuf,
    records: Vec<Value>,
    checks: Vec<String>,
    contexts: Vec<Pubkey>,
    records_to_close: Vec<Pubkey>,
}

fn arg(name: &str) -> String {
    let args: Vec<String> = env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| panic!("missing {name}"))
}

fn git(args: &[&str]) -> String {
    let output = Command::new("git").args(args).output().expect("git");
    String::from_utf8(output.stdout)
        .expect("utf8")
        .trim()
        .to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn ae_bytes(key: &AeKey, amount: u64) -> [u8; 36] {
    key.encrypt(amount).to_bytes()
}

fn elgamal(bytes: &[u8; 64]) -> ElGamalCiphertext {
    ElGamalCiphertext::from_bytes(bytes).expect("ciphertext")
}

fn decryptable(state: &ConfidentialTransferAccount, key: &AeKey) -> Option<u64> {
    AeCiphertext::from_bytes(&state.decryptable_available_balance.0)?.decrypt(key)
}

impl Runner {
    fn send(
        &mut self,
        name: &str,
        instructions: &[Instruction],
        signers: &[&Keypair],
        ok: bool,
    ) -> Value {
        let name = format!("{:02}-{name}", self.records.len());
        let blockhash = self.client.get_latest_blockhash().expect("blockhash");
        let mut all: Vec<&Keypair> = vec![&self.payer];
        all.extend_from_slice(signers);
        let tx = Transaction::new_signed_with_payer(
            instructions,
            Some(&self.payer.pubkey()),
            &all,
            blockhash,
        );
        let config = RpcSendTransactionConfig {
            skip_preflight: true,
            ..RpcSendTransactionConfig::default()
        };
        let signature = self
            .client
            .send_transaction_with_config(&tx, config)
            .unwrap_or_else(|e| panic!("{name}: send: {e}"));
        let deadline = Instant::now() + Duration::from_secs(180);
        while self
            .client
            .get_signature_status_with_commitment(&signature, CommitmentConfig::finalized())
            .expect("status")
            .is_none()
        {
            assert!(
                Instant::now() < deadline,
                "{name}: {signature} not finalized"
            );
            thread::sleep(Duration::from_secs(2));
        }
        let tx = loop {
            let value: Value = self
                .client
                .send(
                    RpcRequest::GetTransaction,
                    json!([signature.to_string(), {
                        "encoding": "json", "commitment": "finalized",
                        "maxSupportedTransactionVersion": 0
                    }]),
                )
                .expect("getTransaction");
            if !value.is_null() {
                break value;
            }
            thread::sleep(Duration::from_secs(2));
        };
        fs::write(
            self.out.join(format!("{name}.transaction.json")),
            serde_json::to_string_pretty(&tx).expect("json"),
        )
        .expect("write");
        let error = tx["meta"]["err"].clone();
        let units = tx["meta"]["computeUnitsConsumed"].clone();
        self.records.push(json!({
            "name": name, "signature": signature.to_string(), "slot": tx["slot"],
            "error": error, "computeUnits": units,
        }));
        let status = if error.is_null() {
            "ok".to_string()
        } else {
            format!("refused {error}")
        };
        println!("{name}: finalized, {units} CU, {status}");
        if ok {
            assert!(
                error.is_null(),
                "{name}: {error} {}",
                tx["meta"]["logMessages"]
            );
        } else {
            assert!(!error.is_null(), "{name} must be refused");
        }
        tx
    }

    fn check(&mut self, what: impl Into<String>) {
        let what = what.into();
        println!("  checked: {what}");
        self.checks.push(what);
    }

    fn data(&self, key: &Pubkey) -> Option<Vec<u8>> {
        self.client
            .get_account_with_commitment(key, CommitmentConfig::finalized())
            .expect("account")
            .value
            .map(|account| account.data)
    }

    fn rent(&self, size: usize) -> u64 {
        self.client
            .get_minimum_balance_for_rent_exemption(size)
            .expect("rent")
    }

    fn deployed_elf(&self, program: &Pubkey) -> Vec<u8> {
        let account = self.data(program).expect("program account");
        let programdata = Pubkey::try_from(&account[4..36]).expect("programdata address");
        let data = self.data(&programdata).expect("programdata");
        data[45..].to_vec()
    }

    fn lab(&self, tag: u8, args: &[u8], accounts: Vec<AccountMeta>) -> Instruction {
        let mut data = vec![tag];
        data.extend_from_slice(args);
        Instruction::new_with_bytes(self.program, &data, accounts)
    }

    fn state(&self, account: &Pubkey) -> ConfidentialTransferAccount {
        let data = self.data(account).expect("token account");
        let state = StateWithExtensions::<TokenAccount>::unpack(&data).expect("unpack");
        *state
            .get_extension::<ConfidentialTransferAccount>()
            .expect("extension")
    }

    fn public_amount(&self, account: &Pubkey) -> u64 {
        let data = self.data(account).expect("token account");
        StateWithExtensions::<TokenAccount>::unpack(&data)
            .expect("unpack")
            .base
            .amount
    }

    /// Verify `proof` into a fresh context-state account the payer controls.
    /// A range proof is too large to share a transaction with the account's
    /// creation; the others are created and verified together.
    fn verify<T, U>(
        &mut self,
        name: &str,
        instruction: ProofInstruction,
        proof: &T,
        large: bool,
    ) -> Pubkey
    where
        T: Pod + ZkProofData<U>,
        U: Pod,
    {
        let context = Keypair::new();
        let size = std::mem::size_of::<ProofContextState<U>>();
        let create = system::create_account(
            &self.payer.pubkey(),
            &context.pubkey(),
            self.rent(size),
            size as u64,
            &solana_zk_elgamal_proof_interface::id(),
        );
        let payer = self.payer.pubkey();
        let verify = instruction.encode_verify_proof(
            Some(ContextStateInfo {
                context_state_account: &context.pubkey(),
                context_state_authority: &payer,
            }),
            proof,
        );
        let limit = ComputeBudgetInstruction::set_compute_unit_limit(PROOF_UNITS);
        if large {
            self.send(&format!("{name}-context"), &[create], &[&context], true);
            self.send(name, &[limit, verify], &[], true);
        } else {
            self.send(name, &[create, limit, verify], &[&context], true);
        }
        self.contexts.push(context.pubkey());
        context.pubkey()
    }

    /// Verify a proof too large for a transaction: write it into an SPL
    /// Record account in chunks, then verify it from there.
    fn verify_from_record<T, U>(
        &mut self,
        name: &str,
        instruction: ProofInstruction,
        proof: &T,
    ) -> Pubkey
    where
        T: Pod + ZkProofData<U>,
        U: Pod,
    {
        let bytes = bytemuck::bytes_of(proof).to_vec();
        let record = Keypair::new();
        let payer = self.payer.pubkey();
        let space = RECORD_HEADER + bytes.len();
        let create = system::create_account(
            &payer,
            &record.pubkey(),
            self.rent(space),
            space as u64,
            &RECORD_PROGRAM,
        );
        let initialize = Instruction::new_with_bytes(
            RECORD_PROGRAM,
            &[0],
            vec![
                AccountMeta::new(record.pubkey(), false),
                AccountMeta::new_readonly(payer, false),
            ],
        );
        self.send(
            &format!("{name}-record"),
            &[create, initialize],
            &[&record],
            true,
        );
        for (i, chunk) in bytes.chunks(RECORD_CHUNK).enumerate() {
            let mut data = vec![1u8];
            data.extend_from_slice(&((i * RECORD_CHUNK) as u64).to_le_bytes());
            data.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
            data.extend_from_slice(chunk);
            let write = Instruction::new_with_bytes(
                RECORD_PROGRAM,
                &data,
                vec![
                    AccountMeta::new(record.pubkey(), false),
                    AccountMeta::new_readonly(payer, true),
                ],
            );
            self.send(&format!("{name}-record-write-{i}"), &[write], &[], true);
        }
        let stored = self.data(&record.pubkey()).expect("record");
        assert_eq!(
            &stored[RECORD_HEADER..],
            &bytes[..],
            "the record holds the proof"
        );
        self.check(format!(
            "{name}: the SPL Record account holds the {}-byte proof",
            bytes.len()
        ));
        self.records_to_close.push(record.pubkey());

        let context = Keypair::new();
        let size = std::mem::size_of::<ProofContextState<U>>();
        let create = system::create_account(
            &payer,
            &context.pubkey(),
            self.rent(size),
            size as u64,
            &solana_zk_elgamal_proof_interface::id(),
        );
        self.send(&format!("{name}-context"), &[create], &[&context], true);
        let verify = instruction.encode_verify_proof_from_account(
            Some(ContextStateInfo {
                context_state_account: &context.pubkey(),
                context_state_authority: &payer,
            }),
            &record.pubkey(),
            RECORD_HEADER as u32,
        );
        let limit = ComputeBudgetInstruction::set_compute_unit_limit(PROOF_UNITS);
        self.send(name, &[limit, verify], &[], true);
        self.contexts.push(context.pubkey());
        context.pubkey()
    }

    fn open_args(holder: &Holder, proof_offset: i8, extra_extension: u16) -> Vec<u8> {
        let mut args = MAX_PENDING_CREDITS.to_le_bytes().to_vec();
        args.extend_from_slice(&ae_bytes(&holder.ae, 0));
        args.push(proof_offset as u8);
        args.extend_from_slice(&extra_extension.to_le_bytes());
        args
    }

    /// A token account for `holder`, configured with its proof verified
    /// into a context-state account first.
    fn open_account(&mut self, name: &str, mint: Pubkey, holder: &Holder, extra: u16) -> Pubkey {
        let proof = build_pubkey_validity_proof_data(&holder.elgamal).expect("proof");
        let context = self.verify::<_, PubkeyValidityProofContext>(
            &format!("{name}-verify-pubkey-validity"),
            ProofInstruction::VerifyPubkeyValidity,
            &proof,
            false,
        );
        let account = Keypair::new();
        let payer = self.payer.pubkey();
        let open = self.lab(
            2,
            &Self::open_args(holder, 0, extra),
            vec![
                AccountMeta::new(payer, true),
                AccountMeta::new(account.pubkey(), true),
                AccountMeta::new_readonly(mint, false),
                AccountMeta::new_readonly(holder.signer.pubkey(), true),
                AccountMeta::new_readonly(context, false),
                AccountMeta::new_readonly(SYSTEM, false),
                AccountMeta::new_readonly(TOKEN_2022, false),
            ],
        );
        self.send(name, &[open], &[&account, &holder.signer], true);
        let state = self.state(&account.pubkey());
        assert_eq!(state.elgamal_pubkey.0, holder.elgamal.pubkey().to_bytes());
        self.check(format!("{name}: configured with the holder's ElGamal key"));
        account.pubkey()
    }

    fn deposit_and_apply(
        &mut self,
        name: &str,
        mint: Pubkey,
        account: Pubkey,
        holder: &Holder,
        amount: u64,
    ) {
        let payer = self.payer.pubkey();
        let mint_to = spl_token_2022_interface::instruction::mint_to_checked(
            &TOKEN_2022,
            &mint,
            &account,
            &payer,
            &[],
            amount,
            DECIMALS,
        )
        .expect("mint_to");
        self.send(&format!("{name}-mint-to"), &[mint_to], &[], true);
        let mut args = amount.to_le_bytes().to_vec();
        args.push(DECIMALS);
        let deposit = self.lab(
            4,
            &args,
            vec![
                AccountMeta::new(account, false),
                AccountMeta::new_readonly(mint, false),
                AccountMeta::new_readonly(holder.signer.pubkey(), true),
                AccountMeta::new_readonly(TOKEN_2022, false),
            ],
        );
        self.send(
            &format!("{name}-deposit"),
            &[deposit],
            &[&holder.signer],
            true,
        );
        let state = self.state(&account);
        let lo = elgamal(&state.pending_balance_lo.0).decrypt_u32(holder.elgamal.secret());
        let hi = elgamal(&state.pending_balance_hi.0).decrypt_u32(holder.elgamal.secret());
        assert_eq!((lo, hi), (Some(amount & 0xffff), Some(amount >> 16)));
        self.check(format!("{name}: the pending balance decrypts to {amount}"));
        let counter = u64::from(state.pending_balance_credit_counter);
        let mut args = counter.to_le_bytes().to_vec();
        args.extend_from_slice(&ae_bytes(&holder.ae, amount));
        let apply = self.lab(
            5,
            &args,
            vec![
                AccountMeta::new(account, false),
                AccountMeta::new_readonly(holder.signer.pubkey(), true),
                AccountMeta::new_readonly(TOKEN_2022, false),
            ],
        );
        self.send(&format!("{name}-apply"), &[apply], &[&holder.signer], true);
        assert_eq!(decryptable(&self.state(&account), &holder.ae), Some(amount));
        self.check(format!("{name}: the available balance is {amount}"));
    }
}

fn main() {
    let program: Pubkey = arg("--program").parse().expect("program id");
    let elf = fs::read(arg("--elf")).expect("elf");
    let payer = read_keypair_file(arg("--payer")).expect("payer keypair");
    let rpc = arg("--rpc");
    let out = PathBuf::from(arg("--out"));
    assert!(
        git(&["status", "--porcelain"]).is_empty(),
        "commit source first"
    );
    let source = git(&["rev-parse", "HEAD"]);
    assert!(!Path::new(&out).exists(), "{} exists", out.display());
    fs::create_dir_all(&out).expect("out");

    let mut r = Runner {
        client: RpcClient::new_with_commitment(rpc.clone(), CommitmentConfig::finalized()),
        payer,
        program,
        out,
        records: Vec::new(),
        checks: Vec::new(),
        contexts: Vec::new(),
        records_to_close: Vec::new(),
    };
    let genesis = r.client.get_genesis_hash().expect("genesis").to_string();
    let payer = r.payer.pubkey();
    assert_eq!(
        r.deployed_elf(&program),
        elf,
        "deployed ELF differs from the tested artifact"
    );
    let token_elf = r.deployed_elf(&TOKEN_2022);
    let release = token_elf
        .windows(9)
        .position(|w| w == b"program@v")
        .map(|at| {
            let end = token_elf[at..]
                .iter()
                .position(|b| !(b.is_ascii_alphanumeric() || b".@".contains(b)))
                .unwrap_or(16);
            String::from_utf8_lossy(&token_elf[at..at + end]).to_string()
        });
    println!(
        "Token-2022 on this cluster: {release:?}, {} bytes",
        token_elf.len()
    );

    let alice = Holder::new();
    let bob = Holder::new();
    let carol = Holder::new();
    let dave = Holder::new();
    let auditor = ElGamalKeypair::new_rand();
    let auditor_key = auditor.pubkey().to_bytes();

    // ── The mint ─────────────────────────────────────────────────────
    let mint = Keypair::new();
    let mut args = vec![0u8];
    args.extend_from_slice(&auditor_key);
    let create = r.lab(
        0,
        &args,
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(mint.pubkey(), true),
            AccountMeta::new_readonly(SYSTEM, false),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    r.send("create-mint", &[create], &[&mint], true);
    let mint = mint.pubkey();
    let data = r.data(&mint).expect("mint");
    assert_eq!(data.len(), CONFIDENTIAL_MINT_LEN);
    assert_eq!(&data[CONFIDENTIAL_MINT_LEN - 32..], &auditor_key);
    r.check("create-mint: a 235-byte confidential mint, approval required, auditor key stored");

    let alice_account = r.open_account("open-alice", mint, &alice, 0);
    assert!(!bool::from(r.state(&alice_account).approved));
    r.check("open-alice: not approved yet, as the mint requires");
    let approve = r.lab(
        3,
        &[],
        vec![
            AccountMeta::new(alice_account, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(payer, true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    r.send("approve-alice", &[approve], &[], true);
    assert!(bool::from(r.state(&alice_account).approved));
    r.check("approve-alice: approved");

    let mut args = vec![1u8];
    args.extend_from_slice(&auditor_key);
    let update = r.lab(
        1,
        &args,
        vec![
            AccountMeta::new(mint, false),
            AccountMeta::new_readonly(payer, true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    r.send("update-mint-auto-approve", &[update], &[], true);
    let data = r.data(&mint).expect("mint");
    let state = StateWithExtensions::<Mint>::unpack(&data).expect("mint");
    let config = state
        .get_extension::<ConfidentialTransferMint>()
        .expect("extension");
    assert!(bool::from(config.auto_approve_new_accounts));
    r.check("update-mint-auto-approve: new accounts are approved when configured");

    // ── Carol, from her ElGamal registry ─────────────────────────────
    let registry_program = spl_elgamal_registry_interface::id();
    let registry = spl_elgamal_registry_interface::get_elgamal_registry_address(
        &carol.signer.pubkey(),
        &registry_program,
    );
    let fund = system::transfer(&payer, &registry, r.rent(64));
    let proof = build_pubkey_validity_proof_data(&carol.elgamal).expect("proof");
    let mut create_registry = spl_elgamal_registry_interface::instruction::create_registry(
        &carol.signer.pubkey(),
        ProofLocation::InstructionOffset(NonZeroI8::new(1).expect("offset"), &proof),
    )
    .expect("create_registry");
    let mut instructions = vec![
        fund,
        ComputeBudgetInstruction::set_compute_unit_limit(PROOF_UNITS),
    ];
    instructions.append(&mut create_registry);
    r.send(
        "create-carol-registry",
        &instructions,
        &[&carol.signer],
        true,
    );
    let stored = r.data(&registry).expect("registry");
    assert_eq!(&stored[..32], carol.signer.pubkey().as_ref());
    assert_eq!(&stored[32..64], &carol.elgamal.pubkey().to_bytes());
    r.check("create-carol-registry: the registry program stored Carol's key");
    let open_from_registry = |r: &Runner, account: Pubkey, owner: Pubkey| {
        r.lab(
            10,
            &[],
            vec![
                AccountMeta::new(payer, true),
                AccountMeta::new(account, true),
                AccountMeta::new_readonly(mint, false),
                AccountMeta::new_readonly(owner, false),
                AccountMeta::new_readonly(registry, false),
                AccountMeta::new_readonly(SYSTEM, false),
                AccountMeta::new_readonly(TOKEN_2022, false),
            ],
        )
    };
    let carol_account = Keypair::new();
    let ix = open_from_registry(&r, carol_account.pubkey(), carol.signer.pubkey());
    r.send("open-carol-from-registry", &[ix], &[&carol_account], true);
    let state = r.state(&carol_account.pubkey());
    assert_eq!(state.elgamal_pubkey.0, carol.elgamal.pubkey().to_bytes());
    assert!(bool::from(state.approved));
    r.check(
        "open-carol-from-registry: configured from the registry, grown by Token-2022, approved",
    );
    let dave_account = Keypair::new();
    let ix = open_from_registry(&r, dave_account.pubkey(), dave.signer.pubkey());
    r.send(
        "open-dave-with-carols-registry",
        &[ix],
        &[&dave_account],
        false,
    );
    assert!(r.data(&dave_account.pubkey()).is_none());
    r.check("open-dave-with-carols-registry: refused, no account left behind");

    // ── Bob, with the proof in the same transaction ─────────────────
    let proof = build_pubkey_validity_proof_data(&bob.elgamal).expect("proof");
    let verify = ProofInstruction::VerifyPubkeyValidity.encode_verify_proof(None, &proof);
    let bob_account = Keypair::new();
    let open = r.lab(
        2,
        &Runner::open_args(&bob, -1, 0),
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(bob_account.pubkey(), true),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(bob.signer.pubkey(), true),
            AccountMeta::new_readonly(INSTRUCTIONS_SYSVAR, false),
            AccountMeta::new_readonly(SYSTEM, false),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    let limit = ComputeBudgetInstruction::set_compute_unit_limit(PROOF_UNITS);
    r.send(
        "open-bob-proof-by-offset",
        &[limit, verify, open],
        &[&bob_account, &bob.signer],
        true,
    );
    let bob_account = bob_account.pubkey();
    let state = r.state(&bob_account);
    assert_eq!(state.elgamal_pubkey.0, bob.elgamal.pubkey().to_bytes());
    assert!(bool::from(state.approved));
    r.check("open-bob-proof-by-offset: configured and approved");

    // ── Alice's balance ──────────────────────────────────────────────
    let payer_key = payer;
    let mint_to = spl_token_2022_interface::instruction::mint_to_checked(
        &TOKEN_2022,
        &mint,
        &alice_account,
        &payer_key,
        &[],
        1_000_000,
        DECIMALS,
    )
    .expect("mint_to");
    r.send("mint-to-alice", &[mint_to], &[], true);
    let mut args = 700_000u64.to_le_bytes().to_vec();
    args.push(DECIMALS);
    let deposit = r.lab(
        4,
        &args,
        vec![
            AccountMeta::new(alice_account, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(alice.signer.pubkey(), true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    r.send("deposit", &[deposit], &[&alice.signer], true);
    assert_eq!(r.public_amount(&alice_account), 300_000);
    let state = r.state(&alice_account);
    let lo = elgamal(&state.pending_balance_lo.0).decrypt_u32(alice.elgamal.secret());
    let hi = elgamal(&state.pending_balance_hi.0).decrypt_u32(alice.elgamal.secret());
    assert_eq!((lo, hi), (Some(700_000 & 0xffff), Some(700_000 >> 16)));
    r.check("deposit: 300,000 left public, the pending balance decrypts to 700,000");
    let mut args = 1u64.to_le_bytes().to_vec();
    args.extend_from_slice(&ae_bytes(&alice.ae, 700_000));
    let apply = r.lab(
        5,
        &args,
        vec![
            AccountMeta::new(alice_account, false),
            AccountMeta::new_readonly(alice.signer.pubkey(), true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    r.send("apply-pending", &[apply], &[&alice.signer], true);
    assert_eq!(
        decryptable(&r.state(&alice_account), &alice.ae),
        Some(700_000)
    );
    r.check("apply-pending: the available balance is 700,000");

    // ── Withdraw ─────────────────────────────────────────────────────
    let withdraw_accounts = |equality: Pubkey, range: Pubkey| {
        vec![
            AccountMeta::new(alice_account, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(equality, false),
            AccountMeta::new_readonly(range, false),
            AccountMeta::new_readonly(alice.signer.pubkey(), true),
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
    let state = r.state(&alice_account);
    let proofs = withdraw_proof_data(
        &elgamal(&state.available_balance.0),
        700_000,
        200_000,
        &alice.elgamal,
    )
    .expect("proofs");
    let equality = r.verify::<_, CiphertextCommitmentEqualityProofContext>(
        "withdraw-verify-equality",
        ProofInstruction::VerifyCiphertextCommitmentEquality,
        &proofs.equality_proof_data,
        false,
    );
    let range = r.verify::<_, BatchedRangeProofContext>(
        "withdraw-verify-range-u64",
        ProofInstruction::VerifyBatchedRangeProofU64,
        &proofs.range_proof_data,
        true,
    );
    let withdraw = r.lab(
        6,
        &withdraw_args(200_000, 500_000),
        withdraw_accounts(equality, range),
    );
    r.send(
        "withdraw",
        std::slice::from_ref(&withdraw),
        &[&alice.signer],
        true,
    );
    assert_eq!(r.public_amount(&alice_account), 500_000);
    assert_eq!(
        decryptable(&r.state(&alice_account), &alice.ae),
        Some(500_000)
    );
    r.check("withdraw: 200,000 back to the public balance, 500,000 confidential");
    r.send("withdraw-replayed", &[withdraw], &[&alice.signer], false);
    assert_eq!(r.public_amount(&alice_account), 500_000);
    r.check("withdraw-replayed: the same proofs are refused, nothing moved");

    // ── Transfer ─────────────────────────────────────────────────────
    let state = r.state(&alice_account);
    let proofs = transfer_split_proof_data(
        &elgamal(&state.available_balance.0),
        &AeCiphertext::from_bytes(&state.decryptable_available_balance.0).expect("ae"),
        123_456,
        &alice.elgamal,
        &alice.ae,
        bob.elgamal.pubkey(),
        Some(auditor.pubkey()),
    )
    .expect("proofs");
    let equality = r.verify::<_, CiphertextCommitmentEqualityProofContext>(
        "transfer-verify-equality",
        ProofInstruction::VerifyCiphertextCommitmentEquality,
        &proofs.equality_proof_data,
        false,
    );
    let validity = r.verify::<_, BatchedGroupedCiphertext3HandlesValidityProofContext>(
        "transfer-verify-validity",
        ProofInstruction::VerifyBatchedGroupedCiphertext3HandlesValidity,
        &proofs
            .ciphertext_validity_proof_data_with_ciphertext
            .proof_data,
        false,
    );
    // A u128 range proof with the compute-budget instruction is a few bytes
    // over the transaction limit, so it is verified from a record account.
    let range = r.verify_from_record::<_, BatchedRangeProofContext>(
        "transfer-verify-range-u128",
        ProofInstruction::VerifyBatchedRangeProofU128,
        &proofs.range_proof_data,
    );
    let auditor_lo = proofs
        .ciphertext_validity_proof_data_with_ciphertext
        .ciphertext_lo
        .0;
    let auditor_hi = proofs
        .ciphertext_validity_proof_data_with_ciphertext
        .ciphertext_hi
        .0;
    let mut args = ae_bytes(&alice.ae, 500_000 - 123_456).to_vec();
    args.extend_from_slice(&auditor_lo);
    args.extend_from_slice(&auditor_hi);
    args.extend_from_slice(&[0, 0, 0]);
    let transfer = r.lab(
        7,
        &args,
        vec![
            AccountMeta::new(alice_account, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new(bob_account, false),
            AccountMeta::new_readonly(equality, false),
            AccountMeta::new_readonly(validity, false),
            AccountMeta::new_readonly(range, false),
            AccountMeta::new_readonly(alice.signer.pubkey(), true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    r.send("transfer", &[transfer], &[&alice.signer], true);
    assert_eq!(
        decryptable(&r.state(&alice_account), &alice.ae),
        Some(500_000 - 123_456)
    );
    let state = r.state(&bob_account);
    let lo = elgamal(&state.pending_balance_lo.0).decrypt_u32(bob.elgamal.secret());
    let hi = elgamal(&state.pending_balance_hi.0).decrypt_u32(bob.elgamal.secret());
    assert_eq!((lo, hi), (Some(123_456 & 0xffff), Some(123_456 >> 16)));
    let lo = elgamal(&auditor_lo).decrypt_u32(auditor.secret());
    let hi = elgamal(&auditor_hi).decrypt_u32(auditor.secret());
    assert_eq!((lo, hi), (Some(123_456 & 0xffff), Some(123_456 >> 16)));
    r.check("transfer: Bob and the auditor both decrypt 123,456; Alice keeps 376,544");
    let mut args = 1u64.to_le_bytes().to_vec();
    args.extend_from_slice(&ae_bytes(&bob.ae, 123_456));
    let apply = r.lab(
        5,
        &args,
        vec![
            AccountMeta::new(bob_account, false),
            AccountMeta::new_readonly(bob.signer.pubkey(), true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    r.send("apply-pending-bob", &[apply], &[&bob.signer], true);
    assert_eq!(decryptable(&r.state(&bob_account), &bob.ae), Some(123_456));
    r.check("apply-pending-bob: Bob's available balance is 123,456");

    // ── Credit toggles ───────────────────────────────────────────────
    let toggle = |r: &Runner, which: u8| {
        r.lab(
            8,
            &[which],
            vec![
                AccountMeta::new(bob_account, false),
                AccountMeta::new_readonly(bob.signer.pubkey(), true),
                AccountMeta::new_readonly(TOKEN_2022, false),
            ],
        )
    };
    let public_transfer = spl_token_2022_interface::instruction::transfer_checked(
        &TOKEN_2022,
        &alice_account,
        &mint,
        &bob_account,
        &alice.signer.pubkey(),
        &[],
        1,
        DECIMALS,
    )
    .expect("transfer_checked");
    let ix = toggle(&r, DISABLE_CONFIDENTIAL);
    r.send("disable-confidential-credits", &[ix], &[&bob.signer], true);
    assert!(!bool::from(
        r.state(&bob_account).allow_confidential_credits
    ));
    let ix = toggle(&r, ENABLE_CONFIDENTIAL);
    r.send("enable-confidential-credits", &[ix], &[&bob.signer], true);
    assert!(bool::from(r.state(&bob_account).allow_confidential_credits));
    let ix = toggle(&r, DISABLE_NON_CONFIDENTIAL);
    r.send(
        "disable-non-confidential-credits",
        &[ix],
        &[&bob.signer],
        true,
    );
    assert!(!bool::from(
        r.state(&bob_account).allow_non_confidential_credits
    ));
    r.send(
        "public-transfer-refused",
        std::slice::from_ref(&public_transfer),
        &[&alice.signer],
        false,
    );
    let ix = toggle(&r, ENABLE_NON_CONFIDENTIAL);
    r.send(
        "enable-non-confidential-credits",
        &[ix],
        &[&bob.signer],
        true,
    );
    r.send(
        "public-transfer",
        &[public_transfer],
        &[&alice.signer],
        true,
    );
    assert_eq!(r.public_amount(&bob_account), 1);
    r.check("credit toggles: each flag flips; a public transfer is refused while they are off");

    // ── Withdraw the rest and empty the account ──────────────────────
    let rest = 500_000 - 123_456;
    let state = r.state(&alice_account);
    let proofs = withdraw_proof_data(
        &elgamal(&state.available_balance.0),
        rest,
        rest,
        &alice.elgamal,
    )
    .expect("proofs");
    let equality = r.verify::<_, CiphertextCommitmentEqualityProofContext>(
        "withdraw-rest-verify-equality",
        ProofInstruction::VerifyCiphertextCommitmentEquality,
        &proofs.equality_proof_data,
        false,
    );
    let range = r.verify::<_, BatchedRangeProofContext>(
        "withdraw-rest-verify-range-u64",
        ProofInstruction::VerifyBatchedRangeProofU64,
        &proofs.range_proof_data,
        true,
    );
    let withdraw = r.lab(
        6,
        &withdraw_args(rest, 0),
        withdraw_accounts(equality, range),
    );
    r.send("withdraw-rest", &[withdraw], &[&alice.signer], true);
    assert_eq!(r.public_amount(&alice_account), 500_000 - 1 + rest);
    r.check("withdraw-rest: the whole confidential balance is public again");
    let state = r.state(&alice_account);
    let zero =
        build_zero_ciphertext_proof_data(&alice.elgamal, &elgamal(&state.available_balance.0))
            .expect("zero proof");
    let verify = ProofInstruction::VerifyZeroCiphertext.encode_verify_proof(None, &zero);
    let empty = r.lab(
        9,
        &[(-1i8) as u8],
        vec![
            AccountMeta::new(alice_account, false),
            AccountMeta::new_readonly(INSTRUCTIONS_SYSVAR, false),
            AccountMeta::new_readonly(alice.signer.pubkey(), true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    let limit = ComputeBudgetInstruction::set_compute_unit_limit(PROOF_UNITS);
    r.send(
        "empty-proof-by-offset",
        &[limit, verify, empty],
        &[&alice.signer],
        true,
    );
    r.check("empty-proof-by-offset: Token-2022 accepted the zero-ciphertext proof");

    // ── Transfer on a fee mint ───────────────────────────────────────
    let fee_mint = Keypair::new();
    let size = ExtensionType::try_calculate_account_len::<Mint>(&[
        ExtensionType::TransferFeeConfig,
        ExtensionType::ConfidentialTransferMint,
        ExtensionType::ConfidentialTransferFeeConfig,
    ])
    .expect("size");
    let withheld_authority = ElGamalKeypair::new_rand();
    use spl_token_2022_interface::extension::{
        confidential_transfer, confidential_transfer_fee, transfer_fee,
    };
    let setup = vec![
        system::create_account(
            &payer,
            &fee_mint.pubkey(),
            r.rent(size),
            size as u64,
            &TOKEN_2022,
        ),
        transfer_fee::instruction::initialize_transfer_fee_config(
            &TOKEN_2022,
            &fee_mint.pubkey(),
            Some(&payer),
            Some(&payer),
            FEE_BASIS_POINTS,
            MAXIMUM_FEE,
        )
        .expect("fee config"),
        confidential_transfer::instruction::initialize_mint(
            &TOKEN_2022,
            &fee_mint.pubkey(),
            Some(payer),
            true,
            Some((*auditor.pubkey()).into()),
        )
        .expect("confidential mint"),
        confidential_transfer_fee::instruction::initialize_confidential_transfer_fee_config(
            &TOKEN_2022,
            &fee_mint.pubkey(),
            Some(payer),
            &(*withheld_authority.pubkey()).into(),
        )
        .expect("confidential fee config"),
        spl_token_2022_interface::instruction::initialize_mint2(
            &TOKEN_2022,
            &fee_mint.pubkey(),
            &payer,
            None,
            DECIMALS,
        )
        .expect("mint"),
    ];
    r.send("create-fee-mint", &setup, &[&fee_mint], true);
    let fee_mint = fee_mint.pubkey();
    let fee_amount = u16::from(ExtensionType::ConfidentialTransferFeeAmount);
    let erin = Holder::new();
    let frank = Holder::new();
    let erin_account = r.open_account("open-erin", fee_mint, &erin, fee_amount);
    let frank_account = r.open_account("open-frank", fee_mint, &frank, fee_amount);
    r.deposit_and_apply("fund-erin", fee_mint, erin_account, &erin, 500_000);

    let amount = 100_000u64;
    let fee = (amount * FEE_BASIS_POINTS as u64)
        .div_ceil(10_000)
        .min(MAXIMUM_FEE);
    let state = r.state(&erin_account);
    let proofs = transfer_with_fee_split_proof_data(
        &elgamal(&state.available_balance.0),
        &AeCiphertext::from_bytes(&state.decryptable_available_balance.0).expect("ae"),
        amount,
        &erin.elgamal,
        &erin.ae,
        frank.elgamal.pubkey(),
        Some(auditor.pubkey()),
        withheld_authority.pubkey(),
        FEE_BASIS_POINTS,
        MAXIMUM_FEE,
    )
    .expect("proofs");
    let equality = r.verify::<_, CiphertextCommitmentEqualityProofContext>(
        "fee-transfer-verify-equality",
        ProofInstruction::VerifyCiphertextCommitmentEquality,
        &proofs.equality_proof_data,
        false,
    );
    let amount_validity = r.verify::<_, BatchedGroupedCiphertext3HandlesValidityProofContext>(
        "fee-transfer-verify-amount-validity",
        ProofInstruction::VerifyBatchedGroupedCiphertext3HandlesValidity,
        &proofs
            .transfer_amount_ciphertext_validity_proof_data_with_ciphertext
            .proof_data,
        false,
    );
    let fee_sigma = r.verify::<_, PercentageWithCapProofContext>(
        "fee-transfer-verify-fee-sigma",
        ProofInstruction::VerifyPercentageWithCap,
        &proofs.percentage_with_cap_proof_data,
        false,
    );
    let fee_validity = r.verify::<_, BatchedGroupedCiphertext2HandlesValidityProofContext>(
        "fee-transfer-verify-fee-validity",
        ProofInstruction::VerifyBatchedGroupedCiphertext2HandlesValidity,
        &proofs.fee_ciphertext_validity_proof_data,
        false,
    );
    let range = r.verify_from_record::<_, BatchedRangeProofContext>(
        "fee-transfer-verify-range-u256",
        ProofInstruction::VerifyBatchedRangeProofU256,
        &proofs.range_proof_data,
    );
    let with_ciphertext = &proofs.transfer_amount_ciphertext_validity_proof_data_with_ciphertext;
    let mut args = ae_bytes(&erin.ae, 500_000 - amount).to_vec();
    args.extend_from_slice(&with_ciphertext.ciphertext_lo.0);
    args.extend_from_slice(&with_ciphertext.ciphertext_hi.0);
    args.extend_from_slice(&[0; 5]);
    let transfer = r.lab(
        11,
        &args,
        vec![
            AccountMeta::new(erin_account, false),
            AccountMeta::new_readonly(fee_mint, false),
            AccountMeta::new(frank_account, false),
            AccountMeta::new_readonly(equality, false),
            AccountMeta::new_readonly(amount_validity, false),
            AccountMeta::new_readonly(fee_sigma, false),
            AccountMeta::new_readonly(fee_validity, false),
            AccountMeta::new_readonly(range, false),
            AccountMeta::new_readonly(erin.signer.pubkey(), true),
            AccountMeta::new_readonly(TOKEN_2022, false),
        ],
    );
    r.send("transfer-with-fee", &[transfer], &[&erin.signer], true);
    assert_eq!(
        decryptable(&r.state(&erin_account), &erin.ae),
        Some(500_000 - amount)
    );
    let state = r.state(&frank_account);
    let lo = elgamal(&state.pending_balance_lo.0)
        .decrypt_u32(frank.elgamal.secret())
        .expect("lo");
    let hi = elgamal(&state.pending_balance_hi.0)
        .decrypt_u32(frank.elgamal.secret())
        .expect("hi");
    assert_eq!(lo + (hi << 16), amount - fee);
    let data = r.data(&frank_account).expect("account");
    let account = StateWithExtensions::<TokenAccount>::unpack(&data).expect("unpack");
    let withheld = account
        .get_extension::<ConfidentialTransferFeeAmount>()
        .expect("fee amount");
    assert_eq!(
        elgamal(&withheld.withheld_amount.0).decrypt_u32(withheld_authority.secret()),
        Some(fee)
    );
    r.check(format!(
        "transfer-with-fee: Frank credited {}, {fee} withheld under the withdraw authority's key",
        amount - fee
    ));

    // ── Give the rent back ───────────────────────────────────────────
    let contexts = std::mem::take(&mut r.contexts);
    for (i, batch) in contexts.chunks(6).enumerate() {
        let closes: Vec<Instruction> = batch
            .iter()
            .map(|context| {
                close_context_state(
                    ContextStateInfo {
                        context_state_account: context,
                        context_state_authority: &payer,
                    },
                    &payer,
                )
            })
            .collect();
        r.send(&format!("close-contexts-{i}"), &closes, &[], true);
    }
    let records = std::mem::take(&mut r.records_to_close);
    for record in &records {
        let close = Instruction::new_with_bytes(
            RECORD_PROGRAM,
            &[3],
            vec![
                AccountMeta::new(*record, false),
                AccountMeta::new_readonly(payer, true),
                AccountMeta::new(payer, false),
            ],
        );
        r.send("close-record", &[close], &[], true);
    }
    r.check(format!(
        "{} context-state accounts and {} record accounts closed, rent returned",
        contexts.len(),
        records.len()
    ));

    assert_eq!(
        r.deployed_elf(&program),
        elf,
        "deployed ELF changed during the run"
    );
    assert_eq!(git(&["rev-parse", "HEAD"]), source);
    assert!(
        git(&["status", "--porcelain"]).is_empty(),
        "source changed during the run"
    );
    let receipt = json!({
        "schema": "hopper.confidential-flow.v1",
        "sourceCommit": source,
        "rpcEndpoint": rpc,
        "genesisHash": genesis,
        "commitment": "finalized",
        "programId": program.to_string(),
        "elfSha256": sha256_hex(&elf),
        "deployedElfMatchesBeforeAndAfter": true,
        "token2022": {"sourceRelease": release, "sha256": sha256_hex(&token_elf), "bytes": token_elf.len()},
        "accounts": {
            "mint": mint.to_string(), "feeMint": fee_mint.to_string(),
            "alice": alice_account.to_string(), "bob": bob_account.to_string(),
            "carol": carol_account.pubkey().to_string(), "carolRegistry": registry.to_string(),
            "erin": erin_account.to_string(), "frank": frank_account.to_string(),
        },
        "checks": r.checks,
        "transactions": r.records,
    });
    fs::write(
        r.out.join("receipt.json"),
        serde_json::to_string_pretty(&receipt).expect("json"),
    )
    .expect("receipt");
    println!("All confidential-flow steps passed.");
}
