#![cfg_attr(target_os = "solana", no_std)]
use hopper_native::{AccountView, Address, ProgramError, ProgramResult};
extern crate alloc;

#[cfg(target_os = "solana")]
hopper_native::program_entrypoint!(process_instruction);
#[cfg(target_os = "solana")]
hopper_native::no_allocator!();
#[cfg(target_os = "solana")]
hopper_native::nostd_panic_handler!();

pub fn process_instruction(_: &Address, accounts: &[AccountView], data: &[u8]) -> ProgramResult {
    let [from, to] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if from.is_signer() {
        return Err(ProgramError::InvalidArgument);
    }
    let result = match data {
        [2] => panic!("intentional abort regression probe"),
        [3] => {
            // SAFETY: a valid allocation layout; the fixture's no-allocator
            // implementation aborts instead of returning allocated memory.
            let ptr = unsafe { alloc::alloc::alloc(core::alloc::Layout::new::<u64>()) };
            core::hint::black_box(ptr);
            return Err(ProgramError::InvalidAccountData);
        }
        [0] => hopper_native::system::Transfer {
            from,
            to,
            lamports: 1,
        }
        .invoke(),
        [1] => hopper_native::token::Transfer {
            from,
            to,
            authority: from,
            amount: 1,
        }
        .invoke(),
        _ => return Err(ProgramError::InvalidInstructionData),
    };
    if result != Err(ProgramError::MissingRequiredSignature) {
        return Err(ProgramError::InvalidAccountData);
    }
    hopper_native::cpi::set_return_data(&[data[0], 0xAC]);
    Ok(())
}
