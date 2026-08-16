//! Canonical token-interface route fixture for Cicada's compiled SVM suite.
//!
//! Unlike the deliberately hostile legacy fixture, this program never writes
//! token-owned account data directly. Every balance change is performed by a
//! `TransferChecked` CPI to the supplied canonical SPL Token or Token-2022
//! program. The hostile commands also use canonical token-program CPIs. One
//! performs a persistent authority mutation; another performs a supply-neutral
//! MintTo-plus-Burn round trip that only a pre-CPI writable-mint gate can stop.

#![cfg_attr(target_os = "solana", no_std)]

use hopper::cpi::{InstructionAccount, InstructionView};
use hopper::prelude::*;

#[cfg(target_os = "solana")]
mod __hopper_sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
}

#[cfg(target_os = "solana")]
hopper::program_entrypoint!(process_instruction, 9);

/// Execute both canonical token transfers with the amounts in the route data.
pub const ROUTE_CANONICAL_SWAP: u8 = 0xB0;
/// Execute both transfers. Tests supply output below the intent minimum.
pub const ROUTE_CANONICAL_UNDERPAY: u8 = 0xB1;
/// Transfer output without spending input. Cicada must reject the settlement.
pub const ROUTE_CANONICAL_NO_INPUT: u8 = 0xB2;
/// Spend input without transferring output. Cicada must reject the settlement.
pub const ROUTE_CANONICAL_NO_OUTPUT: u8 = 0xB3;
/// Execute both transfers, then canonically change the source account owner.
pub const ROUTE_CANONICAL_MUTATE_SOURCE_POLICY: u8 = 0xB4;
/// Execute both legs, then canonically mint and burn one unit.
pub const ROUTE_CANONICAL_SUPPLY_NEUTRAL_MINT_BURN: u8 = 0xB5;

const ROUTE_DATA_LEN: usize = 19;
const REQUIRED_ACCOUNTS: usize = 9;

const SOURCE: usize = 0;
const INPUT_MINT: usize = 1;
const INPUT_SINK: usize = 2;
const VAULT_AUTHORITY: usize = 3;
const OUTPUT_RESERVE: usize = 4;
const OUTPUT_MINT: usize = 5;
const DESTINATION: usize = 6;
const LIQUIDITY_AUTHORITY: usize = 7;
const TOKEN_PROGRAM: usize = 8;

/// Route wire format:
///
/// ```text
/// [command: u8]
/// [input_amount: u64 little-endian]
/// [output_amount: u64 little-endian]
/// [input_decimals: u8]
/// [output_decimals: u8]
/// ```
///
/// Account order is documented in the crate README and deliberately mirrors
/// the two `TransferChecked` instructions. Account 3 is the Cicada vault PDA,
/// whose signer privilege is forwarded by Cicada's signed route CPI. Account
/// 7 is a transaction-signed solver/liquidity authority.
pub fn process_instruction(
    _program_id: &Address,
    accounts: &[AccountView<'_>],
    data: &[u8],
) -> ProgramResult {
    if accounts.len() != REQUIRED_ACCOUNTS || data.len() != ROUTE_DATA_LEN {
        return Err(ProgramError::InvalidInstructionData);
    }
    validate_envelope(accounts)?;

    let command = data[0];
    let input_amount = u64::from_le_bytes(data[1..9].try_into().unwrap());
    let output_amount = u64::from_le_bytes(data[9..17].try_into().unwrap());
    let input_decimals = data[17];
    let output_decimals = data[18];

    match command {
        ROUTE_CANONICAL_SWAP | ROUTE_CANONICAL_UNDERPAY => {
            transfer_input(accounts, input_amount, input_decimals)?;
            transfer_output(accounts, output_amount, output_decimals)
        }
        ROUTE_CANONICAL_NO_INPUT => transfer_output(accounts, output_amount, output_decimals),
        ROUTE_CANONICAL_NO_OUTPUT => transfer_input(accounts, input_amount, input_decimals),
        ROUTE_CANONICAL_MUTATE_SOURCE_POLICY => {
            transfer_input(accounts, input_amount, input_decimals)?;
            transfer_output(accounts, output_amount, output_decimals)?;
            mutate_source_authority(accounts)
        }
        ROUTE_CANONICAL_SUPPLY_NEUTRAL_MINT_BURN => {
            transfer_input(accounts, input_amount, input_decimals)?;
            transfer_output(accounts, output_amount, output_decimals)?;
            exercise_output_supply_round_trip(accounts, output_decimals)
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

fn validate_envelope(accounts: &[AccountView<'_>]) -> ProgramResult {
    accounts[SOURCE].require_writable()?;
    accounts[INPUT_SINK].require_writable()?;
    accounts[VAULT_AUTHORITY].require_signer()?;
    accounts[OUTPUT_RESERVE].require_writable()?;
    accounts[DESTINATION].require_writable()?;
    accounts[LIQUIDITY_AUTHORITY].require_signer()?;

    let kind = TokenProgramKind::for_account(&accounts[SOURCE])?;
    for index in [
        INPUT_MINT,
        INPUT_SINK,
        OUTPUT_RESERVE,
        OUTPUT_MINT,
        DESTINATION,
    ] {
        if TokenProgramKind::for_account(&accounts[index])? != kind {
            return Err(ProgramError::IncorrectProgramId);
        }
    }
    if accounts[TOKEN_PROGRAM].address() != kind.program_id() {
        return Err(ProgramError::IncorrectProgramId);
    }
    accounts[TOKEN_PROGRAM].check_executable()?;
    Ok(())
}

fn transfer_input(accounts: &[AccountView<'_>], amount: u64, decimals: u8) -> ProgramResult {
    hopper::token::require_token_authority(&accounts[SOURCE], &accounts[VAULT_AUTHORITY])?;
    interface_transfer_checked_with_program(
        &accounts[SOURCE],
        &accounts[INPUT_MINT],
        &accounts[INPUT_SINK],
        &accounts[VAULT_AUTHORITY],
        &accounts[TOKEN_PROGRAM],
        amount,
        decimals,
    )
}

fn transfer_output(accounts: &[AccountView<'_>], amount: u64, decimals: u8) -> ProgramResult {
    hopper::token::require_token_authority(
        &accounts[OUTPUT_RESERVE],
        &accounts[LIQUIDITY_AUTHORITY],
    )?;
    interface_transfer_checked_with_program(
        &accounts[OUTPUT_RESERVE],
        &accounts[OUTPUT_MINT],
        &accounts[DESTINATION],
        &accounts[LIQUIDITY_AUTHORITY],
        &accounts[TOKEN_PROGRAM],
        amount,
        decimals,
    )
}

fn mutate_source_authority(accounts: &[AccountView<'_>]) -> ProgramResult {
    let mut data = [0u8; 35];
    data[0] = 6; // SetAuthority
    data[1] = 2; // AccountOwner
    data[2] = 1; // COption::Some
    data[3..].copy_from_slice(accounts[LIQUIDITY_AUTHORITY].address().as_array());
    let metas = [
        InstructionAccount::writable(accounts[SOURCE].address()),
        InstructionAccount::readonly_signer(accounts[VAULT_AUTHORITY].address()),
    ];
    let views = [
        &accounts[SOURCE],
        &accounts[VAULT_AUTHORITY],
        &accounts[TOKEN_PROGRAM],
    ];
    let instruction = InstructionView {
        program_id: accounts[TOKEN_PROGRAM].address(),
        data: &data,
        accounts: &metas,
    };
    hopper::cpi::invoke(&instruction, &views)
}

fn exercise_output_supply_round_trip(accounts: &[AccountView<'_>], decimals: u8) -> ProgramResult {
    let mut mint_data = [0u8; 10];
    mint_data[0] = 14; // MintToChecked
    mint_data[1..9].copy_from_slice(&1u64.to_le_bytes());
    mint_data[9] = decimals;
    let mint_metas = [
        InstructionAccount::writable(accounts[OUTPUT_MINT].address()),
        InstructionAccount::writable(accounts[OUTPUT_RESERVE].address()),
        InstructionAccount::readonly_signer(accounts[LIQUIDITY_AUTHORITY].address()),
    ];
    let mint_views = [
        &accounts[OUTPUT_MINT],
        &accounts[OUTPUT_RESERVE],
        &accounts[LIQUIDITY_AUTHORITY],
        &accounts[TOKEN_PROGRAM],
    ];
    let mint_instruction = InstructionView {
        program_id: accounts[TOKEN_PROGRAM].address(),
        data: &mint_data,
        accounts: &mint_metas,
    };
    hopper::cpi::invoke(&mint_instruction, &mint_views)?;

    let mut burn_data = [0u8; 10];
    burn_data[0] = 15; // BurnChecked
    burn_data[1..9].copy_from_slice(&1u64.to_le_bytes());
    burn_data[9] = decimals;
    let burn_metas = [
        InstructionAccount::writable(accounts[OUTPUT_RESERVE].address()),
        InstructionAccount::writable(accounts[OUTPUT_MINT].address()),
        InstructionAccount::readonly_signer(accounts[LIQUIDITY_AUTHORITY].address()),
    ];
    let burn_views = [
        &accounts[OUTPUT_RESERVE],
        &accounts[OUTPUT_MINT],
        &accounts[LIQUIDITY_AUTHORITY],
        &accounts[TOKEN_PROGRAM],
    ];
    let burn_instruction = InstructionView {
        program_id: accounts[TOKEN_PROGRAM].address(),
        data: &burn_data,
        accounts: &burn_metas,
    };
    hopper::cpi::invoke(&burn_instruction, &burn_views)
}
