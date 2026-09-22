//! Exercise dynamic seed inputs through each public PDA validation path.
#![cfg_attr(target_os = "solana", no_std)]

use hopper::prelude::{AccountView, Address, ProgramError, ProgramResult};
use hopper::substrate::{address::Address as NativeAddress, pda};

#[cfg(target_os = "solana")]
mod sbf {
    hopper::no_allocator!();
    hopper::nostd_panic_handler!();
    hopper::program_entrypoint!(super::process_instruction, 1);
}

pub fn process_instruction(
    program_id: &Address,
    accounts: &[AccountView],
    data: &[u8],
) -> ProgramResult {
    let expected = accounts.first().ok_or(ProgramError::NotEnoughAccountKeys)?;
    let (&api, data) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    let (&count, mut data) = data
        .split_first()
        .ok_or(ProgramError::InvalidInstructionData)?;
    let mut seeds: [&[u8]; 20] = [&[]; 20];
    let selected = seeds
        .get_mut(..count as usize)
        .ok_or(ProgramError::InvalidInstructionData)?;
    for seed in selected.iter_mut() {
        let (&len, tail) = data
            .split_first()
            .ok_or(ProgramError::InvalidInstructionData)?;
        *seed = tail
            .get(..len as usize)
            .ok_or(ProgramError::InvalidInstructionData)?;
        data = &tail[len as usize..];
    }
    let &[bump] = data else {
        return Err(ProgramError::InvalidInstructionData);
    };
    let native_id = NativeAddress::new_from_array(*program_id.as_array());
    let native_expected = NativeAddress::new_from_array(*expected.address().as_array());
    let same_address = |address: NativeAddress| {
        if address == native_expected {
            Ok(())
        } else {
            Err(ProgramError::InvalidSeeds)
        }
    };
    let same_bump = |actual| {
        if actual == bump {
            Ok(())
        } else {
            Err(ProgramError::InvalidSeeds)
        }
    };
    match api {
        0 => same_address(
            pda::create_program_address(selected, &native_id).map_err(ProgramError::from)?,
        ),
        1 => pda::verify_program_address(selected, &native_id, &native_expected)
            .map_err(ProgramError::from),
        2 => {
            let (address, actual) = pda::based_try_find_program_address(selected, &native_id)
                .map_err(ProgramError::from)?;
            same_address(address)?;
            same_bump(actual)
        }
        3 => same_bump(
            pda::find_bump_for_address(selected, &native_id, &native_expected)
                .map_err(ProgramError::from)?,
        ),
        4 => {
            hopper::hopper_runtime::pda::verify_pda_strict(expected.address(), selected, program_id)
        }
        5 => same_bump(hopper::hopper_runtime::pda::find_and_verify_pda(
            expected, selected, program_id,
        )?),
        6 | 7 => {
            let mut full = [&[][..]; 21];
            full[..selected.len()].copy_from_slice(selected);
            let bump_seed = [bump];
            full[selected.len()] = &bump_seed;
            let full = &full[..selected.len() + 1];
            if api == 6 {
                pda::verify_pda_strict(&native_expected, full, &native_id)
                    .map_err(ProgramError::from)
            } else {
                hopper::pda::verify_pda_address(full, program_id, expected.address())
            }
        }
        8 => hopper::pda::verify_pda_with_bump(expected, selected, bump, program_id),
        9 => hopper::hopper_runtime::pda::verify_pda_from_stored_bump(
            expected, selected, 0, program_id,
        ),
        10 => same_bump(hopper::pda::find_bump_for_address(
            selected,
            program_id,
            expected.address(),
        )?),
        11 => same_bump(hopper::pda::find_canonical_bump_checked(
            selected,
            program_id,
            expected.address(),
        )?),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}
