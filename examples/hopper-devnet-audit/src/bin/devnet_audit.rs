use std::{env, path::PathBuf, str::FromStr};

use hopper::prelude::Address;
use hopper_devnet_audit::AuditState;
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{read_keypair_file, Keypair};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;

const DEFAULT_RPC: &str = "https://api.devnet.solana.com";
const DEFAULT_KEYPAIR: &str =
    "C:\\Users\\matts\\KEYPAIRS_BLUEFOOT_LABS\\HoppRy1HbNcHus9rmubDdXejDqAmhi55AURiCrq6tvxT.json";

fn main() {
    if let Err(err) = run() {
        eprintln!("hopper-devnet-audit runner failed: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut rpc_url = DEFAULT_RPC.to_string();
    let mut program_id: Option<Pubkey> = None;
    let mut keypair_path = PathBuf::from(DEFAULT_KEYPAIR);

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
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
                    Pubkey::from_str(&value).map_err(|err| format!("--program-id parse: {err}"))?,
                );
            }
            "--keypair" => {
                keypair_path = PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--keypair requires a path".to_string())?,
                );
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let program_id = program_id.ok_or_else(|| "--program-id is required".to_string())?;

    let payer = read_keypair_file(&keypair_path)
        .map_err(|err| format!("read keypair {}: {err}", keypair_path.display()))?;
    let client = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());
    let state = Keypair::new();
    let remaining_a = Keypair::new();
    let remaining_b = Keypair::new();

    println!("rpc           : {rpc_url}");
    println!("program id    : {program_id}");
    println!("authority     : {}", payer.pubkey());
    println!("state         : {}", state.pubkey());
    println!("alloc space   : {}", AuditState::ALLOC_SPACE);

    let balance = client
        .get_balance(&payer.pubkey())
        .map_err(|err| format!("get_balance: {err}"))?;
    println!("balance       : {} lamports", balance);

    send_instruction(
        &client,
        &payer,
        &[&state],
        Instruction::new_with_bytes(
            program_id,
            &[0, 0],
            vec![
                AccountMeta::new(payer.pubkey(), true),
                AccountMeta::new(state.pubkey(), true),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
        ),
        "initialize",
    )?;

    send_instruction(
        &client,
        &payer,
        &[],
        mutate_instruction(program_id, &payer.pubkey(), &state.pubkey(), 1),
        "rename",
    )?;
    send_instruction(
        &client,
        &payer,
        &[],
        mutate_instruction(program_id, &payer.pubkey(), &state.pubkey(), 2),
        "add_member",
    )?;
    send_instruction(
        &client,
        &payer,
        &[],
        mutate_instruction(program_id, &payer.pubkey(), &state.pubkey(), 3),
        "increment_segment",
    )?;
    send_instruction(
        &client,
        &payer,
        &[],
        mutate_instruction(program_id, &payer.pubkey(), &state.pubkey(), 4),
        "substrate_probe",
    )?;
    send_instruction(
        &client,
        &payer,
        &[&remaining_a, &remaining_b],
        Instruction::new_with_bytes(
            program_id,
            &[6],
            vec![
                AccountMeta::new(payer.pubkey(), true),
                AccountMeta::new(state.pubkey(), false),
                AccountMeta::new_readonly(remaining_a.pubkey(), true),
                AccountMeta::new_readonly(remaining_b.pubkey(), true),
            ],
        ),
        "remaining_signers",
    )?;
    send_instruction(
        &client,
        &payer,
        &[],
        Instruction::new_with_bytes(
            program_id,
            &[5],
            vec![
                AccountMeta::new_readonly(payer.pubkey(), true),
                AccountMeta::new_readonly(state.pubkey(), false),
            ],
        ),
        "audit",
    )?;

    verify_state(&client, &state.pubkey(), &payer.pubkey())?;
    Ok(())
}

fn print_usage() {
    eprintln!("Usage: cargo run -p hopper-devnet-audit --bin devnet-audit -- --program-id <pubkey> [--keypair <path>] [--rpc <url>]");
}

fn mutate_instruction(
    program_id: Pubkey,
    authority: &Pubkey,
    state: &Pubkey,
    discriminator: u8,
) -> Instruction {
    Instruction::new_with_bytes(
        program_id,
        &[discriminator],
        vec![
            AccountMeta::new(*authority, true),
            AccountMeta::new(*state, false),
        ],
    )
}

fn send_instruction(
    client: &RpcClient,
    payer: &Keypair,
    extra_signers: &[&Keypair],
    instruction: Instruction,
    label: &str,
) -> Result<(), String> {
    let recent = client
        .get_latest_blockhash()
        .map_err(|err| format!("{label}: get_latest_blockhash: {err}"))?;
    let mut signers: Vec<&dyn Signer> = Vec::with_capacity(extra_signers.len() + 1);
    signers.push(payer);
    for signer in extra_signers {
        signers.push(*signer);
    }
    let tx =
        Transaction::new_signed_with_payer(&[instruction], Some(&payer.pubkey()), &signers, recent);
    let sig = client
        .send_and_confirm_transaction(&tx)
        .map_err(|err| format!("{label}: send_and_confirm_transaction: {err}"))?;
    println!("{label:<18}: {sig}");
    Ok(())
}

fn verify_state(client: &RpcClient, state: &Pubkey, authority: &Pubkey) -> Result<(), String> {
    let account = client
        .get_account(state)
        .map_err(|err| format!("fetch state account: {err}"))?;
    if account.data.len() != AuditState::ALLOC_SPACE {
        return Err(format!(
            "state data length mismatch: expected {}, got {}",
            AuditState::ALLOC_SPACE,
            account.data.len()
        ));
    }

    let counter = read_u64(&account.data, AuditState::COUNTER_ABS_OFFSET as usize)?;
    let substrate_passes = read_u64(
        &account.data,
        AuditState::SUBSTRATE_PASSES_ABS_OFFSET as usize,
    )?;
    let remaining_signer_checks = read_u64(
        &account.data,
        AuditState::REMAINING_SIGNER_CHECKS_ABS_OFFSET as usize,
    )?;
    let label =
        AuditState::label(&account.data).map_err(|err| format!("read label tail: {err:?}"))?;
    let members =
        AuditState::members(&account.data).map_err(|err| format!("read members tail: {err:?}"))?;
    let authority_addr = Address::new(authority.to_bytes());

    if counter != 1 {
        return Err(format!("counter mismatch: expected 1, got {counter}"));
    }
    if substrate_passes != 1 {
        return Err(format!(
            "substrate_passes mismatch: expected 1, got {substrate_passes}"
        ));
    }
    if remaining_signer_checks != 2 {
        return Err(format!(
            "remaining_signer_checks mismatch: expected 2, got {remaining_signer_checks}"
        ));
    }
    if label != "hopper-live" {
        return Err(format!("label mismatch: expected hopper-live, got {label}"));
    }
    if !members.iter().any(|member| *member == authority_addr) {
        return Err("authority missing from members tail".to_string());
    }

    println!(
        "verified      : counter={counter}, substrate_passes={substrate_passes}, remaining_signer_checks={remaining_signer_checks}, label={label}, members={}",
        members.len()
    );
    Ok(())
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, String> {
    let end = offset
        .checked_add(8)
        .ok_or_else(|| "u64 offset overflow".to_string())?;
    let bytes = data
        .get(offset..end)
        .ok_or_else(|| format!("u64 read out of bounds at offset {offset}"))?;
    let mut array = [0u8; 8];
    array.copy_from_slice(bytes);
    Ok(u64::from_le_bytes(array))
}
