//! Compiled regression for segment guards across lifecycle and CPI boundaries.
#![cfg_attr(target_os = "solana", no_std)]
use hopper_runtime::instruction::{InstructionAccount, InstructionView};
use hopper_runtime::segment_borrow::SegmentBorrowRegistry;
use hopper_runtime::{AccountView, Address, ProgramError, ProgramResult};
#[cfg(target_os = "solana")]
hopper_runtime::program_entrypoint!(process_instruction, 3);
#[cfg(target_os = "solana")]
hopper_runtime::no_allocator!();
#[cfg(target_os = "solana")]
hopper_runtime::nostd_panic_handler!();

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
    let [state, recipient, system] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    state.require_owned_by(program_id)?;
    let before = (state.lamports(), recipient.lamports(), state.data_len());
    let mut registry = SegmentBorrowRegistry::new();
    let mut other = SegmentBorrowRegistry::new();
    match data {
        [0] => {
            let guard = state.segment_mut::<[u8; 8]>(&mut registry, 0, 8)?;
            // Check the invariant before attempting any conflicting access: the
            // vulnerable baseline fails safely here, without manufacturing UB.
            expect(state.check_borrow(), ProgramError::AccountBorrowFailed)?;
            expect(state.check_borrow_mut(), ProgramError::AccountBorrowFailed)?;
            expect(
                state.try_borrow().map(|_| ()),
                ProgramError::AccountBorrowFailed,
            )?;
            expect(
                state.segment_mut::<[u8; 8]>(&mut other, 8, 8).map(|_| ()),
                ProgramError::AccountBorrowFailed,
            )?;
            expect(
                state.close_to(recipient, program_id),
                ProgramError::AccountBorrowFailed,
            )?;
            expect(state.resize(32), ProgramError::AccountBorrowFailed)?;
            expect(state.close(), ProgramError::AccountBorrowFailed)?;
            let metas = [InstructionAccount::writable(state.address())];
            expect(
                hopper_runtime::cpi::invoke::<1>(
                    &InstructionView {
                        program_id: system.address(),
                        accounts: &metas,
                        data: &[],
                    },
                    &[state],
                ),
                ProgramError::AccountBorrowFailed,
            )?;
            drop(guard);
            state.check_borrow_mut()?;
        }
        [1] => {
            let guard = state.segment_ref::<[u8; 8]>(&mut registry, 0, 8)?;
            expect(state.check_borrow_mut(), ProgramError::AccountBorrowFailed)?;
            state.check_borrow()?;
            let second = state.segment_ref::<[u8; 8]>(&mut other, 0, 8)?;
            expect(
                state.close_to(recipient, program_id),
                ProgramError::AccountBorrowFailed,
            )?;
            let metas = [InstructionAccount::writable(state.address())];
            expect(
                hopper_runtime::cpi::invoke::<1>(
                    &InstructionView {
                        program_id: system.address(),
                        accounts: &metas,
                        data: &[],
                    },
                    &[state],
                ),
                ProgramError::AccountBorrowFailed,
            )?;
            drop((guard, second));
            state.check_borrow_mut()?;
        }
        [2] => {
            let mut parts =
                state.split_segments_mut::<[u8; 8], 2>(&mut registry, [(0, 8), (8, 8)])?;
            expect(state.check_borrow(), ProgramError::AccountBorrowFailed)?;
            let [a, b] = parts.all_mut();
            *a = [7; 8];
            *b = [9; 8];
            drop(parts);
            state.check_borrow_mut()?;
        }
        [3] => {
            let guard = state.try_borrow()?;
            expect(
                state.close_to(recipient, program_id),
                ProgramError::AccountBorrowFailed,
            )?;
            drop(guard);
        }
        [4] => expect(
            state.close_to(state, program_id),
            ProgramError::InvalidArgument,
        )?,
        [5] => return state.close_to(recipient, program_id),
        [6] => {
            use hopper_runtime::write_policy::{
                try_install_ambient_gate_with_args, WritePolicy, WriteRange,
            };
            static POLICY: WritePolicy =
                WritePolicy::with_lamports(&[WriteRange::whole_account(0)], &[1]);
            let _gate = try_install_ambient_gate_with_args(accounts, &POLICY, &[])?;
            expect(
                state.close_to(recipient, program_id),
                ProgramError::Custom(0xD000),
            )?;
        }
        _ => return Err(ProgramError::InvalidInstructionData),
    }
    if before != (state.lamports(), recipient.lamports(), state.data_len()) {
        return Err(ProgramError::Custom(901));
    }
    Ok(())
}
