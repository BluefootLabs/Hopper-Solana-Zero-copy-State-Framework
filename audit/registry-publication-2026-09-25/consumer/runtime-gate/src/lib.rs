//! Compiled-SBF regression fixture for the ambient gate's public surfaces.
#![cfg_attr(target_os = "solana", no_std)]

use hopper::hopper_runtime::segment_borrow::SegmentBorrowRegistry;
use hopper::hopper_runtime::write_policy::{
    try_install_ambient_gate_with_args as install, ParametricWriteRange, WritePolicy, WriteRange,
    LAMPORT_GATE_DEPTH_EXCEEDED,
};
use hopper::prelude::*;

/// Exactly 32 bytes including the Hopper header, matching the raw fixture.
#[derive(Clone, Copy)]
#[repr(C)]
#[hopper::state(disc = 82, version = 1)]
pub struct CellProbe {
    pub protected: [u8; 8],
    pub values: [u8; 8],
}

#[derive(Accounts)]
#[accounts(strict_writes, lamports())]
#[instruction(slot: u8)]
pub struct SelectedCell<'info> {
    #[account(cells(slot; values))]
    pub state: Account<'info, CellProbe>,
}

#[cfg(target_os = "solana")]
mod entry {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
    hopper::program_entrypoint!(super::process_instruction, 2);
}

static NARROW: WritePolicy = WritePolicy::new(&[WriteRange::new(0, 8, 8)]);
static WHOLE: WritePolicy = WritePolicy::with_lamports(&[WriteRange::whole_account(0)], &[0]);
static DENY: WritePolicy = WritePolicy::with_lamports(&[], &[]);
static CELL: WritePolicy = WritePolicy::with_parametric(
    &[WriteRange::new(0, 0, 32)],
    &[ParametricWriteRange::new(
        0, 0, 8, 8, 4, 0, "cell", "values",
    )],
);

fn write_cell(account: &AccountView, offset: u32) -> ProgramResult {
    let mut registry = SegmentBorrowRegistry::new();
    let mut cell = account.segment_mut::<[u8; 8]>(&mut registry, offset, 8)?;
    *cell = [7; 8];
    Ok(())
}

fn expect_error(result: ProgramResult, code: u32) -> ProgramResult {
    if result != Err(ProgramError::Custom(code)) {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

pub fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let [state, foreign] = accounts else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };
    if !state.owned_by(program_id) || !foreign.owned_by(program_id) {
        return Err(ProgramError::IncorrectProgramId);
    }
    let governed = &accounts[..1];
    match data {
        [0] => {
            state.try_borrow_mut()?.fill(7);
            Ok(())
        }
        [1] | [2] => {
            let _guard = install(governed, &NARROW, &[])?;
            write_cell(state, if data[0] == 1 { 8 } else { 9 })
        }
        [3] => {
            let _guard = install(governed, &WHOLE, &[])?;
            foreign.try_borrow_mut()?.fill(7);
            Ok(())
        }
        [4] => {
            let outer = install(governed, &WHOLE, &[])?;
            {
                let _inner = install(governed, &DENY, &[])?;
                if install(governed, &WHOLE, &[]).map(|_| ()) != Err(LAMPORT_GATE_DEPTH_EXCEEDED) {
                    return Err(ProgramError::InvalidAccountData);
                }
                expect_error(state.try_borrow_mut().map(|_| ()), 0xD000)?;
            }
            write_cell(state, 8)?; // dropping the inner guard resumes outer
            let inner = install(governed, &DENY, &[])?;
            drop(outer); // dropping out of order must leave inner installed
            expect_error(state.try_borrow_mut().map(|_| ()), 0xD000)?;
            drop(inner);
            foreign.try_borrow_mut()?.fill(7); // last drop clears dispatch
            Ok(())
        }
        [5] | [6] => {
            let _guard = install(governed, &CELL, &[2])?;
            write_cell(state, if data[0] == 5 { 16 } else { 8 })
        }
        [7] => {
            core::mem::forget(install(governed, &NARROW, &[])?);
            state.try_borrow_mut().map(|_| ())
        }
        [8] => {
            let _guard = install(governed, &WHOLE, &[])?;
            // Same value avoids a balance mismatch if the gate regresses.
            foreign.try_set_lamports(foreign.lamports())
        }
        [9] => {
            let _guard = install(governed, &NARROW, &[])?;
            foreign.close()
        }
        [case @ 10..=15] => {
            let mut raw = Context::new(program_id, accounts, data);
            let mut ctx = SelectedCell::bind_with_args(&mut raw, if *case == 15 { 8 } else { 2 })?;
            match case {
                10 | 15 => {
                    *ctx.state_values_cell_mut()? = 7;
                    Ok(())
                }
                11 => ctx
                    .raw()
                    .segment_mut::<u8>(0, CellProbe::VALUES_ABS_OFFSET + 3)
                    .map(|_| ()),
                12 => ctx
                    .raw()
                    .segment_mut::<u8>(0, CellProbe::PROTECTED_ABS_OFFSET)
                    .map(|_| ()),
                13 => ctx.state_values_mut().map(|_| ()),
                14 => state.try_set_lamports(state.lamports()),
                _ => unreachable!(),
            }
        }
        [16] => {
            let mut bytes = state.try_borrow_mut()?;
            if bytes.len() != CellProbe::LEN || bytes.iter().any(|byte| *byte != 0) {
                return Err(ProgramError::AccountAlreadyInitialized);
            }
            hopper::layout::write_header(
                &mut bytes,
                CellProbe::DISC,
                CellProbe::VERSION,
                &CellProbe::LAYOUT_ID,
            )
        }
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
