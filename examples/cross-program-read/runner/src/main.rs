use std::{env, path::PathBuf, str::FromStr};

use hopper::prelude::Address;
use hopper_xp_program_a::Vault;
use solana_client::rpc_client::RpcClient;
use solana_commitment_config::CommitmentConfig;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::{read_keypair_file, Keypair};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_transaction::Transaction;

const DEFAULT_RPC: &str = "https://api.devnet.solana.com";
const DEFAULT_AMOUNT: u64 = 42;
const VAULT_AUTHORITY_OFFSET: usize = hopper::hopper_core::account::HEADER_LEN;
const VAULT_BALANCE_OFFSET: usize = VAULT_AUTHORITY_OFFSET + 32;

fn main() {
    if let Err(err) = run() {
        eprintln!("cross-program devnet runner failed: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut rpc_url = DEFAULT_RPC.to_string();
    let mut keypair_path = env::var_os("SOLANA_KEYPAIR").map(PathBuf::from);
    let mut program_a_id: Option<Pubkey> = None;
    let mut program_b_id: Option<Pubkey> = None;
    let mut amount = DEFAULT_AMOUNT;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--rpc" => {
                rpc_url = args
                    .next()
                    .ok_or_else(|| "--rpc requires a URL".to_string())?;
            }
            "--keypair" => {
                keypair_path = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--keypair requires a path".to_string())?,
                ));
            }
            "--program-a" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--program-a requires a pubkey".to_string())?;
                program_a_id = Some(
                    Pubkey::from_str(&value).map_err(|err| format!("--program-a parse: {err}"))?,
                );
            }
            "--program-b" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--program-b requires a pubkey".to_string())?;
                program_b_id = Some(
                    Pubkey::from_str(&value).map_err(|err| format!("--program-b parse: {err}"))?,
                );
            }
            "--amount" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--amount requires a u64".to_string())?;
                amount = value
                    .parse::<u64>()
                    .map_err(|err| format!("--amount parse: {err}"))?;
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let program_a_id = program_a_id.ok_or_else(|| "--program-a is required".to_string())?;
    let program_b_id = program_b_id.ok_or_else(|| "--program-b is required".to_string())?;
    let keypair_path = keypair_path
        .ok_or_else(|| "--keypair is required unless SOLANA_KEYPAIR is set".to_string())?;
    let payer = read_keypair_file(&keypair_path)
        .map_err(|err| format!("read keypair {}: {err}", keypair_path.display()))?;
    let vault = Keypair::new();
    let client = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());

    println!("rpc           : {rpc_url}");
    println!("program a     : {program_a_id}");
    println!("program b     : {program_b_id}");
    println!("authority     : {}", payer.pubkey());
    println!("vault         : {}", vault.pubkey());
    println!("vault len     : {}", Vault::LEN);
    println!("amount        : {amount}");

    let balance = client
        .get_balance(&payer.pubkey())
        .map_err(|err| format!("get_balance: {err}"))?;
    println!("balance       : {} lamports", balance);

    send_instruction(
        &client,
        &payer,
        &[&vault],
        Instruction::new_with_bytes(
            program_a_id,
            &[0],
            vec![
                AccountMeta::new(payer.pubkey(), true),
                AccountMeta::new(vault.pubkey(), true),
                AccountMeta::new_readonly(Pubkey::default(), false),
            ],
        ),
        "program_a:init",
    )?;

    let mut deposit_data = Vec::with_capacity(9);
    deposit_data.push(1);
    deposit_data.extend_from_slice(&amount.to_le_bytes());
    send_instruction(
        &client,
        &payer,
        &[],
        Instruction::new_with_bytes(
            program_a_id,
            &deposit_data,
            vec![
                AccountMeta::new_readonly(payer.pubkey(), true),
                AccountMeta::new(vault.pubkey(), false),
            ],
        ),
        "program_a:deposit",
    )?;

    send_instruction(
        &client,
        &payer,
        &[],
        program_b_instruction(program_b_id, program_a_id, vault.pubkey(), &[0]),
        "program_b:read",
    )?;

    let mut min_balance_data = Vec::with_capacity(9);
    min_balance_data.push(1);
    min_balance_data.extend_from_slice(&amount.to_le_bytes());
    send_instruction(
        &client,
        &payer,
        &[],
        program_b_instruction(
            program_b_id,
            program_a_id,
            vault.pubkey(),
            &min_balance_data,
        ),
        "program_b:min",
    )?;

    verify_vault(
        &client,
        &vault.pubkey(),
        &program_a_id,
        &payer.pubkey(),
        amount,
    )?;
    Ok(())
}

fn print_usage() {
    eprintln!(
        "Usage: cargo run -p hopper-xp-devnet-runner -- --program-a <pubkey> --program-b <pubkey> --keypair <path> [--rpc <url>] [--amount <u64>]"
    );
    eprintln!("       SOLANA_KEYPAIR may be used instead of --keypair.");
}

fn program_b_instruction(
    program_b_id: Pubkey,
    program_a_id: Pubkey,
    vault: Pubkey,
    data: &[u8],
) -> Instruction {
    Instruction::new_with_bytes(
        program_b_id,
        data,
        vec![
            AccountMeta::new_readonly(program_a_id, false),
            AccountMeta::new_readonly(vault, false),
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

fn verify_vault(
    client: &RpcClient,
    vault: &Pubkey,
    program_a_id: &Pubkey,
    authority: &Pubkey,
    expected_balance: u64,
) -> Result<(), String> {
    let account = client
        .get_account(vault)
        .map_err(|err| format!("fetch vault account: {err}"))?;
    if account.owner != *program_a_id {
        return Err(format!(
            "vault owner mismatch: expected {program_a_id}, got {}",
            account.owner
        ));
    }
    if account.data.len() != Vault::LEN {
        return Err(format!(
            "vault data length mismatch: expected {}, got {}",
            Vault::LEN,
            account.data.len()
        ));
    }

    let authority_bytes = read_32(&account.data, VAULT_AUTHORITY_OFFSET)?;
    if authority_bytes != authority.to_bytes() {
        return Err("vault authority mismatch".to_string());
    }
    let balance = read_u64(&account.data, VAULT_BALANCE_OFFSET)?;
    if balance != expected_balance {
        return Err(format!(
            "vault balance mismatch: expected {expected_balance}, got {balance}"
        ));
    }

    let program_a_addr = Address::new_from_array(program_a_id.to_bytes());
    println!(
        "verified      : owner={}, balance={}, layout={:02x?}",
        program_a_addr,
        balance,
        Vault::LAYOUT_ID
    );
    Ok(())
}

fn read_32(data: &[u8], offset: usize) -> Result<[u8; 32], String> {
    let end = offset
        .checked_add(32)
        .ok_or_else(|| "[u8; 32] offset overflow".to_string())?;
    let bytes = data
        .get(offset..end)
        .ok_or_else(|| format!("[u8; 32] read out of bounds at offset {offset}"))?;
    let mut array = [0u8; 32];
    array.copy_from_slice(bytes);
    Ok(array)
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
