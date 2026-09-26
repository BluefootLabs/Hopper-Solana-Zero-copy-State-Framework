#![cfg_attr(target_os = "solana", no_std)]
use hopper_native::instruction::{InstructionAccount, InstructionView};
use hopper_native::{AccountView, Address, ProgramError, ProgramResult};
#[cfg(target_os = "solana")]
hopper_native::program_entrypoint!(process_instruction, 2);
#[cfg(target_os = "solana")]
hopper_native::no_allocator!();
#[cfg(target_os = "solana")]
hopper_native::nostd_panic_handler!();

pub fn process_instruction(_: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    match (data, accounts) {
        ([0], []) => {
            hopper_native::cpi::set_return_data(&42u64.to_le_bytes());
            Ok(())
        }
        ([1], []) => {
            hopper_native::cpi::set_return_data(&[42]);
            Ok(())
        }
        ([2], []) => Ok(()),
        ([3], [target]) => hopper_native::cpi::invoke::<1>(
            &InstructionView {
                program_id: target.address(),
                accounts: &[],
                data: &[0],
            },
            &[target],
        ),
        ([tag @ 4..=6], [target]) => {
            let callee_tag = [tag - 4];
            hopper_native::return_data::invoke_and_read::<u64, 1>(
                &InstructionView {
                    program_id: target.address(),
                    accounts: &[],
                    data: &callee_tag,
                },
                &[target],
                &[],
            )
            .map(|_| ())
        }
        ([7], [target, nested]) => {
            let metas = [InstructionAccount::readonly(nested.address())];
            hopper_native::return_data::invoke_and_read::<u64, 1>(
                &InstructionView {
                    program_id: target.address(),
                    accounts: &metas,
                    data: &[3],
                },
                &[nested],
                &[],
            )
            .map(|_| ())
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
