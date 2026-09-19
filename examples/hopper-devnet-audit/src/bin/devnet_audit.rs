use std::{
    env,
    path::PathBuf,
    str::FromStr,
    thread,
    time::{Duration, Instant},
};

use hopper::prelude::Address;
use hopper_devnet_audit::AuditState;
use solana_client::{rpc_client::RpcClient, rpc_config::RpcSendTransactionConfig};
use solana_commitment_config::CommitmentConfig;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{read_keypair_file, Keypair};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::{InstructionError, Transaction, TransactionError};

const DEFAULT_RPC: &str = "https://api.devnet.solana.com";
const DEVNET_GENESIS_HASH: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
const FINALIZED_TIMEOUT: Duration = Duration::from_secs(180);
const POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Eq, PartialEq)]
struct AccountSnapshot {
    lamports: u64,
    owner: Pubkey,
    executable: bool,
    rent_epoch: u64,
    data: Vec<u8>,
}

#[derive(Debug)]
struct FinalizedOutcome {
    signature: String,
    slot: u64,
    error: Option<TransactionError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AuditValues {
    counter: u64,
    bump: u8,
    flags: u16,
    substrate_passes: u64,
    remaining_signer_checks: u64,
    proof_checks: u64,
    token_policy_checks: u64,
    field_capability_checks: u64,
    label: String,
    member_count: usize,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("hopper-devnet-audit runner failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut rpc_url = DEFAULT_RPC.to_string();
    let mut program_id: Option<Pubkey> = None;
    let mut keypair_path = env::var_os("SOLANA_KEYPAIR").map(PathBuf::from);

    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--rpc" => {
                rpc_url = args
                    .next()
                    .ok_or_else(|| "--rpc requires a URL".to_string())?;
            }
            "--program-id" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--program-id requires a pubkey".to_string())?;
                program_id = Some(
                    Pubkey::from_str(&value)
                        .map_err(|_| "invalid --program-id pubkey".to_string())?,
                );
            }
            "--keypair" => {
                keypair_path = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--keypair requires a path".to_string())?,
                ));
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    require_devnet_gate()?;
    if rpc_url != DEFAULT_RPC {
        return Err("release evidence requires the public devnet RPC endpoint".to_string());
    }
    let receipt_path = env::var_os("HOPPER_DEVNET_RECEIPT")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "HOPPER_DEVNET_RECEIPT is required".to_string())?;
    let program_id = program_id.ok_or_else(|| "--program-id is required".to_string())?;
    let keypair_path = keypair_path
        .ok_or_else(|| "--keypair is required unless SOLANA_KEYPAIR is set".to_string())?;
    let payer = read_keypair_file(keypair_path)
        .map_err(|_| "failed to read keypair (path redacted)".to_string())?;
    let client = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::finalized());
    let (genesis_hash, node_version, feature_set) = cluster_identity(&client, &rpc_url)?;
    require_executable_program(&client, &rpc_url, &program_id)?;

    let state = Keypair::new();
    let wrong_authority = Keypair::new();
    let remaining_a = Keypair::new();
    let remaining_b = Keypair::new();
    let authority = payer.pubkey();

    // Every account must hold the rent-exempt minimum for its size after a
    // transaction, so the auxiliary signers are funded at exactly that floor
    // and drained by the same amount at cleanup.
    let auxiliary_lamports = client
        .get_minimum_balance_for_rent_exemption(0)
        .map_err(|error| format!("read rent-exempt minimum: {error}"))?;
    let fund_auxiliary_signers = submit_finalized(
        &client,
        &rpc_url,
        &payer,
        &[],
        &[
            system_transfer_instruction(authority, wrong_authority.pubkey(), auxiliary_lamports),
            system_transfer_instruction(authority, remaining_a.pubkey(), auxiliary_lamports),
            system_transfer_instruction(authority, remaining_b.pubkey(), auxiliary_lamports),
        ],
        false,
        "fund-auxiliary-signers",
    )?;
    require_success(&fund_auxiliary_signers, "fund-auxiliary-signers")?;
    for auxiliary in [
        wrong_authority.pubkey(),
        remaining_a.pubkey(),
        remaining_b.pubkey(),
    ] {
        wait_for_snapshot(&client, &rpc_url, &auxiliary, |snapshot| {
            snapshot.lamports == auxiliary_lamports
                && snapshot.owner == Pubkey::default()
                && snapshot.data.is_empty()
        })?;
    }

    let initialize = submit_finalized(
        &client,
        &rpc_url,
        &payer,
        &[&state],
        &[Instruction::new_with_bytes(
            program_id,
            &[0, 0],
            vec![
                AccountMeta::new(authority, true),
                AccountMeta::new(state.pubkey(), true),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
        )],
        false,
        "initialize",
    )?;
    require_success(&initialize, "initialize")?;
    let initialized_expected = AuditValues {
        counter: 0,
        bump: 0,
        flags: 0,
        substrate_passes: 0,
        remaining_signer_checks: 0,
        proof_checks: 0,
        token_policy_checks: 0,
        field_capability_checks: 0,
        label: "devnet-audit".to_string(),
        member_count: 1,
    };
    let initialized = wait_for_snapshot(&client, &rpc_url, &state.pubkey(), |snapshot| {
        decode_state(snapshot, &program_id, &authority) == Ok(initialized_expected.clone())
    })?;
    let initialized_values = decode_state(&initialized, &program_id, &authority)?;
    let initialized_hash = account_sha256(&initialized);

    let wrong_authority_outcome = submit_finalized(
        &client,
        &rpc_url,
        &payer,
        &[&wrong_authority],
        &[mutate_instruction(
            program_id,
            wrong_authority.pubkey(),
            state.pubkey(),
            1,
        )],
        true,
        "wrong-authority",
    )?;
    require_failure(
        &wrong_authority_outcome,
        "wrong-authority",
        InstructionError::InvalidAccountData,
    )?;
    let after_wrong_authority =
        wait_for_exact_snapshot(&client, &rpc_url, &state.pubkey(), &initialized)?;
    let after_wrong_authority_hash = account_sha256(&after_wrong_authority);

    let remaining_count_outcome = submit_finalized(
        &client,
        &rpc_url,
        &payer,
        &[&remaining_a],
        &[Instruction::new_with_bytes(
            program_id,
            &[6],
            vec![
                AccountMeta::new(authority, true),
                AccountMeta::new(state.pubkey(), false),
                AccountMeta::new_readonly(remaining_a.pubkey(), true),
            ],
        )],
        true,
        "insufficient-remaining-signers",
    )?;
    require_failure(
        &remaining_count_outcome,
        "insufficient-remaining-signers",
        InstructionError::MissingAccount,
    )?;
    let after_remaining =
        wait_for_exact_snapshot(&client, &rpc_url, &state.pubkey(), &initialized)?;
    let after_remaining_hash = account_sha256(&after_remaining);

    let readonly_outcome = submit_finalized(
        &client,
        &rpc_url,
        &payer,
        &[],
        &[Instruction::new_with_bytes(
            program_id,
            &[3],
            vec![
                AccountMeta::new(authority, true),
                AccountMeta::new_readonly(state.pubkey(), false),
            ],
        )],
        true,
        "readonly-state-mutation",
    )?;
    require_failure(
        &readonly_outcome,
        "readonly-state-mutation",
        InstructionError::Immutable,
    )?;
    let after_readonly = wait_for_exact_snapshot(&client, &rpc_url, &state.pubkey(), &initialized)?;
    let after_readonly_hash = account_sha256(&after_readonly);

    let rename = send_mutation(
        &client,
        &rpc_url,
        &payer,
        program_id,
        state.pubkey(),
        1,
        "rename",
    )?;
    let add_member = send_mutation(
        &client,
        &rpc_url,
        &payer,
        program_id,
        state.pubkey(),
        2,
        "add-member",
    )?;
    let increment = send_mutation(
        &client,
        &rpc_url,
        &payer,
        program_id,
        state.pubkey(),
        3,
        "increment-segment",
    )?;
    let substrate = send_mutation(
        &client,
        &rpc_url,
        &payer,
        program_id,
        state.pubkey(),
        4,
        "substrate-probe",
    )?;
    let proof = send_mutation(
        &client,
        &rpc_url,
        &payer,
        program_id,
        state.pubkey(),
        7,
        "proof-probe",
    )?;
    let token_policy = send_mutation(
        &client,
        &rpc_url,
        &payer,
        program_id,
        state.pubkey(),
        8,
        "token-policy-probe",
    )?;
    let field_capability = send_mutation(
        &client,
        &rpc_url,
        &payer,
        program_id,
        state.pubkey(),
        9,
        "field-capability-probe",
    )?;
    let remaining_signers = submit_finalized(
        &client,
        &rpc_url,
        &payer,
        &[&remaining_a, &remaining_b],
        &[Instruction::new_with_bytes(
            program_id,
            &[6],
            vec![
                AccountMeta::new(authority, true),
                AccountMeta::new(state.pubkey(), false),
                AccountMeta::new_readonly(remaining_a.pubkey(), true),
                AccountMeta::new_readonly(remaining_b.pubkey(), true),
            ],
        )],
        false,
        "remaining-signers",
    )?;
    require_success(&remaining_signers, "remaining-signers")?;
    let audit = submit_finalized(
        &client,
        &rpc_url,
        &payer,
        &[],
        &[Instruction::new_with_bytes(
            program_id,
            &[5],
            vec![
                AccountMeta::new_readonly(authority, true),
                AccountMeta::new_readonly(state.pubkey(), false),
            ],
        )],
        false,
        "read-audit",
    )?;
    require_success(&audit, "read-audit")?;

    let final_expected = AuditValues {
        counter: 1,
        bump: 0,
        flags: 0,
        substrate_passes: 1,
        remaining_signer_checks: 2,
        proof_checks: 1,
        token_policy_checks: 1,
        field_capability_checks: 1,
        label: "hopper-live".to_string(),
        member_count: 1,
    };
    let final_snapshot = wait_for_snapshot(&client, &rpc_url, &state.pubkey(), |snapshot| {
        decode_state(snapshot, &program_id, &authority) == Ok(final_expected.clone())
    })?;
    let final_values = decode_state(&final_snapshot, &program_id, &authority)?;
    let final_hash = account_sha256(&final_snapshot);

    let cleanup_auxiliary_signers = submit_finalized(
        &client,
        &rpc_url,
        &payer,
        &[&wrong_authority, &remaining_a, &remaining_b],
        &[
            system_transfer_instruction(wrong_authority.pubkey(), authority, auxiliary_lamports),
            system_transfer_instruction(remaining_a.pubkey(), authority, auxiliary_lamports),
            system_transfer_instruction(remaining_b.pubkey(), authority, auxiliary_lamports),
        ],
        false,
        "cleanup-auxiliary-signers",
    )?;
    require_success(&cleanup_auxiliary_signers, "cleanup-auxiliary-signers")?;
    for auxiliary in [
        wrong_authority.pubkey(),
        remaining_a.pubkey(),
        remaining_b.pubkey(),
    ] {
        wait_for_absence(&client, &rpc_url, &auxiliary)?;
    }

    let transactions = [
        transaction_json(
            "fund-auxiliary-signers",
            &fund_auxiliary_signers,
            "succeeded",
            None,
        ),
        transaction_json("initialize", &initialize, "succeeded", None),
        transaction_json(
            "wrong-authority",
            &wrong_authority_outcome,
            "rejected",
            Some("InvalidAccountData"),
        ),
        transaction_json(
            "insufficient-remaining-signers",
            &remaining_count_outcome,
            "rejected",
            Some("MissingAccount"),
        ),
        transaction_json(
            "readonly-state-mutation",
            &readonly_outcome,
            "rejected",
            Some("Immutable"),
        ),
        transaction_json("rename", &rename, "succeeded", None),
        transaction_json("add-member", &add_member, "succeeded", None),
        transaction_json("increment-segment", &increment, "succeeded", None),
        transaction_json("substrate-probe", &substrate, "succeeded", None),
        transaction_json("proof-probe", &proof, "succeeded", None),
        transaction_json("token-policy-probe", &token_policy, "succeeded", None),
        transaction_json(
            "field-capability-probe",
            &field_capability,
            "succeeded",
            None,
        ),
        transaction_json("remaining-signers", &remaining_signers, "succeeded", None),
        transaction_json("read-audit", &audit, "succeeded", None),
        transaction_json(
            "cleanup-auxiliary-signers",
            &cleanup_auxiliary_signers,
            "succeeded",
            None,
        ),
    ]
    .join(",");
    let feature_set_json = feature_set
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string());
    let receipt = format!(
        "{{\"schema\":\"hopper.devnet-evidence.v1\",\"example\":\"hopper-devnet-audit\",\"commitment\":\"finalized\",\"rpc_endpoint\":\"redacted\",\"cluster\":{{\"genesis_hash\":{},\"node_version\":{},\"feature_set\":{}}},\"program_id\":{},\"accounts\":{{\"authority\":{},\"state\":{},\"wrong_authority\":{},\"remaining_signer_a\":{},\"remaining_signer_b\":{}}},\"transactions\":[{}],\"state\":{{\"initialized\":{{\"account_sha256\":{},\"counter\":{},\"label\":{},\"member_count\":{}}},\"rollback\":{{\"wrong_authority\":{{\"pre_account_sha256\":{},\"post_account_sha256\":{},\"exact_snapshot_unchanged\":true}},\"insufficient_remaining_signers\":{{\"pre_account_sha256\":{},\"post_account_sha256\":{},\"exact_snapshot_unchanged\":true}},\"readonly_state_mutation\":{{\"pre_account_sha256\":{},\"post_account_sha256\":{},\"exact_snapshot_unchanged\":true}}}},\"final\":{{\"account_sha256\":{},\"counter\":{},\"bump\":{},\"flags\":{},\"substrate_passes\":{},\"remaining_signer_checks\":{},\"proof_checks\":{},\"token_policy_checks\":{},\"field_capability_checks\":{},\"label\":{},\"member_count\":{}}},\"cleanup\":{{\"auxiliary_accounts_exist\":false}}}}}}",
        json_string(&genesis_hash),
        json_string(&node_version),
        feature_set_json,
        json_string(&program_id.to_string()),
        json_string(&authority.to_string()),
        json_string(&state.pubkey().to_string()),
        json_string(&wrong_authority.pubkey().to_string()),
        json_string(&remaining_a.pubkey().to_string()),
        json_string(&remaining_b.pubkey().to_string()),
        transactions,
        json_string(&initialized_hash),
        initialized_values.counter,
        json_string(&initialized_values.label),
        initialized_values.member_count,
        json_string(&initialized_hash),
        json_string(&after_wrong_authority_hash),
        json_string(&initialized_hash),
        json_string(&after_remaining_hash),
        json_string(&initialized_hash),
        json_string(&after_readonly_hash),
        json_string(&final_hash),
        final_values.counter,
        final_values.bump,
        final_values.flags,
        final_values.substrate_passes,
        final_values.remaining_signer_checks,
        final_values.proof_checks,
        final_values.token_policy_checks,
        final_values.field_capability_checks,
        json_string(&final_values.label),
        final_values.member_count,
    );
    std::fs::write(&receipt_path, format!("{receipt}\n"))
        .map_err(|_| "failed to write HOPPER_DEVNET_RECEIPT (path redacted)".to_string())?;
    println!("wrote hopper.devnet-evidence.v1 receipt");
    Ok(())
}

fn print_usage() {
    eprintln!(
        "Usage: HOPPER_DEVNET=1 HOPPER_DEVNET_RECEIPT=<path> cargo run -p hopper-devnet-audit --features devnet-client --bin devnet_audit -- --program-id <pubkey> --keypair <path> [--rpc <url>]"
    );
    eprintln!("       SOLANA_KEYPAIR may be used instead of --keypair.");
}

fn require_devnet_gate() -> Result<(), String> {
    match env::var("HOPPER_DEVNET") {
        Ok(value) if value == "1" => Ok(()),
        Ok(_) => Err("HOPPER_DEVNET must be exactly 1".to_string()),
        Err(env::VarError::NotPresent) => {
            Err("HOPPER_DEVNET=1 is required for the live devnet runner".to_string())
        }
        Err(env::VarError::NotUnicode(_)) => {
            Err("HOPPER_DEVNET must be valid Unicode and exactly 1".to_string())
        }
    }
}

fn cluster_identity(
    client: &RpcClient,
    rpc_url: &str,
) -> Result<(String, String, Option<u32>), String> {
    let genesis_hash = client
        .get_genesis_hash()
        .map_err(|error| rpc_error("get genesis hash", error, rpc_url))?
        .to_string();
    if genesis_hash != DEVNET_GENESIS_HASH {
        return Err(format!(
            "refusing non-devnet cluster: expected genesis {DEVNET_GENESIS_HASH}, got {genesis_hash}"
        ));
    }
    let version = client
        .get_version()
        .map_err(|error| rpc_error("get node version", error, rpc_url))?;
    Ok((genesis_hash, version.solana_core, version.feature_set))
}

fn require_executable_program(
    client: &RpcClient,
    rpc_url: &str,
    program: &Pubkey,
) -> Result<(), String> {
    let account = client
        .get_account_with_commitment(program, CommitmentConfig::finalized())
        .map_err(|error| rpc_error("get program account", error, rpc_url))?
        .value
        .ok_or_else(|| format!("program account {program} does not exist at finalized"))?;
    if !account.executable {
        return Err(format!("program account {program} is not executable"));
    }
    Ok(())
}

fn mutate_instruction(
    program_id: Pubkey,
    authority: Pubkey,
    state: Pubkey,
    discriminator: u8,
) -> Instruction {
    Instruction::new_with_bytes(
        program_id,
        &[discriminator],
        vec![
            AccountMeta::new(authority, true),
            AccountMeta::new(state, false),
        ],
    )
}

fn system_transfer_instruction(source: Pubkey, destination: Pubkey, lamports: u64) -> Instruction {
    // Stable System Program `Transfer` ABI: bincode enum tag 2 as u32 LE,
    // followed by the u64 LE lamport amount.
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&2u32.to_le_bytes());
    data.extend_from_slice(&lamports.to_le_bytes());
    Instruction {
        program_id: Pubkey::default(),
        accounts: vec![
            AccountMeta::new(source, true),
            AccountMeta::new(destination, false),
        ],
        data,
    }
}

fn send_mutation(
    client: &RpcClient,
    rpc_url: &str,
    payer: &Keypair,
    program_id: Pubkey,
    state: Pubkey,
    discriminator: u8,
    label: &str,
) -> Result<FinalizedOutcome, String> {
    let outcome = submit_finalized(
        client,
        rpc_url,
        payer,
        &[],
        &[mutate_instruction(
            program_id,
            payer.pubkey(),
            state,
            discriminator,
        )],
        false,
        label,
    )?;
    require_success(&outcome, label)?;
    Ok(outcome)
}

fn submit_finalized(
    client: &RpcClient,
    rpc_url: &str,
    payer: &Keypair,
    extra_signers: &[&Keypair],
    instructions: &[Instruction],
    skip_preflight: bool,
    label: &str,
) -> Result<FinalizedOutcome, String> {
    let blockhash = client
        .get_latest_blockhash()
        .map_err(|error| rpc_error(&format!("{label}: get blockhash"), error, rpc_url))?;
    let mut signers: Vec<&dyn Signer> = vec![payer];
    signers.extend(extra_signers.iter().map(|signer| *signer as &dyn Signer));
    let transaction = Transaction::new_signed_with_payer(
        instructions,
        Some(&payer.pubkey()),
        &signers,
        blockhash,
    );
    let signature = client
        .send_transaction_with_config(
            &transaction,
            RpcSendTransactionConfig {
                skip_preflight,
                preflight_commitment: Some(CommitmentConfig::finalized().commitment),
                max_retries: Some(5),
                ..RpcSendTransactionConfig::default()
            },
        )
        .map_err(|error| rpc_error(&format!("{label}: send transaction"), error, rpc_url))?;
    wait_for_finalized(client, rpc_url, signature.to_string(), label)
}

fn wait_for_finalized(
    client: &RpcClient,
    rpc_url: &str,
    signature: String,
    label: &str,
) -> Result<FinalizedOutcome, String> {
    let parsed = signature
        .parse()
        .map_err(|_| format!("{label}: invalid signature returned by RPC"))?;
    let deadline = Instant::now() + FINALIZED_TIMEOUT;
    loop {
        let response = client.get_signature_statuses(&[parsed]).map_err(|error| {
            rpc_error(&format!("{label}: get signature status"), error, rpc_url)
        })?;
        if let Some(status) = response.value.into_iter().next().flatten() {
            if status.satisfies_commitment(CommitmentConfig::finalized()) {
                return Ok(FinalizedOutcome {
                    signature,
                    slot: status.slot,
                    error: status.err,
                });
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{label}: transaction {signature} did not reach finalized before timeout"
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn require_success(outcome: &FinalizedOutcome, label: &str) -> Result<(), String> {
    match &outcome.error {
        None => Ok(()),
        Some(error) => Err(format!(
            "{label}: finalized transaction {} failed: {error}",
            outcome.signature
        )),
    }
}

fn require_failure(
    outcome: &FinalizedOutcome,
    label: &str,
    expected_error: InstructionError,
) -> Result<(), String> {
    match outcome.error.as_ref() {
        Some(TransactionError::InstructionError(0, actual)) if *actual == expected_error => Ok(()),
        Some(actual) => Err(format!(
            "{label}: expected InstructionError(0, {expected_error:?}), got {actual:?}"
        )),
        None => Err(format!(
            "{label}: finalized transaction {} unexpectedly succeeded",
            outcome.signature
        )),
    }
}

fn fetch_snapshot(
    client: &RpcClient,
    rpc_url: &str,
    address: &Pubkey,
) -> Result<Option<AccountSnapshot>, String> {
    let account = client
        .get_account_with_commitment(address, CommitmentConfig::finalized())
        .map_err(|error| rpc_error("get finalized account", error, rpc_url))?
        .value;
    Ok(account.map(|account| AccountSnapshot {
        lamports: account.lamports,
        owner: account.owner,
        executable: account.executable,
        rent_epoch: account.rent_epoch,
        data: account.data,
    }))
}

fn wait_for_snapshot(
    client: &RpcClient,
    rpc_url: &str,
    address: &Pubkey,
    predicate: impl Fn(&AccountSnapshot) -> bool,
) -> Result<AccountSnapshot, String> {
    let deadline = Instant::now() + FINALIZED_TIMEOUT;
    loop {
        if let Some(snapshot) = fetch_snapshot(client, rpc_url, address)? {
            if predicate(&snapshot) {
                return Ok(snapshot);
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "account {address} did not reach expected finalized state before timeout"
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_exact_snapshot(
    client: &RpcClient,
    rpc_url: &str,
    address: &Pubkey,
    expected: &AccountSnapshot,
) -> Result<AccountSnapshot, String> {
    wait_for_snapshot(client, rpc_url, address, |snapshot| snapshot == expected)
}

fn wait_for_absence(client: &RpcClient, rpc_url: &str, address: &Pubkey) -> Result<(), String> {
    let deadline = Instant::now() + FINALIZED_TIMEOUT;
    loop {
        if fetch_snapshot(client, rpc_url, address)?.is_none() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "account {address} remained visible at finalized before timeout"
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn decode_state(
    snapshot: &AccountSnapshot,
    program_id: &Pubkey,
    authority: &Pubkey,
) -> Result<AuditValues, String> {
    if snapshot.owner != *program_id {
        return Err(format!(
            "state owner mismatch: expected {program_id}, got {}",
            snapshot.owner
        ));
    }
    if snapshot.executable {
        return Err("audit state account must not be executable".to_string());
    }
    if snapshot.data.len() != AuditState::ALLOC_SPACE {
        return Err(format!(
            "state data length mismatch: expected {}, got {}",
            AuditState::ALLOC_SPACE,
            snapshot.data.len()
        ));
    }
    if snapshot.data[0] != AuditState::DISC || snapshot.data[1] != AuditState::VERSION {
        return Err("state header discriminator/version mismatch".to_string());
    }
    if snapshot.data[4..12] != AuditState::LAYOUT_ID {
        return Err("state layout id mismatch".to_string());
    }
    let authority_bytes = field_bytes(
        &snapshot.data,
        AuditState::AUTHORITY_ABS_OFFSET as usize,
        32,
        "authority",
    )?;
    if authority_bytes != authority.as_ref() {
        return Err("state authority mismatch".to_string());
    }
    let label = AuditState::label(&snapshot.data)
        .map_err(|error| format!("read label tail: {error:?}"))?
        .to_string();
    let members = AuditState::members(&snapshot.data)
        .map_err(|error| format!("read members tail: {error:?}"))?;
    let authority_address = Address::new(authority.to_bytes());
    if members != [authority_address] {
        return Err("members tail must contain exactly the authority".to_string());
    }
    Ok(AuditValues {
        counter: read_u64(
            &snapshot.data,
            AuditState::COUNTER_ABS_OFFSET as usize,
            "counter",
        )?,
        bump: *snapshot
            .data
            .get(AuditState::BUMP_ABS_OFFSET as usize)
            .ok_or_else(|| "bump field out of bounds".to_string())?,
        flags: read_u16(
            &snapshot.data,
            AuditState::FLAGS_ABS_OFFSET as usize,
            "flags",
        )?,
        substrate_passes: read_u64(
            &snapshot.data,
            AuditState::SUBSTRATE_PASSES_ABS_OFFSET as usize,
            "substrate_passes",
        )?,
        remaining_signer_checks: read_u64(
            &snapshot.data,
            AuditState::REMAINING_SIGNER_CHECKS_ABS_OFFSET as usize,
            "remaining_signer_checks",
        )?,
        proof_checks: read_u64(
            &snapshot.data,
            AuditState::PROOF_CHECKS_ABS_OFFSET as usize,
            "proof_checks",
        )?,
        token_policy_checks: read_u64(
            &snapshot.data,
            AuditState::TOKEN_POLICY_CHECKS_ABS_OFFSET as usize,
            "token_policy_checks",
        )?,
        field_capability_checks: read_u64(
            &snapshot.data,
            AuditState::FIELD_CAPABILITY_CHECKS_ABS_OFFSET as usize,
            "field_capability_checks",
        )?,
        label,
        member_count: members.len(),
    })
}

fn field_bytes<'a>(
    data: &'a [u8],
    offset: usize,
    length: usize,
    label: &str,
) -> Result<&'a [u8], String> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| format!("{label} offset overflow"))?;
    data.get(offset..end)
        .ok_or_else(|| format!("{label} field out of bounds"))
}

fn read_u64(data: &[u8], offset: usize, label: &str) -> Result<u64, String> {
    let bytes = field_bytes(data, offset, 8, label)?;
    let mut value = [0u8; 8];
    value.copy_from_slice(bytes);
    Ok(u64::from_le_bytes(value))
}

fn read_u16(data: &[u8], offset: usize, label: &str) -> Result<u16, String> {
    let bytes = field_bytes(data, offset, 2, label)?;
    let mut value = [0u8; 2];
    value.copy_from_slice(bytes);
    Ok(u16::from_le_bytes(value))
}

fn transaction_json(
    name: &str,
    outcome: &FinalizedOutcome,
    expected: &str,
    expected_error: Option<&str>,
) -> String {
    let error = outcome
        .error
        .as_ref()
        .map(|error| json_string(&format!("{error:?}")))
        .unwrap_or_else(|| "null".to_string());
    let expected_error = expected_error
        .map(json_string)
        .unwrap_or_else(|| "null".to_string());
    format!(
        "{{\"name\":{},\"signature\":{},\"slot\":{},\"outcome\":{},\"expected_error\":{},\"error\":{}}}",
        json_string(name),
        json_string(&outcome.signature),
        outcome.slot,
        json_string(expected),
        expected_error,
        error,
    )
}

fn account_sha256(snapshot: &AccountSnapshot) -> String {
    let mut canonical = Vec::with_capacity(8 + 32 + 1 + 8 + 8 + snapshot.data.len());
    canonical.extend_from_slice(&snapshot.lamports.to_le_bytes());
    canonical.extend_from_slice(snapshot.owner.as_ref());
    canonical.push(u8::from(snapshot.executable));
    canonical.extend_from_slice(&snapshot.rent_epoch.to_le_bytes());
    canonical.extend_from_slice(&(snapshot.data.len() as u64).to_le_bytes());
    canonical.extend_from_slice(&snapshot.data);
    hex(&hopper::hopper_runtime::sha256::sha256(&canonical))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn rpc_error(label: &str, error: impl std::fmt::Display, rpc_url: &str) -> String {
    let detail = format!("{error}").replace(rpc_url, "<redacted-rpc-url>");
    if detail.contains("://") {
        format!("{label}: RPC request failed (URL redacted)")
    } else {
        format!("{label}: {detail}")
    }
}

fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                use std::fmt::Write;
                write!(output, "\\u{:04x}", character as u32).expect("write to String cannot fail");
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devnet_genesis_pin_is_complete() {
        assert_eq!(
            DEVNET_GENESIS_HASH,
            "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG"
        );
    }

    #[test]
    fn rpc_errors_never_echo_credential_bearing_urls() {
        let url = "https://user:secret@example.invalid/path?key=secret";
        let output = rpc_error("probe", format!("request to {url} failed"), url);
        assert!(!output.contains("secret"));
        assert!(!output.contains("://"));
    }

    #[test]
    fn json_string_escapes_evidence_values() {
        assert_eq!(json_string("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    }

    #[test]
    fn system_transfer_uses_the_stable_wire_shape() {
        let source = Pubkey::new_unique();
        let destination = Pubkey::new_unique();
        let instruction = system_transfer_instruction(source, destination, 7);
        assert_eq!(instruction.program_id, Pubkey::default());
        assert_eq!(instruction.data, [2, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(instruction.accounts.len(), 2);
        assert_eq!(instruction.accounts[0].pubkey, source);
        assert!(instruction.accounts[0].is_signer);
        assert!(instruction.accounts[0].is_writable);
        assert_eq!(instruction.accounts[1].pubkey, destination);
        assert!(!instruction.accounts[1].is_signer);
        assert!(instruction.accounts[1].is_writable);
    }
}
