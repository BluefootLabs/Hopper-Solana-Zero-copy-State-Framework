//! Native lifecycle regression and System-funded resize program.
#![cfg_attr(target_os = "solana", no_std)]
use hopper_native::batch::{close_and_transfer, transfer_lamports, ResizeWithPayer};
use hopper_native::{AccountView, Address, ProgramError, ProgramResult, RefMut};
#[cfg(target_os = "solana")]
hopper_native::program_entrypoint!(process_instruction, 4);
#[cfg(target_os = "solana")]
hopper_native::no_allocator!();
#[cfg(target_os = "solana")]
hopper_native::nostd_panic_handler!();

fn expect(result: ProgramResult, error: ProgramError) -> ProgramResult {
    if result == Err(error) {
        Ok(())
    } else {
        Err(ProgramError::Custom(900))
    }
}

pub fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let [state, recipient, payer, system_program] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    state.require_owned_by(program_id)?;
    let before = (
        state.lamports(),
        recipient.lamports(),
        payer.lamports(),
        state.data_len(),
    );
    match data {
        [0] => {
            let guard = state.try_borrow()?;
            expect(
                close_and_transfer(state, recipient),
                ProgramError::AccountBorrowFailed,
            )?;
            drop(guard);
        }
        [1] => transfer_lamports(state, state, 1)?,
        [2] => expect(
            close_and_transfer(state, state),
            ProgramError::InvalidArgument,
        )?,
        [3] => expect(
            close_and_transfer(state, recipient),
            ProgramError::Immutable,
        )?,
        [4] => return transfer_lamports(state, recipient, 100),
        [5, lo, hi] => {
            return ResizeWithPayer {
                account: state,
                payer,
                system_program,
                program_id,
                new_len: u16::from_le_bytes([*lo, *hi]) as usize,
            }
            .invoke()
        }
        [6] => return close_and_transfer(state, recipient),
        [7] => {
            let guard = state.try_borrow()?;
            expect(
                ResizeWithPayer {
                    account: state,
                    payer,
                    system_program,
                    program_id,
                    new_len: 32,
                }
                .invoke(),
                ProgramError::AccountBorrowFailed,
            )?;
            drop(guard);
        }
        [8] => {
            let mut selected = RefMut::map(state.try_borrow_mut()?, |bytes| &mut bytes[8..16]);
            expect(state.check_borrow(), ProgramError::AccountBorrowFailed)?;
            selected.fill(3);
        }
        [9] => {
            let rent = hopper_native::sysvar::Rent::get()?;
            ResizeWithPayer {
                account: state,
                payer,
                system_program,
                program_id,
                new_len: 32,
            }
            .invoke_signed_with_rent(&rent, &[])?;
            let funded = (state.lamports(), payer.lamports());
            expect(
                ResizeWithPayer {
                    account: state,
                    payer,
                    system_program,
                    program_id,
                    new_len: before.3 + 10_241,
                }
                .invoke_signed_with_rent(&rent, &[]),
                ProgramError::InvalidRealloc,
            )?;
            if funded != (state.lamports(), payer.lamports()) || state.data_len() != 32 {
                return Err(ProgramError::Custom(902));
            }
            return Ok(());
        }
        _ => return Err(ProgramError::InvalidInstructionData),
    }
    if before
        != (
            state.lamports(),
            recipient.lamports(),
            payer.lamports(),
            state.data_len(),
        )
    {
        return Err(ProgramError::Custom(901));
    }
    Ok(())
}
