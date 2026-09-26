//! Exercises actual token CPIs followed by on-chain receipt policy checks.
#![cfg_attr(target_os = "solana", no_std)]
use hopper_runtime::instruction::{InstructionAccount, InstructionView};
use hopper_runtime::{AccountView, Address, ProgramError, ProgramResult};
use hopper_solana::{
    interface::interface_transfer_checked_with_program, transfer::TokenTransferSnapshot,
};
#[cfg(target_os = "solana")]
hopper_runtime::program_entrypoint!(process_instruction, 5);
#[cfg(target_os = "solana")]
hopper_runtime::no_allocator!();
#[cfg(target_os = "solana")]
hopper_runtime::nostd_panic_handler!();

pub fn process_instruction(_: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    match (data, accounts) {
        ([0, rest @ ..], [source, mint, destination, authority, token_program])
            if rest.len() == 24 =>
        {
            let amount = u64::from_le_bytes(rest[..8].try_into().unwrap());
            let min_received = u64::from_le_bytes(rest[8..16].try_into().unwrap());
            let actual = u64::from_le_bytes(rest[16..].try_into().unwrap());
            let snap = TokenTransferSnapshot::capture(
                source,
                destination,
                mint.address(),
                amount,
                min_received,
            )?;
            interface_transfer_checked_with_program(
                source,
                mint,
                destination,
                authority,
                token_program,
                actual,
                6,
            )?;
            let result = snap.verify()?;
            let mut returned = [0; 16];
            returned[..8].copy_from_slice(&result.debited.to_le_bytes());
            returned[8..].copy_from_slice(&result.credited.to_le_bytes());
            hopper_runtime::cpi::set_return_data(&returned);
            Ok(())
        }
        ([1, rest @ ..], [source, destination, extra, system]) if rest.len() == 8 => {
            let mut payload = [0; 12];
            payload[0] = 2;
            payload[4..].copy_from_slice(rest);
            let metas = [
                InstructionAccount::writable_signer(source.address()),
                InstructionAccount::writable(destination.address()),
            ];
            hopper_runtime::cpi::invoke_signed_deduped::<3>(
                &InstructionView {
                    program_id: system.address(),
                    accounts: &metas,
                    data: &payload,
                },
                &[extra, destination, source],
                &[],
            )
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
